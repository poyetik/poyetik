//! The unsafe-code gate: where unsafe code and inline assembly may be written, and how test modules are declared.

use alloc::collections::BTreeSet;

use crate::{
    rust_source::{RustSource, TokenKind, tokens_of},
    steps::{Failure, verdict},
    test_code::in_crate,
};

/// The only files allowed to lift the workspace lint against unsafe code, each audited.
///
/// The C interfaces of the WebAssembly modules, the counting allocator of the tests, and the
/// floating-point control register of native engine calls.
pub const UNSAFE_FILES: [&str; 5] = [
    "bindings/audio-wasm/src/ffi.rs",
    "bindings/visual-wasm/src/ffi.rs",
    "bindings/web-page/src/ffi.rs",
    "crates/alloc-counter/src/lib.rs",
    "crates/fpenv/src/word.rs",
];

/// The files of [`UNSAFE_FILES`] made of inline assembly, which Miri cannot run.
///
/// Their crates leave the Miri step ([`unsafe_crates`](crate::steps::unsafe_crates)), and are proven by their own tests
/// on each architecture they support, and by the audit written in the file.
pub const ASSEMBLY_FILES: [&str; 1] = ["crates/fpenv/src/word.rs"];

/// Whether `relative` (the path of a Rust file inside a crate's directory) is the root of a
/// crate target: the library, a binary, a test, a benchmark, an example or a build script.
pub(crate) fn crate_root(relative: &str) -> bool {
    let single = |folder: &str| {
        relative
            .strip_prefix(folder)
            .is_some_and(|file| !file.contains('/'))
    };
    ["src/lib.rs", "src/main.rs", "build.rs"].contains(&relative)
        || ["tests/", "benches/", "examples/", "src/bin/"]
            .into_iter()
            .any(single)
}

/// The one form that declares a unit test module ([`test_declarations`]), as the gates
/// say it: any identifier names the module.
pub const TEST_DECLARATION_FORM: &str =
    "`#[cfg(test)] #[path = \"<name>_tests.rs\"] mod <module>;`";

/// The one form that declares a unit test module ([`test_declarations`]), as written in
/// Rust: any name may replace `x` (in the literal) and `tests` (the module).
pub(crate) const TEST_DECLARATION_TOKENS: &str = "#[cfg(test)] #[path = \"x_tests.rs\"] mod tests;";

/// A declaration of a unit test file in a Rust source ([`test_declarations`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestDeclaration {
    /// The line it starts on.
    pub line: usize,
    /// The file it declares, relative to the root.
    pub file: String,
    /// The range of its tokens among its source's.
    pub tokens: core::ops::Range<usize>,
}

/// The unit test files that `source` declares, each with the line its declaration starts
/// on and the range of its tokens: the one form ([`TEST_DECLARATION_FORM`]), in tokens,
///
/// ```text
/// #[cfg(test)]
/// #[path = "<name>_tests.rs"]
/// mod <identifier>;
/// ```
///
/// where `<name>` is not empty and holds only letters, digits and `_`, written in a plain
/// string literal (no escape, not raw), so that the file is beside `source`
/// (`<directory of source>/<name>_tests.rs`). Comments, strings and spaces apart, nothing
/// else may come between its tokens; their layout is free, as it is to rustc.
#[must_use]
pub fn test_declarations(source: &RustSource) -> Vec<TestDeclaration> {
    let form = tokens_of(TEST_DECLARATION_TOKENS);
    // The module's name, the token before the final `;`: any identifier.
    let module = form.len().saturating_sub(2);
    let dir = source.path.rsplit_once('/').map_or("", |(dir, _)| dir);
    source
        .tokens
        .windows(form.len())
        .enumerate()
        .filter_map(|(at, window)| {
            let shaped = window
                .iter()
                .zip(&form)
                .enumerate()
                .all(|(index, (held, wanted))| {
                    held.kind == wanted.kind
                        && (held.text == wanted.text
                            || held.kind == TokenKind::Literal
                            || index == module)
                });
            let name = window
                .iter()
                .find(|token| token.kind == TokenKind::Literal)?
                .text
                .strip_prefix('"')?
                .strip_suffix("_tests.rs\"")?;
            let word = !name.is_empty()
                && name
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_');
            (shaped && word).then(|| {
                let file = if dir.is_empty() {
                    format!("{name}_tests.rs")
                } else {
                    format!("{dir}/{name}_tests.rs")
                };
                let line = window.first().map_or(0, |first| first.line);
                TestDeclaration {
                    line,
                    file,
                    tokens: at..at.saturating_add(form.len()),
                }
            })
        })
        .collect()
}

