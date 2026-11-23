//! Tests of the loaded-files gate, over the files rustc itself read for every build of the check.

use crate::tests::{artifact, dep_info, failed, fixture, passing, scratch, system};
use crate::{Action, Cmd, Dir, RustCrate, Step, run_steps};

#[test]
fn reads_the_files_a_dep_info_file_lists() {
    let text = "/t/deps/x-1.d: a/src/lib.rs a/src/my\\ file.rs ../../spec/v.json\n\n/t/deps/x-1: a/src/lib.rs a/src/my\\ file.rs ../../spec/v.json\n\na/src/lib.rs:\na/src/my\\ file.rs:\n../../spec/v.json:\n\n# env-dep:CARGO_MANIFEST_DIR=/r/a\n# checksum:x\n";
    assert_eq!(
        crate::loaded_files::dep_info_files(text),
        Ok(vec![
            "a/src/lib.rs".to_owned(),
            "a/src/my file.rs".to_owned(),
            "../../spec/v.json".to_owned()
        ]),
        "each file once, a space unescaped, the targets and comments apart"
    );
    assert_eq!(
        crate::loaded_files::dep_info_files("x-1: a.rs\n\na.rs:\nb.rs\n"),
        Err("a line not as rustc writes dep-info: b.rs".to_owned()),
        "a line of no known form"
    );
    for (file, expected) in [
        ("crates/a/src/lib.rs", "crates/a/src/lib.rs"),
        ("/r/crates/a/src/lib.rs", "crates/a/src/lib.rs"),
        ("crates/a/src/../../../spec/x.json", "spec/x.json"),
        ("crates/a/./src/lib.rs", "crates/a/src/lib.rs"),
        ("../outside/x.rs", "../outside/x.rs"),
        ("crates/../../../x.rs", "../../x.rs"),
        ("/rx/a.rs", "/rx/a.rs"),
        ("/elsewhere/a/../b.rs", "/elsewhere/b.rs"),
        ("/../x.rs", "/../x.rs"),
    ] {
        assert_eq!(
            crate::loaded_files::normalized(file, "/r"),
            expected,
            "{file}"
        );
    }
}

#[test]
fn names_each_file_a_build_of_another_workspace_reads_from_the_root() {
    // rustc names a file relative to the root of the workspace it builds, the GPU
    // workspace's own for its builds; the gates name every file from the repository's root.
    let gpu = crate::CargoWorkspace::Gpu;
    for (file, expected) in [
        ("src/lib.rs", "crates/visual-gpu/src/lib.rs"),
        ("tests/shaders.rs", "crates/visual-gpu/tests/shaders.rs"),
        (
            "../visual-core/src/display.rs",
            "crates/visual-core/src/display.rs",
        ),
        (
            "/r/crates/visual-gpu/src/bench.rs",
            "crates/visual-gpu/src/bench.rs",
        ),
        ("/elsewhere/a.rs", "/elsewhere/a.rs"),
    ] {
        assert_eq!(
            crate::loaded_files::named_from_the_root(file, "/r", gpu),
            expected,
            "{file}"
        );
    }
    assert_eq!(
        crate::loaded_files::named_from_the_root(
            "crates/a/src/lib.rs",
            "/r",
            crate::CargoWorkspace::Root
        ),
        "crates/a/src/lib.rs",
        "the root's builds name their files from the root already"
    );
}

