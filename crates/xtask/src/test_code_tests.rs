//! Tests of the gates that keep test code, test targets and test data where the rules that tell
//! them apart look.

use crate::tests::{
    failed, parsed, scratch, step_found, system, targets_metadata, test_code_gate, test_data_gate,
    test_data_reader_sources, untested_targets_gate, unviable_names_gate,
};
use crate::{CargoWorkspace, Failure, Runner, RustCrate, UNVIABLE_REVIEWED};

#[test]
fn reads_the_lint_forbids_as_no_test_code() {
    let gate =
        |text: &str| test_code_gate(&[("crates/audio-core/src/a.rs".to_owned(), text.to_owned())]);
    // The two lint forbids that apply outside tests, written whole, the attribute's name
    // apart so that the bypass gate does not read this file as lifting a lint.
    let attribute = format!("{}_{}", "cfg", "attr");
    let forbids = format!(
        "#![{attribute}(not(test), forbid(clippy::arithmetic_side_effects))]\n  \
         #![{attribute}(not(test), forbid(clippy::float_arithmetic))]\n"
    );
    assert_eq!(gate(&forbids), Ok(()), "the forbid lines, trimmed");
    assert_eq!(
        gate(&forbids.replace("float_arithmetic))]", "float_arithmetic))] ")),
        Ok(()),
        "trailing spaces trimmed"
    );
    assert_eq!(
        gate(&format!(
            "#![{attribute}(not(test), forbid(clippy::float_arithmetic, unused))]\n"
        )),
        failed(
            "crates/audio-core/src/a.rs:1: names `test` outside test code: tests \
             live in tests/ or in a *_tests.rs file declared `#[cfg(test)] #[path = \
             \"<name>_tests.rs\"] mod <module>;`"
        ),
        "another line"
    );
}

#[test]
fn refuses_test_code_and_files_not_rust_in_the_code() {
    let core = crate::RUST_CRATES[0];
    let build = |of_tests: bool, files: &[&str]| crate::Loaded {
        krate: core,
        target: "audio_core".to_owned(),
        configuration: None,
        of_tests,
        files: files.iter().map(|file| (*file).to_owned()).collect(),
    };
    let tracked: Vec<String> = [
        "crates/audio-core/src/lib.rs",
        "crates/audio-core/src/extra_tests.rs",
        "crates/audio-core/tests/laws.rs",
    ]
    .map(str::to_owned)
    .into();
    let gate = |builds: &[crate::Loaded]| {
        crate::loaded_files_gate(builds, &tracked).map_err(|failure| failure.summary)
    };
    let refused = |problem: &str| Err(problem.to_owned());
    for test_code in [
        "crates/audio-core/src/extra_tests.rs",
        "crates/audio-core/tests/laws.rs",
    ] {
        assert_eq!(
            gate(&[build(false, &["crates/audio-core/src/lib.rs", test_code])]),
            refused(&format!(
                "{test_code}: read by the build of `audio_core` (audio-core), which builds no \
                 tests, but named as test code (tests/, a *_tests.rs file): code that every \
                 rule telling test code by its name skips"
            )),
            "test code compiled into the code: {test_code}"
        );
        assert_eq!(
            gate(&[
                build(false, &["crates/audio-core/src/lib.rs"]),
                build(true, &["crates/audio-core/src/lib.rs", test_code]),
            ]),
            Ok(()),
            "the same file read by the tests alone: {test_code}"
        );
    }
    for (of_tests, data, what) in [
        (false, "crates/audio-core/src/blob.inc", "the build"),
        (true, "crates/audio-core/src/blob.inc", "the test build"),
        (true, "crates/audio-core/tests/data.bin", "the test build"),
    ] {
        assert_eq!(
            gate(&[
                build(false, &["crates/audio-core/src/lib.rs"]),
                build(of_tests, &["crates/audio-core/src/lib.rs", data])
            ]),
            refused(&format!(
                "{data}: read by {what} of `audio_core` (audio-core), not a Rust file: no gate \
                 reads it as code, and only a build of tests reads data, from its crate's \
                 declared test data"
            )),
            "a file that is not Rust, read by {what}: {data}"
        );
    }
    let in_crate = RustCrate {
        test_data: &["crates/audio-core/tests/data/"],
        ..core
    };
    assert_eq!(
        crate::loaded_files_gate(
            &[
                build(false, &["crates/audio-core/src/lib.rs"]),
                crate::Loaded {
                    krate: in_crate,
                    ..build(
                        true,
                        &[
                            "crates/audio-core/src/lib.rs",
                            "crates/audio-core/tests/data/a.bin"
                        ]
                    )
                },
            ],
            &tracked
        ),
        Ok(()),
        "declared test data below the crate, read by a build of tests"
    );
    assert_eq!(
        gate(&[
            build(false, &["crates/audio-core/src/lib.rs"]),
            build(
                true,
                &[
                    "crates/audio-core/src/lib.rs",
                    "conformance/test-matrix.json"
                ]
            )
        ]),
        Ok(()),
        "declared test data read by a build of tests"
    );
}

#[test]
fn refuses_a_reason_naming_a_function_outside_test_code() {
    let production = (
        "crates/audio-hash/src/lib.rs".to_owned(),
        "pub fn unsafe_gate_helper_fn() {}\n".to_owned(),
    );
    let review = "crates/audio-hash/src/lib.rs: replace unsafe_gate_helper_fn with () -- killed by unsafe_gate_helper_fn\n";
    assert_eq!(
        unviable_names_gate(review, core::slice::from_ref(&production)),
        failed(format!(
            "{UNVIABLE_REVIEWED}:1: names `unsafe_gate_helper_fn` as its reason, which no test \
             code defines as a function: a review's reason names the test that kills its \
             viable variant"
        )),
        "a production function named as the test that kills the mutant"
    );
    for test_code in [
        "crates/audio-hash/src/lib_tests.rs",
        "crates/audio-hash/tests/hash.rs",
    ] {
        assert_eq!(
            unviable_names_gate(
                review,
                &[
                    production.clone(),
                    (
                        test_code.to_owned(),
                        "fn unsafe_gate_helper_fn() {}\n".to_owned()
                    )
                ]
            ),
            Ok(()),
            "the mutated function in production code, the reason's in test code: {test_code}"
        );
    }
}

