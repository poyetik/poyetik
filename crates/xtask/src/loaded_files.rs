//! The loaded-files gate, over the files rustc itself read for every build of the check.

use alloc::collections::BTreeSet;

use crate::{
    actions::{list_tracked, read_file},
    changes::{rust_file, rust_test_code},
    release::CARGO_TOML,
    runner::{Ran, Runner},
    steps::{Cmd, Dir, Failure, fused_args, step_verdict},
    test_code::under_root,
    workflow::{NODE_VERSION_FILE, WORKFLOW, workflow_gate},
    workspace::{
        NATIVE_PACKAGES, NATIVE_TARGETS, PORTABILITY_PACKAGES, PORTABILITY_TARGETS, RUST_CRATES,
        RustCrate, WASM_PACKAGE, WASM_TARGET, package_args,
    },
};

/// A command whose builds [`loaded_files_gate`] reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedCommand {
    /// The command: cargo, writing its JSON messages.
    pub cmd: Cmd,
    /// Whether only its builds of tests are read, its other builds being another
    /// command's.
    pub tests_only: bool,
    /// The configuration it builds, as the gate's messages name it: none for the host's
    /// own.
    pub configuration: Option<&'static str>,
}

/// The builds whose files [`loaded_files_gate`] reads: every configuration the check
/// builds, with no code generated where the check's own step would not reuse it.
///
/// On the host: every library and binary of the workspace without its tests (`cargo
/// check`), then what the Rust tests step builds (`cargo test --no-run`, its builds
/// reused), of which only the builds of tests are read. Then the fused test build's own
/// builds ([`fused_command`](crate::steps::fused_command) with `--no-run`, which the fused step reuses), the engine core
/// for each of [`PORTABILITY_TARGETS`] ([`PORTABILITY_PACKAGES`], with [`WASM_PACKAGE`] for
/// [`WASM_TARGET`]) and the native crates, their tests included, for each of
/// [`NATIVE_TARGETS`] ([`NATIVE_PACKAGES`]), each checked (`cargo check`: type-checked, no
/// code generated), every build of theirs read. So a module only another target, the fused
/// math layer or a test build compiles is in the compiler's record too. Cargo's messages
/// are JSON on stdout, the compiler's diagnostics rendered on stderr, so that a build that
/// fails shows them (the loaded-files gate, [`loaded_files_gate`]).
#[must_use]
pub fn loaded_files_commands() -> Vec<LoadedCommand> {
    const JSON: &str = "--message-format=json-render-diagnostics";
    let cargo = |args: &[&str], tests_only, configuration| LoadedCommand {
        cmd: Cmd::new("cargo", args, Dir::Root),
        tests_only,
        configuration,
    };
    let mut commands = vec![
        cargo(
            &["check", "--workspace", "--lib", "--bins", "--locked", JSON],
            false,
            None,
        ),
        cargo(
            &["test", "--workspace", "--locked", "--no-run", JSON],
            true,
            None,
        ),
        cargo(
            &[fused_args(), vec!["--no-run", JSON]].concat(),
            false,
            Some("fused"),
        ),
    ];
    for target in PORTABILITY_TARGETS {
        let wasm = if target == WASM_TARGET {
            package_args(&[WASM_PACKAGE])
        } else {
            Vec::new()
        };
        commands.push(cargo(
            &[
                vec!["check", "--lib"],
                package_args(&PORTABILITY_PACKAGES),
                wasm,
                vec!["--locked", "--target", target, JSON],
            ]
            .concat(),
            false,
            Some(target),
        ));
    }
    for target in NATIVE_TARGETS {
        commands.push(cargo(
            &[
                vec!["check"],
                package_args(&NATIVE_PACKAGES),
                vec![
                    "--lib", "--bins", "--tests", "--locked", "--target", target, JSON,
                ],
            ]
            .concat(),
            false,
            Some(target),
        ));
    }
    commands
}

/// The files rustc read to build one target of a crate, from its dep-info file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    /// The crate of the target.
    pub(crate) krate: RustCrate,
    /// The target's name, as cargo names it.
    pub(crate) target: String,
    /// The configuration built ([`LoadedCommand::configuration`]).
    pub(crate) configuration: Option<&'static str>,
    /// Whether it is a build of tests (unit tests or a test target).
    pub(crate) of_tests: bool,
    /// Every file rustc read, relative to the workspace root when below it, with no `.` or
    /// `..` left, in its order.
    pub(crate) files: Vec<String>,
}

