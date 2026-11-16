//! Tests of the unsafe-code gate: where unsafe code and inline assembly may be written, and how
//! test modules are declared.

use crate::UNSAFE_FILES;
use crate::tests::{
    ONE_UNSAFE_OPERATION, audited, doc_tests_forbid, failed, found, lifting, reports, unsafe_gate,
};

#[test]
fn confines_unsafe_blocks_to_the_audited_files() {
    let forbid = format!("#![forbid({}_{})]\n", "unsafe", "code");
    let deny = format!("#[deny({}_{})]\nmod m;\n", "unsafe", "code");
    let mut sources: Vec<(String, String)> = UNSAFE_FILES
        .iter()
        .map(|path| ((*path).to_owned(), audited(path)))
        .collect();
    sources.push(("crates/a/src/lib.rs".to_owned(), forbid.clone()));
    sources.push(("crates/b/src/lib.rs".to_owned(), deny));
    assert_eq!(
        unsafe_gate(&sources),
        Ok(()),
        "only the audited files lift it"
    );

    let mut extra = sources.clone();
    extra.push(("crates/c/src/x.rs".to_owned(), lifting()));
    extra.push((
        "crates/d/src/y.rs".to_owned(),
        format!("#[allow({}_{})]", "unsafe", "code"),
    ));
    assert_eq!(
        unsafe_gate(&extra),
        found(
            "the lint against unsafe code",
            &[
                "crates/c/src/x.rs lifts the lint against unsafe code".to_owned(),
                "crates/d/src/y.rs lifts the lint against unsafe code".to_owned(),
            ]
        ),
        "another file lifting it"
    );
    assert_eq!(UNSAFE_FILES.len(), 5, "five audited files");
    let mut fifth = sources.clone();
    fifth.push(("crates/fpenv/src/other.rs".to_owned(), lifting()));
    assert_eq!(
        unsafe_gate(&fifth),
        found(
            "the lint against unsafe code",
            &[
                "crates/fpenv/src/other.rs lifts the lint against unsafe code".to_owned(),
                format!(
                    "crates/fpenv/src/other.rs does not hold `{}`",
                    forbid.trim()
                ),
            ]
        ),
        "a fifth file, even in the crate of an audited one"
    );
    let mut no_assembly = sources.clone();
    for (path, text) in &mut no_assembly {
        if crate::ASSEMBLY_FILES.contains(&path.as_str()) {
            *text = format!(
                "{}{ONE_UNSAFE_OPERATION}\n// unsafe {{ ::core::arch::asm!(\"nop\") }} in a comment\nconst A: &str = \"unsafe {{ ::core::arch::asm!() }}\";\n",
                lifting()
            );
        }
    }
    assert_eq!(
        unsafe_gate(&no_assembly),
        failed(
            "the lint against unsafe code: crates/fpenv/src/word.rs is listed in \
             ASSEMBLY_FILES but holds no inline assembly"
        ),
        "an assembly file without assembly"
    );
    for (code, line) in [
        ("fn f(p: *const u8) -> u8 {\n    unsafe { *p }\n}\n", 2),
        ("unsafe fn f() {\n    core::arch::asm!(\"nop\");\n}\n", 1),
        ("unsafe impl Send for S {}\n", 1),
        ("fn f() {\n    unsafe { core::ptr::read(p) }\n}\n", 2),
    ] {
        let mut other = sources.clone();
        for (path, text) in &mut other {
            if crate::ASSEMBLY_FILES.contains(&path.as_str()) {
                *text = format!("{}{code}", audited(path));
            }
        }
        let offset = audited(crate::ASSEMBLY_FILES[0]).lines().count();
        assert_eq!(
            unsafe_gate(&other),
            failed(format!(
                "the lint against unsafe code: {}:{}: `unsafe` not opening `{{ \
                 ::core::arch::asm!(`: an assembly file holds no other unsafe code, which Miri \
                 would not check",
                crate::ASSEMBLY_FILES[0],
                offset + line
            )),
            "other unsafe code beside the assembly: {code}"
        );
    }
    let mut stale = sources;
    stale[0].1 = forbid;
    assert_eq!(
        unsafe_gate(&stale),
        failed(format!(
            "the lint against unsafe code: {} is listed in UNSAFE_FILES but no longer lifts the lint",
            UNSAFE_FILES[0]
        )),
        "a stale entry"
    );
}