#[test]
fn refuses_a_review_naming_a_function_no_source_holds() {
    let sources = vec![
        (
            "crates/audio-core/src/lib_tests.rs".to_owned(),
            "type F = fn(u8) -> u8;\nfn kills_the_viable_variant() {}\n// fn in_a_comment_only() {}\nconst A: &str = \"fn in_a_string_only\";\nfn r#raw_named_one() {}\n".to_owned(),
        ),
        ("crates/audio-hash/tests/x.rs".to_owned(), "pub(crate) fn other_crate_s_test() {}\n".to_owned()),
    ];
    let reviews = "# A comment naming no_such_function_here is prose.\n\ncrates/a/src/lib.rs: replace f -> u8 with 0 -- killed by kills_the_viable_variant and other_crate_s_test\ncrates/a/src/lib.rs: replace g -> u8 with 1 -- killed by raw_named_one, not two_parts nor Not_a_name nor not_A_name\n";
    assert_eq!(
        unviable_names_gate(reviews, &sources),
        Ok(()),
        "every name a function"
    );
    let dead = format!(
        "{reviews}crates/a/src/lib.rs: replace h -> u8 with 2 -- killed by in_a_comment_only\ncrates/a/src/lib.rs: replace i -> u8 with 3 -- killed by in_a_string_only or renamed_since_the_review\ncrates/a/src/lib.rs: replace no_such_mutated_fn -> u8 with 4 -- killed by kills_the_viable_variant\n"
    );
    assert_eq!(
        unviable_names_gate(&dead, &sources),
        Err(Failure {
            summary: "4 problems, listed \
                      above"
                .to_owned(),
            details: [
                (5_usize, "in_a_comment_only"),
                (6, "in_a_string_only"),
                (6, "renamed_since_the_review"),
                (7, "no_such_mutated_fn")
            ]
            .map(|(line, name)| format!(
                "{UNVIABLE_REVIEWED}:{line}: names `{name}`, which no Rust source defines as a \
                 function: a review names the test that kills its viable variant as it is now"
            ))
            .into(),
            usage: false,
        }),
        "names only in prose, one renamed, a mutated function that no longer exists"
    );
}

#[test]
fn refuses_a_test_file_that_loads_another() {
    let gate = |sources: &[(&str, &str)]| {
        let owned: Vec<(String, String)> = sources
            .iter()
            .map(|(path, text)| ((*path).to_owned(), (*text).to_owned()))
            .collect();
        test_code_gate(&owned)
    };
    let declared = "#[cfg(test)]\n#[path = \"plan_tests.rs\"]\nmod tests;\n";
    let inline = "the modules of a *_tests.rs file are inline";
    let (bodyless, includes) = (
        "declares a module without a body, loaded from another file",
        "names `include`, which loads another file",
    );
    let loads = |what: &str, line: usize| {
        format!(
            "crates/audio-core/src/plan_tests.rs:{line}: {what}, which no rule tells \
             as test code: a *_tests.rs file writes its modules inline"
        )
    };
    for (text, expected) in [
        ("#[test]\nfn t() {}\nmod helper;\n", loads(bodyless, 3)),
        ("mod r#helper;\n", loads(bodyless, 1)),
        ("pub(crate) mod  helper ;\n", loads(bodyless, 1)),
        ("mod helper /* a comment */ ;\n", loads(bodyless, 1)),
        (
            "include!(\"h.rs\");\nmod helper;\n",
            format!(
                "2 problems, listed above\n{}\n{}",
                loads(includes, 1),
                loads(bodyless, 2)
            ),
        ),
        (
            "macro_rules! m {\n    ($n:ident) => { mod $n; };\n}\n",
            loads(bodyless, 2),
        ),
        ("/* /* */ \"*/\nmod helper;\n// \"\n", loads(bodyless, 2)),
        (
            "/* /* */ \"*/\ninclude!(\"h.rs\");\n// \"\n",
            loads(includes, 2),
        ),
    ] {
        assert_eq!(
            gate(&[
                ("crates/audio-core/src/plan.rs", declared),
                ("crates/audio-core/src/plan_tests.rs", text),
            ]),
            failed(expected),
            "{text}"
        );
    }
    assert_eq!(
        gate(&[
            ("crates/audio-core/src/plan.rs", declared),
            (
                "crates/audio-core/src/plan_tests.rs",
                "mod inner {\n    #[test]\n    fn t() { let path = include_str!(\"x.json\"); }\n}\n// mod helper;\nconst M: &str = \"mod helper;\";\nfn f(path: &str) -> &str { path }\n#[derive(Debug)] struct S;\n#[path = \"h.rs\"]\nmod h {}\n",
            ),
        ]),
        Ok(()),
        "{inline}; a word `path`, a module in prose and test data are no loading; what a \
         `path` attribute loads is the loaded-files gate's"
    );
}

