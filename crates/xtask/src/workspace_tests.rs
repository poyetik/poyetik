//! Tests of the workspace as the checks see it: its crates, packages, targets, paths and modes.

use core::num::NonZeroUsize;

use crate::tests::{CHANGES_MODE, FAKE_JOBS, failed};
use crate::{
    Action, Cmd, Dir, Mode, NATIVE_TARGETS, PACKAGES, PORTABILITY_TARGETS, RustCrate, check_steps,
};

#[test]
fn builds_and_lints_for_every_target() {
    assert_eq!(PORTABILITY_TARGETS.len(), 4, "four portability targets");
    assert_eq!(
        NATIVE_TARGETS,
        ["aarch64-apple-darwin", "x86_64-unknown-linux-gnu"],
        "both architectures of the native shell"
    );
}

#[test]
fn runs_half_the_threads_as_mutation_jobs() {
    for (threads, jobs) in [(1, 1), (2, 1), (3, 1), (4, 2), (5, 2), (6, 3), (16, 8)] {
        assert_eq!(
            crate::mutation_jobs(NonZeroUsize::new(threads).expect("not zero")).get(),
            jobs,
            "{threads} threads"
        );
    }
}

#[test]
fn measures_each_crate_by_its_own_unit_tests() {
    let lines: Vec<String> = check_steps(CHANGES_MODE)
        .into_iter()
        .filter_map(|step| {
            if let Action::Coverage(coverage) = step.action {
                Some(coverage.command().line())
            } else {
                None
            }
        })
        .collect();
    let mut expected: Vec<String> = crate::RUST_CRATES
        .iter()
        .map(|krate| {
            format!(
                "cargo +nightly-2026-11-01 llvm-cov --manifest-path {}/Cargo.toml --lib --locked \
                 --branch --json --summary-only --output-path target/coverage/{}.json -- --include-ignored",
                krate.dir, krate.name
            )
        })
        .collect();
    expected.extend(
        ["visual-cli", "visual-gpu", "audio-cli", "xtask"]
            .map(|name| format!(
                "cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/{name}/Cargo.toml \
                 --locked --branch --json --summary-only --output-path target/coverage/{name}-main.json -- --include-ignored"
            )),
    );
    assert_eq!(
        lines, expected,
        "each library by its unit tests alone, each binary's main.rs by its crate's tests"
    );
}

#[test]
fn empties_stale_build_outputs_before_the_builds_that_write_them() {
    let steps = check_steps(CHANGES_MODE);
    assert_eq!(
        crate::build_outputs(),
        vec![
            "bindings/audio-wasm/dist".to_owned(),
            "bindings/visual-wasm/dist".to_owned(),
            "bindings/web-page/dist".to_owned()
        ],
        "the outputs of tsc, which never removes a stale one"
    );
    for package in PACKAGES {
        for output in package.outputs {
            let path = format!("{}/{output}", package.dir);
            let emptied = steps.iter().position(
                |step| matches!(&step.action, Action::FreshDirs(paths) if paths.contains(&path)),
            );
            let built = steps
                .iter()
                .position(|step| step.name.starts_with(&format!("{}: ", package.step)));
            assert!(
                emptied.is_some() && emptied < built,
                "{path} is emptied before {} runs",
                package.step
            );
        }
    }
}