#[test]
fn refuses_an_assembly_file_whose_unsafe_block_could_run_more_than_its_assembly() {
    let assembly = crate::ASSEMBLY_FILES[0];
    let sources: Vec<(String, String)> = UNSAFE_FILES
        .iter()
        .map(|path| ((*path).to_owned(), audited(path)))
        .collect();
    assert_eq!(unsafe_gate(&sources), Ok(()), "the canonical file");
    let gate = |path: &str, text: &str| {
        let mut all = sources.clone();
        match all.iter_mut().find(|(file, _)| file == path) {
            Some((_, old)) => *old = text.to_owned(),
            None => all.push((path.to_owned(), text.to_owned())),
        }
        unsafe_gate(&all)
    };
    let refused = |problem: String| failed(format!("the lint against unsafe code: {problem}"));
    let canonical = audited(assembly);
    let lines = canonical.lines().count();
    assert_eq!(
        gate(assembly, &canonical.replace(ONE_UNSAFE_OPERATION, "")),
        refused(format!(
            "{assembly} does not hold `{ONE_UNSAFE_OPERATION}`: an unsafe block of it could \
             hold more than its assembly call"
        )),
        "the lint that keeps one operation per block, lifted by an `expect` unseen"
    );
    let unsafe_line = lines - 3;
    for shadowable in [
        "core::arch::asm!(\"nop\");",
        "self::core::arch::asm!(\"nop\");",
        "::core::arch::asm(\"nop\");",
        "::core::arch::asm!{\"nop\"};",
    ] {
        assert_eq!(
            gate(
                assembly,
                &canonical.replace("::core::arch::asm!(\"nop\");", shadowable)
            ),
            refused(format!(
                "{assembly}:{unsafe_line}: `unsafe` not opening `{{ ::core::arch::asm!(`: an \
                 assembly file holds no other unsafe code, which Miri would not check"
            )),
            "{shadowable}"
        );
    }
    assert_eq!(
        gate(
            assembly,
            &canonical.replace(
                "unsafe {\n        ::core::arch::asm!(",
                "unsafe /* one call */ {\n        :: core :: arch :: asm ! ("
            )
        ),
        Ok(()),
        "the same tokens, comments and spaces apart"
    );
    let hidden = format!(
        "{canonical}/* /* */ \"*/\nfn g(p: *const u8) -> u8 {{ unsafe {{ core::ptr::read(p) }} }}\nmod m {{}}\n// \"\n"
    );
    assert_eq!(
        gate(assembly, &hidden),
        found(
            "the lint against unsafe code",
            &[
                format!(
                    "{assembly}:{}: `unsafe` not opening `{{ ::core::arch::asm!(`: an assembly \
                     file holds no other unsafe code, which Miri would not check",
                    lines + 2
                ),
                format!(
                    "{assembly}:{}: names `mod`: an assembly file declares no module, import, \
                     crate or macro, through which its paths could resolve elsewhere",
                    lines + 3
                ),
            ]
        ),
        "code a nested comment does not hide"
    );
    for (code, word) in [
        ("mod core {}\n", "mod"),
        ("use crate::m as m;\n", "use"),
        ("extern crate alloc;\n", "extern"),
        ("macro_rules! m {\n    () => {};\n}\n", "macro_rules"),
    ] {
        assert_eq!(
            gate(assembly, &format!("{canonical}{code}")),
            refused(format!(
                "{assembly}:{}: names `{word}`: an assembly file declares no module, import, \
                 crate or macro, through which its paths could resolve elsewhere",
                lines + 1
            )),
            "{code}"
        );
    }
}