/// The files of each build cargo made running `commands` ([`loaded_files_commands`]):
/// every build of a command, or only its builds of tests when it says so. Each build is of
/// a crate of `crates`, whose directory below the workspace root its manifest names; each
/// list is read from the dep-info file rustc wrote beside its first output
/// (`<name>-<hash>.d`).
///
/// # Errors
///
/// Returns why a command failed (with what it wrote on stderr, the compiler's rendered
/// diagnostics, as its details), its output is not cargo's JSON messages as read here, a
/// local package is none of `crates`, or a dep-info file cannot be found or read.
pub(crate) fn loaded_files(
    runner: &mut dyn Runner,
    commands: &[LoadedCommand],
    crates: &[RustCrate],
) -> Result<Vec<Loaded>, Failure> {
    let mut loaded = Vec::new();
    for command in commands {
        let Ran {
            stdout,
            stderr,
            ended,
        } = runner.run_for_stdout(&command.cmd);
        ended.map_err(|failure| Failure {
            details: stderr.lines().map(str::to_owned).collect(),
            ..Failure::from(format!("cannot build: {failure}"))
        })?;
        for line in stdout.lines() {
            let Some(artifact) = built_artifact(line, command.tests_only, crates)? else {
                continue;
            };
            let Artifact {
                krate,
                root,
                target,
                dep_info,
                of_tests,
            } = artifact;
            let text = read_file(runner, &dep_info)?;
            let files = dep_info_files(&text)
                .map_err(|unread| Failure::from(format!("{dep_info}: {unread}")))?
                .into_iter()
                .map(|file| normalized(&file, &root))
                .collect();
            loaded.push(Loaded {
                krate,
                target,
                configuration: command.configuration,
                of_tests,
                files,
            });
        }
    }
    Ok(loaded)
}

/// A build of a local package, as cargo's JSON messages report it ([`built_artifact`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Artifact {
    /// Its crate.
    pub(crate) krate: RustCrate,
    /// The workspace root its manifest gives.
    pub(crate) root: String,
    /// The target's name, as cargo names it.
    pub(crate) target: String,
    /// The path of its dep-info file.
    pub(crate) dep_info: String,
    /// Whether it is a build of tests.
    pub(crate) of_tests: bool,
}

/// What one line of cargo's JSON messages says of a build of a local package, if it reports
/// one (of tests only, when `tests_only`), its crate among `crates`.
///
/// # Errors
///
/// Returns why the line is not a JSON message as cargo writes it, why a local package is
/// none of `crates`, or why the dep-info file cannot be told from the build's output.
pub(crate) fn built_artifact(
    line: &str,
    tests_only: bool,
    crates: &[RustCrate],
) -> Result<Option<Artifact>, Failure> {
    let unread = |what: &str| Failure::from(format!("cannot read cargo's message {what}: {line}"));
    let message: serde_json::Value =
        serde_json::from_str(line).map_err(|_| unread("(not JSON)"))?;
    let field = |pointer: &str| message.pointer(pointer).and_then(serde_json::Value::as_str);
    if field("/reason") != Some("compiler-artifact")
        || !field("/package_id")
            .ok_or_else(|| unread("(no package_id)"))?
            .starts_with("path+")
    {
        return Ok(None);
    }
    let of_tests = message
        .pointer("/profile/test")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| unread("(no profile)"))?;
    if tests_only && !of_tests {
        return Ok(None);
    }
    let manifest = field("/manifest_path").ok_or_else(|| unread("(no manifest_path)"))?;
    let (krate, root) = crates
        .iter()
        .find_map(|krate| {
            manifest
                .strip_suffix(&format!("/{}/{CARGO_TOML}", krate.dir))
                .map(|root| (*krate, root.to_owned()))
        })
        .ok_or_else(|| Failure::from(format!("{manifest}: a package of no crate of the checks")))?;
    let target = field("/target/name").ok_or_else(|| unread("(no target name)"))?;
    let output = field("/filenames/0").ok_or_else(|| unread("(no output)"))?;
    let (dir, file) = output
        .rsplit_once('/')
        .ok_or_else(|| unread("(an output with no directory)"))?;
    let name = target.replace('-', "_");
    let stem = file.split('.').next().unwrap_or_default();
    let stem = [stem, stem.strip_prefix("lib").unwrap_or_default()]
        .into_iter()
        .find(|stem| stem.starts_with(&format!("{name}-")))
        .ok_or_else(|| unread("(an output not named <crate>-<hash>)"))?;
    Ok(Some(Artifact {
        krate,
        root,
        target: target.to_owned(),
        dep_info: format!("{dir}/{stem}.d"),
        of_tests,
    }))
}