#[test]
fn builds_each_package_another_uses_before_the_packages_that_use_it() {
    assert_eq!(
        crate::needed_builds()
            .iter()
            .map(|package| package.dir)
            .collect::<Vec<_>>(),
        [
            "bindings/audio-wasm",
            "bindings/visual-wasm",
            "bindings/web-page"
        ],
        "the engine, the visual library and the page module, in that order"
    );
    let named = |dir: &str| {
        PACKAGES
            .into_iter()
            .find(|package| package.dir == dir)
            .expect("a package")
    };
    assert_eq!(
        named("apps/playground").needs,
        [
            crate::Setup::Build("bindings/audio-wasm"),
            crate::Setup::Build("bindings/visual-wasm"),
            crate::Setup::Build("bindings/web-page")
        ],
        "the playground uses the engine and the page module, which uses the visual library"
    );
    assert_eq!(
        named("bindings/web-page").needs,
        [crate::Setup::Build("bindings/visual-wasm")],
        "the page module uses the visual library"
    );
    for mode in [Mode::Full { jobs: FAKE_JOBS }, CHANGES_MODE] {
        let steps = check_steps(mode);
        let emptied = steps
            .iter()
            .position(|step| step.action == Action::FreshDirs(crate::build_outputs()));
        let built_at = |dir: &'static str| {
            steps.iter().position(|step| {
                step.action == Action::Run(Cmd::new("pnpm", &["run", "build"], Dir::Package(dir)))
                    && step.name == format!("Build of {dir}, for the packages that use it")
            })
        };
        for built in crate::needed_builds() {
            let at = built_at(built.dir);
            let first_user = PACKAGES
                .iter()
                .filter(|package| package.needs.contains(&crate::Setup::Build(built.dir)))
                .filter_map(|package| {
                    steps
                        .iter()
                        .position(|step| step.name.starts_with(&format!("{}: ", package.step)))
                })
                .min();
            assert!(
                emptied.is_some() && emptied < at && at < first_user,
                "{mode:?}: {} built after its outputs are emptied, before the first package \
                 using it",
                built.dir
            );
            for setup in built.needs {
                if let crate::Setup::Build(used) = setup {
                    assert!(
                        built_at(used) < at,
                        "{mode:?}: {used} built before {}, which uses it",
                        built.dir
                    );
                }
            }
        }
    }
}

#[test]
fn tells_the_files_of_a_crate_by_its_directory() {
    let core = RustCrate {
        name: "audio-core",
        dir: "crates/audio-core",
        binary: false,
        test_data: &[],
        roles: &[],
        workspace: crate::workspace::CargoWorkspace::Root,
    };
    for (path, held) in [
        ("crates/audio-core/src/lib.rs", Some("src/lib.rs")),
        ("crates/audio-core/", Some("")),
        ("crates/audio-core", None),
        ("crates/audio-core-x/a.rs", None),
        ("crates/audio-corea.rs", None),
        ("x/crates/audio-core/a.rs", None),
        ("crates/engine-cor/a.rs", None),
    ] {
        assert_eq!(core.holds(path), held, "{path}");
    }
}

#[test]
fn derives_each_list_of_crates_from_their_roles() {
    use crate::CrateRole;
    // Each list a fact written here, so that a role gained or lost is a decision of this
    // test too.
    for (role, crates, what) in [
        (
            CrateRole::Library,
            &[
                "audio-core",
                "audio-boxes",
                "audio-patch",
                "audio-hash",
                "audio-host",
                "audio-wasm",
            ][..],
            "the audio library",
        ),
        (
            CrateRole::LibraryTests,
            &["alloc-counter", "fpenv", "audio-battery", "audio-testkit"][..],
            "the counting allocator, the floating-point mode, the fingerprint battery and the test kit",
        ),
        (
            CrateRole::VisualLibrary,
            &[
                "visual-core",
                "visual-boxes",
                "visual-hash",
                "visual-host",
                "visual-gpu",
                "visual-wasm",
            ][..],
            "the visual library, its GPU renderer and its module",
        ),
        (
            CrateRole::VisualTool,
            &["visual-cli"][..],
            "the visual library's command line",
        ),
        (
            CrateRole::CaseReader,
            &["visual-cli"][..],
            "the conformance case reader whose dependencies are pinned by its role",
        ),
        (
            CrateRole::VisualLibraryTests,
            &["visual-testkit", "alloc-counter", "fpenv"][..],
            "the visual test kit, the counting allocator and the floating-point mode",
        ),
        (
            CrateRole::Libm,
            &["audio-core", "audio-testkit"][..],
            "the engine core and the test kit",
        ),
        (
            CrateRole::MathLayer,
            &["audio-core", "audio-boxes"][..],
            "the engine core and the boxes",
        ),
        (
            CrateRole::SharedTests,
            &[
                "visual-testkit",
                "alloc-counter",
                "fpenv",
                "audio-battery",
                "audio-testkit",
            ][..],
            "the counting allocator, the floating-point mode, the fingerprint battery and the test kit",
        ),
    ] {
        assert_eq!(crate::crates_in(role), crates, "{what}");
    }
}

/// The crates of the libraries not built for every portability target, and why: each
/// needs what a bare-metal target has not.
const NOT_PORTABLE: [(&str, &str); 3] = [
    (
        "audio-wasm",
        "the engine's WebAssembly interface: std, built for WebAssembly by its package",
    ),
    (
        "visual-wasm",
        "the visual library's WebAssembly interface: std, built for WebAssembly by its package",
    ),
    (
        "visual-gpu",
        "the visual library on the GPU: std and a GPU API (Metal, Vulkan, DirectX 12), in a workspace of its own",
    ),
];