#[test]
fn refuses_test_code_outside_test_files() {
    let gate = |sources: &[(&str, &str)]| {
        let owned: Vec<(String, String)> = sources
            .iter()
            .map(|(path, text)| ((*path).to_owned(), (*text).to_owned()))
            .collect();
        test_code_gate(&owned)
    };
    let declared = "//! test, in prose\nconst A: &str = \"test\";\nconst B: &str = include_str!(\"x.json\");\n#[cfg(test)]\n#[path = \"plan_tests.rs\"]\nmod tests;\n";
    let tests = "#[test]\nfn f() { assert!(cfg!(test)); }\n";
    let fine = [
        ("crates/audio-core/src/plan.rs", declared),
        ("crates/audio-core/src/plan_tests.rs", tests),
        (
            "crates/audio-cli/tests/binary.rs",
            "#[cfg(test)]\nmod t { include!(\"x.rs\"); }\n",
        ),
        ("crates/audio-cli/tests/data_tests.rs", ""),
        ("README.rs", "#[test] fn f() {}\n"),
        ("crates/other/src/lib.rs", "#[test] fn f() {}\n"),
    ];
    assert_eq!(gate(&fine), Ok(()), "test code where it belongs");
    let names = "names `test` outside test code: tests live in tests/ or in a *_tests.rs file \
                 declared `#[cfg(test)] #[path = \"<name>_tests.rs\"] mod <module>;`";
    for (text, line) in [
        ("#[cfg(test)]\nmod tests {\n    fn f() {}\n}\n", 1_usize),
        ("fn g() {}\n#[cfg(test)]\nfn f() {}\n", 2),
        ("#[cfg_attr(test, derive(Debug))]\nstruct S;\n", 1),
        ("fn f() -> bool { cfg!(test) }\n", 1),
        ("#[cfg(test)]\nmod tests;\n", 1),
        ("#[cfg(any(test, unix))]\nfn f() {}\n", 1),
    ] {
        assert_eq!(
            gate(&[("crates/audio-core/src/a.rs", text)]),
            failed(format!("crates/audio-core/src/a.rs:{line}: {names}")),
            "{text}"
        );
    }
    assert_eq!(
        gate(&[(
            "crates/audio-core/src/a.rs",
            "fn f() {\n    #[test]\n    fn g() {}\n}\n"
        )]),
        failed(format!("crates/audio-core/src/a.rs:2: {names}")),
        "a test function"
    );
    assert_eq!(
        gate(&[("bindings/audio-wasm/src/a.rs", "include!(\"b.rs\");\n")]),
        failed(
            "bindings/audio-wasm/src/a.rs:1: names `include`: code is brought in by `mod` \
             only, so that its path says what it is"
        ),
        "code included"
    );
    assert_eq!(
        gate(&[("crates/audio-core/src/plan_tests.rs", tests)]),
        failed(
            "crates/audio-core/src/plan_tests.rs: a *_tests.rs file no file beside \
             it declares as test code"
        ),
        "an undeclared test file"
    );
    assert_eq!(
        gate(&[
            ("crates/audio-core/src/plan.rs", declared),
            ("crates/audio-core/src/other.rs", declared),
            ("crates/audio-core/src/plan_tests.rs", tests),
            ("crates/audio-core/src/sub/x.rs", declared),
        ]),
        step_found(&[
            "crates/audio-core/src/plan_tests.rs: declared as test code 2 times".to_owned()
        ]),
        "a test file declared twice; one declared from another directory is another file"
    );
    let unknown = gate(&[
        ("crates/audio-core/src/plan.rs", declared),
        ("crates/audio-core/src/plan_tests.rs", tests),
        ("crates/audio-core/src/sub/x.rs", declared),
    ]);
    assert_eq!(
        unknown,
        Ok(()),
        "a declaration of a missing file is the compiler's to refuse"
    );
}

#[test]
fn refuses_test_data_a_crate_does_not_declare() {
    let base = test_data_reader_sources();
    assert_eq!(test_data_gate(&base), Ok(()), "every root read");
    let with = |path: &str, text: &str| {
        let mut sources = base.clone();
        sources.push((path.to_owned(), text.to_owned()));
        test_data_gate(&sources)
    };
    for text in [
        "const A: &str = include_str!(\"../../../spec/render-hash-vectors.json\");\n",
        "const A: &[u8] = include_bytes!(\"./data/own.bin\");\n",
        "fn f() { let _ = Path::new(env!(\"CARGO_MANIFEST_DIR\")).join(\"src\"); }\n",
        "fn f() { let _ = env!(\"CARGO_PKG_NAME\"); let _ = std::env::temp_dir(); }\n",
        "// include_str!(\"../../../spec/x.json\")\nconst A: &str = \"include_str!(\\\"x\\\")\";\n",
    ] {
        assert_eq!(
            with("crates/audio-hash/src/more_tests.rs", text),
            Ok(()),
            "declared data, the crate's own files, other variables, prose: {text}"
        );
    }
    let unread = "crates/audio-hash/src/more_tests.rs:1: the tests of audio-hash read";
    for (text, read) in [
        (
            "const A: &str = include_str!(\"../../../spec/measurement-vectors.json\");\n",
            "spec/measurement-vectors.json",
        ),
        (
            "fn f() { let _ = Path::new(env!(\"CARGO_MANIFEST_DIR\"))\n    .join(\"../../conformance\"); }\n",
            "conformance",
        ),
        (
            "const A: &str = include_str!(\"../../x.json\");\n",
            "crates/x.json",
        ),
    ] {
        assert_eq!(
            with("crates/audio-hash/src/more_tests.rs", text),
            failed(format!("{unread} `{read}`, which is not in its test_data")),
            "{text}"
        );
    }
    for (text, word) in [
        ("const A: &str = include_str!(PATH);\n", "include_str"),
        ("const A: &str = include_bytes!(r\"x\");\n", "include_bytes"),
        (
            "const A: &str = include_str!(\"x\\u{2e}rs\");\n",
            "include_str",
        ),
        ("const A: &str = include_str!(\"x\", );\n", "include_str"),
        (
            "fn f() { let _ = Path::new(env!(\"CARGO_MANIFEST_DIR\")); }\n",
            "env",
        ),
        (
            "fn f() { let _ = Path::new(env!(\"CARGO_MANIFEST_DIR\")).join(DIR); }\n",
            "env",
        ),
        (
            "fn f() { let _ = env!(\"CARGO_MANIFEST_DIR\").join(\"x\"); }\n",
            "env",
        ),
    ] {
        assert_eq!(
            with("crates/audio-hash/src/more_tests.rs", text),
            failed(format!(
                "crates/audio-hash/src/more_tests.rs:1: `{word}!` without a literal \
                 path the gate can follow"
            )),
            "{text}"
        );
    }
    assert_eq!(
        with(
            "crates/audio-hash/src/more_tests.rs",
            "const A: &str = include_str!(\"../../../../x\");\n"
        ),
        failed(
            "crates/audio-hash/src/more_tests.rs:1: `../../../../x` leaves the \
             repository"
        ),
        "outside the repository"
    );
    let without: Vec<(String, String)> = base
        .iter()
        .filter(|(path, _)| path != "crates/audio-hash/tests/data.rs")
        .cloned()
        .collect();
    assert_eq!(
        test_data_gate(&without),
        failed(
            "audio-hash: declares `spec/render-hash-vectors.json` as test data, \
             which no test of it reads"
        ),
        "a stale root"
    );
    assert_eq!(
        with("README.rs", "const A: &str = include_str!(\"x\");\n"),
        Ok(()),
        "a file of no crate"
    );
}