/// The files a dep-info file of rustc lists: one line `<file>:` each (spaces escaped
/// `\ `), after the lines of its targets (`<output>: <file> …`); its comments
/// (`# env-dep:…`) and empty lines apart.
///
/// # Errors
///
/// Returns the first other line.
pub(crate) fn dep_info_files(text: &str) -> Result<Vec<String>, String> {
    let mut files = Vec::new();
    for line in text.lines() {
        if line.is_empty() || line.starts_with('#') || line.contains(": ") {
            continue;
        }
        let file = line
            .strip_suffix(':')
            .ok_or_else(|| format!("a line not as rustc writes dep-info: {line}"))?;
        files.push(file.replace("\\ ", " "));
    }
    Ok(files)
}

/// `file`, as rustc names it (relative to the workspace root `root`, or absolute), relative
/// to `root` when below it, its `.` and `..` resolved.
pub(crate) fn normalized(file: &str, root: &str) -> String {
    let below = file
        .strip_prefix(root)
        .and_then(|rest| rest.strip_prefix('/'))
        .unwrap_or(file);
    let mut parts: Vec<&str> = Vec::new();
    for part in below.split('/') {
        match part {
            "." => {}
            ".." if parts
                .last()
                .is_some_and(|last| !last.is_empty() && *last != "..") =>
            {
                let _parent = parts.pop();
            }
            _ => parts.push(part),
        }
    }
    parts.join("/")
}

/// Lists the workflows tracked by git and reads the one of the checks, then checks them
/// ([`workflow_gate`]).
pub(crate) fn workflows(runner: &mut dyn Runner) -> Result<(), Failure> {
    let workflows: Vec<String> = list_tracked(runner, &["*.yml", "*.yaml"], "the YAML files")?
        .into_iter()
        .filter(|path| path.starts_with(".github/workflows/"))
        .collect();
    let workflow = read_file(runner, WORKFLOW)?;
    workflow_gate(
        &workflows,
        &workflow,
        &read_file(runner, NODE_VERSION_FILE)?,
    )
}

/// Builds the workspace ([`loaded_files_commands`]), then checks the files rustc read
/// ([`loaded_files_gate`]).
pub(crate) fn loaded_files_of_the_workspace(runner: &mut dyn Runner) -> Result<(), Failure> {
    let loaded = loaded_files(runner, &loaded_files_commands(), &RUST_CRATES)?;
    loaded_files_gate(&loaded, &list_tracked(runner, &["*.rs"], "the Rust files")?)
}