#[test]
fn refuses_a_file_that_rebinds_core_in_the_crate_of_an_assembly_file() {
    let assembly = crate::ASSEMBLY_FILES[0];
    let sources: Vec<(String, String)> = UNSAFE_FILES
        .iter()
        .map(|path| ((*path).to_owned(), audited(path)))
        .collect();
    let gate = |path: &str, text: &str| {
        let mut all = sources.clone();
        all.push((path.to_owned(), text.to_owned()));
        unsafe_gate(&all)
    };
    let refused = |problem: String| failed(format!("the lint against unsafe code: {problem}"));
    let forbid = format!("#![forbid({}_{})]\n", "unsafe", "code");
    for (path, code) in [
        ("crates/fpenv/src/lib.rs", "extern crate shadow as core;\n"),
        ("crates/fpenv/src/lib.rs", "use crate::m as core;\n"),
        ("crates/fpenv/src/other.rs", "mod core {}\n"),
        ("crates/fpenv/src/other.rs", "mod r#core {}\n"),
    ] {
        let text = if path.ends_with("lib.rs") {
            format!("//! The crate.\n{}{code}", doc_tests_forbid())
        } else {
            format!("{forbid}{code}")
        };
        assert_eq!(
            gate(path, &text),
            refused(format!(
                "{path}:{}: rebinds `core` in the crate of the assembly file {assembly}, \
                 whose `::core::arch::asm!` must name the real one",
                text.lines().count()
            )),
            "{path}: {code}"
        );
    }
    assert_eq!(
        gate(
            "crates/fpenv/src/other.rs",
            &format!(
                "{forbid}//! as core, mod core\nconst A: &str = \"as core\";\nmod coreless {{}}\n"
            )
        ),
        Ok(()),
        "in prose and strings, and another name"
    );
    assert_eq!(
        gate(
            "crates/audio-core/src/lib.rs",
            &format!("{forbid}{}use crate::m as core;\n", doc_tests_forbid())
        ),
        Ok(()),
        "another crate"
    );
}

#[test]
fn refuses_unsafe_operations_another_file_could_bring_under_the_assembly_file() {
    let forbid = format!("#![forbid({}_{})]\n", "unsafe", "code");
    let sources: Vec<(String, String)> = UNSAFE_FILES
        .iter()
        .map(|path| ((*path).to_owned(), audited(path)))
        .collect();
    let with = |path: &str, text: &str| {
        let mut all = sources.clone();
        all.push((path.to_owned(), text.to_owned()));
        unsafe_gate(&all)
    };
    let word = "unsafe";
    let peek = format!(
        "//! The crate.\n{}macro_rules! peek {{\n    ($p:expr) => {{\n        // SAFETY: read.\n        {word} {{ core::ptr::read($p) }}\n    }};\n}}\n",
        doc_tests_forbid()
    );
    let assembly = crate::ASSEMBLY_FILES[0];
    let refused = |path: &str, line: usize| {
        failed(format!(
            "the lint against unsafe code: {path}:{line}: `unsafe` in the crate of the assembly \
             file {assembly}: its code could run under that file's lifting, where Miri never \
             checks it"
        ))
    };
    assert_eq!(
        with("crates/fpenv/src/lib.rs", &peek),
        refused("crates/fpenv/src/lib.rs", 6),
        "a macro of the library root, used in the assembly file"
    );
    assert_eq!(
        with(
            "crates/fpenv/src/lib_tests.rs",
            &format!("{forbid}{word} fn f() {{}}\n")
        ),
        refused("crates/fpenv/src/lib_tests.rs", 2),
        "an unsafe function of another module"
    );
    assert_eq!(
        with(
            "crates/fpenv/src/lib.rs",
            &format!(
                "//! Not {word} but the assembly.\n{}const A: &str = \"{word}\";\n",
                doc_tests_forbid()
            )
        ),
        Ok(()),
        "the word in prose and strings"
    );
}