#[test]
fn counts_a_read_in_the_crate_as_a_read_of_its_root() {
    // A crate declaring a file of its own as test data, as the loaded-files gate asks of a
    // file that is not Rust a build of its tests loads, reads it from its crate: that read
    // is a read of the root, which no test reads otherwise.
    let krate = RustCrate {
        name: "audio-wasm",
        dir: "bindings/audio-wasm",
        binary: false,
        test_data: &["bindings/audio-wasm/ts/blocks.ts"],
        roles: &[],
        workspace: CargoWorkspace::Root,
    };
    let gate = |text: &str| {
        crate::test_code::crates_test_data_gate(
            &parsed(&[(
                "bindings/audio-wasm/src/lib_tests.rs".to_owned(),
                text.to_owned(),
            )]),
            &[krate],
        )
    };
    assert_eq!(
        gate("const A: &str = include_str!(\"../ts/blocks.ts\");\n"),
        Ok(()),
        "the root read by include_str!"
    );
    assert_eq!(
        gate(
            "fn f() { let _ = Path::new(env!(\"CARGO_MANIFEST_DIR\")).join(\"ts/blocks.ts\"); }\n"
        ),
        Ok(()),
        "the root read from the crate's directory"
    );
    let unread = failed(
        "audio-wasm: declares `bindings/audio-wasm/ts/blocks.ts` as test data, which no test \
         of it reads",
    );
    assert_eq!(
        gate("const A: &str = include_str!(\"../ts/other.ts\");\n"),
        unread,
        "another file of the crate, under no root, read as before but not the root"
    );
    assert_eq!(gate("fn f() {}\n"), unread, "nothing read");
    assert_eq!(
        crate::test_code::crates_test_data_gate(
            &parsed(&[(
                "bindings/audio-wasm/src/lib.rs".to_owned(),
                "const A: &str = include_str!(\"../ts/blocks.ts\");\n".to_owned(),
            )]),
            &[krate],
        ),
        unread,
        "the root read by the library itself, which no test reads"
    );
}

#[test]
fn refuses_the_reads_of_test_data_it_cannot_follow() {
    let with = |path: &str, text: &str| {
        let mut sources = test_data_reader_sources();
        sources.push((path.to_owned(), text.to_owned()));
        test_data_gate(&sources)
    };
    let file = "crates/audio-hash/src/more_tests.rs";
    let manifest = format!("\"{}_{}_{}\"", "CARGO", "MANIFEST", "DIR");
    let run_time = |word: &str| {
        format!(
            "`{word}` of a variable naming the crate's manifest, a directory read at run time \
             the gate cannot follow"
        )
    };
    let path = format!("\"{}_{}_{}\"", "CARGO", "MANIFEST", "PATH");
    for (text, why) in [
        (
            format!("fn f() {{ let _ = std::env::var_os({manifest}); }}\n"),
            run_time("var_os"),
        ),
        (
            format!("fn f() {{ let _ = var ( {manifest} ); }}\n"),
            run_time("var"),
        ),
        (
            format!("fn f() {{ let _ = var({path}); }}\n"),
            run_time("var"),
        ),
        (
            "fn f() { let _ = option_env!(\"X\"); }\n".to_owned(),
            "`option_env!`, which may name a directory of the repository".to_owned(),
        ),
        (
            // A variable of the list read without the macro's `!` is no read the gate knows.
            "fn f() { let _ = option_env(\"POYETIK_ENGINE_BLOCK\"); }\n".to_owned(),
            "`option_env!`, which may name a directory of the repository".to_owned(),
        ),
        (
            "fn f() { let _ = env!(\"OUT_DIR\"); }\n".to_owned(),
            "`env!` of a variable that may name a directory of the repository".to_owned(),
        ),
        (
            format!(
                "fn f() {{ let _ = Path::new(env!({manifest})).join(\"tests\").join(\"../../../spec/x\"); }}\n"
            ),
            "a path joined or left again after the one `.join(\"<literal>\")` the gate follows"
                .to_owned(),
        ),
        (
            format!(
                "fn f() {{ let _ = Path::new(env!({manifest})).join(\"tests\")\n    .parent(); }}\n"
            ),
            "a path joined or left again after the one `.join(\"<literal>\")` the gate follows"
                .to_owned(),
        ),
    ] {
        assert_eq!(
            with(file, &text),
            failed(format!("{file}:1: {why}")),
            "{text}"
        );
    }
    for text in [
        "fn f() { let _ = std::env::var_os(CHILD); let _ = var(\"HOME\"); }\n".to_owned(),
        "fn f() { let _ = var(r\"CARGO_PKG_NAME\"); let _ = var_os(r#\"CARGO\"#); }\n".to_owned(),
        "fn f() { let _ = env!(\"CARGO_BIN_EXE_audio-cli\"); let _ = env!(\"CARGO_PKG_VERSION\"); }\n"
            .to_owned(),
        format!(
            "fn f() {{ let _ = Path::new(env!({manifest})).join(\"src\").display(); let var = 1; }}\n"
        ),
    ] {
        assert_eq!(with(file, &text), Ok(()), "{text}");
    }
    // Data read outside the crate by code that is not a test.
    assert_eq!(
        with(
            "crates/audio-testkit/src/measure/mod.rs",
            "const V: &str = include_str!(\"../../../../spec/measurement-vectors.json\");\n"
        ),
        failed(
            "crates/audio-testkit/src/measure/mod.rs:1: code that is not a test \
             reads `spec/measurement-vectors.json`, outside its crate: only tests read test data"
        ),
        "a library's read is no input any crate declares"
    );
}