/// Checks the files rustc read for each build (from its dep-info file) against what the
/// gates that read sources by their path assume, both ways.
///
/// Every file a build reads is below its crate's directory, or one of the crate's declared
/// test data ([`RustCrate::test_data`]) read by a build of tests, which is not Rust; every
/// file a build reads below its crate is Rust, but the declared test data a build of tests
/// reads, so that no code comes from a file no gate reads as code (`include!` of another
/// extension, a dependency's macro expanding to it); a build of the code reads only files
/// below its crate's `src/`, which the gates reading code by its path read (the math layer
/// gate's [`MATH_LAYER_SOURCES`](crate::math_layer::MATH_LAYER_SOURCES)); every Rust file below a crate that a build reads is
/// tracked by git (`tracked`), so that every gate reading the tracked files read it; every
/// Rust file a build of tests reads is read by a build of the code (a build without its
/// tests), or is test code by its name (`tests/`, a `*_tests.rs` file); and no build of the
/// code reads a file named as test code, which every rule telling test code by its name (the
/// math layer gate, the mutation widening, the proof record's rules) skips. So a module a
/// macro or an attribute brings in, which no gate reading words sees (`mod $name;` expanded
/// in a test file), is told by the compiler's own record.
///
/// The record is that of every build of [`loaded_files_commands`]: the host's, the fused
/// math layer's, and each target's the check builds. What only a build the check makes in
/// another configuration would read is refused at its source: Miri's own `cfg` and the
/// coverage instrumentation's by [`bypass_gate`](crate::reviews::bypass_gate) and the compiler (an unexpected `cfg`);
/// debug assertions off (the release build of the engine package's WebAssembly), the `cfg`
/// of doc tests and of documentation, and a crate's features by [`bypass_gate`](crate::reviews::bypass_gate), unless
/// reviewed in [`BYPASSES_REVIEWED`](crate::reviews::BYPASSES_REVIEWED); an example or a benchmark target by
/// [`untested_targets_gate`](crate::test_code::untested_targets_gate) ([`TARGET_KINDS`](crate::test_code::TARGET_KINDS)). **Not seen**: code under a `cfg` of a
/// target the check does not build (`windows`, an operating system no build targets),
/// which no check compiles at all. **Review only**: what a doc test loads, a crate rustdoc
/// compiles apart that writes no dep-info (the compiler refuses unsafe code there:
/// [`DOC_TESTS_FORBID_UNSAFE_CODE`](crate::unsafety::DOC_TESTS_FORBID_UNSAFE_CODE)).
///
/// # Errors
///
/// Returns every file read outside its crate and declared test data, every Rust file of
/// the declared test data, every file read that is not Rust nor declared test data read by
/// tests, every Rust file read that git does not track, every file named as test code a
/// build of the code reads, every other file such a build reads outside its crate's `src/`,
/// and every Rust file only tests read that is not named as test code, each with the build
/// that read it.
pub fn loaded_files_gate(loaded: &[Loaded], tracked: &[String]) -> Result<(), Failure> {
    let code: BTreeSet<&str> = loaded
        .iter()
        .filter(|build| !build.of_tests)
        .flat_map(|build| build.files.iter().map(String::as_str))
        .collect();
    let mut problems = Vec::new();
    for build in loaded {
        let what = format!(
            "{} of `{}` ({}{})",
            if build.of_tests {
                "the test build"
            } else {
                "the build"
            },
            build.target,
            build.krate.name,
            build
                .configuration
                .map(|configuration| format!(", {configuration}"))
                .unwrap_or_default()
        );
        for file in &build.files {
            let rust = rust_file(file);
            let test_data = build.of_tests
                && build
                    .krate
                    .test_data
                    .iter()
                    .any(|root| under_root(file, root));
            match build.krate.holds(file) {
                None if test_data && rust => problems.push(format!(
                    "{file}: read by {what}, a Rust file of its declared test data: code comes \
                     from its crate alone"
                )),
                None if test_data => {}
                None => problems.push(format!(
                    "{file}: read by {what}, outside its crate{}",
                    if build.of_tests {
                        " and its declared test data"
                    } else {
                        ""
                    }
                )),
                Some(_) if !rust => {
                    if !test_data {
                        problems.push(format!(
                            "{file}: read by {what}, not a Rust file: no gate reads it as code, \
                             and only a build of tests reads data, from its crate's declared \
                             test data"
                        ));
                    }
                }
                Some(_) if !tracked.contains(file) => problems.push(format!(
                    "{file}: read by {what}, but not tracked by git, so no gate read it (git \
                     add it)"
                )),
                Some(relative) if !build.of_tests && rust_test_code(relative) => {
                    problems.push(format!(
                        "{file}: read by {what}, which builds no tests, but named as test code \
                         (tests/, a *_tests.rs file): code that every rule telling test code by \
                         its name skips"
                    ));
                }
                Some(relative) if !build.of_tests && !relative.starts_with("src/") => {
                    problems.push(format!(
                        "{file}: read by {what}, which builds no tests, outside its crate's \
                         src/: the gates that read code by its path read src/"
                    ));
                }
                Some(relative)
                    if build.of_tests
                        && !rust_test_code(relative)
                        && !code.contains(file.as_str()) =>
                {
                    problems.push(format!(
                        "{file}: read only by {what}, but not named as test code (tests/, a \
                         *_tests.rs file): a module brought in by a macro or an attribute"
                    ));
                }
                Some(_) => {}
            }
        }
    }
    step_verdict(problems)
}

#[cfg(test)]
#[path = "loaded_files_tests.rs"]
mod tests;
