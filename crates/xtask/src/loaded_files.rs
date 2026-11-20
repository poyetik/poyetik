//! The loaded-files gate, over the files rustc itself read for every build of the check.

use alloc::collections::BTreeSet;

use crate::{
    actions::{list_tracked, read_all, read_file},
    changes::{rust_file, rust_test_code},
    release::CARGO_TOML,
    runner::{Ran, Runner},
    rust_source::{RustSource, Token, TokenKind},
    steps::{Cmd, Dir, Failure, fused_args, step_verdict},
    test_code::under_root,
    workflow::{NODE_VERSION_FILE, WORKFLOW, workflow_gate},
    workspace::{
        CargoWorkspace, NATIVE_PACKAGES, NATIVE_TARGETS, PORTABILITY_PACKAGES, PORTABILITY_TARGETS,
        RUST_CRATES, RustCrate, WASM_PACKAGES, WASM_TARGET, package_args,
    },
};

/// The WGSL shaders a build may read as code: those the GPU workspace's tests parse and
/// validate with naga (`visual-gpu`'s `tests/shaders.rs`, which validates the
/// libraries' constants embedding them), each listed there and here; a shader added to a
/// library is added to both, which [`validated_shaders_gate`] checks.
pub(crate) const VALIDATED_SHADERS: [&str; 2] = [
    "crates/visual-boxes/src/square.wgsl",
    "crates/visual-core/src/display.wgsl",
];

/// The GPU workspace's test source that validates the shaders.
pub(crate) const SHADER_TEST_SOURCE: &str =
    concat!(crate::workspace::gpu_workspace!(), "/tests/shaders.rs");

/// The test of [`SHADER_TEST_SOURCE`] that validates them.
const SHADER_TEST: &str = "validates_the_library_s_shaders";

/// Checks that every shader of [`VALIDATED_SHADERS`] is validated by the GPU workspace's
/// test ([`SHADER_TEST`] in `test`, the text of [`SHADER_TEST_SOURCE`]), read as rustc
/// reads it, comments apart: its body holds the pair `("<shader>", <crate>::<CONSTANT>)`,
/// where `<crate>` is the crate holding the shader and `<CONSTANT>` is defined in one of
/// `definitions` (that crate's sources, path and text) exactly as
/// `const <CONSTANT>: &str = include_str!("<file>");`, `<file>` being the shader from that
/// source's directory. So a shader added to the list alone, a pair commented out, or a new
/// path given another shader's constant (naga then validating one shader twice and the
/// new one never) is refused; any other spelling of the pair or of the constant is
/// refused as not found, never read as another.
///
/// # Errors
///
/// Returns that a source does not lex or the test is gone, or every shader whose pair is
/// missing, names another crate, or names a constant that is not the shader's text.
pub(crate) fn validated_shaders_gate(
    test: &str,
    definitions: &[(String, String)],
) -> Result<(), Failure> {
    let source = RustSource::parse(SHADER_TEST_SOURCE, test)?;
    let body = test_body(&source.tokens).ok_or_else(|| {
        Failure::from(format!("{SHADER_TEST_SOURCE} has no test `{SHADER_TEST}`"))
    })?;
    let defined = definitions
        .iter()
        .map(|(path, text)| RustSource::parse(path, text))
        .collect::<Result<Vec<RustSource>, Failure>>()?;
    // The pair ends after its constant, rustfmt's trailing comma apart.
    let closes = |rest: &[Token]| match rest {
        [close, ..] if close.kind == TokenKind::Close => true,
        [comma, close, ..] => comma.is_punct(',') && close.kind == TokenKind::Close,
        _ => false,
    };
    let pairs: Vec<(String, &str, &str)> = body
        .windows(6)
        .zip(1_usize..)
        .filter_map(|(window, at)| match window {
            [path, comma, krate, colon, second, constant]
                if path.kind == TokenKind::Literal
                    && comma.is_punct(',')
                    && krate.kind == TokenKind::Ident
                    && colon.is_punct(':')
                    && second.is_punct(':')
                    && constant.kind == TokenKind::Ident
                    && closes(body.get(at.saturating_add(5)..).unwrap_or_default()) =>
            {
                let path = path.text.strip_prefix('"')?.strip_suffix('"')?;
                Some((path.to_owned(), krate.text.as_str(), constant.text.as_str()))
            }
            _ => None,
        })
        .collect();
    let mut problems = Vec::new();
    for shader in VALIDATED_SHADERS {
        let Some((_, krate, constant)) = pairs.iter().find(|(path, _, _)| path == shader) else {
            problems.push(format!(
                "{shader} is a shader a build may read (`VALIDATED_SHADERS` in xtask's \
                 loaded_files.rs) that the test `{SHADER_TEST}` of {SHADER_TEST_SOURCE} does not \
                 validate: add the pair (\"{shader}\", <crate>::<CONSTANT>) there"
            ));
            continue;
        };
        let owner = RUST_CRATES
            .iter()
            .find(|owner| owner.holds(shader).is_some())
            .map(|owner| owner.name.replace('-', "_"));
        if owner.as_deref() != Some(*krate) {
            problems.push(format!(
                "{SHADER_TEST_SOURCE} validates {shader} as `{krate}::{constant}`, a constant \
                 of another crate than the one holding the shader"
            ));
        } else if !defined
            .iter()
            .any(|definition| includes(definition, constant, shader))
        {
            problems.push(format!(
                "{SHADER_TEST_SOURCE} validates {shader} as `{krate}::{constant}`, which its \
                 crate does not define as `const {constant}: &str = include_str!(\"…\");` of \
                 that file"
            ));
        }
    }
    step_verdict(problems)
}