#[test]
fn tells_each_build_and_its_dep_info_file_from_cargo_s_messages() {
    let core = crate::RUST_CRATES[0];
    let read = |line: &str, tests_only: bool| {
        crate::loaded_files::built_artifact(line, tests_only, &crate::RUST_CRATES)
            .map_err(|failure| failure.summary)
    };
    let found = |target: &str, dep_info: &str, test: bool| {
        Ok(Some(crate::loaded_files::Artifact {
            krate: core,
            root: "/r".to_owned(),
            target: target.to_owned(),
            dep_info: dep_info.to_owned(),
            of_tests: test,
        }))
    };
    for (output, target, test, dep_info) in [
        (
            "/t/deps/libaudio_core-1.rmeta",
            "audio_core",
            false,
            "/t/deps/audio_core-1.d",
        ),
        (
            "/t/deps/audio_core-2",
            "audio_core",
            true,
            "/t/deps/audio_core-2.d",
        ),
        (
            "/t/deps/plan_laws-3.exe",
            "plan_laws",
            true,
            "/t/deps/plan_laws-3.d",
        ),
        (
            "/t/deps/libaudio_cli-4.rmeta",
            "audio-cli",
            false,
            "/t/deps/audio_cli-4.d",
        ),
    ] {
        for tests_only in [false, true] {
            assert_eq!(
                read(&artifact(core.dir, target, test, output), tests_only),
                if test || !tests_only {
                    found(target, dep_info, test)
                } else {
                    Ok(None)
                },
                "{output}, only builds of tests read: {tests_only}"
            );
        }
    }
    assert_eq!(
        read(r#"{"reason":"build-finished"}"#, false),
        Ok(None),
        "no build"
    );
    assert_eq!(
        read(
            r#"{"reason":"compiler-artifact","package_id":"registry+x#serde@1.0.0"}"#,
            false
        ),
        Ok(None),
        "a package of a registry"
    );
}

#[test]
fn refuses_a_cargo_message_it_does_not_read() {
    let core = crate::RUST_CRATES[0];
    let read = |line: &str, tests_only: bool| {
        crate::loaded_files::built_artifact(line, tests_only, &crate::RUST_CRATES)
            .map_err(|failure| failure.summary)
    };
    let built = artifact(core.dir, "audio_core", true, "/t/deps/audio_core-2");
    let unread =
        |what: &str, line: &str| Err(format!("cannot read cargo's message {what}: {line}"));
    for (what, line) in [
        ("(not JSON)", "Compiling x".to_owned()),
        (
            "(no package_id)",
            r#"{"reason":"compiler-artifact"}"#.to_owned(),
        ),
        (
            "(no profile)",
            built.replace(r#""profile":{"test":true},"#, ""),
        ),
        (
            "(no manifest_path)",
            built.replace(r#""manifest_path":"/r/crates/audio-core/Cargo.toml","#, ""),
        ),
        (
            "(no target name)",
            built.replace(r#""name":"audio_core""#, r#""kind":[]"#),
        ),
        (
            "(no output)",
            built.replace(r#"["/t/deps/audio_core-2"]"#, "[]"),
        ),
        (
            "(an output with no directory)",
            built.replace("/t/deps/audio_core-2", "audio_core-2"),
        ),
        (
            "(an output not named <crate>-<hash>)",
            built.replace("/t/deps/audio_core-2", "/t/deps/audio_corex-2"),
        ),
    ] {
        assert_eq!(read(&line, true), unread(what, &line), "{what}");
    }
    let foreign = built.replace(
        "/r/crates/audio-core/Cargo.toml",
        "/r/crates/other/Cargo.toml",
    );
    assert_eq!(
        read(&foreign, true),
        Err("/r/crates/other/Cargo.toml: a package of no crate of the checks".to_owned()),
        "a local package the checks do not know"
    );
}

#[test]
fn refuses_a_file_rustc_loads_where_no_gate_looks() {
    let [core, objects] = [crate::RUST_CRATES[0], crate::RUST_CRATES[1]];
    let build = |krate: RustCrate, of_tests: bool, files: &[&str]| crate::Loaded {
        krate,
        target: krate.name.replace('-', "_"),
        configuration: None,
        of_tests,
        files: files.iter().map(|file| (*file).to_owned()).collect(),
    };
    let tracked: Vec<String> = [
        "crates/audio-core/src/lib.rs",
        "crates/audio-core/src/plan_tests.rs",
        "crates/audio-core/src/helper.rs",
        "crates/audio-core/tests/laws.rs",
        "crates/audio-boxes/src/lib.rs",
    ]
    .map(str::to_owned)
    .into();
    let library = build(core, false, &["crates/audio-core/src/lib.rs"]);
    let tests = build(
        core,
        true,
        &[
            "crates/audio-core/src/lib.rs",
            "crates/audio-core/src/plan_tests.rs",
            "crates/audio-core/tests/laws.rs",
            "conformance/test-matrix.json",
        ],
    );
    assert_eq!(
        crate::loaded_files_gate(&[library.clone(), tests], &tracked),
        Ok(()),
        "code, test code and declared test data"
    );
    let gate = |builds: &[crate::Loaded]| {
        crate::loaded_files_gate(builds, &tracked).map_err(|failure| failure.summary)
    };
    let refused = |problem: &str| Err(problem.to_owned());
    assert_eq!(
        gate(&[
            library,
            build(core, true, &["crates/audio-core/src/helper.rs"])
        ]),
        refused(
            "crates/audio-core/src/helper.rs: read only by the test build of `audio_core` \
             (audio-core), but not named as test code (tests/, a *_tests.rs file): a module \
             brought in by a macro or an attribute"
        ),
        "a file only the tests load, named as code"
    );
    assert_eq!(
        gate(&[
            build(
                core,
                false,
                &[
                    "crates/audio-core/src/lib.rs",
                    "crates/audio-core/src/helper.rs"
                ]
            ),
            build(core, true, &["crates/audio-core/src/helper.rs"]),
        ]),
        Ok(()),
        "a file of the code, which the tests load too"
    );
    assert_eq!(
        gate(&[
            build(objects, false, &["crates/audio-boxes/src/lib.rs"]),
            build(core, true, &["crates/audio-boxes/src/lib.rs"])
        ]),
        refused(
            "crates/audio-boxes/src/lib.rs: read by the test build of `audio_core` \
             (audio-core), outside its crate and its declared test data"
        ),
        "another crate's code, which its build reads: not this crate's"
    );
}

#[test]
fn accepts_a_validated_shader_of_src_as_code_and_no_other() {
    let core = crate::RUST_CRATES
        .iter()
        .copied()
        .find(|krate| krate.name == "visual-core")
        .expect("the visual library's core");
    let build = |of_tests: bool, files: &[&str]| crate::Loaded {
        krate: core,
        target: "visual_core".to_owned(),
        configuration: None,
        of_tests,
        files: files.iter().map(|file| (*file).to_owned()).collect(),
    };
    let tracked: Vec<String> = [
        "crates/visual-core/src/lib.rs",
        "crates/visual-core/src/display.wgsl",
        "crates/visual-core/src/other.wgsl",
    ]
    .map(str::to_owned)
    .into();
    let shader = [
        "crates/visual-core/src/lib.rs",
        "crates/visual-core/src/display.wgsl",
    ];
    assert_eq!(
        crate::loaded_files_gate(&[build(false, &shader), build(true, &shader)], &tracked),
        Ok(()),
        "a validated shader of src/, tracked: code"
    );
    assert_eq!(
        crate::loaded_files::VALIDATED_SHADERS,
        [
            "crates/visual-boxes/src/square.wgsl",
            "crates/visual-core/src/display.wgsl"
        ],
        "the shaders visual-gpu's tests validate"
    );
    let untracked: Vec<String> = vec!["crates/visual-core/src/lib.rs".to_owned()];
    for (file, listed, why) in [
        (
            "crates/visual-core/src/display.wgsl",
            &untracked,
            "untracked",
        ),
        (
            "crates/visual-core/src/other.wgsl",
            &tracked,
            "not validated",
        ),
        (
            "crates/visual-core/src/display.wgsl.txt",
            &tracked,
            "not a shader",
        ),
    ] {
        assert_eq!(
            crate::loaded_files_gate(
                &[build(false, &["crates/visual-core/src/lib.rs", file])],
                listed
            )
            .map_err(|failure| failure.summary),
            Err(format!(
                "{file}: read by the build of `visual_core` (visual-core), not a Rust file: no \
                 gate reads it as code, and only a build of tests reads data, from its \
                 crate's declared test data"
            )),
            "{why}"
        );
    }
}

#[test]
fn refuses_a_file_rustc_loads_outside_its_crate_or_git() {
    let core = crate::RUST_CRATES[0];
    let build = |krate: RustCrate, of_tests: bool, files: &[&str]| crate::Loaded {
        krate,
        target: krate.name.replace('-', "_"),
        configuration: None,
        of_tests,
        files: files.iter().map(|file| (*file).to_owned()).collect(),
    };
    let gate = |builds: &[crate::Loaded]| {
        crate::loaded_files_gate(builds, &[]).map_err(|failure| failure.summary)
    };
    let refused = |problem: &str| Err(problem.to_owned());
    assert_eq!(
        gate(&[build(core, false, &["conformance/test-matrix.json"])]),
        refused(
            "conformance/test-matrix.json: read by the build of `audio_core` (audio-core), \
             outside its crate"
        ),
        "test data read by the code"
    );
    assert_eq!(
        gate(&[build(core, true, &["spec/x.json"])]),
        refused(
            "spec/x.json: read by the test build of `audio_core` (audio-core), outside its \
             crate and its declared test data"
        ),
        "data no declaration names"
    );
    assert_eq!(
        gate(&[build(core, false, &["crates/audio-core/src/new.rs"])]),
        refused(
            "crates/audio-core/src/new.rs: read by the build of `audio_core` (audio-core), \
             but not tracked by git, so no gate read it (git add it)"
        ),
        "a Rust file git does not track"
    );
    assert_eq!(
        gate(&[build(core, true, &["crates/audio-core/tests/new_tests.rs"])]),
        refused(
            "crates/audio-core/tests/new_tests.rs: read by the test build of `audio_core` \
             (audio-core), but not tracked by git, so no gate read it (git add it)"
        ),
        "untracked test code"
    );
}

/// The crates of the loaded-files fixture: `configured`, then `loaded`.
const LOADED_FIXTURE_CRATES: [RustCrate; 2] = [
    RustCrate {
        name: "configured",
        dir: "crates/configured",
        binary: false,
        test_data: &[],
        roles: &[],
        workspace: crate::workspace::CargoWorkspace::Root,
    },
    RustCrate {
        name: "loaded",
        dir: "crates/loaded",
        binary: false,
        test_data: &[],
        roles: &[],
        workspace: crate::workspace::CargoWorkspace::Root,
    },
];

/// The host's two builds of the loaded-files fixture, as the gate's first two commands
/// make them, then its library checked for WebAssembly, all in `target_dir`.
fn loaded_fixture_commands(target_dir: &str) -> [crate::LoadedCommand; 3] {
    let json = "--message-format=json-render-diagnostics";
    let command = |args: &[&str], tests_only: bool, configuration: Option<&'static str>| {
        let mut cmd = Cmd::new("cargo", args, Dir::Root);
        cmd.args
            .extend(["--target-dir".to_owned(), target_dir.to_owned()]);
        crate::LoadedCommand {
            cmd,
            tests_only,
            configuration,
        }
    };
    [
        command(
            &["check", "--workspace", "--lib", "--bins", "--locked", json],
            false,
            None,
        ),
        command(
            &["test", "--workspace", "--locked", "--no-run", json],
            true,
            None,
        ),
        command(
            &[
                "check",
                "--lib",
                "--package",
                "configured",
                "--locked",
                "--target",
                crate::WASM_TARGET,
                json,
            ],
            false,
            Some(crate::WASM_TARGET),
        ),
    ]
}

#[test]
fn reads_the_files_real_rustc_loads_for_each_build() {
    let target = scratch("loaded-files");
    let [check, test, wasm] = loaded_fixture_commands(&target.display().to_string());
    let [configured, loaded_crate] = LOADED_FIXTURE_CRATES;
    let read = |commands: &[crate::LoadedCommand]| {
        let mut loaded = crate::loaded_files::loaded_files(
            &mut system(fixture("loaded-files")),
            commands,
            &LOADED_FIXTURE_CRATES,
        )
        .expect("built and read");
        loaded.sort_by(|one, other| {
            (one.krate.name, one.of_tests).cmp(&(other.krate.name, other.of_tests))
        });
        loaded
    };
    let [lib, tests] =
        ["src/lib.rs", "src/lib_tests.rs"].map(|file| format!("crates/loaded/{file}"));
    let helper = "crates/loaded/src/lib_tests/helper.rs".to_owned();
    let [configured_lib, wasm_tests] =
        ["src/lib.rs", "src/wasm_tests.rs"].map(|file| format!("crates/configured/{file}"));
    let blob = "crates/configured/src/blob.inc".to_owned();
    let build = |krate: RustCrate,
                 configuration: Option<&'static str>,
                 of_tests: bool,
                 files: &[&String]| crate::Loaded {
        krate,
        target: krate.name.to_owned(),
        configuration,
        of_tests,
        files: files.iter().map(|file| (*file).clone()).collect(),
    };
    let on_host = read(&[check, test]);
    assert_eq!(
        on_host,
        [
            build(configured, None, false, &[&configured_lib]),
            build(configured, None, true, &[&configured_lib]),
            build(loaded_crate, None, false, &[&lib]),
            build(loaded_crate, None, true, &[&lib, &tests, &helper]),
        ],
        "each library alone, then its tests, with the module the macro brought in"
    );
    let tracked = [
        lib,
        tests,
        helper.clone(),
        configured_lib.clone(),
        wasm_tests.clone(),
    ];
    let gate = |loaded: &[crate::Loaded]| {
        crate::loaded_files_gate(loaded, &tracked).map_err(|failure| failure.summary)
    };
    let brought_in = format!(
        "{helper}: read only by the test build of `loaded` (loaded), but not named as test \
         code (tests/, a *_tests.rs file): a module brought in by a macro or an attribute"
    );
    assert_eq!(
        gate(&on_host),
        Err(brought_in.clone()),
        "what the test code gate cannot see; on the host, nothing of the WebAssembly build"
    );
    let for_wasm = read(&[wasm]);
    assert_eq!(
        for_wasm,
        [build(
            configured,
            Some(crate::WASM_TARGET),
            false,
            &[&configured_lib, &wasm_tests, &blob]
        )],
        "the WebAssembly build reads the files of its configuration"
    );
    let wasm_build = format!(
        "the build of `configured` (configured, {})",
        crate::WASM_TARGET
    );
    assert_eq!(
        crate::loaded_files_gate(&[on_host, for_wasm].concat(), &tracked)
            .map_err(|failure| failure.details),
        Err(vec![
            brought_in,
            format!(
                "{wasm_tests}: read by {wasm_build}, which builds no tests, but named as test \
                 code (tests/, a *_tests.rs file): code that every rule telling test code by \
                 its name skips"
            ),
            format!(
                "{blob}: read by {wasm_build}, not a Rust file: no gate reads it as code, and \
                 only a build of tests reads data, from its crate's declared test data"
            ),
        ]),
        "a test-named file and a file that is not Rust, compiled for WebAssembly only"
    );
}

/// The crate of the loaded-paths fixture, whose tests declare `data/` as their test data.
const PATHS_FIXTURE_CRATE: RustCrate = RustCrate {
    name: "paths",
    dir: "crates/paths",
    binary: false,
    test_data: &["data/"],
    roles: &[],
    workspace: crate::workspace::CargoWorkspace::Root,
};

#[test]
fn refuses_what_path_attributes_load_from_outside_src() {
    let target = scratch("loaded-paths");
    let [check, test, _] = loaded_fixture_commands(&target.display().to_string());
    let loaded = crate::loaded_files::loaded_files(
        &mut system(fixture("loaded-paths")),
        &[check, test],
        &[PATHS_FIXTURE_CRATE],
    )
    .expect("built and read");
    let tracked = [
        "crates/paths/src/lib.rs",
        "crates/paths/src/lib_tests.rs",
        "crates/paths/extra/outside.rs",
        "crates/other/src/lib.rs",
        "data/case.rs",
    ]
    .map(str::to_owned);
    let (build, test_build) = (
        "the build of `paths` (paths)",
        "the test build of `paths` (paths)",
    );
    assert_eq!(
        crate::loaded_files_gate(&loaded, &tracked).map_err(|failure| failure.details),
        Err(vec![
            format!(
                "crates/paths/extra/outside.rs: read by {build}, which builds no tests, \
                 outside its crate's src/: the gates that read code by its path read src/"
            ),
            format!("crates/other/src/lib.rs: read by {build}, outside its crate"),
            format!(
                "crates/other/src/lib.rs: read by {test_build}, outside its crate and its \
                 declared test data"
            ),
            format!(
                "data/case.rs: read by {test_build}, a Rust file of its declared test data: \
                 code comes from its crate alone"
            ),
        ]),
        "a module outside src/, another crate's file, a Rust file of the test data"
    );
}

#[test]
fn shows_the_compiler_s_diagnostic_of_a_build_that_fails() {
    let dir = scratch("loaded-failing");
    std::fs::create_dir_all(dir.join("src")).expect("src");
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"p\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
    )
    .expect("manifest");
    std::fs::write(dir.join("src/lib.rs"), "pub fn f() -> u8 { \"x\" }\n").expect("source");
    let mut cmd = crate::loaded_files_commands()[0].cmd.clone();
    cmd.args.retain(|arg| arg != "--locked");
    cmd.args.extend([
        "--target-dir".to_owned(),
        dir.join("target").display().to_string(),
    ]);
    let failure = crate::loaded_files::loaded_files(
        &mut system(dir.path()),
        &[crate::LoadedCommand {
            cmd,
            tests_only: false,
            configuration: None,
        }],
        &LOADED_FIXTURE_CRATES,
    )
    .expect_err("a type error");
    assert!(
        failure
            .summary
            .ends_with("exited with code 101: error[E0308]: mismatched types"),
        "the compiler's error line: {failure:?}"
    );
    assert!(
        failure
            .details
            .iter()
            .any(|line| line.trim() == "--> src/lib.rs:1:20"),
        "the file and line it names: {failure:?}"
    );
}

#[test]
fn reports_the_builds_and_dep_info_files_it_cannot_read() {
    let step = || {
        vec![Step {
            name: "Loaded".to_owned(),
            action: Action::LoadedFilesGate,
        }]
    };
    let commands = crate::loaded_files_commands();
    let (check, test) = (&commands[0].cmd, &commands[1].cmd);
    let mut broken = passing();
    broken.broken = Some(test.line());
    assert_eq!(
        run_steps(&step(), &mut broken, &mut Vec::new()),
        failed(format!(
            "Loaded: cannot build: cannot run `{}`: not found",
            test.line()
        )),
        "a build that cannot start"
    );
    let mut failing = passing();
    failing.fail = Some(check.line());
    let rendered = "    Checking fpenv v0.1.0\nerror[E0308]: mismatched types\n --> \
                    crates/fpenv/src/lib.rs:1:20\nerror: could not compile \
                    `fpenv` (lib) due to 1 previous error";
    let _rendered = failing
        .files
        .insert(format!("stderr {}", check.line()), format!("{rendered}\n"));
    assert_eq!(
        run_steps(&step(), &mut failing, &mut Vec::new()),
        failed(format!(
            "Loaded: cannot build: `{}` exited with code 3: error[E0308]: mismatched types\n\
             {rendered}",
            check.line()
        )),
        "a build that fails, with the compiler's rendered diagnostic"
    );
    let mut missing = passing();
    let _gone = missing.files.remove("/t/debug/deps/fpenv-1.d");
    assert_eq!(
        run_steps(&step(), &mut missing, &mut Vec::new()),
        failed("Loaded: cannot read /t/debug/deps/fpenv-1.d: missing"),
        "a dep-info file missing"
    );
    let mut garbled = passing();
    let _garbled = garbled
        .files
        .insert("/t/debug/deps/fpenv-2.d".to_owned(), "x\n".to_owned());
    assert_eq!(
        run_steps(&step(), &mut garbled, &mut Vec::new()),
        failed("Loaded: /t/debug/deps/fpenv-2.d: a line not as rustc writes dep-info: x"),
        "a dep-info file not as rustc writes it"
    );
    let mut noise = passing();
    let _noise = noise
        .files
        .insert(format!("stdout {}", check.line()), "warning\n".to_owned());
    assert_eq!(
        run_steps(&step(), &mut noise, &mut Vec::new()),
        failed("Loaded: cannot read cargo's message (not JSON): warning"),
        "an output of no JSON message"
    );
    let mut stray = passing();
    let _stray = stray.files.insert(
        "/t/debug/deps/fpenv-2.d".to_owned(),
        dep_info("/t/debug/deps/fpenv-2", &["crates/fpenv/src/stray.rs"]),
    );
    assert_eq!(
        run_steps(&step(), &mut stray, &mut Vec::new()),
        failed(
            "Loaded: crates/fpenv/src/stray.rs: read by the test \
             build of `fpenv` (fpenv), but not tracked by git, so no gate read it \
             (git add it)"
        ),
        "the gate fed with what the builds read"
    );
}

#[test]
fn checks_the_webassembly_interface_of_each_library_for_its_target() {
    let wasm: Vec<String> = crate::loaded_files_commands()
        .into_iter()
        .filter(|command| command.configuration == Some(crate::WASM_TARGET))
        .map(|command| command.cmd.line())
        .collect();
    assert_eq!(
        wasm,
        [
            "cargo check --lib --package audio-core --package audio-boxes --package audio-patch \
             --package audio-hash --package audio-host --package visual-core --package visual-boxes --package \
             visual-hash --package visual-host --package coordinator --package audio-wasm --package visual-wasm --package web-page --locked \
             --target wasm32-unknown-unknown --message-format=json-render-diagnostics"
        ],
        "the libraries' cores and both interfaces, a module only WebAssembly compiles read too"
    );
}

#[test]
fn reads_the_builds_of_every_workspace() {
    let lines: Vec<(String, bool)> = crate::loaded_files_commands()
        .into_iter()
        .map(|command| (command.cmd.line(), command.tests_only))
        .collect();
    for (expected, tests_only) in [
        (
            "cargo check --manifest-path crates/visual-gpu/Cargo.toml --workspace --lib --bins \
             --locked --message-format=json-render-diagnostics",
            false,
        ),
        (
            "cargo test --manifest-path crates/visual-gpu/Cargo.toml --workspace --locked \
             --no-run --message-format=json-render-diagnostics",
            true,
        ),
    ] {
        assert!(
            lines.contains(&(expected.to_owned(), tests_only)),
            "the GPU workspace's builds, as the root's: {expected} in {lines:?}"
        );
    }
}

/// The sources defining the library's shader constants, as the gate reads them.
fn shader_definitions() -> Vec<(String, String)> {
    vec![
        (
            "crates/visual-boxes/src/square.rs".to_owned(),
            "/// Why.\npub const SQUARE_WGSL: &str = include_str!(\"square.wgsl\");\n".to_owned(),
        ),
        (
            "crates/visual-core/src/display.rs".to_owned(),
            "pub const DISPLAY_WGSL: &str = include_str!(\"display.wgsl\");\nconst OTHER_WGSL: &str = include_str!(\"other.wgsl\");\n".to_owned(),
        ),
    ]
}

#[test]
fn accepts_a_shader_only_when_the_gpu_workspace_s_test_validates_it() {
    use crate::loaded_files::validated_shaders_gate;
    let test = |listed: &str| {
        format!(
            "#[test]\nfn other() {{\n    let _ = (\"crates/visual-core/src/display.wgsl\", visual_core::DISPLAY_WGSL);\n}}\n\n#[test]\nfn validates_the_library_s_shaders() {{\n    for (name, source) in [\n{listed}    ] {{ {{ nested(); }} }}\n}}\n\n#[test]\nfn after() {{}}\n"
        )
    };
    // As rustfmt writes a long pair: on three lines, a comma after the constant.
    let square = "        (\n            \"crates/visual-boxes/src/square.wgsl\",\n            visual_boxes::SQUARE_WGSL,\n        ),\n";
    let display = "        (\"crates/visual-core/src/display.wgsl\", visual_core::DISPLAY_WGSL),\n";
    let definitions = shader_definitions();
    assert_eq!(
        validated_shaders_gate(&test(&format!("{square}{display}")), &definitions),
        Ok(()),
        "each shader the gate accepts, validated through the constant embedding it"
    );
    let unvalidated = "crates/visual-core/src/display.wgsl is a shader a build may read \
                       (`VALIDATED_SHADERS` in xtask's loaded_files.rs) that the test \
                       `validates_the_library_s_shaders` of crates/visual-gpu/tests/shaders.rs \
                       does not validate: add the pair (\"crates/visual-core/src/display.wgsl\", \
                       <crate>::<CONSTANT>) there";
    let not_its_text = "crates/visual-gpu/tests/shaders.rs validates \
                        crates/visual-core/src/display.wgsl as `visual_core::OTHER_WGSL`, which \
                        its crate does not define as `const OTHER_WGSL: &str = \
                        include_str!(\"…\");` of that file";
    for (listed, sources, expected, what) in [
        (
            square.to_owned(),
            definitions.clone(),
            unvalidated.to_owned(),
            "a shader named elsewhere in the file, not in the test that validates them",
        ),
        (
            format!("{square}        // {}", display.trim_start()),
            definitions.clone(),
            unvalidated.to_owned(),
            "the pair commented out",
        ),
        (
            format!(
                "{square}        (\"crates/visual-core/src/display.wgsl\", visual_core::display::DISPLAY_WGSL),\n"
            ),
            definitions.clone(),
            unvalidated.to_owned(),
            "another spelling of the constant's path",
        ),
        (
            format!(
                "{square}        (\"crates/visual-core/src/display.wgsl\", visual_boxes::SQUARE_WGSL),\n"
            ),
            definitions.clone(),
            "crates/visual-gpu/tests/shaders.rs validates crates/visual-core/src/display.wgsl \
             as `visual_boxes::SQUARE_WGSL`, a constant of another crate than the one holding \
             the shader"
                .to_owned(),
            "the new path given another shader's constant",
        ),
        (
            format!(
                "{square}        (\"crates/visual-core/src/display.wgsl\", visual_core::OTHER_WGSL),\n"
            ),
            definitions.clone(),
            not_its_text.to_owned(),
            "a constant of the crate embedding another file",
        ),
        (
            format!("{square}{display}"),
            vec![
                definitions.first().cloned().expect("the square's"),
                (
                    "crates/visual-core/src/display.rs".to_owned(),
                    "pub static DISPLAY_WGSL: &str = include_str!(\"display.wgsl\");\n// const DISPLAY_WGSL: &str = include_str!(\"display.wgsl\");\n".to_owned(),
                ),
                (
                    "display.rs".to_owned(),
                    "const DISPLAY_WGSL: &str = include_str!(\"crates/visual-core/src/display.wgsl\");\n".to_owned(),
                ),
            ],
            "crates/visual-gpu/tests/shaders.rs validates crates/visual-core/src/display.wgsl \
             as `visual_core::DISPLAY_WGSL`, which its crate does not define as `const \
             DISPLAY_WGSL: &str = include_str!(\"…\");` of that file"
                .to_owned(),
            "the constant in another form, commented out, or in a source of no directory",
        ),
    ] {
        assert_eq!(
            validated_shaders_gate(&test(&listed), &sources),
            Err(crate::Failure::from(expected)),
            "{what}"
        );
    }
}

#[test]
fn refuses_a_shader_test_that_is_gone_or_does_not_lex() {
    use crate::loaded_files::validated_shaders_gate;
    assert_eq!(
        validated_shaders_gate("fn nothing() {}\n", &shader_definitions()),
        Err(crate::Failure::from(
            "crates/visual-gpu/tests/shaders.rs has no test `validates_the_library_s_shaders`"
                .to_owned()
        )),
        "the test gone"
    );
    assert_eq!(
        validated_shaders_gate(
            "fn validates_the_library_s_shaders();\n",
            &shader_definitions()
        ),
        Err(crate::Failure::from(
            "crates/visual-gpu/tests/shaders.rs has no test `validates_the_library_s_shaders`"
                .to_owned()
        )),
        "a declaration with no body"
    );
    assert_eq!(
        validated_shaders_gate("fn f() { \"\n", &shader_definitions()),
        Err(crate::Failure::from("crates/visual-gpu/tests/shaders.rs:1: does not lex as Rust (cannot parse string into token stream): the gates read the tokens of every Rust source".to_owned())),
        "a test source that does not lex"
    );
    assert_eq!(
        validated_shaders_gate(
            "fn validates_the_library_s_shaders() {}\n",
            &[(
                "crates/visual-core/src/display.rs".to_owned(),
                "\"".to_owned()
            )]
        ),
        Err(crate::Failure::from("crates/visual-core/src/display.rs:1: does not lex as Rust (cannot parse string into token stream): the gates read the tokens of every Rust source".to_owned())),
        "a definition that does not lex"
    );
}

#[test]
fn refuses_a_pair_holding_more_than_its_constant() {
    let test = |pair: &str| {
        format!(
            "fn validates_the_library_s_shaders() {{\n    for (name, source) in [\n        (\"crates/visual-boxes/src/square.wgsl\", visual_boxes::SQUARE_WGSL),\n        {pair}\n    ] {{}}\n}}\n"
        )
    };
    for pair in [
        "(\"crates/visual-core/src/display.wgsl\", visual_core::DISPLAY_WGSL, other),",
        "(\"crates/visual-core/src/display.wgsl\", visual_core::DISPLAY_WGSL?),",
    ] {
        assert_eq!(
            crate::loaded_files::validated_shaders_gate(&test(pair), &shader_definitions())
                .map_err(|failure| failure.summary),
            Err("crates/visual-core/src/display.wgsl is a shader a build may read (`VALIDATED_SHADERS` in xtask's loaded_files.rs) that the test `validates_the_library_s_shaders` of crates/visual-gpu/tests/shaders.rs does not validate: add the pair (\"crates/visual-core/src/display.wgsl\", <crate>::<CONSTANT>) there".to_owned()),
            "{pair}: no pair"
        );
    }
}