/// Checks that unsafe code is written only in [`UNSAFE_FILES`] and the unit tests they
/// declare: by the compiler where it lints, and by the keyword where it does not.
///
/// Only those files lift the lint against unsafe code (an `allow` or `expect` of it), and
/// each still does: a file lifts it when its tokens name the lint (the identifier
/// `unsafe_code`) outside an attribute of [`UNSAFE_CODE_KEPT`], which forbid or deny it.
/// Every crate root of [`RUST_CRATES`](crate::workspace::RUST_CRATES) (library, binary,
/// test, benchmark, example, build script) holds [`FORBID_UNSAFE_CODE`] among its leading inner
/// attributes, in tokens, so that the compiler refuses any later lifting (E0453); in a crate
/// holding an unsafe file, whose library root cannot forbid it, every other module and every root
/// outside `src/` does, but the unit test modules an unsafe file declares ([`test_declarations`]):
/// they belong to it, under its lifting (and under Miri with it), and lift nothing themselves.
///
/// rustc does not lint code expanded from another crate's macro, so a macro of a crate
/// that forbids unsafe code can still bring it into another. The workspace holds no
/// procedural macro, whose expansion no gate reads (its tokens can come from strings), and
/// no build script ([`TARGET_KINDS`](crate::test_code::TARGET_KINDS), [`untested_targets_gate`](crate::test_code::untested_targets_gate)), so only `macro_rules!`
/// macros and third-party macros expand into its code: no Rust file but those and the
/// unit tests they declare names the keyword `unsafe` in its tokens, prose and strings
/// apart, and no Rust file at all names an identifier of [`REFUSED_ASSEMBLY`]. A macro of a
/// third-party dependency is outside this gate: dependencies are reviewed
/// (`dependencies.txt`). A doc test is prose to the gate and compiled as a crate of its
/// own, outside its crate's attributes: every library root (`src/lib.rs`) holds
/// [`DOC_TESTS_FORBID_UNSAFE_CODE`] among its leading inner attributes, so that the
/// compiler refuses unsafe code in its doc tests.
///
/// Every file of [`ASSEMBLY_FILES`], which leaves the Miri step, must still hold inline
/// assembly, and nothing else unsafe: in its tokens ([`RustSource`]), it names `unsafe` at
/// least once, each time opening [`ASSEMBLY_CALL`], the standard library's `asm!` by a path
/// from the crates' root; it holds [`ONE_UNSAFE_OPERATION`] among its leading inner
/// attributes, so that the compiler keeps each unsafe block to that one operation (an
/// `asm!` operand's included) whatever a later attribute says; and it declares no module,
/// import, crate or macro ([`ASSEMBLY_REFUSED_WORDS`]). Neither an `unsafe fn` nor an
/// `unsafe impl` can hide there. No other file of its crate, its unit tests included, names
/// `unsafe`, since a macro or a function of another file could otherwise bring unsafe code
/// under the assembly file's lifting, and any unsafe operation of the crate is written
/// `unsafe` somewhere in it; nor rebinds `core` (`as core`, `mod core`), the one way left
/// in Rust code to make `::core` name another crate. Any other unsafe code would escape
/// Miri with its crate.
///
/// # Errors
///
/// Returns every other file that lifts the lint, every listed file that no longer does,
/// every file that should forbid it and does not, every assembly file without assembly or
/// its forbid, every `unsafe` of an assembly file that does not open its assembly call,
/// every declaration of an assembly file, every other `unsafe`, every identifier of
/// [`REFUSED_ASSEMBLY`], every library root whose doc tests do not forbid unsafe code, and
/// every rebinding of `core` in another file of an assembly file's crate.
pub fn unsafe_gate(parsed: &[RustSource]) -> Result<(), Failure> {
    let lifting: BTreeSet<&str> = parsed
        .iter()
        .filter(|source| lifts_unsafe_code(source))
        .map(|source| source.path.as_str())
        .collect();
    let allowed: BTreeSet<&str> = UNSAFE_FILES.into_iter().collect();
    let mut problems: Vec<String> =
        lifting
            .difference(&allowed)
            .map(|path| format!("{path} lifts the lint against unsafe code"))
            .chain(allowed.difference(&lifting).map(|path| {
                format!("{path} is listed in UNSAFE_FILES but no longer lifts the lint")
            }))
            .collect();
    for file in ASSEMBLY_FILES {
        let source = parsed.iter().find(|source| source.path == file);
        assembly_problems(file, source, &mut problems);
    }
    let unsafe_tests: BTreeSet<String> = parsed
        .iter()
        .filter(|source| UNSAFE_FILES.contains(&source.path.as_str()))
        .flat_map(test_declarations)
        .map(|declaration| declaration.file)
        .collect();
    unsafe_outside(parsed, &unsafe_tests, &mut problems);
    for source in parsed {
        let path = &source.path;
        let Some((krate, relative)) = in_crate(path) else {
            continue;
        };
        let holds_unsafe = UNSAFE_FILES.iter().any(|file| krate.holds(file).is_some());
        let required = if holds_unsafe {
            !UNSAFE_FILES.contains(&path.as_str())
                && !unsafe_tests.contains(path)
                && relative != "src/lib.rs"
                && (relative.starts_with("src/") || crate_root(relative))
        } else {
            crate_root(relative)
        };
        if required && source.leading_attribute(FORBID_UNSAFE_CODE).is_none() {
            problems.push(format!("{path} does not hold `{FORBID_UNSAFE_CODE}`"));
        }
        if relative == "src/lib.rs"
            && source
                .leading_attribute(DOC_TESTS_FORBID_UNSAFE_CODE)
                .is_none()
        {
            problems.push(format!(
                "{path} does not hold `{DOC_TESTS_FORBID_UNSAFE_CODE}`: its doc tests, each a \
                 crate of its own, would not forbid unsafe code"
            ));
        }
    }
    verdict("the lint against unsafe code", problems)
}