#[test]
fn follows_the_reads_of_test_data_in_tokens() {
    let file = "crates/audio-hash/src/more_tests.rs";
    let with = |text: &str| {
        let mut sources = test_data_reader_sources();
        sources.push((file.to_owned(), text.to_owned()));
        test_data_gate(&sources)
    };
    let manifest = format!("\"{}_{}_{}\"", "CARGO", "MANIFEST", "DIR");
    for (text, why) in [
        (
            "/* /* */ \"*/ const A: &str = include_str!(\"../../../spec/measurement-vectors.json\"); // \"\n".to_owned(),
            format!(
                "{file}:1: the tests of audio-hash read `spec/measurement-vectors.json`, which \
                 is not in its test_data"
            ),
        ),
        (
            "const A: &str = include_str![\"x\"];\n".to_owned(),
            format!("{file}:1: `include_str!` without a literal path the gate can follow"),
        ),
        (
            "fn f() { let _ = Path::new(env!(\"CARGO_MANIFEST_DIR\")).parent(); }\n".to_owned(),
            format!("{file}:1: `env!` without a literal path the gate can follow"),
        ),
        // The one join follows the path's constructor (its `)`), then `.`, then `join`.
        (
            "fn f() { g(env!(\"CARGO_MANIFEST_DIR\") , . join(\"src\")); }\n".to_owned(),
            format!("{file}:1: `env!` without a literal path the gate can follow"),
        ),
        (
            "fn f() { g(Path::new(env!(\"CARGO_MANIFEST_DIR\")) , join(\"src\")); }\n".to_owned(),
            format!("{file}:1: `env!` without a literal path the gate can follow"),
        ),
        (
            "fn f() { let _ = Path::new(env!(\"CARGO_MANIFEST_DIR\")).joined(\"src\"); }\n"
                .to_owned(),
            format!("{file}:1: `env!` without a literal path the gate can follow"),
        ),
    ] {
        assert_eq!(with(&text), failed(why), "{text}");
    }
    // Only `(` opens a call.
    assert_eq!(
        with("fn f() { let _ = var[\"CARGO_MANIFEST_DIR\"]; let var: u8 = 1; var::x(); }\n"),
        Ok(()),
        "no call"
    );
    // Outside the gate's scope: "a turbofish, a raw string or an escape naming it is outside
    // the gate's scope, written only on purpose" (`test_data_gate`'s doc).
    for evasion in [
        format!("fn f() {{ let _ = std::env::var::<&str>({manifest}); }}\n"),
        format!("fn f() {{ let _ = var(r{manifest}); }}\n"),
        "fn f() { let _ = var_os(r#\"CARGO_MANIFEST_DIR\"#); }\n".to_owned(),
        "fn f() { let _ = var(\"CARGO_MANIFEST\\x5fDIR\"); }\n".to_owned(),
    ] {
        assert_eq!(with(&evasion), Ok(()), "{evasion}");
    }
}

#[test]
fn leaves_a_binary_entry_point_to_find_its_repository() {
    let with = |path: &str, text: &str| {
        let mut sources = test_data_reader_sources();
        sources.push((path.to_owned(), text.to_owned()));
        test_data_gate(&sources)
    };
    let root =
        "fn main() { let _ = PathBuf::from(env!(\"CARGO_MANIFEST_DIR\")).join(\"../..\"); }\n";
    assert_eq!(
        with("crates/xtask/src/main.rs", root),
        Ok(()),
        "a binary's entry point, which finds the repository it runs in"
    );
    for path in [
        "crates/audio-hash/src/main.rs",
        "crates/xtask/src/bin/main.rs",
    ] {
        assert_eq!(
            with(path, root),
            failed(format!(
                "{path}:1: code that is not a test reads ``, outside its crate: only \
                 tests read test data"
            )),
            "{path}: not a binary's entry point"
        );
    }
    assert_eq!(
        with("crates/xtask/tests/main.rs", root),
        failed(
            "crates/xtask/tests/main.rs:1: the tests of xtask read ``, which is not in \
             its test_data"
        ),
        "a test's main"
    );
}

/// The failure of the untested targets gate with `problems`.
fn untested_refused(problems: &[&str]) -> Result<(), Failure> {
    step_found(
        &problems
            .iter()
            .map(|problem| (*problem).to_owned())
            .collect::<Vec<_>>(),
    )
}

#[test]
fn lets_a_target_without_modules_turn_its_tests_off() {
    let metadata = targets_metadata(&[
        ("lib", "crates/x/src/lib.rs", true, true),
        ("bin", "crates/x/src/main.rs", false, false),
        ("bin", "crates/x/src/bin/c.rs", false, false),
        ("test", "crates/x/tests/a.rs", true, false),
    ]);
    let sources = [
        (
            "crates/x/src/lib.rs".to_owned(),
            "mod a;\n#[cfg(test)]\n#[path = \"lib_tests.rs\"]\nmod tests;\n".to_owned(),
        ),
        (
            "crates/x/src/main.rs".to_owned(),
            "// The module of a comment, \"mod\" of a string.\nfn main() { let _module = \"mod a;\"; }\n"
                .to_owned(),
        ),
        ("crates/x/src/bin/c.rs".to_owned(), "fn main() {}\n".to_owned()),
    ];
    assert_eq!(
        untested_targets_gate(&metadata, &sources),
        Ok(()),
        "a library tested, roots without modules untested, a test target"
    );
}