#[test]
fn builds_every_crate_of_the_libraries_for_every_portability_target_but_its_shells() {
    let libraries: Vec<&str> = crate::LIBRARIES
        .iter()
        .flat_map(|library| crate::crates_in(library.crates))
        .collect();
    let shells: Vec<&str> = NOT_PORTABLE.iter().map(|(name, _)| *name).collect();
    let portable: Vec<&str> = libraries
        .iter()
        .copied()
        .filter(|name| !shells.contains(name))
        .chain(["coordinator"])
        .collect();
    assert_eq!(
        crate::PORTABILITY_PACKAGES.to_vec(),
        portable,
        "every crate of a library, but its shells, in the order of the crates, then the \
         coordinating layer above both, linked into every product, and nothing else"
    );
    for (shell, why) in NOT_PORTABLE {
        assert!(
            libraries.contains(&shell),
            "{shell} ({why}) is a crate of a library"
        );
    }
}

#[test]
fn requires_every_root_of_the_audio_library_to_deny_its_lints() {
    use crate::{CrateRole, LIBRARY_LINTS, RUST_CRATES, RustSource, library_lints_gate};
    assert_eq!(
        LIBRARY_LINTS,
        [
            "#![deny(clippy::exhaustive_enums)]",
            "#![deny(clippy::exhaustive_structs)]"
        ],
        "the lints the changelog claims of the library"
    );
    let roots: Vec<String> = RUST_CRATES
        .iter()
        .filter(|krate| {
            krate.roles.contains(&CrateRole::Library)
                || krate.roles.contains(&CrateRole::VisualLibrary)
        })
        .map(|krate| format!("{}/src/lib.rs", krate.dir))
        .collect();
    for module in [
        "bindings/audio-wasm/src/lib.rs",
        "crates/visual-core/src/lib.rs",
        "bindings/visual-wasm/src/lib.rs",
    ] {
        assert!(roots.contains(&module.to_owned()), "{module} among them");
    }
    let with = |root: &str, text: &str| -> Vec<RustSource> {
        roots
            .iter()
            .map(|path| {
                let held = if path == root {
                    text.to_owned()
                } else {
                    "//! A crate.\n#![deny(clippy::exhaustive_enums)]\n#![deny(clippy::exhaustive_structs)]\n"
                        .to_owned()
                };
                RustSource::parse(path, &held).expect("valid tokens")
            })
            .collect()
    };
    let root = "bindings/audio-wasm/src/lib.rs";
    assert_eq!(
        library_lints_gate(&with(
            root,
            "//! A crate.\n#![doc(test(attr(x)))]\n#![deny( clippy :: exhaustive_enums )]\n#![deny(clippy::exhaustive_structs)]\npub mod ffi;\n"
        )),
        Ok(()),
        "among other leading attributes, spaced otherwise"
    );
    let refused = failed(format!(
        "the lints of the libraries: {root} holds no leading attribute exactly \
         `#![deny(clippy::exhaustive_enums)]` (one lint per attribute)"
    ));
    for text in [
        "//! A crate.\n#![deny(clippy::exhaustive_enums, clippy::exhaustive_structs)]\n#![deny(clippy::exhaustive_structs)]\n",
        "//! A crate.\n#![deny(clippy::exhaustive_structs)]\npub mod ffi;\n",
        "//! A crate.\n#![deny(clippy::exhaustive_structs)]\npub mod ffi;\n#![deny(clippy::exhaustive_enums)]\n",
        "//! A crate.\n#![deny(clippy::exhaustive_structs)]\n// #![deny(clippy::exhaustive_enums)]\npub mod ffi;\n",
        "//! A crate.\n#![deny(clippy::exhaustive_structs)]\n#![warn(clippy::exhaustive_enums)]\n",
    ] {
        assert_eq!(library_lints_gate(&with(root, text)), refused, "{text}");
    }
    assert_eq!(
        library_lints_gate(&with(
            root,
            "//! A crate.\n#![deny(clippy::exhaustive_enums)]\npub mod ffi;\n"
        )),
        failed(format!(
            "the lints of the libraries: {root} holds no leading attribute exactly \
             `#![deny(clippy::exhaustive_structs)]` (one lint per attribute)"
        )),
        "the structs' lint missing"
    );
    let mut missing = with(root, "");
    missing.retain(|source| source.path != root);
    assert_eq!(
        library_lints_gate(&missing),
        failed(format!(
            "the lints of the libraries: {root}: the root of audio-wasm is not read"
        )),
        "a root missing"
    );
}