/// The inner attribute forbidding unsafe code, which every file [`unsafe_gate`] requires it
/// of holds among its leading inner attributes (`RustSource::leading_attribute`).
pub const FORBID_UNSAFE_CODE: &str = "#![forbid(unsafe_code)]";

/// The inner attribute every library root holds among its leading ones ([`unsafe_gate`]):
/// its doc tests, each compiled as a crate of its own outside the crate's attributes,
/// forbid unsafe code too.
pub const DOC_TESTS_FORBID_UNSAFE_CODE: &str = "#![doc(test(attr(forbid(unsafe_code))))]";

/// The identifiers no Rust file names: assembly outside an `unsafe` block.
///
/// `global_asm!`, a global symbol, and `naked_asm!`, a naked function's body, which neither
/// the one-operation rule of the assembly file nor Miri sees, and which rustc does not lint
/// when another crate's macro expands to it.
pub const REFUSED_ASSEMBLY: [&str; 2] = ["global_asm", "naked_asm"];

/// The attributes that name the lint against unsafe code without lifting it: each forbids
/// or denies it, for the file, for one item, or for the file's doc tests.
pub const UNSAFE_CODE_KEPT: [&str; 5] = [
    FORBID_UNSAFE_CODE,
    DOC_TESTS_FORBID_UNSAFE_CODE,
    "#![deny(unsafe_code)]",
    "#[forbid(unsafe_code)]",
    "#[deny(unsafe_code)]",
];

/// Whether `source` names the lint against unsafe code (the identifier `unsafe_code`)
/// outside every attribute of [`UNSAFE_CODE_KEPT`]: rustc reads lint names in tokens, so
/// any other use of the identifier can lift it (`allow`, `expect`, `warn`, a conditional
/// attribute, a macro writing the attribute).
pub(crate) fn lifts_unsafe_code(source: &RustSource) -> bool {
    let kept: Vec<core::ops::Range<usize>> = UNSAFE_CODE_KEPT
        .into_iter()
        .flat_map(|attribute| source.occurrences(attribute))
        .collect();
    source.tokens.iter().enumerate().any(|(at, token)| {
        token.is_ident("unsafe_code") && !kept.iter().any(|range| range.contains(&at))
    })
}

/// The inner attribute every file of [`ASSEMBLY_FILES`] holds among its leading ones: each
/// unsafe block of it holds one unsafe operation.
///
/// That operation is its assembly call, whatever a later `allow` or `expect` says
/// (E0453); Clippy's `multiple_unsafe_ops_per_block` counts the operations of an `asm!`
/// operand too.
pub const ONE_UNSAFE_OPERATION: &str = "#![forbid(clippy::multiple_unsafe_ops_per_block)]";

/// What every `unsafe` of an assembly file opens, token by token.
///
/// `unsafe { ::core::arch::asm!(`, comments and spaces apart: the standard library's `asm!`
/// by a path that starts at the crates' root (`::core`), which no module of the file can
/// shadow.
pub const ASSEMBLY_CALL: [&str; 13] = [
    "unsafe", "{", ":", ":", "core", ":", ":", "arch", ":", ":", "asm", "!", "(",
];

/// The identifiers an assembly file never holds: each declares a name a path could
/// resolve through, a module (`mod core`), an import, a crate or a macro.
pub const ASSEMBLY_REFUSED_WORDS: [&str; 4] = ["mod", "use", "extern", "macro_rules"];