#[test]
fn refuses_assembly_outside_an_unsafe_block_everywhere() {
    let sources: Vec<(String, String)> = UNSAFE_FILES
        .iter()
        .map(|path| ((*path).to_owned(), audited(path)))
        .collect();
    let forbid = format!("#![forbid({}_{})]\n", "unsafe", "code");
    for (path, text, line) in [
        (
            crate::ASSEMBLY_FILES[0].to_owned(),
            format!(
                "{}core::arch::global_asm!(\"nop\");\n",
                audited(crate::ASSEMBLY_FILES[0])
            ),
            11_usize,
        ),
        (
            "bindings/audio-wasm/src/ffi.rs".to_owned(),
            format!("{}::core::arch::naked_asm!(\"ret\");\n", lifting()),
            5,
        ),
        (
            "crates/audio-core/src/render.rs".to_owned(),
            format!("{forbid}macro_rules! g {{\n    () => {{ global_asm!(\"\") }};\n}}\n"),
            3,
        ),
    ] {
        let mut all = sources.clone();
        all.retain(|(held, _)| *held != path);
        all.push((path.clone(), text.clone()));
        let word = if text.contains("naked") {
            "naked_asm"
        } else {
            "global_asm"
        };
        let found = unsafe_gate(&all);
        assert!(
            reports(
                &found,
                &format!(
                    "{path}:{line}: names `{word}`: assembly outside an `unsafe` block (a global \
                     symbol, a naked function) is written nowhere, since neither the one-operation \
                     rule nor Miri sees it"
                )
            ),
            "{text}: {found:?}"
        );
    }
}

#[test]
fn requires_every_library_root_to_forbid_unsafe_code_in_its_doc_tests() {
    let forbid = format!("#![forbid({}_{})]\n", "unsafe", "code");
    let doc = format!("#![doc(test(attr(forbid({}_{}))))]\n", "unsafe", "code");
    let mut sources: Vec<(String, String)> = UNSAFE_FILES
        .iter()
        .map(|path| ((*path).to_owned(), audited(path)))
        .collect();
    let root = "crates/audio-core/src/lib.rs";
    let gate = |text: String| {
        let mut all = sources.clone();
        all.push((root.to_owned(), text));
        unsafe_gate(&all)
    };
    let refused = |path: &str| {
        format!(
            "{path} does not hold `{}`: its doc tests, each a crate of its own, would not \
             forbid unsafe code",
            doc.trim()
        )
    };
    assert_eq!(
        gate(format!("//! The crate.\n{forbid}{doc}")),
        Ok(()),
        "a library root forbidding unsafe code in its doc tests"
    );
    for text in [
        forbid.clone(),
        format!("{forbid}/* {doc} */"),
        format!("{forbid}fn f() {{}}\n{doc}"),
    ] {
        assert!(reports(&gate(text.clone()), &refused(root)), "{text}");
    }
    sources.retain(|(path, _)| path != "crates/alloc-counter/src/lib.rs");
    sources.push(("crates/alloc-counter/src/lib.rs".to_owned(), lifting()));
    sources.push(("crates/audio-core/src/plan.rs".to_owned(), forbid.clone()));
    sources.push(("crates/audio-cli/src/main.rs".to_owned(), forbid));
    assert_eq!(
        unsafe_gate(&sources),
        failed(format!(
            "the lint against unsafe code: {}",
            refused("crates/alloc-counter/src/lib.rs")
        )),
        "the root of an unsafe crate too; a module and a binary's root need not"
    );
}