#[test]
fn builds_every_no_std_crate_for_every_portability_target_or_says_why_not() {
    use crate::{NO_STD_NOT_PORTABLE, PORTABILITY_PACKAGES, RustSource, no_std_portability_gate};
    for (name, why) in NO_STD_NOT_PORTABLE {
        assert!(
            !PORTABILITY_PACKAGES.contains(&name) && !why.is_empty(),
            "{name}: not built for every target, with its reason"
        );
    }
    let sources = |texts: &[(&str, &str)]| -> Vec<RustSource> {
        texts
            .iter()
            .map(|(path, text)| RustSource::parse(path, text).expect("valid tokens"))
            .collect()
    };
    let portable = "#![no_std]\n#![forbid(unsafe_code)]\n";
    let fpenv = ("crates/fpenv/src/lib.rs", "//! The mode.\n#![no_std]\n");
    let mut held = vec![
        ("crates/coordinator/src/lib.rs", portable),
        ("crates/visual-core/src/lib.rs", portable),
        ("bindings/visual-wasm/src/lib.rs", "//! A shell with std.\n"),
        fpenv,
    ];
    assert_eq!(
        no_std_portability_gate(&sources(&held)),
        Ok(()),
        "every no_std crate built for every target or listed, a crate with std free"
    );
    held[2] = (
        "bindings/visual-wasm/src/lib.rs",
        "//! A shell.\n#![doc(test(attr(x)))]\n#![ no_std ]\n",
    );
    assert_eq!(
        no_std_portability_gate(&sources(&held)),
        failed(
            "the portability of the `no_std` crates: bindings/visual-wasm/src/lib.rs is \
             `no_std` but visual-wasm is not built for every portability target \
             (PORTABILITY_PACKAGES)"
        ),
        "a no_std crate neither built for every target nor listed, spaced otherwise"
    );
    held[2] = (
        "bindings/visual-wasm/src/lib.rs",
        "//! A shell.\n// #![no_std]\npub mod ffi;\n#![no_std]\n",
    );
    assert_eq!(
        no_std_portability_gate(&sources(&held)),
        Ok(()),
        "an attribute commented out or past the first item is no leading attribute"
    );
    held[3] = ("crates/fpenv/src/lib.rs", "//! The mode, with std now.\n");
    assert_eq!(
        no_std_portability_gate(&sources(&held)),
        failed(
            "the portability of the `no_std` crates: fpenv is listed in NO_STD_NOT_PORTABLE \
             but its root is not `no_std`"
        ),
        "a stale exception"
    );
    let _ = held.pop();
    assert_eq!(
        no_std_portability_gate(&sources(&held)),
        failed(
            "the portability of the `no_std` crates: crates/fpenv/src/lib.rs: the root of \
             fpenv, listed in NO_STD_NOT_PORTABLE, is not read"
        ),
        "an exception whose root is missing"
    );
}

#[test]
fn reads_no_std_in_its_one_form_and_at_a_crate_s_root_only() {
    use crate::{RustSource, no_std_portability_gate};
    let gate = |path: &str, text: &str| {
        no_std_portability_gate(&[
            RustSource::parse("crates/fpenv/src/lib.rs", "#![no_std]\n").expect("valid tokens"),
            RustSource::parse(path, text).expect("valid tokens"),
        ])
    };
    let root = "bindings/visual-wasm/src/lib.rs";
    // Each other spelling of `no_std` the language allows, which the gate would read as
    // `std`: refused, naming the one form.
    for text in [
        "//! A shell.\n#![cfg_attr(not(test), no_std)]\n",
        "#![cfg_attr(not(feature = \"std\"), no_std)]\n",
        "#![cfg_attr(all(), no_std, forbid(unsafe_code))]\n",
    ] {
        assert_eq!(
            gate(root, text),
            failed(format!(
                "the portability of the `no_std` crates: {root} declares `no_std` in another \
                 form than `#![no_std]`, the one form this gate reads"
            )),
            "{text}"
        );
    }
    assert_eq!(
        gate(
            root,
            "//! Its name in prose: no_std.\n#![doc = \"no_std\"]\n"
        ),
        Ok(()),
        "the word in a doc comment or a string is no attribute"
    );
    // A crate whose library root is not `src/lib.rs` (a `[lib] path`): its `no_std`, where
    // the gate reads none, refused.
    assert_eq!(
        gate("crates/coordinator/src/root.rs", "#![no_std]\n"),
        failed(
            "the portability of the `no_std` crates: crates/coordinator/src/root.rs declares \
             `no_std` but is not the root `src/lib.rs` of a crate of RUST_CRATES, the one root \
             this gate reads"
        ),
        "a no_std root elsewhere"
    );
}