/// The tokens of the body of [`SHADER_TEST`] in `tokens`, its braces apart: none when the
/// test is not there.
fn test_body(tokens: &[Token]) -> Option<&[Token]> {
    let name = tokens
        .windows(2)
        .position(|pair| matches!(pair, [fn_word, name] if fn_word.is_ident("fn") && name.is_ident(SHADER_TEST)))?;
    let rest = tokens.get(name..)?;
    let open = rest.iter().position(|token| token.opens('{'))?;
    let mut depth = 0_usize;
    let close = rest.iter().enumerate().skip(open).find_map(|(at, token)| {
        match token.kind {
            TokenKind::Open => depth = depth.saturating_add(1),
            TokenKind::Close => depth = depth.saturating_sub(1),
            TokenKind::Ident
            | TokenKind::Punct { .. }
            | TokenKind::Literal
            | TokenKind::Doc { .. } => return None,
        }
        (depth == 0).then_some(at)
    })?;
    rest.get(open.saturating_add(1)..close)
}

/// Whether `definition` defines `constant` exactly as `const <constant>: &str =
/// include_str!("<file>");`, where `<file>`, from the source's directory, is `shader`.
fn includes(definition: &RustSource, constant: &str, shader: &str) -> bool {
    let Some((dir, _)) = definition.path.rsplit_once('/') else {
        return false;
    };
    definition.tokens.windows(12).any(|window| match window {
        [
            const_word,
            name,
            colon,
            and,
            str_word,
            equals,
            include,
            bang,
            open,
            file,
            close,
            semicolon,
        ] => {
            const_word.is_ident("const")
                && name.is_ident(constant)
                && colon.is_punct(':')
                && and.is_punct('&')
                && str_word.is_ident("str")
                && equals.is_punct('=')
                && include.is_ident("include_str")
                && bang.is_punct('!')
                && open.opens('(')
                && file.kind == TokenKind::Literal
                && close.kind == TokenKind::Close
                && semicolon.is_punct(';')
                && file
                    .text
                    .strip_prefix('"')
                    .and_then(|text| text.strip_suffix('"'))
                    .is_some_and(|file| format!("{dir}/{file}") == shader)
        }
        _ => false,
    })
}

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
/// builds ([`fused_command`](crate::steps::fused_command) with `--no-run`, which the fused step reuses), the crates of
/// both libraries for each of [`PORTABILITY_TARGETS`] ([`PORTABILITY_PACKAGES`], with [`WASM_PACKAGES`] for
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
    // Every other workspace's builds, as the root's: the GPU renderer's.
    for workspace in CargoWorkspace::ALL
        .into_iter()
        .filter(|&workspace| workspace != CargoWorkspace::Root)
    {
        let manifest = workspace.manifest_args();
        commands.extend([
            cargo(
                &[
                    &["check"],
                    manifest,
                    &["--workspace", "--lib", "--bins", "--locked", JSON],
                ]
                .concat(),
                false,
                None,
            ),
            cargo(
                &[
                    &["test"],
                    manifest,
                    &["--workspace", "--locked", "--no-run", JSON],
                ]
                .concat(),
                true,
                None,
            ),
        ]);
    }
    for target in PORTABILITY_TARGETS {
        let wasm = if target == WASM_TARGET {
            package_args(&WASM_PACKAGES)
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
                .map(|file| named_from_the_root(&file, &root, krate.workspace))
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

/// `file`, as a build of `workspace` names it, from the repository's root `root`
/// ([`normalized`]): rustc names a file relative to the root of the workspace it builds,
/// which for another workspace than the root's is its directory
/// ([`CargoWorkspace::dir`]), so that a relative name is that directory's.
pub(crate) fn named_from_the_root(file: &str, root: &str, workspace: CargoWorkspace) -> String {
    if workspace == CargoWorkspace::Root || file.starts_with('/') {
        normalized(file, root)
    } else {
        normalized(&format!("{}/{file}", workspace.dir()), root)
    }
}

/// `file`, as rustc names it (relative to the repository's root `root`, or absolute), relative
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
    let test = read_file(runner, SHADER_TEST_SOURCE)?;
    let patterns: BTreeSet<String> = RUST_CRATES
        .iter()
        .filter(|krate| {
            VALIDATED_SHADERS
                .iter()
                .any(|shader| krate.holds(shader).is_some())
        })
        .map(|krate| format!("{}/src/*.rs", krate.dir))
        .collect();
    let patterns: Vec<&str> = patterns.iter().map(String::as_str).collect();
    let definitions = list_tracked(runner, &patterns, "the sources of the shaders' crates")?;
    validated_shaders_gate(&test, &read_all(runner, definitions)?)?;
    let loaded = loaded_files(runner, &loaded_files_commands(), &RUST_CRATES)?;
    loaded_files_gate(
        &loaded,
        &list_tracked(runner, &["*.rs", "*.wgsl"], "the Rust files and shaders")?,
    )
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
/// tests (but a tracked shader of its crate's `src/` the GPU workspace's tests validate,
/// `VALIDATED_SHADERS`), every Rust file read that git does not track, every file named as
/// test code a build of the code reads, every other file such a build reads outside its crate's `src/`,
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
                    // A shader the GPU workspace's tests validate, tracked: code, read as
                    // such ([`VALIDATED_SHADERS`], each a `.wgsl` file of its crate's src/).
                    let shader =
                        VALIDATED_SHADERS.contains(&file.as_str()) && tracked.contains(file);
                    if !shader && !test_data {
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