#[test]
fn refuses_unsafe_outside_the_unsafe_files_and_their_declared_tests() {
    let forbid = format!("#![forbid({}_{})]\n", "unsafe", "code");
    let word = "unsafe";
    let declaration = "#[cfg(test)]\n#[path = \"ffi_tests.rs\"]\nmod tests;\n";
    let sources: Vec<(String, String)> = UNSAFE_FILES
        .iter()
        .map(|path| {
            let text = if *path == "bindings/audio-wasm/src/ffi.rs" {
                format!(
                    "{}{declaration}fn f(p: *const u8) -> u8 {{ {word} {{ *p }} }}\n",
                    audited(path)
                )
            } else {
                format!(
                    "{}fn f(p: *const u8) -> u8 {{ {word} {{ *p }} }}\n",
                    audited(path)
                )
            };
            ((*path).to_owned(), text)
        })
        .filter(|(path, _)| !crate::ASSEMBLY_FILES.contains(&path.as_str()))
        .chain(
            UNSAFE_FILES
                .iter()
                .filter(|path| crate::ASSEMBLY_FILES.contains(path))
                .map(|path| ((*path).to_owned(), audited(path))),
        )
        .chain([(
            "bindings/audio-wasm/src/ffi_tests.rs".to_owned(),
            format!("fn t(p: *const u8) -> u8 {{ {word} {{ *p }} }}\n"),
        )])
        .collect();
    let with = |path: &str, text: &str| {
        let mut all = sources.clone();
        all.push((path.to_owned(), text.to_owned()));
        unsafe_gate(&all)
    };
    assert_eq!(
        unsafe_gate(&sources),
        Ok(()),
        "the unsafe files and the unit tests they declare"
    );
    let problem = |path: &str, line: usize| {
        format!(
            "{path}:{line}: names `unsafe` outside UNSAFE_FILES and the unit tests they declare: \
             rustc does not lint code expanded from another crate's macro, so unsafe code is \
             written only where Miri and the audit see it"
        )
    };
    let peek = format!(
        "{forbid}#[macro_export]\nmacro_rules! peek {{\n    ($p:expr) => {{ {word} {{ ::core::ptr::read_volatile($p) }} }};\n}}\n"
    );
    for (path, text, line) in [
        ("crates/audio-hash/src/lib.rs", peek, 4_usize),
        (
            "crates/audio-core/src/lib.rs",
            format!("{forbid}fn f(p: *const u8) -> u8 {{\n    {word} {{ *p }}\n}}\n"),
            3,
        ),
        (
            "crates/audio-core/src/render.rs",
            format!("{forbid}/* /* */ \"*/ {word} fn f() {{}} // \"\n"),
            2,
        ),
        (
            "bindings/audio-wasm/src/other_tests.rs",
            format!("{forbid}fn t(p: *const u8) -> u8 {{ {word} {{ *p }} }}\n"),
            2,
        ),
        (
            "crates/xtask/tests/fixtures/workspace/src/lib.rs",
            format!("{word} impl Send for S {{}}\n"),
            1,
        ),
    ] {
        let found = with(path, &text);
        assert!(
            reports(&found, &problem(path, line)),
            "{path}: {text}: {found:?}"
        );
    }
    assert_eq!(
        with(
            "crates/audio-core/src/lib.rs",
            &format!(
                "{forbid}{}//! Not {word} code.\nconst A: &str = \"{word}\"; // {word}\n",
                doc_tests_forbid()
            )
        ),
        Ok(()),
        "the word in prose and strings"
    );
}