#[test]
fn refuses_a_target_that_turns_off_tests_it_may_hold() {
    let metadata = targets_metadata(&[
        ("bin", "crates/x/src/main.rs", false, false),
        ("lib", "crates/x/src/lib.rs", false, true),
        ("bin", "crates/x/src/bin/b.rs", false, false),
        ("bin", "crates/x/src/bin/c.rs", false, false),
    ]);
    let sources = [
        (
            "crates/x/src/main.rs".to_owned(),
            "fn main() {}\n#[cfg(test)]\n#[path = \"main_tests.rs\"]\nmod tests;\n".to_owned(),
        ),
        (
            "crates/x/src/lib.rs".to_owned(),
            "//! A library.\npub mod a;\n".to_owned(),
        ),
        (
            "crates/x/src/bin/c.rs".to_owned(),
            "/* /* */ \"*/ mod hidden; // \"\nfn main() {}\n".to_owned(),
        ),
    ];
    assert_eq!(
        untested_targets_gate(&metadata, &sources),
        untested_refused(&[
            "crates/x/src/main.rs:4: declares a module, but its target turns its tests off \
             (`test = false`): only a root that is all its code may, so that no test hides in it",
            "crates/x/src/lib.rs:2: declares a module, but its target turns its tests off \
             (`test = false`): only a root that is all its code may, so that no test hides in it",
            "crates/x/src/bin/b.rs: the root of a target without tests, not among the Rust \
             files read",
            "crates/x/src/bin/c.rs:1: declares a module, but its target turns its tests off \
             (`test = false`): only a root that is all its code may, so that no test hides in it",
        ]),
        "a test module declared, a library's modules, a root never read, a module after a \
         nested comment"
    );
}

#[test]
fn refuses_every_way_a_manifest_offers_to_leave_tests_unrun() {
    let metadata = r#"{"workspace_root":"/r","packages":[{"name":"x","manifest_path":"/r/crates/x/Cargo.toml","targets":[
        {"kind":["test"],"src_path":"/r/crates/x/tests/off.rs","test":false,"doctest":false},
        {"kind":["test"],"src_path":"/r/crates/x/tests/featured.rs","test":true,"doctest":false,"required-features":["never"]},
        {"kind":["test"],"src_path":"/r/crates/x/tests/run.rs","test":true,"doctest":false,"required-features":[]},
        {"kind":["test"],"src_path":"/r/crates/x/tests/dir/main.rs","test":true,"doctest":false},
        {"kind":["test"],"src_path":"/r/crates/x/checks/elsewhere.rs","test":true,"doctest":false},
        {"kind":["bin"],"src_path":"/r/crates/x/tests/bin.rs","test":true,"doctest":false}
    ]}]}"#;
    let source = |path: &str| (path.to_owned(), "#[test]\nfn t() { panic!() }\n".to_owned());
    let sources = [
        source("crates/x/tests/off.rs"),
        source("crates/x/tests/featured.rs"),
        source("crates/x/tests/run.rs"),
        source("crates/x/tests/dir/main.rs"),
        source("crates/x/tests/dir/helper.rs"),
        source("crates/x/tests/fixtures/a/main.rs"),
        source("crates/x/tests/fixtures/a/tests/b.rs"),
        source("crates/x/tests/undeclared.rs"),
        source("crates/x/tests/gone/main.rs"),
        source("crates/x/tests/bin.rs"),
        source("crates/x/tests/notes.txt"),
        source("crates/x/checks/elsewhere.rs"),
        source("crates/xy/tests/other.rs"),
    ];
    let never = "its tests would never run";
    let discovered = "a test file cargo discovers, but no tested test target's root \
                      (`autotests = false`, a target moved or turned off): its tests would \
                      never run";
    assert_eq!(
        untested_targets_gate(metadata, &sources),
        untested_refused(&[
            "crates/x/tests/off.rs: a test target that turns its tests off (`test = false`): \
             they would never run",
            &format!(
                "crates/x/tests/featured.rs: its target needs features (`required-features`), \
                 without which `cargo test` skips it: {never}"
            ),
            &format!("crates/x/tests/off.rs: {discovered}"),
            &format!("crates/x/tests/undeclared.rs: {discovered}"),
            &format!("crates/x/tests/gone/main.rs: {discovered}"),
            &format!("crates/x/tests/bin.rs: {discovered}"),
        ]),
        "a test target turned off (its root declares no module), one needing features, and \
         files cargo discovers that no tested test target builds; a test target elsewhere, \
         a helper beside a test's main, fixtures below a directory and another crate's files \
         are fine"
    );
}

#[test]
fn refuses_a_library_without_doc_tests() {
    for kind in ["lib", "rlib"] {
        let metadata = targets_metadata(&[(kind, "crates/x/src/lib.rs", true, false)]);
        assert_eq!(
            untested_targets_gate(&metadata, &[]),
            untested_refused(&[
                "crates/x/src/lib.rs: its doc tests are turned off (`doctest = false`): they \
                 would never run"
            ]),
            "{kind}"
        );
    }
    let cdylib = targets_metadata(&[("cdylib", "crates/x/src/lib.rs", true, false)]);
    assert_eq!(
        untested_targets_gate(&cdylib, &[]),
        Ok(()),
        "a C library alone has no doc tests"
    );
}

/// The problem of the untested targets gate with a target of `kind` rooted at `root`.
fn refused_kind(root: &str, kind: &str) -> String {
    format!(
        "{root}: a `{kind}` target, a kind outside `TARGET_KINDS`: the checks prove only \
         those (what a procedural macro or a build script generates or runs at build time no \
         gate reads)"
    )
}