#[test]
fn knows_the_kind_of_every_tracked_text_file() {
    // Finding F12-1 of the fourth deep review of visual block 2 (#34): the gates of code
    // each listed their own extensions, and two `.astro` files and every `.cts`, `.cjs` and
    // `.jsx` file to come escaped the hints gate. Every tracked text file is of a known kind
    // (`SOURCE_KINDS`, whose lists `reads_every_kind_its_predicate_names` holds).
    let paths =
        |list: &[&str]| -> Vec<String> { list.iter().map(|&path| path.to_owned()).collect() };
    assert_eq!(
        crate::source_kinds_gate(&paths(&[
            "apps/roadmap/src/pages/index.astro",
            "x.cts",
            "crates/a/src/lib.rs",
            "README.md",
            "pnpm-lock.yaml",
            "Cargo.lock",
            ".githooks/pre-push",
            "LICENSE-MIT",
            "apps/LICENSE",
            ".node-version",
            "crates/visual-core/src/display.wgsl",
        ])),
        Ok(()),
        "code and the other kinds, each with its reason"
    );
    let unknown = |path: &str| {
        format!(
            "{path}: a tracked text file of no known kind: add its pattern to SOURCE_KINDS, \
             with what of it is prose and whether it is code, or why no gate of code reads \
             it (crates/xtask/src/workspace.rs)"
        )
    };
    assert_eq!(
        crate::source_kinds_gate(&paths(&["tools/run.sh"])),
        failed(unknown("tools/run.sh"))
    );
    assert_eq!(
        crate::source_kinds_gate(&paths(&["Makefile", "x.rs", "LICENSE", "a.githooks"])),
        Err(crate::Failure {
            summary: "3 problems, listed above".to_owned(),
            details: vec![
                unknown("Makefile"),
                unknown("LICENSE"),
                unknown("a.githooks")
            ],
            usage: false,
        }),
        "no extension, a license outside a folder, a pattern's prefix as a suffix"
    );
}