/// Adds to `problems` those of `source`, the assembly file `file` ([`unsafe_gate`]), if it
/// was read: no [`ONE_UNSAFE_OPERATION`] line, an `unsafe` not opening [`ASSEMBLY_CALL`],
/// none at all, or an identifier of [`ASSEMBLY_REFUSED_WORDS`].
pub(crate) fn assembly_problems(
    file: &str,
    source: Option<&RustSource>,
    problems: &mut Vec<String>,
) {
    if source
        .and_then(|source| source.leading_attribute(ONE_UNSAFE_OPERATION))
        .is_none()
    {
        problems.push(format!(
            "{file} does not hold `{ONE_UNSAFE_OPERATION}`: an unsafe block of it could hold \
             more than its assembly call"
        ));
    }
    let tokens = source.map_or(&[][..], |source| source.tokens.as_slice());
    let mut unsafe_at = 0_usize;
    for (at, token) in tokens.iter().enumerate() {
        let line = token.line;
        if token.is_ident("unsafe") {
            unsafe_at = unsafe_at.saturating_add(1);
            let opened = tokens
                .get(at..)
                .unwrap_or_default()
                .iter()
                .map(|opening| opening.text.as_str())
                .take(ASSEMBLY_CALL.len());
            if !opened.eq(ASSEMBLY_CALL) {
                problems.push(format!(
                    "{file}:{line}: `unsafe` not opening `{{ ::core::arch::asm!(`: an assembly \
                     file holds no other unsafe code, which Miri would not check"
                ));
            }
        } else if token.kind == TokenKind::Ident
            && ASSEMBLY_REFUSED_WORDS.contains(&token.text.as_str())
        {
            problems.push(format!(
                "{file}:{line}: names `{}`: an assembly file declares no module, import, \
                 crate or macro, through which its paths could resolve elsewhere",
                token.text
            ));
        }
    }
    if unsafe_at == 0 {
        problems.push(format!(
            "{file} is listed in ASSEMBLY_FILES but holds no inline assembly"
        ));
    }
}

/// Adds to `problems` every identifier of [`REFUSED_ASSEMBLY`] in `sources`, every
/// `unsafe` outside [`UNSAFE_FILES`] and the unit tests they declare (`tests`), and every
/// rebinding of `core` (`as core`, `mod core`) in the crate of an assembly file
/// ([`unsafe_gate`]).
///
/// rustc does not lint code expanded from another crate's macro against unsafe code, so a
/// `forbid` cannot keep a macro of a safe crate from bringing unsafe code into another:
/// the keyword `unsafe` is refused in every other file, test code included. In the crate
/// of an assembly file ([`ASSEMBLY_FILES`]), which Miri never runs, it is refused in every
/// other file, its own unit tests included, since code of any of them (a macro, a function
/// it calls) can run under its lifting; a rebinding of `core` there would change what its
/// `::core::arch::asm!` names.
pub(crate) fn unsafe_outside(
    sources: &[RustSource],
    tests: &BTreeSet<String>,
    problems: &mut Vec<String>,
) {
    for source in sources {
        let path = source.path.as_str();
        for token in &source.tokens {
            if token.kind == TokenKind::Ident && REFUSED_ASSEMBLY.contains(&token.text.as_str()) {
                problems.push(format!(
                    "{path}:{}: names `{}`: assembly outside an `unsafe` block (a global \
                     symbol, a naked function) is written nowhere, since neither the \
                     one-operation rule nor Miri sees it",
                    token.line, token.text
                ));
            }
        }
        if UNSAFE_FILES.contains(&path) && !ASSEMBLY_FILES.contains(&path) {
            continue;
        }
        let assembly = in_crate(path).and_then(|(krate, _)| {
            ASSEMBLY_FILES
                .into_iter()
                .find(|file| krate.holds(file).is_some())
        });
        if assembly == Some(path) {
            continue;
        }
        let tokens = &source.tokens;
        for (at, token) in tokens.iter().enumerate() {
            let line = token.line;
            match assembly {
                _ if !token.is_ident("unsafe") => {}
                Some(file) => problems.push(format!(
                    "{path}:{line}: `unsafe` in the crate of the assembly file {file}: its \
                     code could run under that file's lifting, where Miri never checks it"
                )),
                None if tests.contains(path) => {}
                None => problems.push(format!(
                    "{path}:{line}: names `unsafe` outside UNSAFE_FILES and the unit tests they \
                     declare: rustc does not lint code expanded from another crate's macro, so \
                     unsafe code is written only where Miri and the audit see it"
                )),
            }
            let rebinds = (token.is_ident("as") || token.is_ident("mod"))
                && tokens
                    .get(at.saturating_add(1))
                    .is_some_and(|next| next.is_ident("core"));
            if let Some(file) = assembly.filter(|_| rebinds) {
                problems.push(format!(
                    "{path}:{line}: rebinds `core` in the crate of the assembly file {file}, \
                     whose `::core::arch::asm!` must name the real one"
                ));
            }
        }
    }
}

#[cfg(test)]
#[path = "unsafety_tests.rs"]
mod tests;