#[test]
fn makes_the_compiler_refuse_unsafe_outside_the_audited_files() {
    let lint = format!("{}_{}", "unsafe", "code");
    let forbid = format!("#![forbid({lint})]\n");
    let mut sources: Vec<(String, String)> = UNSAFE_FILES
        .iter()
        .map(|path| ((*path).to_owned(), audited(path)))
        .collect();
    for path in [
        "crates/audio-core/src/lib.rs",
        "crates/audio-cli/src/main.rs",
        "crates/audio-cli/tests/binary.rs",
        "crates/audio-cli/build.rs",
        "crates/audio-cli/src/bin/other.rs",
        "bindings/audio-wasm/src/engine.rs",
        "bindings/audio-wasm/tests/web.rs",
    ] {
        let text = if path.ends_with("/src/lib.rs") {
            format!("{forbid}{}", doc_tests_forbid())
        } else {
            forbid.clone()
        };
        sources.push((path.to_owned(), text));
    }
    for path in [
        "crates/audio-core/src/plan.rs",
        "crates/audio-cli/tests/data/fixture.rs",
        "bindings/audio-wasm/src/lib.rs",
        "README.rs",
    ] {
        let text = if path.ends_with("/src/lib.rs") {
            format!("{}fn plain() {{}}\n", doc_tests_forbid())
        } else {
            "fn plain() {}\n".to_owned()
        };
        sources.push((path.to_owned(), text));
    }
    assert_eq!(
        unsafe_gate(&sources),
        Ok(()),
        "every root forbids, modules inherit, the unsafe crate's root and files apart"
    );
    let mut missing = sources.clone();
    for (path, _) in [
        ("crates/audio-core/src/lib.rs", ""),
        ("crates/audio-cli/tests/binary.rs", ""),
        ("bindings/audio-wasm/src/engine.rs", ""),
    ] {
        missing.retain(|(source, _)| source != path);
        missing.push((
            path.to_owned(),
            format!("{}fn plain() {{}}\n", doc_tests_forbid()),
        ));
    }
    assert_eq!(
        unsafe_gate(&missing),
        found(
            "the lint against unsafe code",
            &[
                format!(
                    "crates/audio-core/src/lib.rs does not hold `{}`",
                    forbid.trim()
                ),
                format!(
                    "crates/audio-cli/tests/binary.rs does not hold `{}`",
                    forbid.trim()
                ),
                format!(
                    "bindings/audio-wasm/src/engine.rs does not hold `{}`",
                    forbid.trim()
                ),
            ]
        ),
        "a library root, a test root, a module of the unsafe crate"
    );
    for text in [
        format!("#[deny({lint})] #[expect({lint}, reason = \"r\")] mod m;\n"),
        format!("#[expect({lint}, reason = \"r\")] // deny({lint})\n"),
    ] {
        let mut probe = sources.clone();
        probe.push(("crates/audio-core/src/m.rs".to_owned(), text.clone()));
        assert_eq!(
            unsafe_gate(&probe),
            found(
                "the lint against unsafe code",
                &["crates/audio-core/src/m.rs lifts the lint against unsafe code".to_owned()]
            ),
            "only the exact attribute is exempt: {text}"
        );
    }
}

#[test]
fn lets_the_test_modules_of_an_unsafe_file_inherit_its_lifting() {
    let forbid = format!("#![forbid({}_{})]\n", "unsafe", "code");
    let declaration = "#[cfg(test)]\n#[path = \"ffi_tests.rs\"]\nmod tests;\n";
    let mut sources: Vec<(String, String)> = UNSAFE_FILES
        .iter()
        .map(|path| {
            let text = if *path == "bindings/audio-wasm/src/ffi.rs" {
                format!("{}{declaration}", audited(path))
            } else {
                audited(path)
            };
            ((*path).to_owned(), text)
        })
        .collect();
    sources.push((
        "bindings/audio-wasm/src/ffi_tests.rs".to_owned(),
        "fn pointer() {}\n".to_owned(),
    ));
    sources.push((
        "bindings/audio-wasm/src/engine.rs".to_owned(),
        format!("{forbid}{declaration}").replace("ffi_tests", "engine_tests"),
    ));
    sources.push((
        "bindings/audio-wasm/src/engine_tests.rs".to_owned(),
        forbid.clone(),
    ));
    assert_eq!(
        unsafe_gate(&sources),
        Ok(()),
        "the test module of the FFI file is under its lifting"
    );
    let mut safe_tests = sources.clone();
    for (path, text) in &mut safe_tests {
        if path == "bindings/audio-wasm/src/engine_tests.rs" {
            *text = "fn plain() {}\n".to_owned();
        }
    }
    assert_eq!(
        unsafe_gate(&safe_tests),
        failed(format!(
            "the lint against unsafe code: bindings/audio-wasm/src/engine_tests.rs does not hold `{}`",
            forbid.trim()
        )),
        "the test module of a safe file forbids it like any module"
    );
    let mut undeclared = sources;
    for (path, text) in &mut undeclared {
        if path == "bindings/audio-wasm/src/ffi.rs" {
            *text = audited(path);
        }
    }
    assert_eq!(
        unsafe_gate(&undeclared),
        failed(format!(
            "the lint against unsafe code: bindings/audio-wasm/src/ffi_tests.rs does not hold `{}`",
            forbid.trim()
        )),
        "a file the unsafe file does not declare is not its test module"
    );
}