#[test]
fn reads_every_kind_its_predicate_names() {
    // Findings H7-3, H12-1 and H17-2 of the fifth deep review of visual block 2 (#34): the
    // gates typed their own lists of files, and the vocabulary, the citation, the bypass and
    // the Stryker-disable gates each missed kinds of code and text. Every list is now a
    // filter of one table, `SOURCE_KINDS`; each is held here to the kinds it names.
    use crate::{Reading, SourceRole};
    let scripts = [
        "*.ts", "*.tsx", "*.mts", "*.cts", "*.js", "*.jsx", "*.mjs", "*.cjs",
    ];
    let code: Vec<&str> = core::iter::once("*.rs")
        .chain(scripts)
        .chain(["*.astro"])
        .collect();
    assert_eq!(crate::code_sources(), code, "Rust and every script kind");
    assert_eq!(
        crate::ts_sources(),
        code.get(1..).unwrap_or_default(),
        "every script kind"
    );
    assert_eq!(
        crate::stryker_sources(),
        scripts,
        "the Astro templates apart"
    );
    assert_eq!(
        crate::bypass_sources(),
        ["*mutants.toml", "*Cargo.toml"]
            .into_iter()
            .chain(code.iter().copied())
            .collect::<Vec<_>>(),
        "the configurations of the proofs and every code file"
    );
    assert_eq!(
        crate::vocabulary_sources(),
        core::iter::once("*.rs")
            .chain(scripts)
            .chain(["*.md", "spec/*.json", "conformance/*.json"])
            .collect::<Vec<_>>(),
        "Markdown and code, the Astro templates apart (#24 step 35)"
    );
    let unread = [
        "*.lock",
        "*.svg",
        "*.inc",
        ".node-version",
        "LICENSE-*",
        "*/LICENSE",
    ];
    assert_eq!(
        crate::citation_sources(),
        crate::source_kinds()
            .map(|kind| kind.pattern)
            .filter(|pattern| !unread.contains(pattern))
            .collect::<Vec<_>>(),
        "every kind a sentence can be written in"
    );
    let hash = [
        "*.toml",
        "*.yml",
        "*.yaml",
        "*.gitignore",
        "*.gitattributes",
        "*.editorconfig",
        ".githooks/*",
    ];
    for kind in crate::source_kinds() {
        let sample = kind.pattern.replace('*', "x");
        let reading = crate::reading_of(&sample);
        assert_eq!(reading, kind.reading, "{sample}");
        let expected = match kind.pattern {
            "*.md" => Reading::Markdown,
            "*.txt" => Reading::Text,
            "*.rs" => Reading::Rust,
            "*.json" => Reading::Json,
            "*.wgsl" | "*.html" | "*.css" => Reading::Comments,
            pattern if hash.contains(&pattern) => Reading::Hash,
            pattern if unread.contains(&pattern) => Reading::Nothing,
            _ => Reading::Script,
        };
        assert_eq!(reading, expected, "{sample}");
        assert_eq!(
            crate::cli::mutable_source(&sample),
            scripts.contains(&kind.pattern),
            "{sample}"
        );
        assert_eq!(
            crate::logical_text::prose_line(&sample, "# a b", false).map(|(found, _)| found),
            match reading {
                Reading::Markdown | Reading::Text => Some(crate::logical_text::ProseKind::Text),
                Reading::Hash => Some(crate::logical_text::ProseKind::Hash),
                Reading::Rust
                | Reading::Script
                | Reading::Json
                | Reading::Comments
                | Reading::Nothing => None,
            },
            "{sample}"
        );
        match kind.role {
            SourceRole::Other(reason) => assert!(!reason.is_empty(), "{sample}"),
            SourceRole::Rust | SourceRole::Script { .. } => {
                assert!(code.contains(&kind.pattern), "{sample}");
            }
        }
    }
    assert_eq!(crate::reading_of("Makefile"), Reading::Nothing, "no kind");
}

#[test]
fn lists_no_kind_of_source_by_hand() {
    // Lesson of findings H7-3, H12-1 and H17-2 of the fifth deep review of visual block 2
    // (#34): a gate that typed its own list of extensions missed kinds. No call listing
    // tracked files names two kinds of code of `SOURCE_KINDS` itself (one kind it reads
    // alone, or kinds of configuration, are its choice): a list of kinds of code is a filter
    // of the table. Read from the tokens of every module that lists files.
    let patterns = crate::code_sources();
    let typed = |text: &str| -> Vec<Vec<String>> {
        let tokens = crate::rust_source::tokens_of(text);
        tokens
            .iter()
            .enumerate()
            .filter(|(_, token)| token.is_ident("list_tracked"))
            .filter_map(|(at, _)| {
                let open = at.saturating_add(1);
                let _group = tokens.get(open).filter(|group| group.opens('('))?;
                let end = crate::rust_source::close_of(&tokens, open);
                let kinds: Vec<String> = tokens
                    .get(open..end)
                    .unwrap_or_default()
                    .iter()
                    .filter(|token| token.kind == crate::rust_source::TokenKind::Literal)
                    .map(|token| token.text.trim_matches('"').to_owned())
                    .filter(|literal| patterns.contains(&literal.as_str()))
                    .collect();
                (kinds.len() > 1).then_some(kinds)
            })
            .collect()
    };
    assert_eq!(
        typed(
            "fn f() { list_tracked(runner, &[\"*.ts\", \"*\", \"*.tsx\"], \"the files\"); \
             list_tracked(runner, &[\"*.rs\", \"*.wgsl\"], \"Rust\"); list_tracked }"
        ),
        [["*.ts", "*.tsx"]],
        "two kinds of code typed in a call are found; one kind of code beside another kind, \
         another pattern and a call's name alone not"
    );
    for (module, text) in [
        ("actions.rs", include_str!("actions.rs")),
        ("changes.rs", include_str!("changes.rs")),
        ("loaded_files.rs", include_str!("loaded_files.rs")),
    ] {
        assert_eq!(typed(text), Vec::<Vec<String>>::new(), "{module}");
    }
}