#[test]
fn refuses_a_target_of_a_kind_the_workspace_does_not_build() {
    for (kind, root) in [
        ("proc-macro", "crates/x/src/lib.rs"),
        ("custom-build", "crates/x/build.rs"),
        ("staticlib", "crates/x/src/lib.rs"),
        ("bench", "crates/x/benches/b.rs"),
    ] {
        let metadata = targets_metadata(&[
            ("lib", "crates/x/src/lib.rs", true, true),
            (kind, root, false, false),
        ]);
        assert_eq!(
            untested_targets_gate(&metadata, &[]),
            untested_refused(&[&refused_kind(root, kind)]),
            "{kind}"
        );
    }
    let metadata = r#"{"workspace_root":"/r","packages":[{"name":"x","manifest_path":"/r/crates/x/Cargo.toml","targets":[
        {"kind":["cdylib","rlib","dylib"],"src_path":"/r/crates/x/src/lib.rs","test":true,"doctest":true}
    ]}]}"#;
    assert_eq!(
        untested_targets_gate(metadata, &[]),
        untested_refused(&[&refused_kind("crates/x/src/lib.rs", "dylib")]),
        "a kind beside accepted ones"
    );
}

#[test]
fn refuses_a_procedural_macro_and_a_build_script_cargo_reports() {
    let dir = scratch("target-kinds");
    for (path, text) in [
        (
            "Cargo.toml",
            "[workspace]\nmembers = [\"m\", \"b\"]\nresolver = \"3\"\n",
        ),
        (
            "m/Cargo.toml",
            "[package]\nname = \"m\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[lib]\nproc-macro = true\n",
        ),
        ("m/src/lib.rs", "//! M.\n"),
        (
            "b/Cargo.toml",
            "[package]\nname = \"b\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
        ),
        ("b/build.rs", "fn main() {}\n"),
        ("b/src/lib.rs", "//! B.\n"),
    ] {
        let file = dir.join(path);
        std::fs::create_dir_all(file.parent().expect("parent")).expect("directory");
        std::fs::write(file, text).expect("file");
    }
    let metadata = system(dir.path())
        .capture(&crate::metadata_command(CargoWorkspace::Root))
        .expect("cargo metadata");
    let sources = [
        ("m/src/lib.rs".to_owned(), "//! M.\n".to_owned()),
        ("b/src/lib.rs".to_owned(), "//! B.\n".to_owned()),
        ("b/build.rs".to_owned(), "fn main() {}\n".to_owned()),
    ];
    assert_eq!(
        untested_targets_gate(&metadata, &sources),
        untested_refused(&[
            &refused_kind("m/src/lib.rs", "proc-macro"),
            &refused_kind("b/build.rs", "custom-build"),
        ]),
        "both kinds as cargo reports them"
    );
}