#[test]
fn reads_only_the_one_form_of_a_test_declaration() {
    let declared = |path: &str, text: &str| -> Vec<(usize, String)> {
        let source = crate::RustSource::parse(path, text).expect("lexes");
        crate::test_declarations(&source)
            .into_iter()
            .map(|declaration| (declaration.line, declaration.file))
            .collect()
    };
    assert_eq!(
        declared(
            "crates/a/src/x.rs",
            "fn f() {}\n#[cfg(test)]\n#[path = \"x_tests.rs\"]\nmod tests;\n  #[cfg(test)]\n  #[path = \"y_2_tests.rs\"]\n  mod other_1;\n"
        ),
        vec![
            (2, "crates/a/src/x_tests.rs".to_owned()),
            (5, "crates/a/src/y_2_tests.rs".to_owned()),
        ],
        "every declaration, indented or not, beside its file"
    );
    assert_eq!(
        declared("x.rs", "#[cfg(test)]\n#[path = \"x_tests.rs\"]\nmod tests;"),
        vec![(1, "x_tests.rs".to_owned())],
        "a file at the root"
    );
    assert_eq!(
        declared(
            "x.rs",
            "#[cfg(test)] #[path=\"x_tests.rs\"]\n\n  mod /* the tests */ tests ;"
        ),
        vec![(1, "x_tests.rs".to_owned())],
        "the same tokens however they are laid out, as rustc reads them"
    );
    for text in [
        "#[path = \"x_tests.rs\"]\nmod tests;\n",
        "#[cfg(not(test))]\n#[path = \"x_tests.rs\"]\nmod tests;\n",
        "#[cfg(test)]\n#[path = \"../x_tests.rs\"]\nmod tests;\n",
        "#[cfg(test)]\n#[path = \"d/x_tests.rs\"]\nmod tests;\n",
        "#[cfg(test)]\n#[path = \"_tests.rs\"]\nmod tests;\n",
        "#[cfg(test)]\n#[path = \"x.rs\"]\nmod tests;\n",
        "/*\n#[cfg(test)]\n#[path = \"x_tests.rs\"]\nmod tests;\n*/\n",
        "/* /* */\n#[cfg(test)]\n#[path = \"x_tests.rs\"]\nmod tests;\n*/\n",
        "const S: &str = \"\n#[cfg(test)]\n#[path = \\\"x_tests.rs\\\"]\nmod tests;\n\";\n",
        "#[cfg(test)]\n#[path = r\"x_tests.rs\"]\nmod tests;\n",
        "#[cfg(test)]\n#[path = \"x\\x5ftests.rs\"]\nmod tests;\n",
        "#[cfg(test)]\n#[path = \"x_tests.rs\"]\nmod tests {}\n",
        "#[cfg(test)]\n#[path = \"x_tests.rs\"]\nmod ;\n",
        "#[cfg(test)]\n#[path = \"x_tests.rs\"]\nmod a::b;\n",
        "#[cfg(test)]\n#[path = \"x_tests.rs\"]\npub mod tests;\n",
        "#[cfg(test)]\n#[path = \"x_tests.rs\"]\nmod tests;;\n",
        "#[cfg(test)]\n#[path = \"x_tests.rs\"]\n",
    ] {
        assert_eq!(
            declared("crates/a/src/x.rs", text),
            vec![],
            "not the form: {text}"
        );
    }
}