#[test]
fn refuses_metadata_it_cannot_read_whole() {
    let gate = |metadata: &str| untested_targets_gate(metadata, &[]);
    assert_eq!(
        gate("{"),
        failed("invalid cargo metadata: EOF while parsing an object at line 1 column 1"),
        "not JSON"
    );
    assert_eq!(
        gate(r#"{"packages":[]}"#),
        failed("cargo metadata names no workspace root"),
        "no root"
    );
    assert_eq!(
        gate(r#"{"workspace_root":"/r","packages":[{"name":"x"}]}"#),
        failed("cargo metadata lists no targets of x"),
        "no targets"
    );
    let unexpected = "x: a target cargo metadata does not describe as expected (`test`, \
                      `doctest`, a source below the workspace root)";
    for target in [
        r#"{"kind":["bin"],"src_path":"/r/a.rs","doctest":false}"#,
        r#"{"kind":["bin"],"src_path":"/r/a.rs","test":false}"#,
        r#"{"kind":["bin"],"src_path":"/elsewhere/a.rs","test":false,"doctest":false}"#,
        r#"{"kind":["bin"],"src_path":"/ra.rs","test":false,"doctest":false}"#,
        r#"{"kind":["bin"],"test":false,"doctest":false}"#,
    ] {
        assert_eq!(
            gate(&format!(
                r#"{{"workspace_root":"/r","packages":[{{"name":"x","manifest_path":"/r/Cargo.toml","targets":[{target}]}}]}}"#
            )),
            untested_refused(&[unexpected]),
            "{target}"
        );
    }
    let unplaced = "x: a package cargo metadata does not describe as expected (a \
                    `manifest_path` below the workspace root)";
    for package in [
        r#""name":"x""#,
        r#""name":"x","manifest_path":"/elsewhere/Cargo.toml""#,
        r#""name":"x","manifest_path":"/r/x/Cargo.tom""#,
        r#""name":"x","manifest_path":"/r/xCargo.toml""#,
        r#""name":"x","manifest_path":7"#,
    ] {
        assert_eq!(
            gate(&format!(
                r#"{{"workspace_root":"/r","packages":[{{{package},"targets":[]}}]}}"#
            )),
            untested_refused(&[unplaced]),
            "{package}"
        );
    }
}

#[test]
fn reads_the_targets_of_real_cargo_metadata() {
    let dir = scratch("untested-targets");
    std::fs::create_dir_all(dir.join("src")).expect("src");
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"a\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[lib]\npath = \"src/lib.rs\"\ndoctest = false\n\n[[bin]]\nname = \"a\"\npath = \"src/main.rs\"\ntest = false\n",
    )
    .expect("manifest");
    let metadata = system(dir.path())
        .capture(&crate::metadata_command(CargoWorkspace::Root))
        .expect("cargo metadata");
    let sources = [
        ("src/lib.rs".to_owned(), "//! A.\n".to_owned()),
        (
            "src/main.rs".to_owned(),
            "mod b;\nfn main() {}\n".to_owned(),
        ),
    ];
    assert_eq!(
        untested_targets_gate(&metadata, &sources),
        untested_refused(&[
            "src/lib.rs: its doc tests are turned off (`doctest = false`): they would never run",
            "src/main.rs:1: declares a module, but its target turns its tests off (`test = \
             false`): only a root that is all its code may, so that no test hides in it",
        ]),
        "both settings as cargo reports them"
    );
}

#[test]
fn accepts_a_build_variable_naming_no_file() {
    let mut sources = test_data_reader_sources();
    sources.push((
        "crates/audio-hash/src/lib.rs".to_owned(),
        "const B: Option<&str> = option_env!(\"POYETIK_ENGINE_BLOCK\");\n".to_owned(),
    ));
    assert_eq!(
        test_data_gate(&sources),
        Ok(()),
        "the engine block a build names"
    );
}

#[test]
fn refuses_a_compile_fail_doc_test_that_names_no_error_code() {
    let gate = |text: &str| {
        crate::compile_fail_codes_gate(&parsed(&[(
            "crates/a/src/lib.rs".to_owned(),
            text.to_owned(),
        )]))
    };
    let fence = "```";
    for (form, text) in [
        (
            "alone",
            format!("/// {fence}compile_fail\n/// x\n/// {fence}\nfn f() {{}}\n"),
        ),
        (
            "after the language",
            format!("/// {fence}rust,compile_fail\n/// x\n/// {fence}\nfn f() {{}}\n"),
        ),
        (
            "before an edition",
            format!("/// {fence}compile_fail,edition2024\n/// x\n/// {fence}\nfn f() {{}}\n"),
        ),
        (
            "split at a space",
            format!("/// {fence} rust compile_fail\n/// x\n/// {fence}\nfn f() {{}}\n"),
        ),
        (
            "in tildes",
            "/// ~~~compile_fail\n/// x\n/// ~~~\nfn f() {}\n".to_owned(),
        ),
        (
            "of a module",
            format!("//! {fence}compile_fail\n//! x\n//! {fence}\n"),
        ),
        (
            "in a block",
            format!("/**\n * {fence}compile_fail\n * x\n * {fence}\n */\nfn f() {{}}\n"),
        ),
        (
            "in an attribute",
            format!("#[doc = \"{fence}compile_fail\\nx\\n{fence}\"]\nfn f() {{}}\n"),
        ),
        (
            "in an inner attribute",
            format!("#![doc = r\"{fence}compile_fail\"]\n"),
        ),
        (
            "a code of three digits",
            format!("/// {fence}compile_fail,E059\n/// x\n/// {fence}\nfn f() {{}}\n"),
        ),
        (
            "a code of four characters, not all digits",
            format!("/// {fence}compile_fail,E05x9\n/// x\n/// {fence}\nfn f() {{}}\n"),
        ),
        (
            "after a closed block",
            format!(
                "/// {fence}\n/// x\n/// {fence}\n/// {fence}compile_fail\n/// y\n/// {fence}\nfn f() {{}}\n"
            ),
        ),
        (
            "after a block left open by another item",
            format!(
                "/// {fence}\n/// x\nfn f() {{}}\n/// {fence}compile_fail\n/// y\n/// {fence}\nfn g() {{}}\n"
            ),
        ),
    ] {
        let line = text
            .lines()
            .position(|line| line.contains("compile_fail"))
            .map(|at| at.saturating_add(1))
            .expect("a line");
        assert_eq!(
            gate(&text),
            step_found(&[format!(
                "crates/a/src/lib.rs:{line}: a compile_fail doc test that names no error code \
                 (`compile_fail,E…`), so rustdoc accepts it whatever it fails with: name the \
                 code it fails with"
            )]),
            "{form}: {text}"
        );
    }
}

#[test]
fn accepts_a_compile_fail_doc_test_naming_its_code_and_every_text_that_is_none() {
    let gate = |text: &str| {
        crate::compile_fail_codes_gate(&parsed(&[(
            "crates/a/src/lib.rs".to_owned(),
            text.to_owned(),
        )]))
    };
    let fence = "```";
    for (form, text) in [
        (
            "its code named",
            format!("/// {fence}compile_fail,E0599\n/// x\n/// {fence}\nfn f() {{}}\n"),
        ),
        (
            "its codes named",
            format!("/// {fence}rust,compile_fail,E0599,E0425\n/// x\n/// {fence}\nfn f() {{}}\n"),
        ),
        (
            "a block that compiles",
            format!("/// {fence}rust\n/// x\n/// {fence}\nfn f() {{}}\n"),
        ),
        (
            "inside a block",
            format!(
                "/// {fence}text\n/// {fence}compile_fail is a word here\n/// {fence}\nfn f() {{}}\n"
            ),
        ),
        (
            "past a shorter fence inside a longer one",
            "/// ````\n/// ```\n/// ```compile_fail\n/// ````\nfn f() {}\n".to_owned(),
        ),
        (
            "past a fence of the other mark",
            "/// ~~~\n/// ```\n/// ```compile_fail\n/// ~~~\nfn f() {}\n".to_owned(),
        ),
        (
            "past a fence followed by words, which closes nothing",
            "/// ```\n/// ```rust\n/// ```compile_fail\n/// ```\nfn f() {}\n".to_owned(),
        ),
        (
            "inside a longer fence",
            "/// ````\n/// ```compile_fail\n/// ````\nfn f() {}\n".to_owned(),
        ),
        (
            "in prose",
            "/// a `compile_fail` doc test\nfn f() {}\n".to_owned(),
        ),
        (
            "in a comment",
            format!("// {fence}compile_fail\nfn f() {{}}\n"),
        ),
        (
            "in a string",
            format!("const A: &str = \"{fence}compile_fail\";\n"),
        ),
        (
            "the value of no string",
            "#[doc = concat!(\"a\", \"b\")]\nfn f() {}\n".to_owned(),
        ),
        (
            "no doc attribute",
            "#[must_use]\nfn f() -> u8 { 0 }\n".to_owned(),
        ),
        (
            "a `#` that opens no attribute",
            format!("macro_rules! m {{\n    (# (doc = \"{fence}compile_fail\")) => {{}};\n}}\n"),
        ),
    ] {
        assert_eq!(gate(&text), Ok(()), "{form}: {text}");
    }
}
