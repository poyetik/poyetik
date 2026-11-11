//! Tests of the check pipeline, with a fake runner.

extern crate alloc;

use alloc::collections::BTreeMap;
use core::fmt::Write as _;
use core::num::NonZeroUsize;
use core::time::Duration;
use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::process::Command;

use super::{
    Action, BestEffort, CARGO_TOML, CHANGELOG, CHANGES_DIFF, COMMIT_HEADER_MAX, Cmd,
    CommandFailure, Coverage, Dir, EQUIVALENT_REVIEWED, Exit, Extent, Failed, Failure, MUTANTS_OUT,
    Measured, Mode, NATIVE_TARGETS, OnFailure, PACKAGES, PORTABILITY_TARGETS, Runner, RustCrate,
    Step, SystemRunner, UNSAFE_FILES, UNVIABLE_REPORT, UNVIABLE_REVIEWED, USAGE, Version,
    bless_benchmarks_steps, bless_step, check_steps, cli, commit_msg_gate, coverage_gate,
    equivalent_gate, license_gate, membership_gate, package_jsons, preflight_gate, release_gate,
    run_steps, run_steps_then, ts_coverage_gate, ts_mutation_gate, unsafe_gate, unviable_gate,
};

/// Runs git in `dir` hermetically, and returns its stdout: no variable of an enclosing git
/// process (a hook's `GIT_DIR` …) reaches it, nor the user's or the system's configuration,
/// and commits get a fixed identity.
fn hermetic_git(dir: &Path, args: &[&str]) -> String {
    let mut command = Command::new("git");
    let _same = command
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null");
    for variable in super::LOCAL_GIT_VARIABLES {
        let _without = command.env_remove(variable);
    }
    let output = command.output().expect("git runs");
    assert!(output.status.success(), "git {args:?} in {}", dir.display());
    String::from_utf8(output.stdout).expect("UTF-8")
}

/// A fresh, empty directory for one test, below the system's temporary directory.
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("xtask-{name}-{}", std::process::id()));
    if let Err(error) = std::fs::remove_dir_all(&dir) {
        assert_eq!(error.kind(), io::ErrorKind::NotFound, "clean start");
    }
    std::fs::create_dir_all(&dir).expect("scratch directory");
    dir
}

/// A fixture of xtask's tests (`tests/fixtures/<name>`), the directory its commands run in.
fn fixture(name: &str) -> std::path::PathBuf {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    fixtures.join(name)
}

/// What the real `cargo metadata` ([`super::metadata_command`]) says of the fixture
/// workspace, and the root it names, from which it writes every path.
fn fixture_metadata() -> (String, String) {
    let metadata = SystemRunner {
        root: fixture("workspace"),
    }
    .capture(&super::metadata_command())
    .expect("cargo metadata");
    let json: serde_json::Value = serde_json::from_str(&metadata).expect("JSON");
    let root = json
        .get("workspace_root")
        .and_then(serde_json::Value::as_str)
        .expect("a workspace root")
        .to_owned();
    (metadata, root)
}

/// Runs [`cli`] on UTF-8 arguments.
fn run_cli(
    args: &[String],
    runner: &mut dyn Runner,
    out: &mut dyn io::Write,
    err: &mut dyn io::Write,
) -> u8 {
    let args: Vec<OsString> = args.iter().map(OsString::from).collect();
    cli(&args, runner, out, err)
}

/// The failure a test expects, written as it is shown: its summary line, then one line
/// per detail.
fn failed(text: impl AsRef<str>) -> Result<(), Failure> {
    let mut lines = text.as_ref().lines().map(str::to_owned);
    Err(Failure {
        summary: lines.next().unwrap_or_default(),
        details: lines.collect(),
    })
}

/// The failure of a gate that found `problems` about `context`, as [`super::verdict`]
/// shows it (tested on its own by `shows_one_problem_inline_and_lists_several`).
fn found(context: &str, problems: &[String]) -> Result<(), Failure> {
    match problems {
        [only] => failed(format!("{context}: {only}")),
        _ => Err(Failure {
            summary: format!("{context}: {} problems, listed above", problems.len()),
            details: problems.to_vec(),
        }),
    }
}

/// How the fake reads the state of its working tree, and what it answers: one untracked
/// file.
const TREE_STATUS: (&str, &str) = (
    "git status --porcelain --untracked-files=all",
    "?? notes.txt\n",
);

/// The changes of the tracked files of the fake: none.
const TREE_DIFF: (&str, &str) = ("git diff --no-ext-diff --no-textconv HEAD", "");

/// How the fake reads what is checked out, and what it answers: a commit on a branch.
const TREE_HEAD: (&str, &str) = (
    "git rev-parse HEAD --symbolic-full-name HEAD",
    "abc1230000000000000000000000000000000000\nrefs/heads/main\n",
);

/// A Dependabot configuration whose two ecosystems wait a week.
const DEPENDABOT: &str = "version: 2\nupdates:\n  - package-ecosystem: cargo\n    cooldown:\n      default-days: 7\n  - package-ecosystem: npm\n    cooldown:\n      default-days: 7\n";

/// A cargo-deny configuration holding libm to no feature.
const DENY: &str = "[bans]\n\n[[bans.features]]\ncrate = \"libm\"\nexact = true\nallow = []\n";

/// The Cargo metadata of a workspace at `/r` whose crate `x`, a tested library, depends on
/// another crate of the workspace and on one third-party crate, beside the crates of the
/// audio library, which depend on nothing but the boxes' tests on the test kit, and the
/// crates of shared test code, the test kit depending on the counting allocator; each has
/// only the test target reading its test data ([`test_data_reader_sources`]).
const METADATA: &str = r#"{"workspace_root":"/r","packages":[{"name":"x","manifest_path":"/r/crates/x/Cargo.toml","dependencies":[{"name":"engine-core","kind":null,"path":"/r/crates/engine-core"},{"name":"serde_json","kind":null}],"targets":[{"kind":["lib"],"src_path":"/r/crates/x/src/lib.rs","test":true,"doctest":true}]},{"name":"engine-core","manifest_path":"/r/crates/engine-core/Cargo.toml","targets":[{"kind":["test"],"src_path":"/r/crates/engine-core/tests/data.rs","test":true,"doctest":false}]},{"name":"engine-objects","manifest_path":"/r/crates/engine-objects/Cargo.toml","dependencies":[{"name":"engine-testkit","kind":"dev","path":"/r/crates/engine-testkit"}],"targets":[{"kind":["test"],"src_path":"/r/crates/engine-objects/tests/data.rs","test":true,"doctest":false}]},{"name":"engine-patch","manifest_path":"/r/crates/engine-patch/Cargo.toml","targets":[{"kind":["test"],"src_path":"/r/crates/engine-patch/tests/data.rs","test":true,"doctest":false}]},{"name":"engine-hash","manifest_path":"/r/crates/engine-hash/Cargo.toml","targets":[{"kind":["test"],"src_path":"/r/crates/engine-hash/tests/data.rs","test":true,"doctest":false}]},{"name":"engine-fpenv","manifest_path":"/r/crates/engine-fpenv/Cargo.toml","targets":[{"kind":["test"],"src_path":"/r/crates/engine-fpenv/tests/data.rs","test":true,"doctest":false}]},{"name":"engine-wasm","manifest_path":"/r/bindings/wasm/Cargo.toml","targets":[]},{"name":"engine-testkit","manifest_path":"/r/crates/engine-testkit/Cargo.toml","dependencies":[{"name":"engine-alloc-counter","kind":null,"path":"/r/crates/engine-alloc-counter"}],"targets":[{"kind":["test"],"src_path":"/r/crates/engine-testkit/tests/data.rs","test":true,"doctest":false}]},{"name":"engine-alloc-counter","manifest_path":"/r/crates/engine-alloc-counter/Cargo.toml","targets":[]}]}"#;

/// The two license listings, written out: the production dependencies of what ships
/// (the engine package, the playground and the roadmap), then everything.
const LICENSE_LINES: [&str; 2] = [
    "pnpm licenses list --json --prod --filter ./bindings/wasm --filter ./apps/playground --filter ./apps/roadmap",
    "pnpm licenses list --json --recursive",
];

/// The two history listings, written out: every message, then the files touched by each
/// commit after the changelog boundary.
const HISTORY_LINES: [&str; 3] = [
    "git log --format=%x1e%H%x1f%B HEAD",
    "git log --format=%x1e%H --name-only 8182fda87a1c631295548ff52eeb4ea08335fa36..HEAD",
    "git log --format=%H abb3bb6f37e23ab4addd534ec4bc1fc3aca9477f..HEAD",
];

/// The threads of the fake's machine: six, so that the jobs of a full check (three) differ
/// from it.
const FAKE_PARALLELISM: NonZeroUsize = NonZeroUsize::new(6).expect("six is not zero");

/// The jobs of the fake's checks: half of [`FAKE_PARALLELISM`].
const FAKE_JOBS: NonZeroUsize = NonZeroUsize::new(3).expect("three is not zero");

/// The default check on the fake's machine.
const CHANGES_MODE: Mode = Mode::Changes { jobs: FAKE_JOBS };

/// What the fake's git answers about its tree and its changes, and its compiler and linker
/// driver about themselves.
const GIT_ANSWERS: [(&str, &str); 12] = [
    TREE_STATUS,
    PUSH_STATUS,
    HIDDEN,
    HOST,
    LINKER,
    TREE_DIFF,
    TREE_HEAD,
    MERGE_BASE,
    CHANGES,
    NEW_RUST_FILES,
    COMMON_DIR,
    HEAD_TREE,
];

/// How the fake reads the status that must be empty for a push or a proof record, and what
/// it answers: nothing uncommitted.
const PUSH_STATUS: (&str, &str) = (
    "git status --porcelain --untracked-files=all --ignore-submodules=none",
    "",
);

/// How the fake lists the tracked files with their tags, and what it answers: none hidden.
const HIDDEN: (&str, &str) = (
    "git ls-files -v",
    "H Cargo.toml
H crates/a/src/b.rs
",
);

/// How the fake's rustc names its host, and what it answers.
const HOST: (&str, &str) = (
    "rustc -vV",
    "rustc 1.99.1 (58cf8f9c3 2026-10-13)
binary: rustc
commit-hash: 58cf8f9c35b355199727627d07ae34a4e2846d67
commit-date: 2026-10-13
host: aarch64-apple-darwin
release: 1.99.1
LLVM version: 23.1.1
",
);

/// How the fake's linker driver names its version, and what it answers.
const LINKER: (&str, &str) = (
    "cc --version",
    "Apple clang version 14.0.3 (clang-1403.0.22.14.1)
Target: arm64-apple-darwin22.6.0
",
);

/// How the fake names git's common directory, and what it answers: the repository's own.
const COMMON_DIR: (&str, &str) = ("git rev-parse --git-common-dir", ".git\n");

/// How the fake reads the tree of the commit checked out, and what it answers.
const HEAD_TREE: (&str, &str) = (
    "git log -1 --format=%T HEAD",
    "def4560000000000000000000000000000000000\n",
);

/// How the fake finds the base of the changes, and what it answers: the upstream resolves.
const MERGE_BASE: (&str, &str) = (
    "git merge-base HEAD @{upstream}",
    "abc1230000000000000000000000000000000000\n",
);

/// How the fake reads the changes since [`MERGE_BASE`], and what it answers: one changed
/// line of a Rust file.
const CHANGES: (&str, &str) = (
    "git diff --no-color --no-ext-diff --no-textconv --no-renames --src-prefix=a/ --dst-prefix=b/ abc1230000000000000000000000000000000000",
    "diff --git a/crates/a/src/b.rs b/crates/a/src/b.rs\n--- a/crates/a/src/b.rs\n+++ b/crates/a/src/b.rs\n@@ -1 +1 @@\n-fn f() {}\n+fn f() -> u8 { 1 }\n",
);

/// How the fake lists the new Rust files, and what it answers: none.
const NEW_RUST_FILES: (&str, &str) = ("git ls-files -z --others --exclude-standard -- *.rs", "");

/// Records commands and answers with scripted results.
#[derive(Debug, Default)]
struct Fake {
    ran: Vec<String>,
    fail: Option<String>,
    broken: Option<String>,
    killed: Option<String>,
    /// A command that, when it runs, rewrites a file: (command, file, new text).
    rewrite: Option<(String, String, String)>,
    files: BTreeMap<String, String>,
    /// What [`Runner::environment`] answers.
    environment: super::Environment,
    /// Its clock: [`FAKE_TICK`] more at each reading.
    clock: Duration,
}

/// How much the fake's clock advances at each reading: every step of its checks takes that
/// long, and a run of `n` steps `n` times as long.
const FAKE_TICK: Duration = Duration::from_millis(1_500);

impl Runner for Fake {
    fn run(&mut self, cmd: &Cmd) -> Result<(), CommandFailure> {
        let line = cmd.line();
        self.ran.push(line.clone());
        if let Some((command, file, text)) = &self.rewrite
            && *command == line
        {
            let _previous = self.files.insert(file.clone(), text.clone());
        }
        let failed = if self.broken.as_deref() == Some(line.as_str()) {
            Failed::NotStarted("not found".to_owned())
        } else if self.killed.as_deref() == Some(line.as_str()) {
            Failed::Ended {
                exit: Exit::Signal(9_i32),
                reason: None,
            }
        } else if self.fail.as_deref() == Some(line.as_str()) {
            // A failing command exits with 3: a code that is neither 0 nor 1, so that the
            // message is seen to report the real one.
            Failed::Ended {
                exit: Exit::Code(3_i32),
                reason: None,
            }
        } else {
            return Ok(());
        };
        Err(CommandFailure { line, failed })
    }

    /// Answers with the file, fails as absent, or as not UTF-8 when the test breaks
    /// `read <path>`.
    fn read(&mut self, path: &str) -> io::Result<String> {
        self.ran.push(format!("read {path}"));
        if self.broken.as_deref() == Some(format!("read {path}").as_str()) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "not UTF-8"));
        }
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "missing"))
    }

    /// Records the file among its files, or fails when the test breaks `write <path>`.
    fn write(&mut self, path: &str, text: &str) -> io::Result<()> {
        let line = format!("write {path}");
        self.ran.push(line.clone());
        if self.broken.as_deref() == Some(line.as_str()) {
            return Err(io::Error::other("read-only"));
        }
        let _previous = self.files.insert(path.to_owned(), text.to_owned());
        Ok(())
    }

    /// A machine of [`FAKE_PARALLELISM`] threads.
    fn parallelism(&mut self) -> NonZeroUsize {
        FAKE_PARALLELISM
    }

    fn environment(&mut self) -> super::Environment {
        self.environment.clone()
    }

    fn clock(&mut self) -> Duration {
        self.clock = self.clock.saturating_add(FAKE_TICK);
        self.clock
    }

    fn fresh_dir(&mut self, path: &str) -> io::Result<()> {
        let line = format!("fresh {path}");
        self.ran.push(line.clone());
        if self.broken.as_deref() == Some(line.as_str()) {
            return Err(io::Error::other("busy"));
        }
        Ok(())
    }

    /// Answers a captured command with the file named `capture <command line>`, or fails
    /// with code 1 saying "failed". The tools of the preflight answer with a version unless
    /// the test breaks them.
    fn capture(&mut self, cmd: &Cmd) -> Result<String, CommandFailure> {
        let line = cmd.line();
        self.ran.push(format!("capture {line}"));
        if self.broken.as_deref() == Some(line.as_str()) {
            return Err(CommandFailure::of(
                cmd,
                Failed::NotStarted("not found".to_owned()),
            ));
        }
        let tool = super::TOOLS.iter().any(|tool| {
            tool.program == cmd.program
                && tool
                    .args
                    .iter()
                    .copied()
                    .eq(cmd.args.iter().map(String::as_str))
        });
        if tool {
            return Ok(tool_answer(
                cmd.program,
                cmd.args.first().map(String::as_str),
            ));
        }
        self.files
            .get(&format!("capture {line}"))
            .cloned()
            .or_else(|| self.git_from_its_diff(cmd))
            .ok_or_else(|| {
                CommandFailure::of(
                    cmd,
                    Failed::Ended {
                        exit: Exit::Code(1_i32),
                        reason: Some("failed".to_owned()),
                    },
                )
            })
    }

    /// Matches the pathspecs this crate uses, `*<suffix>`, against its files (the answers of
    /// captured commands apart).
    fn tracked(&mut self, patterns: &[&str]) -> Result<Vec<String>, CommandFailure> {
        let line = format!("list {}", patterns.join(" "));
        self.ran.push(line.clone());
        if self.broken.as_deref() == Some(line.as_str()) {
            return Err(CommandFailure {
                line: format!("git ls-files -- {}", patterns.join(" ")),
                failed: Failed::Ended {
                    exit: Exit::Code(128_i32),
                    reason: Some("fatal: not a git repository".to_owned()),
                },
            });
        }
        Ok(self
            .files
            .keys()
            .filter(|path| !path.starts_with("capture "))
            // Files outside the repository (the toolchain's) are never tracked.
            .filter(|path| !path.starts_with('/'))
            .filter(|path| {
                patterns
                    .iter()
                    .filter_map(|pattern| pattern.split_once('*'))
                    .any(|(prefix, suffix)| path.starts_with(prefix) && path.ends_with(suffix))
            })
            .cloned()
            .collect())
    }
}

impl Fake {
    /// What git answers to `cmd`, unscripted, from the diff a test scripted for the same
    /// commit ([`diff_since`]), as real git does: the paths of its sections, NUL-separated
    /// (`diff --name-only -z`), or its sections but those below the directories excluded by
    /// `:(exclude)<dir>/` pathspecs.
    fn git_from_its_diff(&self, cmd: &Cmd) -> Option<String> {
        let args: Vec<&str> = cmd.args.iter().map(String::as_str).collect();
        if cmd.program != "git" {
            return None;
        }
        match args.as_slice() {
            [
                "diff",
                "--no-color",
                "--name-only",
                "-z",
                "--no-renames",
                since,
            ] => Some(
                diff_sections(self.files.get(&diff_since(since))?)
                    .into_iter()
                    .fold(String::new(), |mut listed, (path, _)| {
                        listed.push_str(&path);
                        listed.push('\0');
                        listed
                    }),
            ),
            [
                "diff",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "--no-renames",
                "--src-prefix=a/",
                "--dst-prefix=b/",
                since,
                "--",
                excluded @ ..,
            ] => {
                let dirs: Vec<&str> = excluded
                    .iter()
                    .map(|pathspec| pathspec.strip_prefix(":(exclude)"))
                    .collect::<Option<_>>()?;
                Some(
                    diff_sections(self.files.get(&diff_since(since))?)
                        .into_iter()
                        .filter(|(path, _)| !dirs.iter().any(|dir| path.starts_with(dir)))
                        .map(|(_, section)| section)
                        .collect(),
                )
            }
            _ => None,
        }
    }
}

/// The sections of `diff`, a diff a test scripted, each with the path it changes.
fn diff_sections(diff: &str) -> Vec<(String, String)> {
    let mut sections: Vec<(String, String)> = Vec::new();
    for line in diff.split_inclusive('\n') {
        if let Some(rest) = line.strip_prefix("diff --git a/") {
            let (path, _) = rest
                .split_once(" b/")
                .expect("a scripted diff names its path");
            sections.push((path.to_owned(), String::new()));
        }
        if let Some((_, section)) = sections.last_mut() {
            section.push_str(line);
        }
    }
    sections
}

/// One file of an llvm-cov JSON summary: its path and its (count, covered) lines,
/// functions and branches.
type Measure<'path> = (&'path str, (u64, u64), (u64, u64), (u64, u64));

/// An llvm-cov JSON summary of these files, with their absolute paths.
fn report(files: &[Measure<'_>]) -> String {
    let entry = |(path, lines, functions, branches): &Measure<'_>| {
        format!(
            r#"{{"filename":"/repo/{path}","summary":{{"lines":{{"count":{},"covered":{}}},"functions":{{"count":{},"covered":{}}},"branches":{{"count":{},"covered":{}}}}}}}"#,
            lines.0, lines.1, functions.0, functions.1, branches.0, branches.1
        )
    };
    format!(
        r#"{{"data":[{{"files":[{}]}}]}}"#,
        files.iter().map(entry).collect::<Vec<_>>().join(",")
    )
}

/// A report covering the measured file of `coverage` fully.
fn full_report(coverage: Coverage) -> String {
    let file = match coverage.measured {
        Measured::Library => "lib.rs",
        Measured::Binary => "main.rs",
    };
    report(&[(
        &format!("{}/src/{file}", coverage.krate.dir),
        (10, 10),
        (3, 3),
        (4, 4),
    )])
}

/// The library measurement of a crate named `name` in `crates/a`.
const CRATE_A: Coverage = Coverage {
    krate: RustCrate {
        name: "a",
        dir: "crates/a",
        binary: true,
        test_data: &[],
        roles: &[],
    },
    measured: Measured::Library,
};

/// The binary measurement of [`CRATE_A`].
const CRATE_A_MAIN: Coverage = Coverage {
    measured: Measured::Binary,
    ..CRATE_A
};

/// An attribute lifting the lint against unsafe code, built so that this file does not
/// contain the lint name itself (the unsafe gate reads it too).
fn lifting() -> String {
    format!(
        "#![expect(\n    {}_{},\n    reason = \"audited\"\n)]\n",
        "unsafe", "code"
    )
}

/// The text of the audited file `path`: [`lifting`], and inline assembly in the files of
/// `ASSEMBLY_FILES`.
fn audited(path: &str) -> String {
    if super::ASSEMBLY_FILES.contains(&path) {
        format!(
            "{}fn f() {{\n    unsafe {{\n        core::arch::asm!(\"nop\");\n    }}\n}}\n",
            lifting()
        )
    } else {
        lifting()
    }
}

/// The licenses of what the apps ship, all allowed.
const SHIPPED: &str = r#"{"MIT":[{"name":"react"},{"name":"react-dom"}],"ISC":[{"name":"a"}]}"#;

/// The licenses of all tooling, all allowed (sharp's library by its exception).
const TOOLING: &str = r#"{"Apache-2.0":[{"name":"typescript"}],"MPL-2.0":[{"name":"axe-core"}],"LGPL-3.0-or-later":[{"name":"@img/sharp-libvips-linux-x64"}]}"#;

/// A Vitest coverage summary at 100%.
const TS_COVERED: &str = r#"{"total":{"lines":{"total":4,"covered":4},"statements":{"total":5,"covered":5},"functions":{"total":2,"covered":2},"branches":{"total":3,"covered":3}}}"#;

/// A Stryker report where every mutant was killed (or timed out, or was disabled).
const TS_KILLED: &str = r#"{"files":{"src/a.ts":{"mutants":[{"status":"Killed","mutatorName":"A","location":{"start":{"line":1}}},{"status":"Timeout","mutatorName":"B","location":{"start":{"line":2}}}]}}}"#;

/// A Stryker report of the shared policy: killed mutants, and one ignored where the
/// reviewed disable comment of [`DISABLED_SOURCE`] says so.
const TS_KILLED_OR_DISABLED: &str = r#"{"files":{"src/a.ts":{"mutants":[{"status":"Killed","mutatorName":"A","location":{"start":{"line":1}}}]},"x.ts":{"mutants":[{"status":"Ignored","mutatorName":"EqualityOperator","location":{"start":{"line":3}}}]}}}"#;

/// A TypeScript source of the shared policy whose third line disables one mutator.
const DISABLED_SOURCE: (&str, &str) = (
    "tooling/config/x.ts",
    "const a = 1;\n// Stryker disable next-line EqualityOperator: same result\nwhile (a < 2) {}\n",
);

/// A Stryker configuration that hides nothing.
const STRYKER_CONFIG: &str = r#"{"$schema":"s","testRunner":"vitest","mutate":["src/**/*.ts"],"reporters":["json"],"thresholds":{"break":100}}"#;

/// Gives the fake a toolchain whose float sources are consistent with its `clippy.toml`.
fn float_sources(fake: &mut Fake) {
    let (sources, clippy) = float_fixture();
    let sysroot = fake.files.insert(
        "capture rustc +nightly-2026-11-01 --print sysroot".to_owned(),
        format!("{FAKE_SYSROOT}\n"),
    );
    let version = fake.files.insert(
        "capture rustc +nightly-2026-11-01 --version".to_owned(),
        FAKE_NIGHTLY.to_owned(),
    );
    assert!(version.is_none(), "fresh fake");
    let banned = fake.files.insert(super::CLIPPY_TOML.to_owned(), clippy);
    assert!(sysroot.is_none() && banned.is_none(), "fresh fake");
    for (path, text) in sources {
        let source = fake.files.insert(path, text);
        assert!(source.is_none(), "fresh fake");
    }
}

/// For each crate that declares test data, an integration test reading every root of it
/// (path and text).
fn test_data_reader_sources() -> Vec<(String, String)> {
    let mut sources = Vec::new();
    for krate in super::RUST_CRATES {
        if krate.test_data.is_empty() {
            continue;
        }
        let mut text = format!("#![forbid({}_{})]\n", "unsafe", "code");
        for root in krate.test_data {
            text.push_str("fn f() { let _ = Path::new(env!(\"CARGO_MANIFEST_DIR\")).join(\"../../");
            text.push_str(root);
            text.push_str("\"); }\n");
        }
        sources.push((format!("{}/tests/data.rs", krate.dir), text));
    }
    sources
}

/// Adds to the fake [`test_data_reader_sources`].
fn test_data_readers(fake: &mut Fake) {
    for (path, text) in test_data_reader_sources() {
        assert!(fake.files.insert(path, text).is_none(), "fresh fake");
    }
}

/// The unviable mutants of a passing run, their reviews, and the list of every mutant.
fn unviable_answers(fake: &mut Fake) {
    let report = "crates/a/src/b.rs:3:5: replace f -> T with Default::default()\n";
    let reviewed =
        "crates/a/src/b.rs: replace f -> T with Default::default() -- T has no Default\n";
    for (path, text) in [
        (UNVIABLE_REPORT.to_owned(), report.to_owned()),
        (UNVIABLE_REVIEWED.to_owned(), reviewed.to_owned()),
        (
            format!("capture {}", super::mutants_list_command().line()),
            format!("{report}crates/a/src/b.rs:9:1: replace g with ()\n"),
        ),
    ] {
        assert!(fake.files.insert(path, text).is_none(), "fresh fake");
    }
}

fn passing() -> Fake {
    let mut fake = Fake::default();
    let fused = fake.files.insert(
        format!("capture {}", super::fused_command().line()),
        format!("running 1 test\n{}\n", super::FUSED_PROOF),
    );
    assert!(fused.is_none(), "fresh fake");
    coverage_reports(&mut fake);
    unviable_answers(&mut fake);
    test_data_readers(&mut fake);
    let equivalent = "tooling/config/x.ts: EqualityOperator at while (a < 2) {} -- same result\n";
    typescript_reports(&mut fake);
    for (command, answer) in GIT_ANSWERS {
        let recorded = fake
            .files
            .insert(format!("capture {command}"), answer.to_owned());
        assert!(recorded.is_none(), "fresh fake");
    }
    for (line, licenses) in LICENSE_LINES.iter().zip([SHIPPED, TOOLING]) {
        let replaced = fake
            .files
            .insert(format!("capture {line}"), licenses.to_owned());
        assert!(replaced.is_none(), "fresh fake");
    }
    for path in UNSAFE_FILES {
        let previous_unsafe = fake.files.insert(path.to_owned(), audited(path));
        assert!(previous_unsafe.is_none(), "fresh fake");
    }
    float_sources(&mut fake);
    for (line, text) in HISTORY_LINES
        .into_iter()
        .zip([HISTORY, HISTORY_FILES, HISTORY_SCOPED])
    {
        let logged = fake
            .files
            .insert(format!("capture {line}"), text.to_owned());
        assert!(logged.is_none(), "fresh fake");
    }
    for (path, text) in [
        (super::WORKFLOW, PINNED_WORKFLOW),
        (super::NODE_VERSION_FILE, "22.23.4\n"),
        (super::RUST_TOOLCHAIN_FILE, TOOLCHAIN),
    ] {
        let pinned = fake.files.insert(path.to_owned(), text.to_owned());
        assert!(pinned.is_none(), "fresh fake");
    }
    for (path, text) in READMES {
        let readme = fake.files.insert(path.to_owned(), text.to_owned());
        assert!(readme.is_none(), "fresh fake");
    }
    let third = fake
        .files
        .insert(DISABLED_SOURCE.0.to_owned(), DISABLED_SOURCE.1.to_owned());
    let bypasses = fake.files.insert(
        super::BYPASSES_REVIEWED.to_owned(),
        SCOPE_REVIEWED.to_owned(),
    );
    assert!(bypasses.is_none(), "fresh fake");
    let fourth = fake
        .files
        .insert(EQUIVALENT_REVIEWED.to_owned(), equivalent.to_owned());
    assert!(third.is_none() && fourth.is_none(), "fresh fake");
    for (path, text) in [
        (CARGO_TOML.to_owned(), CARGO),
        (CHANGELOG.to_owned(), LOG),
        (
            "COMMIT_EDITMSG".to_owned(),
            "feat(engine-core): add the plan\n",
        ),
        (super::CARGO_CONFIG_FILE.to_owned(), super::CARGO_CONFIG),
        (super::ROADMAP_BOARD.to_owned(), BOARD),
        (super::VOCABULARY.to_owned(), FAKE_WORDS),
        (super::DEPENDABOT.to_owned(), DEPENDABOT),
        (
            super::PNPM_WORKSPACE.to_owned(),
            "minimumReleaseAge: 10080\n",
        ),
        (super::DENY_TOML.to_owned(), DENY),
        (
            super::DEPENDENCIES_REVIEWED.to_owned(),
            "x: serde_json (normal) -- reads JSON\n",
        ),
        (
            format!("capture {}", super::metadata_command().line()),
            METADATA,
        ),
    ]
    .into_iter()
    .chain(package_jsons().into_iter().map(|path| (path, PACKAGE)))
    .chain(
        super::RUST_CRATES
            .iter()
            .map(|krate| (format!("{}/Cargo.toml", krate.dir), MEMBER)),
    ) {
        let earlier = fake.files.insert(path, text.to_owned());
        assert!(earlier.is_none(), "fresh fake");
    }
    fake
}

/// Gives every coverage measurement of `fake` a full report, and the locality fixture the
/// report the gate must refuse.
fn coverage_reports(fake: &mut Fake) {
    for step in check_steps(CHANGES_MODE) {
        if let Action::Coverage(coverage) = step.action {
            let previous = fake.files.insert(coverage.report(), full_report(coverage));
            assert!(previous.is_none(), "fresh fake");
        }
    }
    let locality = Coverage {
        krate: super::COVERAGE_LOCALITY,
        measured: Measured::Library,
    };
    let refused = report(&[(
        &format!("{}/src/lib.rs", super::COVERAGE_LOCALITY.dir),
        (10, 7),
        (3, 2),
        (2, 2),
    )]);
    let previous = fake.files.insert(locality.report(), refused);
    assert!(previous.is_none(), "fresh fake");
}

/// The review, in the fake's register, of the one source of the fake no package measures.
const SCOPE_REVIEWED: &str = "tooling/config/x.ts: not measured -- a source of the fake\n";

/// A coverage summary like [`TS_COVERED`], measuring `src/a.ts` of the package in `dir`,
/// with the absolute path Vitest writes.
fn ts_covered(dir: &str) -> String {
    TS_COVERED.replacen(
        "{\"total\"",
        &format!("{{\"/r/{dir}/src/a.ts\":{{}},\"total\""),
        1,
    )
}

/// Gives every package of `fake` a source, `src/a.ts`, a fully covered, fully killed report
/// measuring it and a Stryker configuration that hides nothing and mutates it.
fn typescript_reports(fake: &mut Fake) {
    for package in PACKAGES {
        let source = fake.files.insert(
            format!("{}/src/a.ts", package.dir),
            "export const A = 1;\n".to_owned(),
        );
        assert!(source.is_none(), "fresh fake");
        let summary = fake.files.insert(
            format!("{}/coverage/coverage-summary.json", package.dir),
            ts_covered(package.dir),
        );
        let mutations = if package.dir == "tooling/config" {
            TS_KILLED_OR_DISABLED
        } else {
            TS_KILLED
        };
        let mutation = fake.files.insert(
            format!("{}/reports/mutation/mutation.json", package.dir),
            mutations.to_owned(),
        );
        let config = fake.files.insert(
            format!("{}/stryker.config.json", package.dir),
            STRYKER_CONFIG.to_owned(),
        );
        assert!(
            summary.is_none() && mutation.is_none() && config.is_none(),
            "fresh fake"
        );
    }
}

/// What an installed tool of the preflight answers: its pinned version for a pinned Cargo
/// tool, a recent Node.js, any version for the others.
fn tool_answer(program: &str, subcommand: Option<&str>) -> String {
    let pinned = super::PINNED_TOOLS
        .iter()
        .find(|(name, _)| subcommand == name.strip_prefix("cargo-"));
    match (program, pinned) {
        ("node", _) => "v22.23.4\n".to_owned(),
        ("git", _) => "git version 2.39.2 (Apple Git-143)\n".to_owned(),
        ("rustc", _) => "rustc 1.99.1 (58cf8f9c3 2026-10-13)\n".to_owned(),
        (_, Some((name, version))) => format!("{name} {version}\n"),
        _ => "1.0\n".to_owned(),
    }
}

/// An environment that lets the checks run: no build variable, no Cargo configuration from
/// outside the repository.
const CLEAN: super::Environment = super::Environment {
    variables: Vec::new(),
    cargo_configs: Vec::new(),
};

/// A toolchain file pinning the release the fake's `rustc` answers.
const TOOLCHAIN: &str = "# Pinned.\n[toolchain]\nchannel = \"1.99.1\"\nprofile = \"minimal\"\n";

/// The condition every job of the workflow holds: a push runs it only when asked with `[ci]`.
const CI_CONDITION: &str =
    "if: github.event_name != 'push' || contains(github.event.head_commit.message, '[ci]')";

/// A workflow that pins everything, as the workflow gate requires.
const PINNED_WORKFLOW: &str = concat!(
    "on:\n  push:\n  pull_request:\n  workflow_dispatch:\n\npermissions:\n  contents: read\n\n",
    "concurrency:\n  group: ${{ (github.event_name != 'push' || contains(github.event.head_commit.message, '[ci]')) && format('quality-{0}', github.ref) || format('skipped-{0}', github.run_id) }}\n  cancel-in-progress: true\n\n",
    "jobs:\n",
    "  check:\n",
    "    if: github.event_name != 'push' || contains(github.event.head_commit.message, '[ci]')\n",
    "    runs-on: ubuntu-24.04\n",
    "    strategy:\n      fail-fast: false\n      matrix:\n",
    "        part: [rust, coverage, mutation-0, mutation-1, mutation-2, mutation-3, mutation-4, mutation-5, mutation-6, mutation-7, ",
    "miri",
    ", gates, tooling-config, tooling-oracle, bindings-wasm, apps-playground, apps-roadmap]\n",
    "    steps:\n",
    "      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1\n        with:\n          fetch-depth: 0\n          persist-credentials: false\n",
    "      - name: Install Rust toolchains\n        run: rustup toolchain install nightly-2026-11-01 --profile minimal\n",
    "      - uses: taiki-e/install-action@cfdb7ed4ccc7c7dce37759eae89dc5674b469b39\n        with:\n          tool: cargo-llvm-cov@0.9.1,cargo-mutants@27.1.0,cargo-deny@0.20.2\n",
    "      - uses: actions/setup-node@949feb2413d6458794dcd2491c4babbbce0c15c1 # v7.1.0\n        with:\n          node-version-file: .node-version\n",
    "      - uses: Swatinem/rust-cache@d1527c8f67ef9af357290475d4af4b0fab8f12e0 # v2.9.3\n        with:\n          cache-on-failure: true\n",
    "      - name: Start an audio server with a null sink\n        run: |\n",
    "          sudo apt-get update\n",
    "          sudo apt-get install --yes --no-install-recommends pulseaudio=1:16.1+dfsg1-2ubuntu10.1\n",
    "          pulseaudio --start --exit-idle-time=-1\n",
    "          pactl load-module module-null-sink sink_name=null\n",
    "          pactl set-default-sink null\n",
    "          pactl get-default-sink | grep --quiet --line-regexp null || { echo \"::error::no audio sink: Firefox cannot play live (the runner has no sound card)\"; exit 1; }\n",
    "      - name: One part of the full check\n        run: cargo xtask check --full --part ${{ matrix.part }}\n",
    "      - uses: actions/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9 # v7.0.2\n        with:\n          name: unviable-${{ matrix.part }}\n          path: mutants.out/unviable.txt\n",
    "      - uses: actions/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9 # v7.0.2\n        if: failure()\n        with:\n          path: |\n            mutants.out\n            target/coverage\n            bindings/wasm/dist/engine.wasm\n            apps/*/coverage\n            apps/*/reports\n            bindings/*/coverage\n            bindings/*/reports\n            tooling/*/coverage\n            tooling/*/reports\n",
    "  final:\n",
    "    needs: check\n",
    "    if: github.event_name != 'push' || contains(github.event.head_commit.message, '[ci]')\n",
    "    runs-on: ubuntu-24.04\n",
    "    steps:\n",
    "      - uses: actions/download-artifact@9000827ccba6bdab643e8b6fd33ac0654aef8333 # v8.0.2\n        with:\n          pattern: unviable-*\n          path: mutants.out/shards\n",
    "      - name: The final part\n        run: cargo xtask check --full --part final\n",
);

/// A history whose two commits follow the convention, the change of behavior with its
/// changelog entry, as `git log` writes it for the history gate.
const HISTORY: &str = "\u{1e}bbbbbbbbbb\u{1f}feat(engine-core): add the plan\n\nWhy.\n\u{1e}aaaaaaaaaa\u{1f}docs: start\n";

/// The files the commits of [`HISTORY`] touched.
const HISTORY_FILES: &str =
    "\u{1e}bbbbbbbbbb\n\nCHANGELOG.md\ncrates/a.rs\n\u{1e}aaaaaaaaaa\n\nREADME.md\n";

/// The commits of [`HISTORY`] whose scope is held to the registered scopes: both.
const HISTORY_SCOPED: &str = "bbbbbbbbbb\naaaaaaaaaa\n";

/// The READMEs of a repository whose parts are all described: each TypeScript package has
/// its own, each folder names its parts, the root names the folders.
const READMES: [(&str, &str); 10] = [
    (
        "README.md",
        "| `apps/` | | `bindings/` | | `crates/` | | `tooling/` |",
    ),
    (
        "crates/README.md",
        "`engine-core` `engine-objects` `engine-patch` `engine-hash` `engine-alloc-counter` \
         `engine-fpenv` `engine-battery` `engine-testkit` `engine-cli` `xtask`",
    ),
    ("apps/README.md", "| `roadmap/` | | `playground/` |"),
    ("bindings/README.md", "- [`wasm/`](wasm/)"),
    ("tooling/README.md", "| `config/` | | `oracle/` |"),
    ("apps/roadmap/README.md", "# roadmap"),
    ("apps/playground/README.md", "# playground"),
    ("bindings/wasm/README.md", "# wasm"),
    ("tooling/config/README.md", "# config"),
    ("tooling/oracle/README.md", "# oracle"),
];

const CARGO: &str = "[workspace]\nmembers = []\n\n[workspace.package]\nedition = \"2024\"\nversion = \"0.2.0\"\n\n[other]\nversion = \"9.9.9\"\n";
/// The manifest of a crate that takes the workspace version.
const MEMBER: &str = "[package]\nversion.workspace = true\nrepository.workspace = true\n\n[lints]\nworkspace = true\n";
const PACKAGE: &str = r#"{"name": "x", "version": "0.2.0", "engines": {"node": ">=22.18"}}"#;
/// Every workspace manifest with the same content.
fn packages(text: &str) -> Vec<(String, String)> {
    package_jsons()
        .into_iter()
        .map(|path| (path, text.to_owned()))
        .collect()
}

/// The roadmap's manifest.
const ROADMAP_PACKAGE: &str = "apps/roadmap/package.json";

/// Every manifest correct except the roadmap one, with `text`.
fn roadmap_package(text: &str) -> Vec<(String, String)> {
    package_jsons()
        .into_iter()
        .map(|path| {
            let content = if path == ROADMAP_PACKAGE {
                text
            } else {
                PACKAGE
            };
            (path, content.to_owned())
        })
        .collect()
}

const LOG: &str = "# Changelog\n\nIntro.\n\n## [Unreleased]\n\n### Added\n\n- x\n\n## [0.2.0] - 2026-11-07\n\n## [0.1.10] - 2026-10-01\n\n## [0.1.2] - 2026-09-30\n";

fn command_lines(steps: &[Step]) -> Vec<String> {
    steps
        .iter()
        .map(|step| match &step.action {
            Action::Run(cmd) => format!("{:?}: {} {}", cmd.dir, cmd.program, cmd.args.join(" ")),
            Action::Coverage(coverage) => format!("coverage {}", coverage.command().line()),
            Action::CoverageLocality => "coverage locality".to_owned(),
            Action::RustMutantsOfChanges { jobs } => {
                format!("rust mutants of the changes ({jobs} jobs)")
            }
            Action::UnviableGate(extent) => format!("unviable gate ({extent:?})"),
            Action::MutationProofRecorded => "mutation proof recorded".to_owned(),
            Action::Bless(options) => format!("bless {}", super::bless_command(options).line()),
            Action::UnviableReviewsCurrent => "unviable reviews current".to_owned(),
            Action::EquivalentGate => "equivalent gate".to_owned(),
            Action::VersionGate => "version gate".to_owned(),
            Action::ReadmeGate => "readme gate".to_owned(),
            Action::VocabularyGate => "vocabulary gate".to_owned(),
            Action::FreshDirs(paths) => format!("fresh {}", paths.join(" ")),
            Action::TypeScriptReportsGate(package) => {
                format!("typescript reports gate of {}", package.dir)
            }
            Action::StrykerConfigGate => "stryker config gate".to_owned(),
            Action::TypeScriptScopeGate(package) => {
                format!("typescript scope gate of {}", package.dir)
            }
            Action::TypeScriptMutationGate { package, extent } => {
                format!("typescript mutation gate of {} ({extent:?})", package.dir)
            }
            Action::BypassGate => "bypass gate".to_owned(),
            Action::MembershipGate => "membership gate".to_owned(),
            Action::LicenseGate => "license gate".to_owned(),
            Action::UnsafeGate => "unsafe gate".to_owned(),
            Action::CargoConfigGate => "cargo config gate".to_owned(),
            Action::TestCodeGate => "test code gate".to_owned(),
            Action::TestDataGate => "test data gate".to_owned(),
            Action::UntestedTargetsGate => "untested targets gate".to_owned(),
            Action::FloatMethodGate => "float method gate".to_owned(),
            Action::DependencyGate => "dependency gate".to_owned(),
            Action::PinGate => "pin gate".to_owned(),
            Action::LibraryGate => "library gate".to_owned(),
            Action::CitationGate => "citation gate".to_owned(),
            Action::FusedBuild => "fused build".to_owned(),
            Action::MathLayerGate => "math layer gate".to_owned(),
            Action::SupplyChainGate => "supply chain gate".to_owned(),
            Action::Preflight => "preflight".to_owned(),
            Action::HistoryGate => "history gate".to_owned(),
            Action::WorkflowGate => "workflow gate".to_owned(),
            Action::PartsGate => "parts gate".to_owned(),
            Action::ShardsUnviableGate(reports) => {
                format!("unviable gate over {}", reports.join(" "))
            }
            Action::TreeRecorded => "tree recorded".to_owned(),
            Action::TreeUnchanged => "tree unchanged".to_owned(),
        })
        .collect()
}

/// The sysroot the fake nightly answers.
const FAKE_SYSROOT: &str = "/toolchain";

/// The version the fake nightly answers: newer than [`RELEASE`].
const FAKE_NIGHTLY: &str = "rustc 1.101.0-nightly (f9ceb4a4d 2026-10-31)\n";

/// The release of the fake toolchain file, [`TOOLCHAIN`].
const RELEASE: super::Release = (1, 99, 1);

/// A source of the standard library declaring `methods` (name, stability, documentation)
/// in an `impl` block of `float`, between a constant, a call of the reviewed macro
/// `float_impl!` and the closing brace; each method's body calls a macro.
fn float_source(float: &str, methods: &[(&str, bool, &str)]) -> String {
    use core::fmt::Write as _;
    let mut text = format!(
        "//! Floats.\n\npub mod consts {{}}\n\nimpl {float} {{\n    pub const RADIX: u32 = 2;\n\n    float_impl! {{\n        Self = {float},\n    }}\n\n"
    );
    for (name, stable, doc) in methods {
        let attribute = if *stable {
            "#[stable(feature = \"rust1\", since = \"1.0.0\")]"
        } else {
            "#[unstable(feature = \"x\", issue = \"1\")]"
        };
        let _written = writeln!(
            text,
            "    /// {doc}\n    ///\n    /// ```\n    /// let x = 1.0;\n    /// ```\n    {attribute}\n    #[inline]\n    pub const fn {name}(self) -> {float} {{\n        debug_assert!(true);\n        self\n    }}\n"
        );
    }
    text.push_str("}\n\nfn outside() {}\n");
    text
}

/// The source of the reviewed macro `float_impl!`: it defines a constant, and names a
/// function only in a comment and in an attribute.
const FLOAT_MACRO_SOURCE: &str = "#[rustc_macro_transparency = \"semiopaque\"]\npub(crate) macro float_impl(\n    Self = $SelfT:ty,\n) {\n    /// Not a fn here.\n    #[doc = concat!(\"fn get_exp(a: \", stringify!($SelfT), \")\")]\n    pub const RADIX: u32 = 2;\n}\n";

/// Consistent float sources, every reviewed method stable in `core`, `sin` banned (and
/// documented as such) and an unstable `gamma` in `std`, then the source of `float_impl!`,
/// with the `clippy.toml` banning `sin` (and a method that is not a float's).
fn float_fixture() -> (Vec<(String, String)>, String) {
    let reviewed: Vec<(&str, bool, &str)> = super::DETERMINISTIC_FLOAT_METHODS
        .iter()
        .map(|(name, _)| (*name, true, "Exact."))
        .collect();
    let std_methods = [
        ("sin", true, "# Unspecified precision"),
        ("gamma", false, "non-deterministic"),
    ];
    let sources = super::FLOAT_SOURCES
        .iter()
        .map(|path| {
            let float = if path.ends_with("f32.rs") {
                "f32"
            } else {
                "f64"
            };
            let methods: &[(&str, bool, &str)] = if path.contains("/core/") {
                &reviewed
            } else {
                &std_methods
            };
            (
                format!("{FAKE_SYSROOT}/{path}"),
                float_source(float, methods),
            )
        })
        .chain(super::FLOAT_ITEM_MACROS.iter().map(|(_, path)| {
            (
                format!("{FAKE_SYSROOT}/{path}"),
                FLOAT_MACRO_SOURCE.to_owned(),
            )
        }))
        .collect();
    let clippy = "disallowed-methods = [\n  { path = \"std::env::args\", reason = \"r\" },\n  { path = \"f32::sin\", reason = \"r\" },\n  { path = \"f64::sin\", reason = \"r\" },\n]\n".to_owned();
    (sources, clippy)
}

#[test]
fn accepts_every_float_method_banned_or_reviewed() {
    let (sources, clippy) = float_fixture();
    assert_eq!(
        super::float_gate(&sources, FAKE_NIGHTLY, RELEASE, &clippy),
        Ok(()),
        "consistent"
    );
    let methods = super::parse_float_methods(
        "f64.rs",
        &float_source(
            "f64",
            &[
                ("abs", true, "Exact."),
                ("gamma", false, "Unspecified precision"),
                ("log2", true, "Unspecified precision"),
            ],
        ),
    )
    .expect("parsed");
    assert_eq!(
        methods,
        vec![
            super::FloatMethod {
                float: "f64".to_owned(),
                name: "abs".to_owned(),
                since: Some((1, 0, 0)),
                documented_varying: false
            },
            super::FloatMethod {
                float: "f64".to_owned(),
                name: "gamma".to_owned(),
                since: None,
                documented_varying: true
            },
            super::FloatMethod {
                float: "f64".to_owned(),
                name: "log2".to_owned(),
                since: Some((1, 0, 0)),
                documented_varying: true
            },
        ],
        "every method, with its stability and documentation"
    );
    let unsafe_method = "impl f32 {\n    #[stable(feature = \"a\", since = \"1.0.0\")]\n    pub unsafe fn to_int_unchecked<Int>(self) -> Int {}\n    /// It is non-deterministic.\n    #[stable(feature = \"a\", since = \"1.0.0\")]\n    pub fn sin(self) -> f32 {}\n}\n";
    let parsed = super::parse_float_methods("f32.rs", unsafe_method).expect("parsed");
    assert_eq!(
        parsed
            .iter()
            .map(|method| (method.name.as_str(), method.documented_varying))
            .collect::<Vec<_>>(),
        [("to_int_unchecked", false), ("sin", true)],
        "unsafe and const functions, and the documentation of each method alone"
    );
}

/// A change of a source's text.
type SourceEdit = Box<dyn Fn(String) -> String>;

/// The fixture with one change of the `core` source of `f64`: `edit` maps its text.
fn float_sources_with(edit: impl Fn(String) -> String) -> Vec<(String, String)> {
    float_fixture()
        .0
        .into_iter()
        .map(|(path, text)| {
            if path.ends_with("core/src/num/f64.rs") {
                (path, edit(text))
            } else {
                (path, text)
            }
        })
        .collect()
}

#[test]
fn refuses_a_float_method_neither_banned_nor_reviewed() {
    let (_, clippy) = float_fixture();
    let sources =
        float_sources_with(|text| text.replace("pub const fn abs(", "pub const fn lerp("));
    assert_eq!(
        super::float_gate(&sources, FAKE_NIGHTLY, RELEASE, &clippy),
        found(
            "the float methods of the pinned toolchain",
            &[
                "`f64::lerp` is neither banned in clippy.toml nor reviewed as deterministic (DETERMINISTIC_FLOAT_METHODS in xtask)".to_owned(),
            ]
        ),
        "a new method"
    );
    // A method only `f64` lacks keeps its review current through `f32`.
    let unstable = float_sources_with(|text| {
        text.replace("#[stable(feature = \"rust1\", since = \"1.0.0\")]\n    #[inline]\n    pub const fn abs(", "#[unstable(feature = \"x\", issue = \"1\")]\n    #[inline]\n    pub const fn abs(")
    });
    assert_eq!(
        super::float_gate(&unstable, FAKE_NIGHTLY, RELEASE, &clippy),
        Ok(()),
        "an unstable method needs no classification"
    );
}

#[test]
fn refuses_a_float_method_both_banned_and_reviewed_or_documented_as_varying() {
    let (sources, clippy) = float_fixture();
    let both = clippy.replace("]\n", "  { path = \"f64::abs\", reason = \"r\" },\n]\n");
    assert_eq!(
        super::float_gate(&sources, FAKE_NIGHTLY, RELEASE, &both),
        found(
            "the float methods of the pinned toolchain",
            &["`f64::abs` is both banned in clippy.toml and reviewed as deterministic".to_owned()]
        ),
        "both"
    );
    for warning in [
        "# Unspecified precision",
        "may return either, non-deterministically",
    ] {
        let varying = float_sources_with(|text| {
            text.replace("    /// Exact.\n    ///\n    /// ```\n    /// let x = 1.0;\n    /// ```\n    #[stable(feature = \"rust1\", since = \"1.0.0\")]\n    #[inline]\n    pub const fn sqrt(", &format!("    /// {warning}\n    #[stable(feature = \"rust1\", since = \"1.0.0\")]\n    #[inline]\n    pub const fn sqrt("))
        });
        assert_eq!(
            super::float_gate(&varying, FAKE_NIGHTLY, RELEASE, &clippy),
            found(
                "the float methods of the pinned toolchain",
                &["`f64::sqrt` is reviewed as deterministic, but its documentation says its result may differ: ban it in clippy.toml".to_owned()]
            ),
            "{warning}"
        );
    }
}

#[test]
fn refuses_stale_float_bans_and_reviews() {
    let (sources, clippy) = float_fixture();
    let stale = clippy.replace("]\n", "  { path = \"f64::lerp\", reason = \"r\" },\n]\n");
    assert_eq!(
        super::float_gate(&sources, FAKE_NIGHTLY, RELEASE, &stale),
        found(
            "the float methods of the pinned toolchain",
            &["clippy.toml bans `f64::lerp`, which the pinned toolchain does not have".to_owned()]
        ),
        "a stale ban"
    );
    // An unstable method can be banned: it exists.
    let unstable = clippy.replace("]\n", "  { path = \"f64::gamma\", reason = \"r\" },\n]\n");
    assert_eq!(
        super::float_gate(&sources, FAKE_NIGHTLY, RELEASE, &unstable),
        Ok(()),
        "an unstable ban"
    );
    let gone: Vec<(String, String)> = sources
        .into_iter()
        .map(|(path, text)| {
            (
                path,
                text.replace("pub const fn trunc(", "pub const fn trunk("),
            )
        })
        .collect();
    let with_trunk = clippy.replace("]\n", "  { path = \"f32::trunk\", reason = \"r\" },\n  { path = \"f64::trunk\", reason = \"r\" },\n]\n");
    assert_eq!(
        super::float_gate(&gone, FAKE_NIGHTLY, RELEASE, &with_trunk),
        found(
            "the float methods of the pinned toolchain",
            &[
                "`trunc` is reviewed as deterministic, but no stable float method has that name"
                    .to_owned()
            ]
        ),
        "a stale review"
    );
}

#[test]
fn reads_release_numbers() {
    assert_eq!(
        super::parse_release("1.99.1"),
        Some((1, 99, 1)),
        "a release"
    );
    assert_eq!(
        super::parse_release("1.101.0"),
        Some((1, 101, 0)),
        "numbers, not text"
    );
    for text in [
        "1.99", "1.99.1.0", "1.99.x", "1..1", "", "1.99.+1", "1.99. 1",
    ] {
        assert_eq!(super::parse_release(text), None, "{text:?}");
    }
}

#[test]
fn audits_the_methods_of_the_pinned_release_only() {
    let (_, clippy) = float_fixture();
    // Stabilized after the release, or by the nightly only: not callable with the release.
    for since in ["1.100.0", "CURRENT_RUSTC_VERSION"] {
        let later = float_sources_with(|text| {
            text.replace(
                "#[stable(feature = \"rust1\", since = \"1.0.0\")]\n    #[inline]\n    pub const fn abs(",
                &format!("#[stable(feature = \"rust1\", since = \"1.0.0\")]\n    #[inline]\n    pub const fn abs(self) -> f64 {{\n        self\n    }}\n\n    #[stable(feature = \"lerp\", since = \"{since}\")]\n    pub const fn lerp("),
            )
        });
        assert_eq!(
            super::float_gate(&later, FAKE_NIGHTLY, RELEASE, &clippy),
            Ok(()),
            "{since}"
        );
    }
    // Stabilized by the release itself: audited.
    let now = float_sources_with(|text| {
        text.replace(
            "#[stable(feature = \"rust1\", since = \"1.0.0\")]\n    #[inline]\n    pub const fn abs(",
            "#[stable(feature = \"rust1\", since = \"1.0.0\")]\n    #[inline]\n    pub const fn abs(self) -> f64 {\n        self\n    }\n\n    #[stable(feature = \"lerp\", since = \"1.99.1\")]\n    pub const fn lerp(",
        )
    });
    assert_eq!(
        super::float_gate(&now, FAKE_NIGHTLY, RELEASE, &clippy),
        found(
            "the float methods of the pinned toolchain",
            &["`f64::lerp` is neither banned in clippy.toml nor reviewed as deterministic (DETERMINISTIC_FLOAT_METHODS in xtask)".to_owned()]
        ),
        "stabilized in the release"
    );
    // A reviewed method the release does not have yet is a stale review.
    let (sources, _) = float_fixture();
    let unreleased: Vec<(String, String)> = sources
        .into_iter()
        .map(|(path, text)| {
            (
                path,
                text.replace(
                    "since = \"1.0.0\")]\n    #[inline]\n    pub const fn trunc(",
                    "since = \"1.100.0\")]\n    #[inline]\n    pub const fn trunc(",
                ),
            )
        })
        .collect();
    assert_eq!(
        super::float_gate(&unreleased, FAKE_NIGHTLY, RELEASE, &clippy),
        found(
            "the float methods of the pinned toolchain",
            &[
                "`trunc` is reviewed as deterministic, but no stable float method has that name"
                    .to_owned()
            ]
        ),
        "not in the release yet"
    );
}

#[test]
fn refuses_sources_older_than_the_release() {
    let (sources, clippy) = float_fixture();
    for version in [
        "rustc 1.98.0-nightly (a 2026-08-01)\n",
        "rustc 1.99.0 (a 2026-09-01)\n",
        "rustc\n",
        "rustc nightly (a)\n",
    ] {
        assert_eq!(
            super::float_gate(&sources, version, RELEASE, &clippy),
            found(
                "the float methods of the pinned toolchain",
                &[format!(
                    "the sources come from `{}`, not from a release at least 1.99.1: they may lack its methods",
                    version.trim()
                )]
            ),
            "{version}"
        );
    }
    for version in [
        "rustc 1.99.1 (58cf8f9c3 2026-10-13)\n",
        "rustc 1.99.1-nightly (a 2026-10-01)\n",
    ] {
        assert_eq!(
            super::float_gate(&sources, version, RELEASE, &clippy),
            Ok(()),
            "{version}"
        );
    }
}

#[test]
fn refuses_float_sources_it_cannot_read() {
    let (_, clippy) = float_fixture();
    let edits: [(SourceEdit, &str); 8] = [
        (
            Box::new(|text: String| {
                text.replacen(
                    "since = \"1.0.0\")]\n    #[inline]\n    pub const fn abs(",
                    "since = \"1.0\")]\n    #[inline]\n    pub const fn abs(",
                    1,
                )
            }),
            "`#[stable(feature = \"rust1\", since = \"1.0\")]` gives no release the method was stabilized in",
        ),
        (
            Box::new(|text: String| {
                text.replacen("#[stable(feature = \"rust1\", since = \"1.0.0\")]\n    #[inline]\n    pub const fn abs(", "#[stable(feature = \"rust1\")]\n    #[inline]\n    pub const fn abs(", 1)
            }),
            "`#[stable(feature = \"rust1\")]` gives no release the method was stabilized in",
        ),
        (
            Box::new(|text: String| text.replace("impl f64 {", "impl F64 {")),
            "0 `impl f32 {` or `impl f64 {` blocks, expected one",
        ),
        (
            Box::new(|text: String| format!("{text}impl f64 {{\n}}\n")),
            "2 `impl f32 {` or `impl f64 {` blocks, expected one",
        ),
        (
            Box::new(|text: String| text.replace("\n}\n\nfn outside", "\n\nfn outside")),
            "the `impl f64 {` block is never closed",
        ),
        (
            Box::new(|text: String| {
                text.replace("    pub const fn abs(", "        pub const fn abs(")
            }),
            "`pub const fn abs(self) -> f64 {` is not a method of the expected shape, with its stability",
        ),
        (
            Box::new(|text: String| text.replace("    pub const fn abs(", "    pub const fn (")),
            "`pub const fn (self) -> f64 {` is not a method of the expected shape, with its stability",
        ),
        (
            Box::new(|text: String| {
                text.replacen("#[stable(feature = \"rust1\", since = \"1.0.0\")]\n    #[inline]\n    pub const fn abs(", "#[inline]\n    pub const fn abs(", 1)
            }),
            "`pub const fn abs(self) -> f64 {` is not a method of the expected shape, with its stability",
        ),
    ];
    for (edit, problem) in edits {
        let sources = float_sources_with(edit);
        let path = format!("{FAKE_SYSROOT}/{}", super::FLOAT_SOURCES[1]);
        assert_eq!(
            super::float_gate(&sources, FAKE_NIGHTLY, RELEASE, &clippy)
                .map_err(|failure| failure.summary),
            Err(format!(
                "the float methods of the pinned toolchain: {path}: {problem}"
            )),
            "{problem}"
        );
    }
    assert_eq!(
        super::parse_float_methods("f64.rs", "impl f64 {\n    pub const RADIX: u32 = 2;\n}\n"),
        Err("f64.rs: no method in the `impl f64 {` block".to_owned()),
        "an empty block"
    );
}

#[test]
fn refuses_a_macro_among_the_float_methods_unless_reviewed() {
    let (_, clippy) = float_fixture();
    let path = format!("{FAKE_SYSROOT}/{}", super::FLOAT_SOURCES[1]);
    // A macro may define methods: among the items, only a reviewed one is accepted (the
    // fixture calls `float_impl!` there, and a macro inside each method's body).
    for call in ["cfg_select! {", "methods!(", "float_impl2!{", "!x {"] {
        let sources =
            float_sources_with(|text| text.replace("    float_impl! {", &format!("    {call}")));
        assert_eq!(
            super::float_gate(&sources, FAKE_NIGHTLY, RELEASE, &clippy),
            found(
                "the float methods of the pinned toolchain",
                &[format!(
                    "{path}: `{call}` calls a macro among the items of the `impl f64 {{` block that is not reviewed as defining only constants (FLOAT_ITEM_MACROS in xtask)"
                )]
            ),
            "{call}"
        );
    }
}

#[test]
fn refuses_a_reviewed_float_macro_that_may_define_methods() {
    let (sources, clippy) = float_fixture();
    let (name, source) = super::FLOAT_ITEM_MACROS[0];
    let macro_path = format!("{FAKE_SYSROOT}/{source}");
    let edited = |from: &str, to: &str| -> Vec<(String, String)> {
        sources
            .iter()
            .map(|(path, text)| {
                if *path == macro_path {
                    (path.clone(), text.replace(from, to))
                } else {
                    (path.clone(), text.clone())
                }
            })
            .collect()
    };
    let radix = "    pub const RADIX: u32 = 2;\n";
    for (line, problem) in [
        ("pub fn lerp(self) -> Self {}", "may define a method"),
        ("const fn helper() {}", "may define a method"),
        ("methods! {}", "may define a method"),
    ] {
        assert_eq!(
            super::float_gate(
                &edited(radix, &format!("{radix}    {line}\n")),
                FAKE_NIGHTLY,
                RELEASE,
                &clippy
            ),
            found(
                "the float methods of the pinned toolchain",
                &[format!(
                    "{macro_path}: `{line}` {problem} in `{name}!`, which is reviewed as defining only constants"
                )]
            ),
            "{line}"
        );
    }
    assert_eq!(
        super::float_gate(
            &edited("macro float_impl(", "macro float_impl2("),
            FAKE_NIGHTLY,
            RELEASE,
            &clippy
        ),
        found(
            "the float methods of the pinned toolchain",
            &[format!(
                "{macro_path}: no `macro {name}(` definition, reviewed in FLOAT_ITEM_MACROS"
            )]
        ),
        "another macro"
    );
    let unaudited: Vec<(String, String)> = sources
        .iter()
        .filter(|(path, _)| *path != macro_path)
        .cloned()
        .collect();
    assert_eq!(
        super::float_gate(&unaudited, FAKE_NIGHTLY, RELEASE, &clippy),
        found(
            "the float methods of the pinned toolchain",
            &[format!(
                "`{name}!` is reviewed as defining only constants, but its source {source} is not audited"
            )]
        ),
        "an unaudited macro"
    );
}

#[test]
fn reads_the_float_sources_of_the_toolchain_s_sysroot() {
    let mut fake = passing();
    let step = Step {
        name: "floats".to_owned(),
        action: Action::FloatMethodGate,
    };
    assert_eq!(
        run_steps(core::slice::from_ref(&step), &mut fake, &mut Vec::new()),
        Ok(()),
        "read and checked"
    );
    let read: Vec<&String> = fake
        .ran
        .iter()
        .filter(|line| line.starts_with("read ") || line.starts_with("capture rustc"))
        .collect();
    assert_eq!(
        read,
        [
            "read rust-toolchain.toml",
            "capture rustc +nightly-2026-11-01 --version",
            "capture rustc +nightly-2026-11-01 --print sysroot",
            "read /toolchain/lib/rustlib/src/rust/library/core/src/num/f32.rs",
            "read /toolchain/lib/rustlib/src/rust/library/core/src/num/f64.rs",
            "read /toolchain/lib/rustlib/src/rust/library/std/src/num/f32.rs",
            "read /toolchain/lib/rustlib/src/rust/library/std/src/num/f64.rs",
            "read /toolchain/lib/rustlib/src/rust/library/core/src/num/float_macros.rs",
            "read clippy.toml",
        ],
        "the release, the nightly's version and sysroot, then every source, then the bans"
    );
    let unpinned = fake.files.insert(
        super::RUST_TOOLCHAIN_FILE.to_owned(),
        "[toolchain]\nchannel = \"stable\"\n".to_owned(),
    );
    assert!(unpinned.is_some(), "replaced");
    assert_eq!(
        run_steps(core::slice::from_ref(&step), &mut fake, &mut Vec::new())
            .map_err(|failure| failure.summary),
        Err("floats: rust-toolchain.toml must pin one release: channel = \"X.Y.Z\"".to_owned()),
        "no release"
    );
    let pinned = fake
        .files
        .insert(super::RUST_TOOLCHAIN_FILE.to_owned(), TOOLCHAIN.to_owned());
    assert!(pinned.is_some(), "replaced");
    fake.broken = Some("rustc +nightly-2026-11-01 --version".to_owned());
    let unversioned = run_steps(core::slice::from_ref(&step), &mut fake, &mut Vec::new())
        .expect_err("no version");
    assert_eq!(
        unversioned.summary,
        "floats: cannot find the toolchain's sources: cannot run `rustc +nightly-2026-11-01 --version`: not found",
        "no version"
    );
    fake.broken = Some("rustc +nightly-2026-11-01 --print sysroot".to_owned());
    let failure = run_steps(core::slice::from_ref(&step), &mut fake, &mut Vec::new())
        .expect_err("no sysroot");
    assert!(
        failure.summary
            == "floats: cannot find the toolchain's sources: cannot run `rustc +nightly-2026-11-01 --print sysroot`: not found",
        "{}",
        failure.summary
    );
}

#[test]
fn lists_every_check_in_order() {
    let steps = check_steps(CHANGES_MODE);
    assert_eq!(
        command_lines(&steps),
        vec![
            "preflight",
            "tree recorded",
            "Root: pnpm install --frozen-lockfile",
            "fresh tooling/config/coverage tooling/config/reports/mutation tooling/oracle/coverage tooling/oracle/reports/mutation bindings/wasm/coverage bindings/wasm/reports/mutation apps/playground/coverage apps/playground/reports/mutation apps/roadmap/coverage apps/roadmap/reports/mutation",
            "fresh bindings/wasm/dist",
            "equivalent gate",
            "bypass gate",
            "unsafe gate",
            "cargo config gate",
            "test code gate",
            "test data gate",
            "untested targets gate",
            "Root: cargo run --package engine-cli --locked --quiet -- check conformance",
            "Root: cargo deny --locked check",
            "dependency gate",
            "pin gate",
            "library gate",
            "math layer gate",
            "supply chain gate",
            "membership gate",
            "readme gate",
            "citation gate",
            "vocabulary gate",
            "version gate",
            "history gate",
            "workflow gate",
            "parts gate",
            "Root: pnpm audit",
            "license gate",
            "stryker config gate",
            "Root: cargo fmt --all --check",
            "Root: cargo clippy --workspace --all-targets --locked",
            "float method gate",
            "Root: cargo doc --workspace --no-deps --document-private-items --locked",
            "Root: cargo test --workspace --locked -- --include-ignored",
            "fused build",
            "Root: cargo build --package engine-core --package engine-objects --package engine-patch --package engine-hash --locked --target aarch64-apple-darwin",
            "Root: cargo build --package engine-core --package engine-objects --package engine-patch --package engine-hash --locked --target x86_64-unknown-linux-gnu",
            "Root: cargo build --package engine-core --package engine-objects --package engine-patch --package engine-hash --locked --target wasm32-unknown-unknown",
            "Root: cargo build --package engine-core --package engine-objects --package engine-patch --package engine-hash --locked --target thumbv7em-none-eabihf",
            "Root: cargo clippy --package engine-fpenv --package engine-cli --all-targets --locked --target aarch64-apple-darwin",
            "Root: cargo clippy --package engine-fpenv --package engine-cli --all-targets --locked --target x86_64-unknown-linux-gnu",
            "Root: cargo build --lib --package engine-fpenv --package engine-cli --locked --target aarch64-apple-darwin",
            "Root: cargo build --lib --package engine-fpenv --package engine-cli --locked --target x86_64-unknown-linux-gnu",
            "fresh target/coverage",
            "coverage locality",
            "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/engine-core/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/engine-core.json -- --include-ignored",
            "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/engine-objects/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/engine-objects.json -- --include-ignored",
            "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/engine-patch/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/engine-patch.json -- --include-ignored",
            "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/engine-hash/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/engine-hash.json -- --include-ignored",
            "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/engine-alloc-counter/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/engine-alloc-counter.json -- --include-ignored",
            "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/engine-fpenv/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/engine-fpenv.json -- --include-ignored",
            "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/engine-battery/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/engine-battery.json -- --include-ignored",
            "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/engine-testkit/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/engine-testkit.json -- --include-ignored",
            "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/engine-cli/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/engine-cli.json -- --include-ignored",
            "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path bindings/wasm/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/engine-wasm.json -- --include-ignored",
            "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/xtask/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/xtask.json -- --include-ignored",
            "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/engine-cli/Cargo.toml --locked --branch --json --summary-only --output-path target/coverage/engine-cli-main.json -- --include-ignored",
            "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/xtask/Cargo.toml --locked --branch --json --summary-only --output-path target/coverage/xtask-main.json -- --include-ignored",
            &format!(
                "Root: cargo +nightly-2026-11-01 {} test --package engine-alloc-counter --package engine-wasm --locked -- --include-ignored",
                "miri"
            ),
            "Package(\"tooling/config\"): pnpm run check",
            "Package(\"tooling/config\"): pnpm exec stryker run stryker.config.json --incremental --incrementalFile reports/stryker-incremental.json",
            "typescript reports gate of tooling/config",
            "typescript scope gate of tooling/config",
            "fresh target/conformance",
            "Root: cargo run --package engine-cli --locked --quiet -- export conformance target/conformance",
            "Package(\"tooling/oracle\"): pnpm run check",
            "Package(\"tooling/oracle\"): pnpm exec stryker run stryker.config.json --incremental --incrementalFile reports/stryker-incremental.json",
            "typescript reports gate of tooling/oracle",
            "typescript scope gate of tooling/oracle",
            "Package(\"bindings/wasm\"): pnpm run check",
            "Package(\"bindings/wasm\"): pnpm exec stryker run stryker.config.json --incremental --incrementalFile reports/stryker-incremental.json",
            "typescript reports gate of bindings/wasm",
            "typescript scope gate of bindings/wasm",
            "Package(\"bindings/wasm\"): pnpm run build",
            "Package(\"apps/playground\"): pnpm run check",
            "Package(\"apps/playground\"): pnpm exec stryker run stryker.config.json --incremental --incrementalFile reports/stryker-incremental.json",
            "typescript reports gate of apps/playground",
            "typescript scope gate of apps/playground",
            "Package(\"apps/roadmap\"): pnpm run check",
            "Package(\"apps/roadmap\"): pnpm exec stryker run stryker.config.json --incremental --incrementalFile reports/stryker-incremental.json",
            "typescript reports gate of apps/roadmap",
            "typescript scope gate of apps/roadmap",
            "fresh mutants.out",
            "rust mutants of the changes (3 jobs)",
            "unviable gate (Partial)",
            "unviable reviews current",
            "mutation proof recorded",
            "tree unchanged",
        ],
        "the default check pipeline"
    );
}

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
fn names_every_check_in_order() {
    let steps = check_steps(CHANGES_MODE);
    assert_eq!(
        steps
            .iter()
            .map(|step| step.name.as_str())
            .collect::<Vec<_>>(),
        vec![
            "Tools and platform",
            "Working tree recorded",
            "JavaScript dependencies",
            "Fresh TypeScript report directories",
            "Fresh TypeScript build outputs",
            "TypeScript equivalent mutants reviewed",
            "Every bypass of coverage or mutation testing reviewed",
            "Unsafe code confined to the audited files",
            "Cargo configuration: no build flag, a locked alias",
            "Test code only in tests/ and declared *_tests.rs files",
            "The tests of each crate read only its declared test data",
            "Tests turned off only where no test can hide, doc tests kept",
            "Conformance (native)",
            "Rust supply chain",
            "Every direct dependency reviewed",
            "Bit-defining dependencies pinned to one exact version",
            "The audio library depends only on its own crates, its tests also on its test crates; every crate only tests use is shared test code",
            "Box arithmetic only through the math layer: libm in engine-core and engine-testkit only, no raw float in box code, no iterator reduction outside the math layer",
            "Supply-chain settings: a week before a release, libm without features",
            "Packages and crates known to the checks",
            "Every part described in a README",
            "Every roadmap step cited exists on the board",
            "The words of the glossary, not their synonyms",
            "One version everywhere, changelog well formed",
            "Commit messages, and a changelog entry for each change of behavior",
            "CI workflow pinned",
            "The parts of CI run every check of the full check once",
            "JavaScript vulnerabilities",
            "JavaScript licenses",
            "Stryker configurations hide no mutant",
            "Rust formatting",
            "Rust lints",
            "Float methods of the pinned toolchain banned or reviewed as deterministic",
            "Rust documentation",
            "Rust tests",
            "Boxes unchanged under the math layer's fused multiply-add option (a test build)",
            "Engine portability (aarch64-apple-darwin)",
            "Engine portability (x86_64-unknown-linux-gnu)",
            "Engine portability (wasm32-unknown-unknown)",
            "Engine portability (thumbv7em-none-eabihf)",
            "Native shell lints (aarch64-apple-darwin)",
            "Native shell lints (x86_64-unknown-linux-gnu)",
            "Native code of engine-cli and engine-fpenv built, inline assembly included (aarch64-apple-darwin)",
            "Native code of engine-cli and engine-fpenv built, inline assembly included (x86_64-unknown-linux-gnu)",
            "Fresh Rust coverage reports",
            "Rust coverage counts a crate's own unit tests only",
            "Rust coverage of engine-core: 100% by its own unit tests",
            "Rust coverage of engine-objects: 100% by its own unit tests",
            "Rust coverage of engine-patch: 100% by its own unit tests",
            "Rust coverage of engine-hash: 100% by its own unit tests",
            "Rust coverage of engine-alloc-counter: 100% by its own unit tests",
            "Rust coverage of engine-fpenv: 100% by its own unit tests",
            "Rust coverage of engine-battery: 100% by its own unit tests",
            "Rust coverage of engine-testkit: 100% by its own unit tests",
            "Rust coverage of engine-cli: 100% by its own unit tests",
            "Rust coverage of engine-wasm: 100% by its own unit tests",
            "Rust coverage of xtask: 100% by its own unit tests",
            "Rust coverage of the engine-cli binary: 100% by its tests",
            "Rust coverage of the xtask binary: 100% by its tests",
            "Unsafe code under Miri",
            "Shared TypeScript policy (each rule proven to reject what it forbids)",
            "Mutation testing of tooling/config (what changed since the last run)",
            "TypeScript coverage and mutation reports of tooling/config",
            "Every TypeScript source of tooling/config measured and mutated, or reviewed",
            "Fresh conformance export directory",
            "Conformance export (native WAV files)",
            "Conformance oracle (independent, TypeScript)",
            "Mutation testing of tooling/oracle (what changed since the last run)",
            "TypeScript coverage and mutation reports of tooling/oracle",
            "Every TypeScript source of tooling/oracle measured and mutated, or reviewed",
            "Engine WASM checks",
            "Mutation testing of bindings/wasm (what changed since the last run)",
            "TypeScript coverage and mutation reports of bindings/wasm",
            "Every TypeScript source of bindings/wasm measured and mutated, or reviewed",
            "Engine WASM build, for the packages that use it",
            "Playground checks",
            "Mutation testing of apps/playground (what changed since the last run)",
            "TypeScript coverage and mutation reports of apps/playground",
            "Every TypeScript source of apps/playground measured and mutated, or reviewed",
            "Roadmap checks",
            "Mutation testing of apps/roadmap (what changed since the last run)",
            "TypeScript coverage and mutation reports of apps/roadmap",
            "Every TypeScript source of apps/roadmap measured and mutated, or reviewed",
            "Fresh Rust mutation report",
            "Rust mutation testing of the changes",
            "Rust unviable mutants reviewed",
            "Rust unviable mutant reviews all current",
            "Rust mutation proof recorded for the next default check",
            "Working tree unchanged by the checks",
        ],
        "step names"
    );
}

/// The steps of the default check in the order of the full check, without the two it adds
/// after its unviable gate.
fn default_steps_as_full() -> Vec<Step> {
    let mut changes: Vec<Step> = super::full_order(CHANGES_MODE)
        .into_iter()
        .map(|(_, step)| step)
        .collect();
    let current = changes
        .iter()
        .position(|step| step.action == Action::UnviableReviewsCurrent)
        .expect("the default check refuses stale reviews");
    let before = changes
        .get(current.saturating_sub(1))
        .map(|step| step.action.clone());
    let added: Vec<Action> = before
        .into_iter()
        .chain(
            changes
                .drain(current..=current.saturating_add(1))
                .map(|step| step.action),
        )
        .collect();
    assert_eq!(
        added,
        [
            Action::UnviableGate(Extent::Partial),
            Action::UnviableReviewsCurrent,
            Action::MutationProofRecorded
        ],
        "after the unviable gate"
    );
    changes
}

#[test]
fn the_default_check_runs_the_steps_of_its_full_order_cheapest_first() {
    let sorted = |steps: &[Step]| {
        let mut lines = command_lines(steps);
        lines.sort();
        lines
    };
    let unordered: Vec<Step> = super::full_order(CHANGES_MODE)
        .into_iter()
        .map(|(_, step)| step)
        .collect();
    let default = check_steps(CHANGES_MODE);
    assert_ne!(
        command_lines(&default),
        command_lines(&unordered),
        "reordered"
    );
    assert_eq!(
        sorted(&default),
        sorted(&unordered),
        "the same steps, each once"
    );
    let jobs = NonZeroUsize::new(3).expect("three is not zero");
    let full: Vec<Step> = super::full_order(Mode::Full { jobs })
        .into_iter()
        .map(|(_, step)| step)
        .collect();
    assert_eq!(
        check_steps(Mode::Full { jobs }),
        full,
        "the full check keeps its order"
    );
}

#[test]
fn prepares_what_a_part_needs_before_its_first_step_in_the_default_check() {
    let roles = super::roles(CHANGES_MODE);
    let mut checked = 0_usize;
    for (at, (role, step)) in roles.iter().enumerate() {
        let super::Role::Proof(part) = role else {
            continue;
        };
        for setup in part.needs() {
            let preparing: Vec<usize> = roles
                .iter()
                .enumerate()
                .filter(|(_, (other, _))| *other == super::Role::Setup(setup))
                .map(|(index, _)| index)
                .collect();
            assert!(
                !preparing.is_empty() && preparing.iter().all(|index| *index < at),
                "{setup:?} prepared before {}",
                step.name
            );
            checked = checked.saturating_add(1);
        }
    }
    assert!(checked > 0, "some part needs a preparation");
}

#[test]
fn the_full_check_differs_only_by_its_mutation_testing() {
    let jobs = NonZeroUsize::new(3).expect("three is not zero");
    // The default check refuses the stale reviews its partial run cannot see, then
    // records the proof of its mutation testing, which the full check never reads nor
    // writes: two steps more, after the unviable gate.
    let changes = default_steps_as_full();
    let full = check_steps(Mode::Full { jobs });
    assert!(
        !full.iter().any(|step| matches!(
            step.action,
            Action::MutationProofRecorded | Action::UnviableReviewsCurrent
        )),
        "the full check records nothing, and its complete run sees stale reviews"
    );
    assert_eq!(changes.len(), full.len(), "the same number of steps");
    let differences: Vec<(String, String)> = command_lines(&changes)
        .into_iter()
        .zip(command_lines(&full))
        .filter(|(default, every)| default != every)
        .collect();
    let stryker = |dir: &str, force: &str| {
        (
            format!(
                "Package(\"{dir}\"): pnpm exec stryker run stryker.config.json --incremental --incrementalFile reports/stryker-incremental.json"
            ),
            format!(
                "Package(\"{dir}\"): pnpm exec stryker run stryker.config.json --incremental --incrementalFile reports/stryker-incremental.json{force}"
            ),
        )
    };
    let mut expected = vec![
        (
            "rust mutants of the changes (3 jobs)".to_owned(),
            "Root: cargo mutants --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --workspace --jobs 3 --timeout-multiplier 15 --minimum-test-timeout 60 -- -- --include-ignored".to_owned(),
        ),
        (
            "unviable gate (Partial)".to_owned(),
            "unviable gate (Complete)".to_owned(),
        ),
    ];
    expected.extend(
        PACKAGES
            .iter()
            .map(|package| stryker(package.dir, " --force")),
    );
    assert_eq!(differences, expected, "only the mutation testing differs");
    let names: Vec<(&str, &str)> = changes
        .iter()
        .zip(&full)
        .map(|(default, every)| (default.name.as_str(), every.name.as_str()))
        .filter(|(default, every)| default != every)
        .collect();
    assert_eq!(
        names,
        vec![
            (
                "Rust mutation testing of the changes",
                "Rust mutation testing"
            ),
            (
                "Mutation testing of tooling/config (what changed since the last run)",
                "Mutation testing of tooling/config (every mutant)"
            ),
            (
                "Mutation testing of tooling/oracle (what changed since the last run)",
                "Mutation testing of tooling/oracle (every mutant)"
            ),
            (
                "Mutation testing of bindings/wasm (what changed since the last run)",
                "Mutation testing of bindings/wasm (every mutant)"
            ),
            (
                "Mutation testing of apps/playground (what changed since the last run)",
                "Mutation testing of apps/playground (every mutant)"
            ),
            (
                "Mutation testing of apps/roadmap (what changed since the last run)",
                "Mutation testing of apps/roadmap (every mutant)"
            ),
        ],
        "the steps that differ say how much they mutate"
    );
    assert_eq!(
        MUTANTS_OUT, "mutants.out",
        "the directory cargo-mutants writes"
    );
    assert!(
        UNVIABLE_REPORT.starts_with(&format!("{MUTANTS_OUT}/")),
        "the unviable report is in it"
    );
}

/// The steps of the default check from the Rust mutation testing of the changes to its
/// unviable gate.
fn rust_mutation_of_changes() -> Vec<Step> {
    let steps = check_steps(CHANGES_MODE);
    let start = steps
        .iter()
        .position(|step| matches!(step.action, Action::RustMutantsOfChanges { .. }))
        .expect("the default check mutates the changes");
    steps.into_iter().skip(start).take(2).collect()
}

#[test]
fn mutates_the_changes_since_the_merge_base_with_the_upstream() {
    let mut fake = passing();
    let mut out = Vec::new();
    assert_eq!(
        run_steps(&rust_mutation_of_changes(), &mut fake, &mut out),
        Ok(()),
        "mutated"
    );
    assert_eq!(
        fake.ran,
        vec![
            "capture git ls-files -v",
            "capture git merge-base HEAD @{upstream}",
            "write target/xtask/mutants-base",
            "capture git rev-parse --git-common-dir",
            "read .git/poyetik/rust-mutants-proven/abc1230000000000000000000000000000000000",
            "capture git ls-files -z --others --exclude-standard -- *.rs",
            "capture git diff --no-color --name-only -z --no-renames abc1230000000000000000000000000000000000",
            "capture git diff --no-color --no-ext-diff --no-textconv --no-renames --src-prefix=a/ --dst-prefix=b/ abc1230000000000000000000000000000000000",
            "write target/xtask/changes.diff",
            "cargo mutants --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --workspace --in-diff target/xtask/changes.diff --jobs 3 --timeout-multiplier 15 --minimum-test-timeout 60 -- -- --include-ignored",
            "read mutants.out/unviable.txt",
            "read mutants-unviable.txt",
        ],
        "the diff since the merge base is written, then mutated; a change to no crate's test \
         code or test data widens it to no crate, xtask included"
    );
    assert_eq!(
        CHANGES_DIFF, "target/xtask/changes.diff",
        "below the ignored target/"
    );
    assert_eq!(
        fake.files.get(CHANGES_DIFF).map(String::as_str),
        Some(CHANGES.1),
        "the diff as git wrote it"
    );
    assert_eq!(
        String::from_utf8(out).expect("UTF-8"),
        "▶ [1/2] Rust mutation testing of the changes\n  the changes since abc1230000000000000000000000000000000000 (merge base with @{upstream})\n  done in 1.5 s\n▶ [2/2] Rust unviable mutants reviewed\n  done in 1.5 s\n✔ all 2 checks passed in 3.0 s\n",
        "the base is said"
    );
}

#[test]
fn falls_back_to_origin_main_then_main() {
    for (resolved, against) in [
        (vec!["origin/main", "main"], "origin/main"),
        (vec!["main"], "main"),
    ] {
        let mut fake = passing();
        let _upstream = fake.files.remove(&format!("capture {}", MERGE_BASE.0));
        for base in &resolved {
            let _base = fake.files.insert(
                format!("capture git merge-base HEAD {base}"),
                "abc1230000000000000000000000000000000000\n".to_owned(),
            );
        }
        let mut out = Vec::new();
        assert_eq!(
            run_steps(&rust_mutation_of_changes(), &mut fake, &mut out),
            Ok(()),
            "{against}"
        );
        let tried: Vec<&str> = fake
            .ran
            .iter()
            .map(String::as_str)
            .filter(|line| line.starts_with("capture git merge-base"))
            .collect();
        let expected: Vec<String> = ["@{upstream}", "origin/main", "main"]
            .iter()
            .take_while(|base| **base != against)
            .chain(core::iter::once(&against))
            .map(|base| format!("capture git merge-base HEAD {base}"))
            .collect();
        assert_eq!(tried, expected, "tried in order until {against}");
        assert!(
            String::from_utf8(out)
                .expect("UTF-8")
                .contains(&format!("(merge base with {against})")),
            "{against} said"
        );
    }
    let mut none = passing();
    let _upstream = none.files.remove(&format!("capture {}", MERGE_BASE.0));
    assert_eq!(
        run_steps(&rust_mutation_of_changes(), &mut none, &mut Vec::new()),
        failed(
            "Rust mutation testing of the changes: no base to measure the changes against: \
             HEAD shares no history with any of @{upstream}, origin/main, main (run `cargo xtask check --full`)\n\
             `git merge-base HEAD @{upstream}` exited with code 1: failed\n\
             `git merge-base HEAD origin/main` exited with code 1: failed\n\
             `git merge-base HEAD main` exited with code 1: failed"
        ),
        "no base: the check fails, saying why each could not be used"
    );
    assert!(
        !none
            .ran
            .iter()
            .any(|line| line.starts_with("cargo mutants")),
        "nothing mutated"
    );
}

/// The diff git writes for a change of one line of `path`.
fn one_line_change(path: &str) -> String {
    format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -1 +1 @@\n-a\n+b\n")
}

#[test]
fn mutates_a_whole_crate_whose_test_code_changed() {
    for test_code in [
        "crates/engine-core/tests/laws.rs",
        "crates/engine-core/src/plan_tests.rs",
    ] {
        let mut fake = passing();
        let _changes = fake
            .files
            .insert(format!("capture {}", CHANGES.0), one_line_change(test_code));
        for (path, text) in [
            ("crates/engine-core/src/lib.rs", "fn a() {}\n"),
            ("crates/engine-core/src/plan_tests.rs", "fn t() {}\n"),
            ("crates/engine-core/tests/laws.rs", "fn l() {}\n"),
            ("crates/engine-objects/src/lib.rs", "fn o() {}\n"),
        ] {
            let _source = fake.files.insert(path.to_owned(), text.to_owned());
        }
        let mut out = Vec::new();
        assert_eq!(
            run_steps(&rust_mutation_of_changes(), &mut fake, &mut out),
            Ok(()),
            "{test_code}"
        );
        assert_eq!(
            fake.files.get(CHANGES_DIFF).cloned(),
            Some(super::new_file_diff(
                "crates/engine-core/src/lib.rs",
                "fn a() {}\n"
            )),
            "{test_code}: every source of its crate added whole, nothing else of it, its test \
             code and other crates apart"
        );
        assert!(
            fake.ran.contains(&format!(
                "{} -- :(exclude)crates/engine-core/",
                diff_since(MERGE_BASE.1.trim())
            )),
            "{test_code}: git leaves the crate out of the diff"
        );
        assert!(
            String::from_utf8(out)
                .expect("UTF-8")
                .contains("  the test code of engine-core changed: every mutant of engine-core\n"),
            "{test_code}: said"
        );
    }
}

#[test]
fn mutates_a_whole_crate_whose_test_data_changed() {
    for (data, hash_reads, xtask_reads) in [
        ("spec/render-hash-vectors.json", true, false),
        (".githooks/pre-push", false, true),
        ("apps/roadmap/board.json", false, false),
        ("spec/render-hash-vectors.json.orig", false, false),
        (".githooks-old/pre-push", false, false),
    ] {
        let mut fake = passing();
        let _changes = fake
            .files
            .insert(format!("capture {}", CHANGES.0), one_line_change(data));
        for (path, text) in [
            ("crates/engine-hash/src/lib.rs", "fn h() {}\n"),
            ("crates/engine-hash/src/lib_tests.rs", "fn t() {}\n"),
            ("crates/engine-core/src/lib.rs", "fn a() {}\n"),
            ("crates/xtask/src/lib.rs", "fn x() {}\n"),
        ] {
            let _source = fake.files.insert(path.to_owned(), text.to_owned());
        }
        let mut out = Vec::new();
        assert_eq!(
            run_steps(&rust_mutation_of_changes(), &mut fake, &mut out),
            Ok(()),
            "{data}"
        );
        let hash = super::new_file_diff("crates/engine-hash/src/lib.rs", "fn h() {}\n");
        let xtask = super::new_file_diff("crates/xtask/src/lib.rs", "fn x() {}\n");
        let expected = format!(
            "{}{}{}",
            one_line_change(data),
            if hash_reads { hash.as_str() } else { "" },
            if xtask_reads { xtask.as_str() } else { "" }
        );
        assert_eq!(
            fake.files.get(CHANGES_DIFF).cloned(),
            Some(expected),
            "{data}: the crates that read it, whole, and no other (xtask's tests read its \
             fixtures and the hooks)"
        );
        let said = String::from_utf8(out).expect("UTF-8");
        assert_eq!(
            said.contains("  the test data of engine-hash changed: every mutant of engine-hash\n"),
            hash_reads,
            "{data}: said"
        );
        assert_eq!(
            said.contains("  the test data of xtask changed: every mutant of xtask\n"),
            xtask_reads,
            "{data}: said of xtask"
        );
    }
}

#[test]
fn mutates_everything_when_what_every_test_uses_changed() {
    // The fake's git shows no lock file at the base: every crate may build differently.
    for path in [
        "Cargo.toml",
        "Cargo.lock",
        "rust-toolchain.toml",
        "tooling/x/Cargo.toml",
        "doc/Cargo.lock",
        "apps/a/rust-toolchain.toml",
    ] {
        let mut fake = passing();
        let _changes = fake
            .files
            .insert(format!("capture {}", CHANGES.0), one_line_change(path));
        let mut out = Vec::new();
        assert_eq!(
            run_steps(&rust_mutation_of_changes(), &mut fake, &mut out),
            Ok(()),
            "{path}"
        );
        assert!(
            fake.ran.contains(
                &"cargo mutants --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --workspace --jobs 3 --timeout-multiplier 15 --minimum-test-timeout 60 -- -- --include-ignored".to_owned()
            ) && !fake.ran.iter().any(|line| line.starts_with("write target/xtask/changes")),
            "{path}: every mutant of the workspace, no diff: {:?}",
            fake.ran
        );
        let unnarrowed = if path == "Cargo.lock" {
            " and cannot be narrowed (`git show abc1230000000000000000000000000000000000:Cargo.lock` \
             exited with code 1: failed)"
        } else {
            ""
        };
        assert!(
            String::from_utf8(out).expect("UTF-8").contains(&format!(
                "  {path} changed{unnarrowed}: every Rust mutant\n"
            )),
            "{path}: said, with why a lock file is not narrowed"
        );
    }
    for path in [
        "Cargo.toml.orig",
        "crates/engine-core/Cargo.toml.orig",
        "crates/engine-testkits/a.rs",
        "doc/Cargo.lock.md",
        "doc/crates/engine-testkit/a.rs",
        "crates/engine-core/src/Cargo.toml.rs",
    ] {
        let mut fake = passing();
        let _changes = fake
            .files
            .insert(format!("capture {}", CHANGES.0), one_line_change(path));
        assert_eq!(
            run_steps(&rust_mutation_of_changes(), &mut fake, &mut Vec::new()),
            Ok(()),
            "{path}"
        );
        assert!(
            fake.ran
                .iter()
                .any(|line| line.contains("--in-diff target/xtask/changes.diff ")),
            "{path}: only the changes"
        );
    }
    let mut broken = passing();
    let _changes = broken.files.insert(
        format!("capture {}", CHANGES.0),
        one_line_change("Cargo.lock"),
    );
    broken.fail =
        Some("cargo mutants --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --workspace --jobs 3 --timeout-multiplier 15 --minimum-test-timeout 60 -- -- --include-ignored".to_owned());
    assert_eq!(
        run_steps(&rust_mutation_of_changes(), &mut broken, &mut Vec::new()),
        failed(
            "Rust mutation testing of the changes: `cargo mutants --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --workspace --jobs 3 --timeout-multiplier 15 --minimum-test-timeout 60 -- -- --include-ignored` exited with code 3"
        ),
        "a surviving mutant of the workspace fails"
    );
}

#[test]
fn gives_each_changed_file_one_section_when_its_crate_is_tested_whole() {
    let mut fake = passing();
    let _changes = fake.files.insert(
        format!("capture {}", CHANGES.0),
        format!(
            "{}{}{}",
            one_line_change("crates/engine-core/src/lib.rs"),
            one_line_change("crates/engine-core/src/plan_tests.rs"),
            one_line_change("crates/engine-hash/src/lib.rs")
        ),
    );
    for (path, text) in [
        ("crates/engine-core/src/lib.rs", "fn a() {}\nfn b() {}\n"),
        ("crates/engine-core/src/plan_tests.rs", "fn t() {}\n"),
        ("crates/engine-hash/src/lib.rs", "fn h() {}\n"),
    ] {
        let _source = fake.files.insert(path.to_owned(), text.to_owned());
    }
    let _new = fake.files.insert(
        "crates/engine-core/src/new.rs".to_owned(),
        "fn n() {}\n".to_owned(),
    );
    let _listed = fake.files.insert(
        format!("capture {}", NEW_RUST_FILES.0),
        "crates/engine-core/src/new.rs\0".to_owned(),
    );
    let said = mutate_changes(&mut fake);
    assert_eq!(
        fake.files.get(CHANGES_DIFF).cloned(),
        Some(format!(
            "{}{}{}",
            one_line_change("crates/engine-hash/src/lib.rs"),
            super::new_file_diff("crates/engine-core/src/lib.rs", "fn a() {}\nfn b() {}\n"),
            super::new_file_diff("crates/engine-core/src/new.rs", "fn n() {}\n")
        )),
        "the source changed and widened, and the new file, once each and whole; the other \
         crate's change as git wrote it"
    );
    assert!(
        said.contains("  the test code of engine-core changed: every mutant of engine-core\n"),
        "said: {said}"
    );
    // A git that ignored the exclusion: the diff would hold the crate's source twice.
    let mut careless = passing();
    let _careless_changes = careless.files.insert(
        format!("capture {}", CHANGES.0),
        format!(
            "{}{}",
            one_line_change("crates/engine-core/src/lib.rs"),
            one_line_change("crates/engine-core/src/plan_tests.rs")
        ),
    );
    let _ignored = careless.files.insert(
        format!(
            "{} -- :(exclude)crates/engine-core/",
            diff_since(MERGE_BASE.1.trim())
        ),
        one_line_change("crates/engine-core/src/lib.rs"),
    );
    let _source = careless.files.insert(
        "crates/engine-core/src/lib.rs".to_owned(),
        "fn a() {}\n".to_owned(),
    );
    assert_eq!(
        run_steps(&rust_mutation_of_changes(), &mut careless, &mut Vec::new()),
        failed(
            "Rust mutation testing of the changes: the diff for cargo-mutants holds `diff --git \
             a/crates/engine-core/src/lib.rs b/crates/engine-core/src/lib.rs` twice, whose \
             mutants it would miss"
        ),
        "refused before cargo-mutants reads it"
    );
    assert!(
        !careless
            .ran
            .iter()
            .any(|line| line.starts_with("cargo mutants")
                || line == "write target/xtask/changes.diff"),
        "nothing written, nothing mutated"
    );
}

#[test]
fn finds_a_section_written_twice() {
    let twice = format!(
        "{}{}",
        one_line_change("crates/a/src/b.rs"),
        super::new_file_diff("crates/a/src/b.rs", "fn f() {}\n")
    );
    assert_eq!(
        super::repeated_section(&twice),
        Some("diff --git a/crates/a/src/b.rs b/crates/a/src/b.rs"),
        "a change and the whole file"
    );
    let once = format!(
        "{}{}",
        one_line_change("crates/a/src/b.rs"),
        super::new_file_diff(
            "crates/a/src/c.rs",
            "diff --git a/crates/a/src/b.rs b/crates/a/src/b.rs\n"
        )
    );
    assert_eq!(
        super::repeated_section(&once),
        None,
        "a header as a line of a file is no section"
    );
    assert_eq!(super::repeated_section(""), None, "no section");
}

#[test]
fn reads_the_paths_real_git_changes_whatever_they_hold() {
    let dir = scratch("changed-paths");
    let quoted = "crates/engine-core/tests/caf\u{e9}.json";
    let spaced = "crates/engine-core/tests/x b/y.rs";
    let hash = "crates/engine-hash/src/lib.rs";
    for path in [quoted, spaced, hash] {
        let file = dir.join(path);
        std::fs::create_dir_all(file.parent().expect("a parent")).expect("its directory");
        std::fs::write(&file, "a\n").expect("a file");
    }
    let _init = hermetic_git(&dir, &["init", "--quiet", "--initial-branch=main"]);
    let _add = hermetic_git(&dir, &["add", "."]);
    let _commit = hermetic_git(&dir, &["commit", "--quiet", "-m", "first"]);
    for path in [quoted, spaced, hash] {
        std::fs::write(dir.join(path), "b\n").expect("a change");
    }
    std::fs::write(dir.join("crates/engine-core/tests/new.rs"), "fn n() {}\n").expect("a new file");
    let mut runner = SystemRunner { root: dir.clone() };
    assert_eq!(
        super::changed_since(&mut runner, "HEAD").map_err(|failure| failure.summary),
        Ok(vec![quoted.to_owned(), spaced.to_owned(), hash.to_owned()]),
        "a path git would quote, and one holding ` b/`, as they are"
    );
    assert_eq!(
        super::new_rust_files(&mut runner).map_err(|failure| failure.summary),
        Ok(vec!["crates/engine-core/tests/new.rs".to_owned()]),
        "the new Rust file"
    );
    let core = super::RUST_CRATES
        .iter()
        .find(|krate| krate.name == "engine-core")
        .expect("the engine core");
    let diff = super::diff_since(&mut runner, "HEAD", &[core]).expect("a diff");
    assert_eq!(
        diff,
        format!(
            "diff --git a/{hash} b/{hash}\nindex 7898192..6178079 100644\n--- a/{hash}\n+++ b/{hash}\n@@ -1 +1 @@\n-a\n+b\n"
        ),
        "git leaves out the crate excluded"
    );
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn reads_the_record_real_git_holds_at_a_commit() {
    let dir = scratch("record-at");
    std::fs::create_dir_all(dir.join("conformance/algorithms.json.d")).expect("a directory");
    std::fs::write(dir.join("conformance/algorithms.json.d/x"), "x\n").expect("a near name");
    std::fs::write(dir.join("conformance/algorithms.jsonx"), "y\n").expect("a near name");
    let _init = hermetic_git(&dir, &["init", "--quiet", "--initial-branch=main"]);
    let _add = hermetic_git(&dir, &["add", "."]);
    let _commit = hermetic_git(&dir, &["commit", "--quiet", "-m", "without"]);
    let without = hermetic_git(&dir, &["rev-parse", "HEAD"]);
    std::fs::write(
        dir.join("conformance/algorithms.json"),
        "{\n  \"a\": 1\n}\n",
    )
    .expect("a record");
    let _add_record = hermetic_git(&dir, &["add", "."]);
    let _record = hermetic_git(&dir, &["commit", "--quiet", "-m", "with"]);
    let with = hermetic_git(&dir, &["rev-parse", "HEAD"]);
    let mut runner = SystemRunner { root: dir.clone() };
    assert_eq!(
        super::record_at(&mut runner, without.trim()).map_err(|failure| failure.summary),
        Ok(None),
        "none at a commit without it, whatever names begin like it"
    );
    assert_eq!(
        super::record_at(&mut runner, with.trim()).map_err(|failure| failure.summary),
        Ok(Some("{\n  \"a\": 1\n}\n".to_owned())),
        "its bytes at a commit with it"
    );
    assert!(
        super::record_at(&mut runner, "0000000000000000000000000000000000000000").is_err_and(
            |failure| failure.summary.starts_with(
                "cannot read conformance/algorithms.json at the merge base \
                 0000000000000000000000000000000000000000: "
            )
        ),
        "a commit git does not have"
    );
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn refuses_to_mutate_a_tree_where_git_hides_files() {
    for (listed, file) in [
        (
            "H Cargo.toml\nh crates/engine-core/src/plan.rs\n",
            "crates/engine-core/src/plan.rs",
        ),
        ("S crates/a/src/b.rs\n", "crates/a/src/b.rs"),
    ] {
        let mut fake = passing();
        drop(
            fake.files
                .insert(format!("capture {}", HIDDEN.0), listed.to_owned()),
        );
        assert_eq!(
            run_steps(&rust_mutation_of_changes(), &mut fake, &mut Vec::new()),
            failed(format!(
                "Rust mutation testing of the changes: `{file}` is hidden from git status \
                 (assume-unchanged or skip-worktree): its changes would not be mutated (clear it \
                 with `git update-index --no-assume-unchanged --no-skip-worktree <file>`)"
            )),
            "{listed}"
        );
        assert!(
            !fake
                .ran
                .iter()
                .any(|line| line.starts_with("cargo mutants")),
            "{listed}: nothing mutated"
        );
    }
    let mut unlisted = passing();
    unlisted.broken = Some(HIDDEN.0.to_owned());
    assert_eq!(
        run_steps(&rust_mutation_of_changes(), &mut unlisted, &mut Vec::new()),
        failed(format!(
            "Rust mutation testing of the changes: cannot read the work tree: cannot run `{}`: \
             not found",
            HIDDEN.0
        )),
        "the hidden files cannot be listed"
    );
}

#[test]
fn mutates_nothing_without_a_change() {
    let mut fake = passing();
    let _changes = fake
        .files
        .insert(format!("capture {}", CHANGES.0), String::new());
    let _report = fake.files.remove(UNVIABLE_REPORT);
    let mut out = Vec::new();
    assert_eq!(
        run_steps(&rust_mutation_of_changes(), &mut fake, &mut out),
        Ok(()),
        "an empty diff passes"
    );
    assert!(
        !fake.ran.iter().any(|line| line.starts_with("cargo mutants")
            || line.starts_with("write target/xtask/changes")),
        "no diff written, nothing mutated: {:?}",
        fake.ran
    );
    assert!(
        String::from_utf8(out).expect("UTF-8").contains(
            "  no change since abc1230000000000000000000000000000000000 (merge base with @{upstream}): no mutant to test\n"
        ),
        "said"
    );
}

#[test]
fn mutates_new_rust_files_whole() {
    let mut fake = passing();
    for (path, text) in [
        ("crates/a/src/new.rs", "fn g() {}\nfn h() {}\n"),
        ("crates/a/src/open.rs", "fn k() {}"),
        ("crates/a/src/empty.rs", ""),
    ] {
        let _new = fake.files.insert(path.to_owned(), text.to_owned());
    }
    let _listed = fake.files.insert(
        format!("capture {}", NEW_RUST_FILES.0),
        "crates/a/src/new.rs\0crates/a/src/open.rs\0crates/a/src/empty.rs\0".to_owned(),
    );
    assert_eq!(
        run_steps(&rust_mutation_of_changes(), &mut fake, &mut Vec::new()),
        Ok(()),
        "mutated"
    );
    assert_eq!(
        fake.files.get(CHANGES_DIFF).map(String::as_str),
        Some(
            format!(
                "{}diff --git a/crates/a/src/new.rs b/crates/a/src/new.rs\nnew file mode 100644\n--- /dev/null\n+++ b/crates/a/src/new.rs\n@@ -0,0 +1,2 @@\n+fn g() {{}}\n+fn h() {{}}\n\
                 diff --git a/crates/a/src/open.rs b/crates/a/src/open.rs\nnew file mode 100644\n--- /dev/null\n+++ b/crates/a/src/open.rs\n@@ -0,0 +1,1 @@\n+fn k() {{}}\n\\ No newline at end of file\n",
                CHANGES.1
            )
            .as_str()
        ),
        "each new file added whole, the empty one apart"
    );
    let mut only_new = passing();
    let _changes = only_new
        .files
        .insert(format!("capture {}", CHANGES.0), String::new());
    let _new = only_new
        .files
        .insert("crates/a/src/new.rs".to_owned(), "fn g() {}\n".to_owned());
    let _only = only_new.files.insert(
        format!("capture {}", NEW_RUST_FILES.0),
        "crates/a/src/new.rs\0".to_owned(),
    );
    assert_eq!(
        run_steps(&rust_mutation_of_changes(), &mut only_new, &mut Vec::new()),
        Ok(()),
        "mutated"
    );
    assert!(
        only_new
            .ran
            .iter()
            .any(|line| line.starts_with("cargo mutants")),
        "a new file alone is a change"
    );
    let mut unreadable = passing();
    let _gone = unreadable.files.insert(
        format!("capture {}", NEW_RUST_FILES.0),
        "crates/a/src/gone.rs\0".to_owned(),
    );
    assert_eq!(
        run_steps(
            &rust_mutation_of_changes(),
            &mut unreadable,
            &mut Vec::new()
        ),
        failed("Rust mutation testing of the changes: cannot read crates/a/src/gone.rs: missing"),
        "a new file that cannot be read"
    );
}

#[test]
fn reports_why_the_changes_cannot_be_mutated() {
    for (broken, expected) in [
        (
            CHANGES.0,
            format!("cannot read the changes: cannot run `{}`: not found", CHANGES.0),
        ),
        (
            NEW_RUST_FILES.0,
            format!(
                "cannot list the new Rust files: cannot run `{}`: not found",
                NEW_RUST_FILES.0
            ),
        ),
        (
            "write target/xtask/changes.diff",
            "cannot write target/xtask/changes.diff: read-only".to_owned(),
        ),
        (
            "cargo mutants --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --workspace --in-diff target/xtask/changes.diff --jobs 3 --timeout-multiplier 15 --minimum-test-timeout 60 -- -- --include-ignored",
            "cannot run `cargo mutants --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --workspace --in-diff target/xtask/changes.diff --jobs 3 --timeout-multiplier 15 --minimum-test-timeout 60 -- -- --include-ignored`: not found".to_owned(),
        ),
    ] {
        let mut fake = passing();
        fake.broken = Some(broken.to_owned());
        assert_eq!(
            run_steps(&rust_mutation_of_changes(), &mut fake, &mut Vec::new()),
            failed(format!("Rust mutation testing of the changes: {expected}")),
            "{broken}"
        );
    }
}

#[test]
fn a_partial_run_finds_no_stale_review_but_every_unreviewed_mutant() {
    let report = "crates/a/src/b.rs:3:5: replace f -> T with y\n";
    let reviewed = "crates/a/src/z.rs: replace z -> Z with w -- elsewhere\n";
    assert_eq!(
        unviable_gate("", Extent::Partial, reviewed),
        Ok(()),
        "a review of a mutant the run did not generate is not stale"
    );
    assert_eq!(
        unviable_gate("", Extent::Complete, reviewed),
        found(
            &format!("unviable mutants must be reviewed in {UNVIABLE_REVIEWED}"),
            &["stale review: crates/a/src/z.rs: replace z -> Z with w".to_owned()]
        ),
        "after a complete run it is"
    );
    assert_eq!(
        unviable_gate(report, Extent::Partial, &format!("{reviewed}no reason\n")),
        found(
            &format!("unviable mutants must be reviewed in {UNVIABLE_REVIEWED}"),
            &[
                "review without a reason: no reason".to_owned(),
                "not reviewed: crates/a/src/b.rs: replace f -> T with y".to_owned()
            ]
        ),
        "an unreviewed mutant and a malformed review are still refused"
    );
    let twice = "crates/a/src/b.rs:3:5: replace f -> T with y\ncrates/a/src/b.rs:9:5: replace f -> T with y\n";
    assert_eq!(
        unviable_gate(
            twice,
            Extent::Partial,
            "crates/a/src/b.rs: replace f -> T with y -- once\n"
        ),
        found(
            &format!("unviable mutants must be reviewed in {UNVIABLE_REVIEWED}"),
            &[
                "not reviewed: crates/a/src/b.rs: replace f -> T with y (2 found, 1 reviewed)"
                    .to_owned()
            ]
        ),
        "each occurrence still needs its review"
    );
}

#[test]
fn only_a_partial_run_may_leave_no_unviable_report() {
    let gate = |extent: Extent| {
        vec![Step {
            name: "gate".to_owned(),
            action: Action::UnviableGate(extent),
        }]
    };
    let mut partial = passing();
    let _report = partial.files.remove(UNVIABLE_REPORT);
    assert_eq!(
        run_steps(&gate(Extent::Partial), &mut partial, &mut Vec::new()),
        Ok(()),
        "cargo-mutants writes no report when the changes hold no mutant"
    );
    let mut complete = passing();
    let _complete = complete.files.remove(UNVIABLE_REPORT);
    assert_eq!(
        run_steps(&gate(Extent::Complete), &mut complete, &mut Vec::new()),
        failed(format!("gate: cannot read {UNVIABLE_REPORT}: missing")),
        "a complete run always writes it"
    );
    let mut unreadable = Unreadable(passing());
    assert_eq!(
        run_steps(&gate(Extent::Partial), &mut unreadable, &mut Vec::new()),
        failed(format!("gate: cannot read {UNVIABLE_REPORT}: denied")),
        "a report that exists but cannot be read is never taken for none"
    );
}

/// A runner whose files all exist but cannot be read.
struct Unreadable(Fake);

impl Runner for Unreadable {
    fn run(&mut self, cmd: &Cmd) -> Result<(), CommandFailure> {
        self.0.run(cmd)
    }
    fn read(&mut self, _path: &str) -> io::Result<String> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"))
    }
    fn fresh_dir(&mut self, path: &str) -> io::Result<()> {
        self.0.fresh_dir(path)
    }
    fn tracked(&mut self, patterns: &[&str]) -> Result<Vec<String>, CommandFailure> {
        self.0.tracked(patterns)
    }
    fn capture(&mut self, cmd: &Cmd) -> Result<String, CommandFailure> {
        self.0.capture(cmd)
    }
    fn write(&mut self, path: &str, text: &str) -> io::Result<()> {
        self.0.write(path, text)
    }
    fn parallelism(&mut self) -> NonZeroUsize {
        self.0.parallelism()
    }
    fn environment(&mut self) -> super::Environment {
        self.0.environment()
    }
    fn clock(&mut self) -> Duration {
        self.0.clock()
    }
}

#[test]
fn reads_every_kind_of_mutation_target() {
    use super::{Target, mutation_target};
    let krate = |name: &str| {
        *super::RUST_CRATES
            .iter()
            .find(|krate| krate.name == name)
            .expect("a crate")
    };
    let package = |dir: &str| {
        *PACKAGES
            .iter()
            .find(|package| package.dir == dir)
            .expect("a package")
    };
    assert_eq!(
        mutation_target("engine-core"),
        Ok(Target::Crate(krate("engine-core"))),
        "a crate, by its name"
    );
    assert_eq!(
        mutation_target("crates/xtask/src/lib.rs"),
        Ok(Target::RustFile {
            krate: krate("xtask"),
            path: "crates/xtask/src/lib.rs".to_owned()
        }),
        "a Rust file, in its crate"
    );
    assert_eq!(
        mutation_target("bindings/wasm/src/ffi.rs"),
        Ok(Target::RustFile {
            krate: krate("engine-wasm"),
            path: "bindings/wasm/src/ffi.rs".to_owned()
        }),
        "a Rust file of the crate that shares a package's directory"
    );
    assert_eq!(
        mutation_target("bindings/wasm"),
        Ok(Target::Package(package("bindings/wasm"))),
        "a package, by its directory"
    );
    for (path, dir, source) in [
        ("apps/roadmap/src/board.ts", "apps/roadmap", "src/board.ts"),
        (
            "apps/playground/src/app.tsx",
            "apps/playground",
            "src/app.tsx",
        ),
        (
            "bindings/wasm/ts/worklet/x.mts",
            "bindings/wasm",
            "ts/worklet/x.mts",
        ),
        ("tooling/oracle/src/a.cts", "tooling/oracle", "src/a.cts"),
        ("tooling/config/eslint.js", "tooling/config", "eslint.js"),
        ("tooling/config/a.jsx", "tooling/config", "a.jsx"),
        ("tooling/config/a.mjs", "tooling/config", "a.mjs"),
        ("tooling/config/a.cjs", "tooling/config", "a.cjs"),
    ] {
        assert_eq!(
            mutation_target(path),
            Ok(Target::TypeScriptFile {
                package: package(dir),
                source: source.to_owned()
            }),
            "{path}: a source, in its package"
        );
    }
}

#[test]
fn refuses_what_is_no_mutation_target() {
    for (text, reason) in [
        (
            "apps/roadmap/test/board.test.ts",
            "\"apps/roadmap/test/board.test.ts\" is test code: name the source it tests",
        ),
        (
            "apps/roadmap/src/board.test.ts",
            "\"apps/roadmap/src/board.test.ts\" is test code: name the source it tests",
        ),
        (
            "crates/xtask/../engine-core/src/lib.rs",
            "\"crates/xtask/../engine-core/src/lib.rs\" is not a plain path from the root (no `.`, `..` or empty part)",
        ),
        (
            "./crates/xtask/src/lib.rs",
            "\"./crates/xtask/src/lib.rs\" is not a plain path from the root (no `.`, `..` or empty part)",
        ),
        (
            "crates/xtask//src/lib.rs",
            "\"crates/xtask//src/lib.rs\" is not a plain path from the root (no `.`, `..` or empty part)",
        ),
        (
            "apps/roadmap/",
            "\"apps/roadmap/\" is not a plain path from the root (no `.`, `..` or empty part)",
        ),
        (
            "crates/xtask",
            "\"crates/xtask\" is no crate, Rust file of a crate, TypeScript package or source of one",
        ),
        (
            "crates/other/src/lib.rs",
            "\"crates/other/src/lib.rs\" is no crate, Rust file of a crate, TypeScript package or source of one",
        ),
        (
            "crates/xtaskx/src/lib.rs",
            "\"crates/xtaskx/src/lib.rs\" is no crate, Rust file of a crate, TypeScript package or source of one",
        ),
        (
            "apps/roadmap/src/page.astro",
            "\"apps/roadmap/src/page.astro\" is no crate, Rust file of a crate, TypeScript package or source of one",
        ),
        (
            "apps/roadmap/README.md",
            "\"apps/roadmap/README.md\" is no crate, Rust file of a crate, TypeScript package or source of one",
        ),
        (
            "apps/roadmap/src/lib.rs",
            "\"apps/roadmap/src/lib.rs\" is no crate, Rust file of a crate, TypeScript package or source of one",
        ),
        (
            "crates/engine-core/src/a.ts",
            "\"crates/engine-core/src/a.ts\" is no crate, Rust file of a crate, TypeScript package or source of one",
        ),
        (
            "crates/xtask/src/lib_tests.rs",
            "\"crates/xtask/src/lib_tests.rs\" is test code: name the source it tests",
        ),
        (
            "crates/xtask/tests/binary.rs",
            "\"crates/xtask/tests/binary.rs\" is test code: name the source it tests",
        ),
        (
            "crates/xtask/tests/fixtures/coverage-locality/src/lib.rs",
            "\"crates/xtask/tests/fixtures/coverage-locality/src/lib.rs\" is test code: name the source it tests",
        ),
        (
            "crates/xtask/src/lib.RS",
            "\"crates/xtask/src/lib.RS\" is no crate, Rust file of a crate, TypeScript package or source of one",
        ),
        (
            "\u{1b}[2J",
            "\"\\u{1b}[2J\" is no crate, Rust file of a crate, TypeScript package or source of one",
        ),
    ] {
        assert_eq!(
            super::mutation_target(text),
            Err(reason.to_owned()),
            "{text}"
        );
    }
}

#[test]
fn mutates_every_mutant_of_one_target() {
    use super::{mutants_steps, mutation_target};
    let jobs = NonZeroUsize::new(3).expect("three is not zero");
    let lines = |text: &str| {
        command_lines(&mutants_steps(
            &mutation_target(text).expect("a target"),
            jobs,
        ))
    };
    assert_eq!(
        lines("engine-core"),
        vec![
            "fresh mutants.out",
            "Root: cargo mutants --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --package engine-core --jobs 3 --timeout-multiplier 15 --minimum-test-timeout 60 -- -- --include-ignored",
            "unviable gate (Partial)",
        ],
        "a crate"
    );
    assert_eq!(
        lines("crates/xtask/src/lib.rs"),
        vec![
            "fresh mutants.out",
            "Root: cargo mutants --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --package xtask --file crates/xtask/src/lib.rs --jobs 3 --timeout-multiplier 15 --minimum-test-timeout 60 -- -- --include-ignored",
            "unviable gate (Partial)",
        ],
        "a Rust file"
    );
    assert_eq!(
        lines("apps/roadmap"),
        vec![
            "fresh apps/roadmap/reports/mutation",
            "Package(\"apps/roadmap\"): pnpm exec stryker run stryker.config.json --incremental --incrementalFile reports/stryker-incremental.json --force",
            "typescript mutation gate of apps/roadmap (Complete)",
        ],
        "a package, every mutant, its incremental file rewritten"
    );
    assert_eq!(
        lines("apps/roadmap/src/board.ts"),
        vec![
            "fresh apps/roadmap/reports/mutation",
            "Package(\"apps/roadmap\"): pnpm exec stryker run stryker.config.json --mutate src/board.ts",
            "typescript mutation gate of apps/roadmap (Partial)",
        ],
        "a source: its incremental file untouched"
    );
    let names = |text: &str| -> Vec<String> {
        mutants_steps(&mutation_target(text).expect("a target"), jobs)
            .into_iter()
            .map(|step| step.name)
            .collect()
    };
    assert_eq!(
        names("crates/xtask/src/lib.rs"),
        vec![
            "Fresh Rust mutation report",
            "Rust mutation testing of crates/xtask/src/lib.rs",
            "Rust unviable mutants reviewed"
        ],
        "named"
    );
    assert_eq!(
        names("engine-core")[1],
        "Rust mutation testing of engine-core",
        "named"
    );
    assert_eq!(
        names("apps/roadmap/src/board.ts"),
        vec![
            "Fresh mutation report of apps/roadmap",
            "Mutation testing of apps/roadmap/src/board.ts",
            "TypeScript mutants of apps/roadmap/src/board.ts killed"
        ],
        "named"
    );
    assert_eq!(
        names("apps/roadmap")[1],
        "Mutation testing of apps/roadmap",
        "named"
    );
}

#[test]
fn cli_mutates_one_target() {
    let mut out = Vec::new();
    let mut fake = passing();
    assert_eq!(
        run_cli(
            &["mutants".to_owned(), "engine-core".to_owned()],
            &mut fake,
            &mut out,
            &mut Vec::new()
        ),
        0,
        "a crate"
    );
    assert_eq!(
        fake.ran,
        vec![
            "fresh mutants.out",
            "cargo mutants --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --package engine-core --jobs 3 --timeout-multiplier 15 --minimum-test-timeout 60 -- -- --include-ignored",
            "read mutants.out/unviable.txt",
            "read mutants-unviable.txt",
        ],
        "in half the threads of the machine"
    );
    assert!(
        String::from_utf8(out)
            .expect("UTF-8")
            .ends_with("  done in 1.5 s\n✔ all 3 checks passed in 4.5 s\n"),
        "said"
    );
    let mut source = passing();
    assert_eq!(
        run_cli(
            &["mutants".to_owned(), "apps/roadmap/src/a.ts".to_owned()],
            &mut source,
            &mut Vec::new(),
            &mut Vec::new()
        ),
        0,
        "a source that exists"
    );
    assert_eq!(
        source.ran.first().map(String::as_str),
        Some("read apps/roadmap/src/a.ts"),
        "read first"
    );
    assert!(
        source
            .ran
            .contains(&"pnpm exec stryker run stryker.config.json --mutate src/a.ts".to_owned())
            && source
                .ran
                .contains(&"read apps/roadmap/reports/mutation/mutation.json".to_owned()),
        "mutated, its report gated: {:?}",
        source.ran
    );
    let mut survivor = passing();
    let _report = survivor.files.insert(
        "apps/roadmap/reports/mutation/mutation.json".to_owned(),
        one_mutant("Survived", "M", 4),
    );
    let mut err = Vec::new();
    assert_eq!(
        run_cli(
            &["mutants".to_owned(), "apps/roadmap".to_owned()],
            &mut survivor,
            &mut Vec::new(),
            &mut err
        ),
        1,
        "a survivor fails"
    );
    assert_eq!(
        String::from_utf8(err).expect("UTF-8"),
        "error: TypeScript mutants of apps/roadmap killed: apps/roadmap: mutants not killed: apps/roadmap/x.ts:4 M Survived\n",
        "named"
    );
    let mut unlisted = passing();
    let _sources = unlisted.broken.replace("list *.ts *.tsx".to_owned());
    assert_eq!(
        run_cli(
            &["mutants".to_owned(), "apps/roadmap".to_owned()],
            &mut unlisted,
            &mut Vec::new(),
            &mut Vec::new()
        ),
        1,
        "the disable comments must be read"
    );
}

#[test]
fn refuses_a_mutation_target_in_one_line() {
    for (args, reason) in [
        (vec!["mutants"], "`mutants` takes one target"),
        (vec!["mutants", "a", "b"], "`mutants` takes one target"),
        (
            vec!["mutants", "crates/xtask/src/gone.rs"],
            "cannot read crates/xtask/src/gone.rs: missing",
        ),
        (
            vec!["mutants", "apps/roadmap/src/gone.ts"],
            "cannot read apps/roadmap/src/gone.ts: missing",
        ),
    ] {
        let args: Vec<String> = args.into_iter().map(str::to_owned).collect();
        let mut usage = Vec::new();
        let mut fake = passing();
        assert_eq!(
            run_cli(&args, &mut fake, &mut Vec::new(), &mut usage),
            2,
            "{args:?}"
        );
        assert_eq!(
            String::from_utf8(usage).expect("UTF-8"),
            format!("error: {reason} (see: cargo xtask help)\n"),
            "{args:?}"
        );
        assert!(
            !fake.ran.iter().any(|line| !line.starts_with("read ")),
            "{args:?}: nothing runs"
        );
    }
}

#[test]
fn runs_half_the_threads_as_mutation_jobs() {
    for (threads, jobs) in [(1, 1), (2, 1), (3, 1), (4, 2), (5, 2), (6, 3), (16, 8)] {
        assert_eq!(
            super::mutation_jobs(NonZeroUsize::new(threads).expect("not zero")).get(),
            jobs,
            "{threads} threads"
        );
    }
}

/// The test timeout scales with the jobs that share the processors: five times the
/// unmutated tests' time and at least twenty seconds per job (cargo-mutants' defaults,
/// written here as literals), never cargo-mutants' own unscaled timeout.
#[test]
fn scales_the_mutant_timeout_with_the_jobs() {
    for (jobs, multiplier, floor) in [(1, "5", "20"), (2, "10", "40"), (8, "40", "160")] {
        assert_eq!(
            super::mutant_timeout_args(NonZeroUsize::new(jobs).expect("not zero")),
            [
                "--jobs".to_owned(),
                jobs.to_string(),
                "--timeout-multiplier".to_owned(),
                multiplier.to_owned(),
                "--minimum-test-timeout".to_owned(),
                floor.to_owned(),
            ],
            "{jobs} jobs"
        );
    }
}

#[test]
fn resolves_directories() {
    let root = Path::new("/repo");
    assert_eq!(Dir::Root.under(root), Path::new("/repo"), "root");
    assert_eq!(
        Dir::Package("apps/roadmap").under(root),
        Path::new("/repo/apps/roadmap"),
        "a package"
    );
}

#[test]
fn accepts_full_coverage() {
    assert_eq!(
        coverage_gate(
            &report(&[
                ("crates/a/src/lib.rs", (10, 10), (3, 3), (4, 4)),
                ("crates/a/src/b.rs", (1, 1), (1, 1), (0, 0)),
            ]),
            CRATE_A
        ),
        Ok(()),
        "every file of the crate at 100%, a file without branches included"
    );
    assert_eq!(
        coverage_gate(
            &report(&[
                ("crates/a/src/lib.rs", (10, 1), (3, 1), (4, 1)),
                ("crates/a/src/main.rs", (5, 5), (1, 1), (0, 0)),
            ]),
            CRATE_A_MAIN
        ),
        Ok(()),
        "a binary: only its main.rs is read"
    );
}

#[test]
fn names_the_files_that_are_not_fully_covered() {
    assert_eq!(
        coverage_gate(
            &report(&[
                ("crates/a/src/b.rs", (10, 9), (3, 3), (4, 4)),
                ("crates/a/src/c.rs", (10, 10), (3, 2), (4, 3)),
            ]),
            CRATE_A
        ),
        failed(format!(
            "a is not covered 100% by its own unit tests: 2 problems, listed above\n\
             crates/a/src/b.rs: lines 9/10\n\
             crates/a/src/c.rs: functions 2/3, branches 3/4\n\
             the lines: cargo {} llvm-cov --manifest-path crates/a/Cargo.toml --lib --branch \
             --show-missing-lines -- --include-ignored",
            super::NIGHTLY_TOOLCHAIN
        )),
        "every file and every incomplete kind, with the command showing the lines"
    );
    assert_eq!(
        coverage_gate(
            &report(&[("crates/a/src/main.rs", (5, 4), (1, 1), (0, 0))]),
            CRATE_A_MAIN
        ),
        failed(format!(
            "the a binary is not covered 100% by its tests: crates/a/src/main.rs: lines 4/5\n\
             the lines: cargo {} llvm-cov --manifest-path crates/a/Cargo.toml --branch \
             --show-missing-lines -- --include-ignored",
            super::NIGHTLY_TOOLCHAIN
        )),
        "a binary, measured by every test of its crate"
    );
}

#[test]
fn coverage_gate_refuses_files_of_another_crate() {
    assert_eq!(
        coverage_gate(
            &report(&[
                ("crates/a/src/lib.rs", (10, 10), (3, 3), (4, 4)),
                ("crates/ab/src/lib.rs", (10, 10), (3, 3), (4, 4)),
            ]),
            CRATE_A
        ),
        failed(
            "a is not covered 100% by its own unit tests: /repo/crates/ab/src/lib.rs is not a file of a"
        ),
        "the report of another crate, even a fully covered one"
    );
}

#[test]
fn coverage_gate_refuses_an_empty_report() {
    assert_eq!(
        coverage_gate(&report(&[]), CRATE_A),
        failed("a is not covered 100% by its own unit tests: no file of a measured"),
        "a report of nothing proves nothing"
    );
    assert_eq!(
        coverage_gate(
            &report(&[("crates/a/src/lib.rs", (10, 10), (3, 3), (4, 4))]),
            CRATE_A_MAIN
        ),
        failed("the a binary is not covered 100% by its tests: no file of a measured"),
        "a binary report without its main.rs"
    );
}

#[test]
fn rejects_unreadable_reports() {
    assert!(
        coverage_gate("not json", CRATE_A)
            .is_err_and(|error| error.summary.starts_with("invalid coverage report: ")),
        "not JSON"
    );
    assert_eq!(
        coverage_gate(r#"{"data":[{"totals":{}}]}"#, CRATE_A),
        failed("coverage report has no files"),
        "no file list"
    );
    assert_eq!(
        coverage_gate(
            r#"{"data":[{"files":[{"filename":"/r/crates/a/src/lib.rs","summary":{"lines":{"count":1,"covered":1}}}]}]}"#,
            CRATE_A
        ),
        failed("coverage report has no functions total"),
        "a missing total"
    );
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
    let mut expected: Vec<String> = super::RUST_CRATES
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
        ["engine-cli", "xtask"]
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
fn requires_the_gate_to_refuse_a_path_only_an_integration_test_reaches() {
    let locality = Coverage {
        krate: super::COVERAGE_LOCALITY,
        measured: Measured::Library,
    };
    let step: Vec<Step> = check_steps(CHANGES_MODE)
        .into_iter()
        .filter(|step| step.action == Action::CoverageLocality)
        .collect();
    let mut refused = passing();
    assert_eq!(
        run_steps(&step, &mut refused, &mut Vec::new()),
        Ok(()),
        "the gate refuses the fixture"
    );
    assert!(
        refused.ran.contains(&locality.command().line()),
        "the fixture is measured"
    );
    let mut accepted = passing();
    let _full = accepted
        .files
        .insert(locality.report(), full_report(locality));
    assert_eq!(
        run_steps(&step, &mut accepted, &mut Vec::new()),
        failed(format!(
            "Rust coverage counts a crate's own unit tests only: the coverage gate accepted {}: \
             a path reached only by an integration test counts as covered",
            super::COVERAGE_LOCALITY.dir
        )),
        "a gate that counts integration tests"
    );
    let mut elsewhere = passing();
    let _empty = elsewhere.files.insert(locality.report(), report(&[]));
    assert_eq!(
        run_steps(&step, &mut elsewhere, &mut Vec::new()),
        failed(format!(
            "Rust coverage counts a crate's own unit tests only: the coverage gate refused {} \
             for another reason: coverage-locality is not covered 100% by its own unit tests: \
             no file of coverage-locality measured",
            super::COVERAGE_LOCALITY.dir
        )),
        "a refusal that proves nothing"
    );
}

#[test]
fn runs_every_step_and_reports_success() {
    let mut fake = passing();
    let mut out = Vec::new();
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut fake, &mut out),
        Ok(()),
        "all green"
    );
    let (tools, commands, emptied, listed, captured, written) = (10, 58, 14, 24, 37, 3);
    let read = 2
        + 7
        + 14
        + 3
        + 7
        + 21
        + 3
        + 1
        + 7
        + 3
        + 12
        + 10
        + 21
        + 8
        + 2
        + 12
        + 40
        + 20
        + 3
        + 3 * 9
        + 3 * 12
        + 1
        + 90;
    assert_eq!(
        fake.ran.len(),
        tools + commands + emptied + listed + captured + written + read,
        "the preflight asks ten tools; 58 commands run (formatting, lints, documentation, \
         tests, four portability builds, the native shell's lints and the build of its libraries for its two architectures, fourteen coverage measurements each after emptying \
         the coverage build, the mutants of the changes, Miri, the \
         conformance check and export, deny, the JavaScript install, audit, the engine build and five package \
         checks each followed by its Stryker run); fourteen directories are emptied; git lists \
         thirteen kinds of files (the box sources and the text files that cite the roadmap among them), the Rust files again for the test code, test data and untested targets gates, the TypeScript ones again for the reports and scope gates of \
         each package after the first; the licenses twice and the history three times, the state \
         of the tree (before, for the proof record and after) six times, the nightly's version and sysroot, the Cargo metadata (for the untested targets, dependency, pin, library and math layer gates), the output of the fused test build, the merge base, the changed paths, the changes, the \
         new Rust files and the list of every mutant once, git's common directory twice, the tree checked out, \
         cargo-mutants' version, rustc's host and build, the linker driver's and git's versions, the status for a push and the hidden files for the mutation step and the proof record; the measured base, the diff and the \
         proof record are written once each; the proof record, the measured base and the \
         toolchain file are read for it; and the gates read the nine fake test data readers for the unsafe, bypass and vocabulary gates, every Rust file (twelve) for the test code, test data and untested targets gates, .node-version and the toolchain file for the preflight, the toolchain file, the four float sources and the float macros of the nightly and clippy.toml for the float gate, fourteen coverage reports, \
         the unviable report and reviews, the register of the pin gate, the 90 text files of the citation gate (the board among them), the reviews again against the list of every mutant, six TypeScript sources and their reviews, the nine \
         sources, the twelve Cargo manifests and the register of the bypass gate, the three unsafe files, the Cargo \
         configuration, the six package manifests and the register of the dependency gate, the three \
         supply-chain configurations, twelve crate manifests, ten READMEs, the changelog, ten READMEs, three Rust and six TypeScript sources and the vocabulary of the vocabulary gate, eight version files, the workflow and \
         .node-version, five Stryker configurations and the six package manifests, and for each of the five \
         packages, the six TypeScript sources and two reports of its reports gate, then its \
         Stryker configuration, coverage summary, mutation report and the register of its \
         scope gate"
    );
    for path in [CARGO_TOML.to_owned(), CHANGELOG.to_owned()]
        .into_iter()
        .chain(package_jsons())
    {
        assert!(fake.ran.contains(&format!("read {path}")), "{path} read");
    }
    assert!(
        fake.ran.contains(&format!("read {UNVIABLE_REVIEWED}")),
        "reviews read"
    );
    assert!(
        fake.ran
            .contains(&"read target/coverage/xtask-main.json".to_owned()),
        "coverage reports read"
    );
    let text = String::from_utf8(out).expect("utf-8 output");
    assert!(
        text.starts_with("▶ [1/89] Tools and platform\n"),
        "numbered progress: {text}"
    );
    assert!(
        text.contains("▶ [80/89] Roadmap checks\n  done in 1.5 s\n"),
        "last step announced: {text}"
    );
    assert!(
        text.ends_with("  done in 1.5 s\n✔ all 89 checks passed in 2 min 13 s\n"),
        "summary: {text}"
    );
}

#[test]
fn stops_at_the_first_failing_command() {
    let mut fake = passing();
    fake.fail = Some("cargo test --workspace --locked -- --include-ignored".to_owned());
    let mut out = Vec::new();
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut fake, &mut out),
        failed(
            "Rust tests: `cargo test --workspace --locked -- --include-ignored` exited with code 3"
        ),
        "failure is named"
    );
    assert_eq!(
        fake.ran.last().map(String::as_str),
        Some("cargo test --workspace --locked -- --include-ignored"),
        "nothing runs after the failure"
    );
    assert!(
        !fake.ran.iter().any(|line| line.starts_with("cargo mutants")
            || line.starts_with("cargo +nightly-2026-11-01 llvm-cov")),
        "the longer steps never start"
    );
}

/// Five command steps, `One` to `Five`, each running `x <its number>`.
fn five_steps() -> Vec<Step> {
    ["one", "two", "three", "four", "five"]
        .into_iter()
        .map(|number| Step {
            name: number.to_uppercase(),
            action: Action::Run(Cmd::new("x", &[number], Dir::Root)),
        })
        .collect()
}

/// A fake where the second of [`five_steps`] fails and the fourth cannot start.
fn failing_twice() -> Fake {
    Fake {
        fail: Some("x two".to_owned()),
        broken: Some("x four".to_owned()),
        ..Fake::default()
    }
}

#[test]
fn runs_every_step_then_reports_every_failure_when_asked() {
    let mut fake = failing_twice();
    let mut out = Vec::new();
    assert_eq!(
        run_steps_then(&five_steps(), &mut fake, &mut out, OnFailure::RunTheRest),
        failed(
            "2 of 5 checks failed, listed above\n\
             TWO: `x two` exited with code 3\n\
             FOUR: cannot run `x four`: not found"
        ),
        "both failures, each with its step and its own error"
    );
    assert_eq!(
        fake.ran,
        ["x one", "x two", "x three", "x four", "x five"],
        "every step ran"
    );
    assert_eq!(
        String::from_utf8(out).expect("UTF-8"),
        "▶ [1/5] ONE\n  done in 1.5 s\n▶ [2/5] TWO\n  failed after 1.5 s\n▶ [3/5] THREE\n  done in \
         1.5 s\n▶ [4/5] FOUR\n  failed after 1.5 s\n▶ [5/5] FIVE\n  done in 1.5 s\n",
        "no claim that the checks passed"
    );
    let mut passing = Fake::default();
    let mut progress = Vec::new();
    assert_eq!(
        run_steps_then(
            &five_steps(),
            &mut passing,
            &mut progress,
            OnFailure::RunTheRest
        ),
        Ok(()),
        "no failure"
    );
    assert!(
        String::from_utf8(progress)
            .expect("UTF-8")
            .ends_with("▶ [5/5] FIVE\n  done in 1.5 s\n✔ all 5 checks passed in 7.5 s\n"),
        "passed"
    );
}

#[test]
fn shows_tenths_below_a_minute_then_whole_seconds() {
    let cases = [
        (0_u64, "0.0 s"),
        (99, "0.0 s"),
        (100, "0.1 s"),
        (1_250, "1.2 s"),
        (59_999, "59.9 s"),
        (60_000, "1 min 00 s"),
        (61_999, "1 min 01 s"),
        (754_000, "12 min 34 s"),
        (3_599_999, "59 min 59 s"),
        (3_600_000, "1 h 00 min 00 s"),
        (3_725_000, "1 h 02 min 05 s"),
        (45_296_000, "12 h 34 min 56 s"),
    ];
    for (millis, text) in cases {
        assert_eq!(
            super::duration_text(Duration::from_millis(millis)),
            text,
            "{millis} ms"
        );
    }
}

#[test]
fn the_system_clock_advances_across_a_command() {
    let mut runner = SystemRunner {
        root: scratch("clock"),
    };
    let before = runner.clock();
    let _version = runner
        .capture(&Cmd::new("git", &["--version"], Dir::Root))
        .expect("git answers");
    let after = runner.clock();
    assert!(
        after > before,
        "starting a process takes time: {before:?}, then {after:?}"
    );
}

#[test]
fn stops_at_the_first_failure_unless_asked_to_run_the_rest() {
    let mut fake = failing_twice();
    let mut out = Vec::new();
    assert_eq!(
        run_steps_then(&five_steps(), &mut fake, &mut out, OnFailure::Stop),
        failed("TWO: `x two` exited with code 3"),
        "the first failure alone"
    );
    assert_eq!(fake.ran, ["x one", "x two"], "nothing after it");
    assert_eq!(
        String::from_utf8(out).expect("UTF-8"),
        "▶ [1/5] ONE\n  done in 1.5 s\n▶ [2/5] TWO\n  failed after 1.5 s\n",
        "the time of the failed step said too"
    );
    let mut stopped = failing_twice();
    assert_eq!(
        run_steps(&five_steps(), &mut stopped, &mut Vec::new()),
        failed("TWO: `x two` exited with code 3"),
        "run_steps stops"
    );
    assert_eq!(
        CHANGES_MODE.on_failure(),
        OnFailure::Stop,
        "the default check stops, for fast feedback"
    );
    assert_eq!(
        Mode::Full { jobs: FAKE_JOBS }.on_failure(),
        OnFailure::RunTheRest,
        "the full check runs every step"
    );
}

#[test]
fn reports_commands_that_cannot_start() {
    let mut fake = passing();
    fake.broken = Some("cargo fmt --all --check".to_owned());
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut fake, &mut Vec::new()),
        failed("Rust formatting: cannot run `cargo fmt --all --check`: not found"),
        "start failure is named"
    );
}

#[test]
fn fails_on_missing_or_incomplete_coverage() {
    let core = Coverage {
        krate: super::RUST_CRATES[0],
        measured: Measured::Library,
    };
    let mut missing = passing();
    let _removed = missing.files.remove(&core.report());
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut missing, &mut Vec::new()),
        failed(
            "Rust coverage of engine-core: 100% by its own unit tests: cannot read \
             target/coverage/engine-core.json: missing"
        ),
        "missing report"
    );
    let mut partial = passing();
    let _replaced = partial.files.insert(
        core.report(),
        report(&[("crates/engine-core/src/lib.rs", (2, 1), (1, 1), (1, 1))]),
    );
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut partial, &mut Vec::new()),
        failed(
            "Rust coverage of engine-core: 100% by its own unit tests: engine-core is not \
             covered 100% by its own unit tests: crates/engine-core/src/lib.rs: lines 1/2\n\
             the lines: cargo +nightly-2026-11-01 llvm-cov --manifest-path \
             crates/engine-core/Cargo.toml --lib --branch --show-missing-lines -- --include-ignored"
        ),
        "incomplete coverage"
    );
    let mut broken = passing();
    broken.fail = Some(core.command().line());
    assert!(
        run_steps(&check_steps(CHANGES_MODE), &mut broken, &mut Vec::new())
            .is_err_and(|failure| failure.summary.ends_with("` exited with code 3")),
        "a measurement that fails"
    );
}

/// An output that always fails.
struct Broken;
impl io::Write for Broken {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("closed"))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn reports_a_broken_output() {
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut passing(), &mut Broken),
        failed("cannot write to stdout: closed"),
        "cannot even announce a step"
    );
    let one = vec![check_steps(CHANGES_MODE).remove(0)];
    let mut limited = Limited(1);
    assert_eq!(
        run_steps(&one, &mut passing(), &mut limited),
        failed("cannot write to stdout: closed"),
        "cannot write the summary"
    );
}

/// An output that accepts `n` complete lines, then fails.
struct Limited(usize);
impl io::Write for Limited {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.0 == 0 {
            return Err(io::Error::other("closed"));
        }
        if buf.contains(&b'\n') {
            self.0 = self.0.saturating_sub(1);
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn cli_runs_check_and_reports_errors() {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert_eq!(
        run_cli(&["check".to_owned()], &mut passing(), &mut out, &mut err),
        0,
        "check passes"
    );
    assert!(err.is_empty(), "no error output");

    let mut failing = passing();
    failing.fail = Some("cargo deny --locked check".to_owned());
    let mut err2 = Vec::new();
    assert_eq!(
        run_cli(
            &["check".to_owned()],
            &mut failing,
            &mut Vec::new(),
            &mut err2
        ),
        1,
        "check fails"
    );
    assert_eq!(
        String::from_utf8(err2).expect("UTF-8"),
        "error: Rust supply chain: `cargo deny --locked check` exited with code 3\n",
        "the command and its exit code on stderr"
    );

    let mut killed = passing();
    killed.killed = Some("cargo deny --locked check".to_owned());
    let mut err3 = Vec::new();
    assert_eq!(
        run_cli(
            &["check".to_owned()],
            &mut killed,
            &mut Vec::new(),
            &mut err3
        ),
        1,
        "a stopped command fails the check"
    );
    assert_eq!(
        String::from_utf8(err3).expect("UTF-8"),
        "error: Rust supply chain: `cargo deny --locked check` was stopped by signal 9 (SIGKILL)\n",
        "explained"
    );

    let mut help = Vec::new();
    assert_eq!(
        run_cli(
            &["help".to_owned()],
            &mut passing(),
            &mut help,
            &mut Vec::new()
        ),
        0,
        "help succeeds"
    );
    assert_eq!(help, format!("{USAGE}\n").into_bytes(), "help on stdout");
    assert_eq!(
        run_cli(
            &["help".to_owned()],
            &mut passing(),
            &mut Broken,
            &mut Vec::new()
        ),
        1,
        "broken stdout"
    );

    assert_eq!(
        run_cli(&[], &mut passing(), &mut Vec::new(), &mut Broken),
        2,
        "broken stderr still tells"
    );
    assert_eq!(
        run_cli(
            &["commit-msg".to_owned(), "missing".to_owned()],
            &mut passing(),
            &mut Vec::new(),
            &mut Broken
        ),
        1,
        "broken stderr still fails"
    );
}

/// An output whose reader is gone, or that fails another way.
struct Pipe(io::ErrorKind);
impl io::Write for Pipe {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::from(self.0))
    }
    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::from(self.0))
    }
}

#[test]
fn ignores_a_closed_reader_but_not_other_output_errors() {
    use io::Write as _;
    let mut closed = BestEffort(Pipe(io::ErrorKind::BrokenPipe));
    assert_eq!(
        closed.write(b"abc").map_err(|error| error.kind()),
        Ok(3),
        "write"
    );
    assert_eq!(
        closed.flush().map_err(|error| error.kind()),
        Ok(()),
        "flush"
    );
    let mut full = BestEffort(Pipe(io::ErrorKind::StorageFull));
    assert_eq!(
        full.write(b"abc").map_err(|error| error.kind()),
        Err(io::ErrorKind::StorageFull),
        "write"
    );
    assert_eq!(
        full.flush().map_err(|error| error.kind()),
        Err(io::ErrorKind::StorageFull),
        "flush"
    );
}

#[test]
fn shows_command_lines() {
    let cmd = Cmd {
        program: "cargo",
        args: vec!["fmt".to_owned(), "--check".to_owned()],
        dir: Dir::Root,
    };
    assert_eq!(cmd.line(), "cargo fmt --check", "program and arguments");
    let bare = Cmd {
        program: "pnpm",
        args: vec![],
        dir: Dir::Root,
    };
    assert_eq!(bare.line(), "pnpm", "no trailing space");
}

#[test]
fn system_runner_runs_commands_and_reads_files() {
    let mut runner = SystemRunner {
        root: fixture("coverage-locality"),
    };
    let missing = Cmd {
        program: "no-such-program-xyz",
        args: vec![],
        dir: Dir::Root,
    };
    assert!(
        runner
            .run(&missing)
            .is_err_and(|failure| matches!(failure.failed, Failed::NotStarted(_))
                && failure
                    .to_string()
                    .starts_with("cannot run `no-such-program-xyz`: ")),
        "a program that cannot start"
    );
    // Commands that print nothing, so that the checks' output holds no line of a test.
    #[cfg(unix)]
    {
        let shell = |script: &str| Cmd {
            program: "sh",
            args: vec!["-c".to_owned(), script.to_owned()],
            dir: Dir::Root,
        };
        assert_eq!(runner.run(&shell("true")), Ok(()), "a successful command");
        assert_eq!(
            runner
                .run(&shell("exit 101"))
                .map_err(|failure| failure.failed),
            Err(Failed::Ended {
                exit: Exit::Code(101_i32),
                reason: None
            }),
            "a failing command and its code, its error output left on the terminal"
        );
        let killed = Cmd {
            program: "sh",
            args: vec!["-c".to_owned(), "kill -9 $$".to_owned()],
            dir: Dir::Root,
        };
        assert_eq!(
            runner.run(&killed).map_err(|failure| failure.to_string()),
            Err("`sh -c kill -9 $$` was stopped by signal 9 (SIGKILL)".to_owned()),
            "a command a signal stopped, named"
        );
    }
    assert!(
        runner
            .read("Cargo.toml")
            .is_ok_and(|text| text.contains("[workspace]")),
        "reads below the root"
    );
    assert!(runner.read("no-such-file").is_err(), "missing file");
}

#[test]
fn system_runner_writes_files_and_counts_threads() {
    let root = scratch("write");
    let mut runner = SystemRunner { root: root.clone() };
    assert!(runner.write("a/b/c.diff", "text\n").is_ok(), "written");
    assert_eq!(
        std::fs::read_to_string(root.join("a/b/c.diff")).expect("readable"),
        "text\n",
        "its directory created"
    );
    assert!(runner.write("a/b/c.diff", "new\n").is_ok(), "rewritten");
    assert_eq!(
        std::fs::read_to_string(root.join("a/b/c.diff")).expect("readable"),
        "new\n",
        "replaced"
    );
    assert!(
        runner.write("a/b/c.diff/d", "").is_err(),
        "cannot create a directory where a file is"
    );
    assert!(runner.write("a/b", "").is_err(), "cannot write a directory");
    assert_eq!(
        runner.parallelism(),
        std::thread::available_parallelism().expect("known here"),
        "the machine's"
    );
    std::fs::remove_dir_all(&root).expect("cleanup");
}

#[test]
fn system_runner_empties_fresh_directories() {
    let root = std::env::temp_dir().join(format!("xtask-fresh-{}", std::process::id()));
    if let Err(error) = std::fs::remove_dir_all(&root) {
        assert_eq!(error.kind(), io::ErrorKind::NotFound, "clean start");
    }
    std::fs::create_dir_all(&root).expect("temporary root");
    let mut runner = SystemRunner { root: root.clone() };
    assert!(runner.fresh_dir("out/wav").is_ok(), "created when missing");
    std::fs::write(root.join("out/wav/stale.wav"), b"old").expect("stale file");
    assert!(runner.fresh_dir("out/wav").is_ok(), "emptied when present");
    assert_eq!(
        std::fs::read_dir(root.join("out/wav"))
            .expect("still a directory")
            .count(),
        0,
        "the stale file is gone"
    );
    std::fs::write(root.join("file"), b"").expect("a plain file");
    assert!(
        runner.fresh_dir("file/below").is_err(),
        "cannot create under a file"
    );
    // A path that exists but cannot be emptied as a directory (a file): emptying it fails
    // with the reason of the removal, not of the creation that would follow (the file is
    // already there), and with no permission bit involved, so that it fails for root too.
    std::fs::write(root.join("plain"), b"kept").expect("a plain file");
    assert_eq!(
        runner.fresh_dir("plain").map_err(|error| error.kind()),
        Err(io::ErrorKind::NotADirectory),
        "a path that cannot be emptied is an error, said by its removal"
    );
    assert_eq!(
        std::fs::read(root.join("plain")).expect("still there"),
        b"kept",
        "and is left as it was"
    );
    std::fs::remove_dir_all(&root).expect("cleanup");
}

#[test]
fn reports_a_directory_that_cannot_be_emptied() {
    let mut fake = passing();
    fake.broken = Some("fresh target/conformance".to_owned());
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut fake, &mut Vec::new()),
        failed("Fresh conformance export directory: cannot empty target/conformance: busy"),
        "named and explained"
    );
}

#[test]
fn refuses_reviews_of_mutants_that_no_longer_exist() {
    let gate = super::unviable_reviews_gate;
    let context = "every review of mutants-unviable.txt must name a mutant that exists";
    let listed = "crates/a/src/b.rs:3:5: replace f -> T with Default::default()\n\
                  crates/a/src/b.rs:9:5: replace += with *= in g\n\
                  crates/a/src/b.rs:12:5: replace += with *= in g\n\n";
    let reviewed = "# a comment\n\ncrates/a/src/b.rs: replace += with *= in g -- one\n\
                    crates/a/src/b.rs: replace += with *= in g -- two\n";
    assert_eq!(
        gate(listed, reviewed),
        Ok(()),
        "every review names a mutant, as often as it exists; unreviewed mutants are another gate's"
    );
    assert_eq!(gate(listed, ""), Ok(()), "no review at all");
    assert_eq!(
        gate(
            listed,
            &format!("{reviewed}crates/a/src/b.rs: replace += with *= in g -- three\n")
        ),
        found(
            context,
            &[
                "stale review: crates/a/src/b.rs: replace += with *= in g (2 found, 3 reviewed)"
                    .to_owned()
            ]
        ),
        "reviewed more often than it exists"
    );
    assert_eq!(
        gate(
            listed,
            "crates/a/src/b.rs: replace += with -= in g -- gone\nno reason here\n"
        ),
        found(
            context,
            &[
                "review without a reason: no reason here".to_owned(),
                "stale review: crates/a/src/b.rs: replace += with -= in g".to_owned(),
            ]
        ),
        "a mutant that no longer exists, and a malformed review"
    );
    assert_eq!(
        gate("not a mutant\n", ""),
        found(context, &["unrecognized mutant: not a mutant".to_owned()]),
        "a listed line that is not a mutant is refused, never skipped"
    );
}

#[test]
fn the_default_check_refuses_stale_reviews_of_unviable_mutants() {
    let steps = check_steps(CHANGES_MODE);
    let mut fake = passing();
    let _stale = fake.files.insert(
        UNVIABLE_REVIEWED.to_owned(),
        "crates/a/src/b.rs: replace f -> T with Default::default() -- T has no Default\n\
         crates/a/src/c.rs: replace h with () -- gone\n"
            .to_owned(),
    );
    assert_eq!(
        run_steps(&steps, &mut fake, &mut Vec::new()),
        failed(
            "Rust unviable mutant reviews all current: every review of mutants-unviable.txt \
             must name a mutant that exists: stale review: crates/a/src/c.rs: replace h with ()"
        ),
        "a review of a mutant cargo-mutants no longer lists"
    );
    let mut unlisted = passing();
    let _gone = unlisted
        .files
        .remove(&format!("capture {}", super::mutants_list_command().line()));
    assert_eq!(
        run_steps(&steps, &mut unlisted, &mut Vec::new()),
        failed(
            "Rust unviable mutant reviews all current: cannot list the Rust mutants: \
             `cargo mutants --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --list \
             --workspace` exited with code 1: failed"
        ),
        "the list cannot be made"
    );
}

#[test]
fn accepts_exactly_the_reviewed_unviable_mutants() {
    let report = "crates/a/src/b.rs:3:5: replace f -> T with Default::default()\n\ncrates/a/src/c.rs:10:1: replace g -> U with x\n";
    let reviewed = "# reviewed mutants\n\ncrates/a/src/c.rs: replace g -> U with x -- U has no x\n  crates/a/src/b.rs: replace f -> T with Default::default() -- T has no Default  \n";
    assert_eq!(
        unviable_gate(report, Extent::Complete, reviewed),
        Ok(()),
        "same set, lines ignored"
    );
    assert_eq!(
        unviable_gate("", Extent::Complete, ""),
        Ok(()),
        "nothing unviable"
    );
}

#[test]
fn rejects_unreviewed_stale_and_malformed_entries() {
    let error = unviable_gate(
        "crates/a/src/b.rs:3:5: replace f -> T with y\nnot a mutant line\ncrates/a/src/b.rs:9:9\n",
        Extent::Complete,
        "crates/a/src/z.rs: replace z -> Z with w -- old\ncrates/a/src/q.rs: replace q -> Q with w --  \nno separator\n",
    );
    assert_eq!(
        error,
        failed(format!(
            "unviable mutants must be reviewed in {UNVIABLE_REVIEWED}: 6 problems, listed above\n{}",
            [
                "unrecognized mutant: not a mutant line",
                "unrecognized mutant: crates/a/src/b.rs:9:9",
                "review without a reason: crates/a/src/q.rs: replace q -> Q with w --",
                "review without a reason: no separator",
                "not reviewed: crates/a/src/b.rs: replace f -> T with y",
                "stale review: crates/a/src/z.rs: replace z -> Z with w",
            ]
            .join("\n")
        )),
        "every problem is listed"
    );
}

#[test]
fn fails_when_unviable_files_are_missing_or_unreviewed() {
    let mut no_report = passing();
    let removed_report = no_report.files.remove(UNVIABLE_REPORT);
    assert!(removed_report.is_some(), "report removed");
    let full = Mode::Full {
        jobs: NonZeroUsize::MIN,
    };
    assert_eq!(
        run_steps(&check_steps(full), &mut no_report, &mut Vec::new()),
        failed(format!(
            "Rust unviable mutants reviewed: cannot read {UNVIABLE_REPORT}: missing"
        )),
        "a full run always writes its report"
    );
    let mut no_reviews = passing();
    let removed_reviews = no_reviews.files.remove(UNVIABLE_REVIEWED);
    assert!(removed_reviews.is_some(), "reviews removed");
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut no_reviews, &mut Vec::new()),
        failed(format!(
            "Rust unviable mutants reviewed: cannot read {UNVIABLE_REVIEWED}: missing"
        )),
        "missing reviews"
    );
    let mut unreviewed = passing();
    let _previous = unreviewed
        .files
        .insert(UNVIABLE_REVIEWED.to_owned(), String::new());
    assert!(
        run_steps(&check_steps(CHANGES_MODE), &mut unreviewed, &mut Vec::new())
            .is_err_and(|error| error.summary.contains("not reviewed: crates/a/src/b.rs")),
        "unreviewed mutant"
    );
}

#[test]
fn accepts_exactly_the_reviewed_equivalent_mutants() {
    let source = |text: &str| vec![("a.ts".to_owned(), text.to_owned())];
    let disabled =
        "x\n  // Stryker disable next-line EqualityOperator: overwritten later\n  i < n\n";
    assert_eq!(
        equivalent_gate(
            &source(disabled),
            "# header\n\na.ts: EqualityOperator at i < n -- overwritten later\n"
        ),
        Ok(()),
        "reviewed"
    );
    assert_eq!(
        equivalent_gate(&source("plain"), ""),
        Ok(()),
        "nothing disabled"
    );
    let error = equivalent_gate(&source(disabled), "b.ts: X at y -- old\nno reason here\n");
    assert_eq!(
        error,
        failed(format!(
            "disabled mutants must be reviewed in {EQUIVALENT_REVIEWED}: 3 problems, listed above\n{}",
            [
                "review without a reason: no reason here",
                "not reviewed: a.ts: EqualityOperator at i < n",
                "stale review: b.ts: X at y",
            ]
            .join("\n")
        )),
        "every problem is listed"
    );
}

#[test]
fn refuses_broad_or_unexplained_disables() {
    for (text, line) in [
        ("// Stryker disable all\nx\n", 1_usize),
        ("// Stryker disable next-line all: everything\nx\n", 1),
        ("a\n// Stryker disable next-line A,B: two\nx\n", 2),
        (
            "// Stryker disable next-line Equality Operator: spaced\nx\n",
            1,
        ),
        ("// Stryker disable next-line EqualityOperator:   \nx\n", 1),
        ("// Stryker disable next-line EqualityOperator\nx\n", 1),
        (
            "// Stryker disable next-line EqualityOperator: last line",
            1,
        ),
        (
            "// Stryker disable next-line EqualityOperator: blank next\n   \n",
            1,
        ),
    ] {
        assert_eq!(
            equivalent_gate(&[("a.ts".to_owned(), text.to_owned())], ""),
            failed(format!(
                "disabled mutants must be reviewed in {EQUIVALENT_REVIEWED}: a.ts:{line}: write `// Stryker disable next-line <Mutator>: <reason>` above the line"
            )),
            "{text:?}"
        );
    }
}

#[test]
fn fails_when_equivalent_files_cannot_be_read() {
    let mut no_list = passing();
    no_list.broken = Some("list *.ts *.tsx".to_owned());
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut no_list, &mut Vec::new()),
        failed(
            "TypeScript equivalent mutants reviewed: cannot list the TypeScript files: `git ls-files -- *.ts *.tsx` exited with code 128: fatal: not a git repository"
        ),
        "no listing"
    );
    let mut no_reviews = passing();
    let removed = no_reviews.files.remove(EQUIVALENT_REVIEWED);
    assert!(removed.is_some(), "reviews removed");
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut no_reviews, &mut Vec::new()),
        failed(format!(
            "TypeScript equivalent mutants reviewed: cannot read {EQUIVALENT_REVIEWED}: missing"
        )),
        "missing reviews"
    );
}

#[test]
fn system_runner_lists_the_tracked_typescript_files() {
    // A repository of its own: the tests also run from copies without git history.
    let root = std::env::temp_dir().join(format!("xtask-git-{}", std::process::id()));
    std::fs::create_dir_all(root.join("src")).expect("temporary repository");
    for (file, text) in [
        ("src/a.ts", "a"),
        ("src/b.tsx", "b"),
        ("src/c.js", "c"),
        ("d.ts", "d"),
        ("src/naïve.ts", "n"),
    ] {
        std::fs::write(root.join(file), text).expect("source file");
    }
    std::fs::write(root.join("untracked.ts"), "u").expect("untracked file");
    let _init = hermetic_git(&root, &["init", "--quiet"]);
    let _add = hermetic_git(&root, &["add", "src", "d.ts"]);
    let files = SystemRunner { root: root.clone() }.tracked(&["*.ts", "*.tsx"]);
    std::fs::remove_dir_all(&root).expect("cleanup");
    assert_eq!(
        files.expect("git lists the files"),
        vec![
            "d.ts".to_owned(),
            "src/a.ts".to_owned(),
            "src/b.tsx".to_owned(),
            "src/naïve.ts".to_owned()
        ],
        "tracked TypeScript only, a name out of ASCII as it is (git quotes it by default)"
    );
    let outside = std::env::temp_dir().join(format!("xtask-no-git-{}", std::process::id()));
    std::fs::create_dir_all(&outside).expect("temporary directory");
    let error = SystemRunner {
        root: outside.clone(),
    }
    .tracked(&["*.ts", "*.tsx"]);
    std::fs::remove_dir_all(&outside).expect("cleanup");
    assert!(
        error.is_err_and(|failure| failure
            .to_string()
            .contains("exited with code 128: fatal: not a git repository")),
        "not a repository, in git's words"
    );
}

#[test]
fn parses_semantic_versions() {
    assert_eq!(
        Version::parse("0.1.0"),
        Some(Version {
            major: 0,
            minor: 1,
            patch: 0
        }),
        "basic"
    );
    assert_eq!(
        Version::parse("10.20.30"),
        Some(Version {
            major: 10,
            minor: 20,
            patch: 30
        }),
        "multi-digit"
    );
    for bad in [
        "",
        "1",
        "1.2",
        "1.2.3.4",
        "01.2.3",
        "1.02.3",
        "1.2.03",
        "1.2.x",
        "1..3",
        "-1.2.3",
        "1.2.3-rc.1",
        "+1.2.3",
    ] {
        assert_eq!(Version::parse(bad), None, "{bad:?} is not X.Y.Z");
    }
    assert_eq!(
        Version {
            major: 1,
            minor: 2,
            patch: 3
        }
        .to_string(),
        "1.2.3",
        "display"
    );
    assert!(
        Version::parse("0.10.0") > Version::parse("0.9.9"),
        "numeric ordering"
    );
}

#[test]
fn accepts_a_consistent_release_setup() {
    assert_eq!(
        release_gate(CARGO, &packages(PACKAGE), LOG),
        Ok(()),
        "current version is the latest release"
    );
    let next = CARGO.replace("0.2.0\"\n\n[other]", "0.3.0\"\n\n[other]");
    assert_eq!(
        release_gate(&next, &packages(&PACKAGE.replace("0.2.0", "0.3.0")), LOG),
        Ok(()),
        "a newer, unreleased version"
    );
    assert_eq!(
        release_gate(
            CARGO,
            &packages(PACKAGE),
            "# Changelog\n\n## [Unreleased]\n"
        ),
        Ok(()),
        "no release yet"
    );
}

#[test]
fn rejects_inconsistent_versions() {
    assert_eq!(
        release_gate(
            CARGO,
            &roadmap_package(r#"{"version": "0.3.0", "engines": {"node": ">=22.18"}}"#),
            LOG
        ),
        found(
            "the versions and the changelog",
            &[format!(
                "{ROADMAP_PACKAGE}: version 0.3.0 differs from the workspace version 0.2.0"
            )]
        ),
        "different versions"
    );
    assert_eq!(
        release_gate(CARGO, &roadmap_package("not json"), LOG),
        found(
            "the versions and the changelog",
            &[
                format!(
                    "{ROADMAP_PACKAGE}: engines.node is (missing), the packages support Node.js \
                     >=22.18 (NODE_FLOOR)"
                ),
                format!(
                    "{ROADMAP_PACKAGE}: version (missing) differs from the workspace version 0.2.0"
                )
            ]
        ),
        "unreadable package"
    );
    assert_eq!(
        release_gate(
            CARGO,
            &packages(r#"{"version": "0.1.0", "engines": {"node": ">=22.18"}}"#),
            LOG
        ),
        found(
            "the versions and the changelog",
            &package_jsons()
                .iter()
                .map(|path| format!(
                    "{path}: version 0.1.0 differs from the workspace version 0.2.0"
                ))
                .collect::<Vec<_>>()
        ),
        "every manifest is checked"
    );
    assert_eq!(
        release_gate(
            "[workspace]\n",
            &packages(r#"{"version": 1, "engines": {"node": ">=22.18"}}"#),
            LOG
        ),
        found(
            "the versions and the changelog",
            &[format!(
                "{CARGO_TOML}: [workspace.package] version must be X.Y.Z"
            )]
        ),
        "no workspace version, and none in the package either"
    );
    let bad = CARGO.replace("0.2.0\"\n\n[other]", "0.2\"\n\n[other]");
    assert_eq!(
        release_gate(&bad, &packages(&PACKAGE.replace("0.2.0", "0.2")), LOG),
        found(
            "the versions and the changelog",
            &[format!(
                "{CARGO_TOML}: [workspace.package] version must be X.Y.Z"
            )]
        ),
        "not X.Y.Z"
    );
    let old = CARGO.replace("0.2.0\"\n\n[other]", "0.1.9\"\n\n[other]");
    assert_eq!(
        release_gate(&old, &packages(&PACKAGE.replace("0.2.0", "0.1.9")), LOG),
        found(
            "the versions and the changelog",
            &["version 0.1.9 is older than the latest release 0.2.0".to_owned()]
        ),
        "older than released"
    );
}

#[test]
fn rejects_a_malformed_changelog() {
    assert_eq!(
        release_gate(
            CARGO,
            &packages(PACKAGE),
            "Changelog\n## [0.2.0] - 2026-11-07\n"
        ),
        found(
            "the versions and the changelog",
            &[
                format!("{CHANGELOG}: must start with \"# Changelog\""),
                format!("{CHANGELOG}: the first section must be \"## [Unreleased]\"")
            ]
        ),
        "missing title and unreleased section"
    );
    for heading in [
        "## [0.2] - 2026-11-07",
        "## 0.2.0 - 2026-11-07",
        "## [0.2.0] 2026-11-07",
        "## [0.2.0] - 26-11-07",
        "## [0.2.0] - 2026-1-07",
        "## [0.2.0] - 2026-11-7",
        "## [0.2.0] - 2026-11-07-01",
        "## [0.2.0] - 2026-11",
        "## [0.2.0] - 2026-aa-07",
    ] {
        assert_eq!(
            release_gate(
                CARGO,
                &packages(PACKAGE),
                &format!("# Changelog\n## [Unreleased]\n{heading}\n")
            ),
            found(
                "the versions and the changelog",
                &[format!("{CHANGELOG}: malformed heading \"{heading}\"")]
            ),
            "{heading}"
        );
    }
    for order in [
        "## [0.1.0] - 2026-01-01\n## [0.2.0] - 2026-02-01",
        "## [0.2.0] - 2026-01-01\n## [0.2.0] - 2026-02-01",
    ] {
        assert_eq!(
            release_gate(
                CARGO,
                &packages(PACKAGE),
                &format!("# Changelog\n## [Unreleased]\n{order}\n")
            ),
            found(
                "the versions and the changelog",
                &[format!(
                    "{CHANGELOG}: releases must be in strictly decreasing order"
                )]
            ),
            "{order}"
        );
    }
}

#[test]
fn accepts_conventional_commit_messages() {
    for message in [
        "feat: add the plan",
        "fix(engine-core): keep box state across blocks\n\nBody text.\nCo-Authored-By: x",
        "refactor(xtask)!: rename the gate",
        "build(engine-wasm): bump the format",
        "chore(roadmap): tick",
        "docs(deep-review): add a check",
        "test(config): cover",
        "docs!: rewrite the readme",
        "# comment from git\nci: run on every push\n# another comment\n",
    ] {
        assert_eq!(commit_msg_gate(message), Ok(()), "{message:?}");
    }
    let header = format!("feat: {}", "x".repeat(COMMIT_HEADER_MAX - 6));
    assert_eq!(
        commit_msg_gate(&header),
        Ok(()),
        "header of exactly the maximum length"
    );
    for kind in [
        "feat", "fix", "docs", "test", "refactor", "perf", "build", "ci", "chore", "style",
        "revert",
    ] {
        assert_eq!(
            commit_msg_gate(&format!("{kind}: x")),
            Ok(()),
            "{kind} is allowed"
        );
    }
}

#[test]
fn rejects_other_commit_messages() {
    let expected = |reason: &str| {
        failed(format!(
            "invalid commit message: {reason}\nexpected: <type>(<scope>)!: <summary>, type one of feat, fix, docs, test, refactor, perf, build, ci, chore, style, revert"
        ))
    };
    let long = format!("feat: {}", "x".repeat(COMMIT_HEADER_MAX - 5));
    let unknown = |scope: &str| {
        format!(
            "unknown scope \"{scope}\": name a crate, a package or a scope of xtask's \
             COMMIT_SCOPE_EXTRAS (engine-core, engine-objects, engine-patch, engine-hash, \
             engine-alloc-counter, engine-fpenv, engine-battery, engine-testkit, engine-cli, \
             engine-wasm, xtask, config, oracle, playground, roadmap, spec, conformance, ci, \
             build, release, changelog, glossary, deep-review)"
        )
    };
    let cases: [(&str, String); 18] = [
        ("refactor(wasm): rename", unknown("wasm")),
        ("feat(core)!: add", unknown("core")),
        ("docs(bindings): x", unknown("bindings")),
        ("fix(x2): y", unknown("x2")),
        ("", "missing \"<type>: \" prefix".to_owned()),
        (
            "Merge whatever I want",
            "missing \"<type>: \" prefix".to_owned(),
        ),
        (
            "Merge branch 'main' into feature",
            "missing \"<type>: \" prefix".to_owned(),
        ),
        (
            "Revert \"anything goes",
            "missing \"<type>: \" prefix".to_owned(),
        ),
        ("Add the plan", "missing \"<type>: \" prefix".to_owned()),
        ("feature: add", "unknown type \"feature\"".to_owned()),
        ("Feat: add", "unknown type \"Feat\"".to_owned()),
        (
            "feat(): add",
            "the scope must be lowercase letters, digits or hyphens".to_owned(),
        ),
        (
            "feat(Core): add",
            "the scope must be lowercase letters, digits or hyphens".to_owned(),
        ),
        (
            "feat(core: add",
            "the scope must be lowercase letters, digits or hyphens".to_owned(),
        ),
        (
            "feat:  ",
            "the summary must be non-empty and must not end with a period".to_owned(),
        ),
        (
            "feat: add the plan.",
            "the summary must be non-empty and must not end with a period".to_owned(),
        ),
        (
            long.as_str(),
            format!("the header is longer than {COMMIT_HEADER_MAX} characters"),
        ),
        (
            "feat: add\nbody right away",
            "a blank line must separate the header from the body".to_owned(),
        ),
    ];
    for (message, reason) in cases {
        assert_eq!(commit_msg_gate(message), expected(&reason), "{message:?}");
    }
}

#[test]
fn cli_checks_commit_messages() {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let args = ["commit-msg".to_owned(), "COMMIT_EDITMSG".to_owned()];
    assert_eq!(
        run_cli(&args, &mut passing(), &mut out, &mut err),
        0,
        "a valid message"
    );
    assert!(out.is_empty() && err.is_empty(), "silent on success");

    let mut bad = passing();
    let replaced = bad
        .files
        .insert("COMMIT_EDITMSG".to_owned(), "wip".to_owned());
    assert!(replaced.is_some(), "message replaced");
    let (mut bad_out, mut bad_err) = (Vec::new(), Vec::new());
    assert_eq!(
        run_cli(&args, &mut bad, &mut bad_out, &mut bad_err),
        1,
        "an invalid message"
    );
    assert_eq!(
        String::from_utf8(bad_err).expect("UTF-8"),
        "error: invalid commit message: missing \"<type>: \" prefix\n",
        "one line on stderr"
    );
    assert_eq!(
        String::from_utf8(bad_out).expect("UTF-8"),
        "expected: <type>(<scope>)!: <summary>, type one of feat, fix, docs, test, refactor, \
         perf, build, ci, chore, style, revert\n",
        "the expected form on stdout"
    );

    let mut missing_err = Vec::new();
    let missing = ["commit-msg".to_owned(), "nowhere".to_owned()];
    assert_eq!(
        run_cli(&missing, &mut passing(), &mut Vec::new(), &mut missing_err),
        1,
        "a missing file"
    );
    assert_eq!(
        missing_err, b"error: cannot read nowhere: missing\n",
        "named on stderr"
    );
}

#[test]
fn release_gate_runs_on_the_real_files() {
    let mut fake = passing();
    let replaced = fake.files.insert(CHANGELOG.to_owned(), "nope".to_owned());
    assert!(replaced.is_some(), "changelog replaced");
    assert!(
        run_steps(&check_steps(CHANGES_MODE), &mut fake, &mut Vec::new()).is_err_and(|error| {
            error
                .summary
                .starts_with("One version everywhere, changelog well formed: ")
        }),
        "release gate failure is named"
    );
    for path in [CARGO_TOML.to_owned(), CHANGELOG.to_owned()]
        .into_iter()
        .chain(package_jsons())
    {
        let mut without = passing();
        let removed = without.files.remove(&path);
        assert!(removed.is_some(), "{path} removed");
        // The release step alone: the membership gate before it also notices a removal.
        let release: Vec<Step> = check_steps(CHANGES_MODE)
            .into_iter()
            .filter(|step| step.action == Action::VersionGate)
            .collect();
        assert_eq!(
            run_steps(&release, &mut without, &mut Vec::new()),
            failed(format!(
                "One version everywhere, changelog well formed: cannot read {path}: missing"
            )),
            "missing {path}"
        );
    }
}

/// What the preflight asks and reads in a passing fake: the ten tools, `.node-version`
/// and the toolchain file.
const PREFLIGHT_RAN: [&str; 12] = [
    "capture git --version",
    "capture cargo --version",
    "capture cargo +nightly-2026-11-01 --version",
    "capture cargo +nightly-2026-11-01 miri --version",
    "capture cargo llvm-cov --version",
    "capture cargo mutants --version",
    "capture cargo deny --version",
    "capture node --version",
    "capture pnpm --version",
    "capture rustc --version",
    "read .node-version",
    "read rust-toolchain.toml",
];

#[test]
fn bless_benchmarks_counts_and_rewrites_the_baseline() {
    let steps = bless_benchmarks_steps();
    assert_eq!(
        steps
            .iter()
            .map(|step| step.name.as_str())
            .collect::<Vec<_>>(),
        ["Build the engine module", "Bless the benchmark baseline"],
        "names"
    );
    assert_eq!(
        command_lines(&steps),
        vec![
            "Package(\"bindings/wasm\"): pnpm run build:wasm",
            "Package(\"bindings/wasm\"): node scripts/benchmarks.ts bless"
        ],
        "builds the module, then runs the benchmark command's bless, which no package script runs"
    );
    let mut fake = passing();
    let mut out = Vec::new();
    assert_eq!(
        run_cli(
            &["bless-benchmarks".to_owned()],
            &mut fake,
            &mut out,
            &mut Vec::new()
        ),
        0,
        "bless-benchmarks succeeds"
    );
    assert_eq!(
        fake.ran,
        [
            PREFLIGHT_RAN.as_slice(),
            &["pnpm run build:wasm", "node scripts/benchmarks.ts bless"]
        ]
        .concat(),
        "the preflight, then only the build and the benchmark bless run"
    );
    assert_eq!(
        String::from_utf8(out).expect("utf-8"),
        "▶ [1/3] Tools and platform\n  done in 1.5 s\n▶ [2/3] Build the engine module\n  done in \
         1.5 s\n▶ [3/3] Bless the benchmark baseline\n  done in 1.5 s\n✔ all 3 checks passed in \
         4.5 s\n",
        "progress"
    );
}

/// What `cargo xtask bless` tells about its options.
const BLESS_USAGE: &str = "`bless` takes only `--case-changed <name>` (repeatable), \
                           `--battery-changed` and `--from-nothing`";

/// How the fake lists the record of the conformance references at its merge base.
const RECORD_AT_BASE: &str = "capture git ls-tree --name-only \
                              abc1230000000000000000000000000000000000 -- \
                              conformance/algorithms.json";

/// How the fake shows the record of the conformance references at its merge base.
const SHOW_RECORD: &str =
    "capture git show abc1230000000000000000000000000000000000:conformance/algorithms.json";

/// The command `cargo xtask bless` runs, without options.
const BLESS_RAN: &str = "cargo run --package engine-cli --locked --quiet -- bless conformance \
                         --spent target/xtask/spent-algorithms.json";

#[test]
fn bless_reports_the_failure_of_the_bless_it_runs() {
    let mut fake = Fake {
        fail: Some(BLESS_RAN.to_owned()),
        ..passing()
    };
    drop(fake.files.insert(RECORD_AT_BASE.to_owned(), String::new()));
    let mut err = Vec::new();
    assert_eq!(
        run_cli(&["bless".to_owned()], &mut fake, &mut Vec::new(), &mut err),
        1,
        "a failing bless fails the command"
    );
    assert_eq!(
        fake.ran.last().map(String::as_str),
        Some(BLESS_RAN),
        "the bless ran last"
    );
    assert_eq!(
        String::from_utf8(err).expect("utf-8"),
        format!("error: Bless conformance references: `{BLESS_RAN}` exited with code 3\n"),
        "the bless's own failure, once"
    );
}

#[test]
fn bless_renders_and_rewrites_the_references() {
    let step = bless_step(&super::BlessOptions::default());
    assert_eq!(step.name, "Bless conformance references", "name");
    assert_eq!(
        command_lines(&[step]),
        [format!("bless {BLESS_RAN}")],
        "blesses with the record at the merge base"
    );
    let mut fake = passing();
    drop(fake.files.insert(
        RECORD_AT_BASE.to_owned(),
        "conformance/algorithms.json\n".to_owned(),
    ));
    drop(
        fake.files
            .insert(SHOW_RECORD.to_owned(), "{\"spent\": 1}\n".to_owned()),
    );
    let mut out = Vec::new();
    assert_eq!(
        run_cli(&["bless".to_owned()], &mut fake, &mut out, &mut Vec::new()),
        0,
        "bless succeeds"
    );
    assert_eq!(
        fake.ran,
        [
            PREFLIGHT_RAN.as_slice(),
            &[
                "capture git merge-base HEAD @{upstream}",
                RECORD_AT_BASE,
                SHOW_RECORD,
                "write target/xtask/spent-algorithms.json",
                BLESS_RAN,
            ]
        ]
        .concat(),
        "the preflight, the record at the merge base, then bless with it"
    );
    assert_eq!(
        fake.files.get(super::SPENT_RECORD).map(String::as_str),
        Some("{\"spent\": 1}\n"),
        "the record at the merge base, as git gives it"
    );
    assert_eq!(
        String::from_utf8(out).expect("utf-8"),
        "▶ [1/2] Tools and platform\n  done in 1.5 s\n▶ [2/2] Bless conformance references\n  \
         spent: its versions at abc1230000000000000000000000000000000000 (merge base with \
         @{upstream})\n  done in 1.5 s\n✔ all 2 checks passed in 3.0 s\n",
        "progress"
    );
    let mut named = passing();
    drop(named.files.insert(RECORD_AT_BASE.to_owned(), String::new()));
    let mut named_out = Vec::new();
    assert_eq!(
        run_cli(
            &[
                "bless",
                "--case-changed",
                "a",
                "--from-nothing",
                "--case-changed",
                "b c",
                "--battery-changed"
            ]
            .map(str::to_owned),
            &mut named,
            &mut named_out,
            &mut Vec::new()
        ),
        0,
        "bless with every option succeeds"
    );
    assert_eq!(
        named.ran.last().map(String::as_str),
        Some(
            format!(
                "{BLESS_RAN} --case-changed a --case-changed b c --battery-changed --from-nothing"
            )
            .as_str()
        ),
        "every option is forwarded, the cases in order"
    );
    assert!(
        !named.ran.contains(&SHOW_RECORD.to_owned()),
        "nothing to show"
    );
    assert_eq!(
        named.files.get(super::SPENT_RECORD).map(String::as_str),
        Some("null\n"),
        "no record at the merge base: nothing spent"
    );
    assert!(
        String::from_utf8(named_out)
            .expect("utf-8")
            .contains("  spent: none at abc1230000000000000000000000000000000000"),
        "said"
    );
}

#[test]
fn blesses_only_with_the_record_at_the_merge_base() {
    let mut cases: Vec<(Fake, &str)> = Vec::new();
    let mut baseless = passing();
    let _upstream = baseless.files.remove(&format!("capture {}", MERGE_BASE.0));
    cases.push((
        baseless,
        "error: Bless conformance references: no base to measure the changes against",
    ));
    let mut unlisted = passing();
    unlisted.broken = Some(RECORD_AT_BASE.trim_start_matches("capture ").to_owned());
    cases.push((
        unlisted,
        "error: Bless conformance references: cannot read conformance/algorithms.json at the \
         merge base abc1230000000000000000000000000000000000: ",
    ));
    let mut unshown = passing();
    drop(unshown.files.insert(
        RECORD_AT_BASE.to_owned(),
        "conformance/algorithms.json\n".to_owned(),
    ));
    cases.push((
        unshown,
        "error: Bless conformance references: cannot read conformance/algorithms.json at the \
         merge base abc1230000000000000000000000000000000000: ",
    ));
    let mut unwritable = passing();
    drop(
        unwritable
            .files
            .insert(RECORD_AT_BASE.to_owned(), String::new()),
    );
    unwritable.broken = Some("write target/xtask/spent-algorithms.json".to_owned());
    cases.push((
        unwritable,
        "error: Bless conformance references: cannot write target/xtask/spent-algorithms.json: \
         read-only",
    ));
    for (mut fake, said) in cases {
        let mut err = Vec::new();
        assert_eq!(
            run_cli(&["bless".to_owned()], &mut fake, &mut Vec::new(), &mut err),
            1,
            "{said}"
        );
        assert!(
            String::from_utf8(err).expect("utf-8").starts_with(said),
            "{said}"
        );
        assert!(
            !fake.ran.contains(&BLESS_RAN.to_owned()),
            "{said}: nothing blessed"
        );
    }
}

#[test]
fn blesses_only_where_the_checks_could_run() {
    // A bless writes what every later check compares with: it starts with the preflight of
    // the check, so that a build drifted by the environment (a profile, a wrapper, an
    // outside Cargo configuration) is never blessed.
    for (task, script) in [
        ("bless", BLESS_RAN),
        ("bless-benchmarks", "node scripts/benchmarks.ts bless"),
    ] {
        let mut fake = passing();
        fake.environment.variables = vec!["CARGO_PROFILE_RELEASE_OPT_LEVEL".to_owned()];
        let mut err = Vec::new();
        assert_eq!(
            run_cli(&[task.to_owned()], &mut fake, &mut Vec::new(), &mut err),
            1,
            "{task}: a drifting variable fails"
        );
        assert_eq!(
            String::from_utf8(err).expect("utf-8"),
            "error: Tools and platform: the checks cannot run here: \
             CARGO_PROFILE_RELEASE_OPT_LEVEL is set, which changes what cargo builds or how it \
             lints: unset it\n",
            "{task}: the preflight's reason"
        );
        assert!(
            !fake.ran.contains(&script.to_owned()),
            "{task}: nothing blessed"
        );
        let mut configured = passing();
        configured.environment.cargo_configs = vec!["/home/x/.cargo/config.toml".to_owned()];
        assert_eq!(
            run_cli(
                &[task.to_owned()],
                &mut configured,
                &mut Vec::new(),
                &mut Vec::new()
            ),
            1,
            "{task}: an outside Cargo configuration fails"
        );
        assert!(
            !configured.ran.contains(&script.to_owned()),
            "{task}: nothing blessed"
        );
    }
}

#[test]
fn accepts_only_fully_covered_typescript_packages() {
    assert_eq!(ts_coverage_gate("p", TS_COVERED), Ok(()), "100%");
    assert_eq!(
        ts_coverage_gate(
            "p",
            r#"{"total":{"lines":{"total":4,"covered":3},"statements":{"total":0,"covered":0},"functions":{"total":2},"branches":{"total":3,"covered":3}}}"#
        ),
        Err(
            "p: coverage is not 100%: lines 3/4, statements not measured, no functions total"
                .to_owned()
        ),
        "every kind is checked"
    );
    assert_eq!(
        ts_coverage_gate(
            "p",
            r#"{"total":{"lines":{"total":1,"covered":1},"statements":{"total":1,"covered":1},"functions":{"total":1,"covered":1},"branches":{"total":0,"covered":0}}}"#
        ),
        Ok(()),
        "code without a branch, every line covered"
    );
    assert_eq!(
        ts_coverage_gate(
            "p",
            r#"{"total":{"lines":{"total":0,"covered":0},"statements":{"total":0,"covered":0},"functions":{"total":0,"covered":0},"branches":{"total":0,"covered":0}}}"#
        ),
        Err("p: coverage is not 100%: lines not measured, statements not measured, functions not measured".to_owned()),
        "a report that measured nothing"
    );
    assert_eq!(
        ts_coverage_gate(
            "p",
            r#"{"total":{"lines":{"total":1,"covered":1},"statements":{"total":1,"covered":1},"functions":{"total":1,"covered":1}}}"#
        ),
        Err("p: coverage is not 100%: no branches total".to_owned()),
        "branches absent, not zero"
    );
    assert!(
        ts_coverage_gate("p", "nope")
            .is_err_and(|error| error.starts_with("p: invalid coverage summary: ")),
        "unreadable"
    );
}

/// The disable comment of [`DISABLED_SOURCE`].
fn disabled() -> Vec<super::Disable> {
    vec![super::Disable {
        path: DISABLED_SOURCE.0.to_owned(),
        line: 3,
        mutator: "EqualityOperator".to_owned(),
    }]
}

/// A Stryker report of `a.ts` holding one mutant.
fn one_mutant(status: &str, mutator: &str, line: u64) -> String {
    format!(
        r#"{{"files":{{"x.ts":{{"mutants":[{{"status":"{status}","mutatorName":"{mutator}","location":{{"start":{{"line":{line}}}}}}}]}}}}}}"#
    )
}

#[test]
fn accepts_only_typescript_packages_whose_mutants_are_all_killed() {
    assert_eq!(
        ts_mutation_gate(
            "tooling/config",
            TS_KILLED_OR_DISABLED,
            &disabled(),
            Extent::Complete
        ),
        Ok(()),
        "killed, timed out, or ignored where a reviewed comment says so"
    );
    for status in [
        "Survived",
        "NoCoverage",
        "RuntimeError",
        "CompileError",
        "Pending",
        "Whatever",
    ] {
        assert_eq!(
            ts_mutation_gate(
                "tooling/config",
                &one_mutant(status, "M", 7),
                &disabled(),
                Extent::Complete
            ),
            failed(format!(
                "tooling/config: mutants not killed: tooling/config/x.ts:7 M {status}"
            )),
            "{status} is not killed"
        );
    }
    for (mutator, line, package) in [
        ("EqualityOperator", 4, "tooling/config"),
        ("BooleanLiteral", 3, "tooling/config"),
        ("EqualityOperator", 3, "tooling/oracle"),
    ] {
        assert_eq!(
            ts_mutation_gate(
                package,
                &one_mutant("Ignored", mutator, line),
                &disabled(),
                Extent::Complete
            ),
            failed(format!(
                "{package}: mutants not killed: {package}/x.ts:{line} {mutator} Ignored"
            )),
            "ignored where no reviewed comment says so: {mutator} at {package}/x.ts:{line}"
        );
    }
    assert_eq!(
        ts_mutation_gate(
            "p",
            r#"{"files":{"a.ts":{"mutants":[{"status":"Survived","mutatorName":"A","location":{"start":{"line":1}}},{"status":"NoCoverage","mutatorName":"B","location":{"start":{"line":2}}},{"status":"Killed"}]}}}"#,
            &[],
            Extent::Partial
        ),
        failed(
            "p: mutants not killed: 2 problems, listed above\np/a.ts:1 A Survived\np/a.ts:2 B NoCoverage"
        ),
        "every mutant named"
    );
    for empty in [r#"{"files":{}}"#, r#"{"files":{"a.ts":{}}}"#, "{}"] {
        assert_eq!(
            ts_mutation_gate("p", empty, &[], Extent::Complete),
            failed("p: mutants not killed: no mutant in the mutation report"),
            "{empty}"
        );
        assert_eq!(
            ts_mutation_gate("p", empty, &[], Extent::Partial),
            Ok(()),
            "{empty}: a partial run may mutate nothing"
        );
    }
    assert!(
        ts_mutation_gate("p", "[", &[], Extent::Partial)
            .is_err_and(|error| error.summary.starts_with("p: invalid mutation report: ")),
        "unreadable"
    );
}

#[test]
fn refuses_stryker_configurations_that_hide_mutants() {
    assert_eq!(
        super::stryker_config_gate("p", STRYKER_CONFIG),
        Ok(()),
        "nothing hidden"
    );
    let hiding = r#"{"ignoreStatic":true,"mutator":{"excludedMutations":["StringLiteral"]},"mutate":["src/**/*.ts","!src/x.ts"],"reporters":["html"]}"#;
    assert_eq!(
        super::stryker_config_gate("p", hiding),
        failed(
            "p/stryker.config.json hides mutants: 4 problems, listed above\n\
             `ignoreStatic` is not allowed: it could hide mutants\n\
             `mutator` is not allowed: it could hide mutants\n\
             `!src/x.ts` in mutate excludes files from mutation\n\
             the `json` reporter is missing: the mutation report is not written"
        ),
        "a key, a negated glob and no report"
    );
    assert!(
        super::stryker_config_gate("p", "{").is_err_and(|failure| failure
            .summary
            .starts_with("p: invalid Stryker configuration: ")),
        "unreadable"
    );
    assert_eq!(
        super::stryker_config_gate("p", "[]"),
        failed("p: the Stryker configuration is not an object"),
        "not an object"
    );
}

#[test]
fn requires_every_bypass_of_a_proof_to_be_reviewed() {
    let rust = format!(
        "#[test]\n#[{}]\nfn a() {{}}\n#[{}::{}]\nfn b() {{}}\nlet c = d.iter().{}(1);\n",
        "ignore", "mutants", "skip", "skip"
    );
    let typescript = format!(
        "it.{}(\"x\", () => {{}});\n/* {} {} next */\n",
        "only", "v8", "ignore"
    );
    let sources = vec![
        ("crates/a/src/lib.rs".to_owned(), rust),
        ("apps/a/src/a.test.ts".to_owned(), typescript.clone()),
        (
            "tooling/config/test/policy/focused.ts".to_owned(),
            typescript,
        ),
        ("crates/a/mutants.toml".to_owned(), String::new()),
        ("README.md".to_owned(), format!("it.{}(", "only")),
    ];
    let ignore = format!("#[{}]", "ignore");
    let skip = format!("#[{}::{}]", "mutants", "skip");
    let v8 = format!("{} {}", "v8", "ignore");
    let mutants_skip = format!("{}::{}", "mutants", "skip");
    assert_eq!(
        bypass_gate_text(&sources, ""),
        failed(format!(
            "bypasses of coverage or mutation testing must be reviewed in proof-bypasses.txt: \
             5 problems, listed above\n\
             not reviewed: apps/a/src/a.test.ts: .{only}( at it.{only}(\"x\", () => {{}});\n\
             not reviewed: apps/a/src/a.test.ts: {v8} at /* {v8} next */\n\
             not reviewed: crates/a/mutants.toml: a cargo-mutants configuration\n\
             not reviewed: crates/a/src/lib.rs: #[{ignore_word} at {ignore}\n\
             not reviewed: crates/a/src/lib.rs: {mutants_skip} at {skip}",
            only = "only",
            ignore_word = "ignore",
        )),
        "every token in the language where it acts, a policy fixture and prose apart, an \
         iterator's skip apart"
    );
    let reviewed = format!(
        "crates/a/mutants.toml: a cargo-mutants configuration -- reason\n\
         crates/a/src/lib.rs: #[{} at {ignore} -- reason\n",
        "ignore"
    );
    assert_eq!(
        bypass_gate_text(&sources[2..4], &reviewed),
        failed(format!(
            "bypasses of coverage or mutation testing must be reviewed in proof-bypasses.txt: \
             stale review: crates/a/src/lib.rs: #[{} at {ignore}",
            "ignore"
        )),
        "a reviewed occurrence passes, a review of nothing is stale"
    );
}

/// The words of the bypasses, built so that this file does not hold them: a doc test fence,
/// the lint of unexpected `cfg`s, the declaration of an expected one, the comments of the
/// TypeScript coverage tools (v8, c8, istanbul, Node.js) and their option ignoring methods.
fn bypass_words() -> [String; 8] {
    [
        "`".repeat(3),
        format!("{}_{}", "unexpected", "cfgs"),
        format!("{}-{}", "check", "cfg"),
        format!("{} {}", "v8", "ignore"),
        format!("{} {}", "c8", "ignore"),
        format!("{} {}", "istanbul", "ignore"),
        format!("{} {}", "node:coverage", "ignore"),
        format!("{}{}", "ignore", "ClassMethods"),
    ]
}

/// Requires the bypass gate to refuse each of `refused` (path, text, token), reported at the
/// line where it starts: past a first line of code or a manifest section.
fn assert_refused(refused: Vec<(&str, String, String)>) {
    for (path, text, token) in refused {
        let at = text
            .lines()
            .find(|line| !line.starts_with("const ") && !line.starts_with('['))
            .unwrap_or_default();
        let sources = [(path.to_owned(), text.clone())];
        let failure = bypass_gate_text(&sources, "");
        assert!(
            failure.as_ref().is_err_and(|failure| failure
                .summary
                .contains(&format!("not reviewed: {path}: {token} at {}", at.trim()))),
            "{path}: {text:?} refused as {token}: {failure:?}"
        );
    }
}

#[test]
fn refuses_every_form_of_a_rust_bypass() {
    let [fence, cfgs, check_cfg, ..] = bypass_words();
    let attr = format!("{}_{}", "cfg", "attr");
    assert_refused(vec![
        (
            "crates/a/src/lib.rs",
            format!("/// {fence}{}\n", "ignore"),
            format!("{fence}{}", "ignore"),
        ),
        (
            "crates/a/src/lib.rs",
            format!("//! {fence}rust,{}\n", "ignore"),
            format!("{fence}{}", "ignore"),
        ),
        (
            "crates/a/src/lib.rs",
            format!("/// {}{}\n", "~".repeat(3), "no_run"),
            format!("{fence}{}", "no_run"),
        ),
        (
            "crates/a/src/lib.rs",
            format!("#[{}(any(), {})]\n", attr, "ignore"),
            attr.clone(),
        ),
        (
            "crates/a/src/lib.rs",
            format!("#[{} (all(), {})]\n", attr, "ignore"),
            attr,
        ),
        (
            "crates/a/src/lib.rs",
            format!("#[cfg(all(test, not({})))]\n", "miri"),
            "miri".to_owned(),
        ),
        (
            "crates/a/src/lib.rs",
            format!("#[cfg(not({}))]\n", "miri"),
            "miri".to_owned(),
        ),
        (
            "crates/a/src/lib.rs",
            format!("#[cfg(not( r#{} ))]\n", "miri"),
            "miri".to_owned(),
        ),
        (
            "crates/a/src/lib.rs",
            format!("if cfg!({}) {{\n", "miri"),
            "miri".to_owned(),
        ),
        (
            "crates/a/src/lib.rs",
            format!("# [ {} ]\n", "ignore"),
            format!("#[{}", "ignore"),
        ),
        (
            "crates/a/src/lib.rs",
            format!("#[{} = \"slow\"]\n", "ignore"),
            format!("#[{}", "ignore"),
        ),
        (
            "crates/a/src/lib.rs",
            format!("#[r#{}]\n", "ignore"),
            format!("r#{}", "ignore"),
        ),
        (
            "crates/a/src/lib.rs",
            format!("#![allow({cfgs}, reason = \"x\")]\n"),
            cfgs.clone(),
        ),
        (
            "crates/a/src/lib.rs",
            format!("#[expect({cfgs})]\n"),
            cfgs.clone(),
        ),
        (
            "crates/a/build.rs",
            format!("println!(\"cargo::rustc-{check_cfg}=cfg(x)\");\n"),
            check_cfg.clone(),
        ),
        (
            "crates/a/Cargo.toml",
            format!("[lints.rust]\n{cfgs} = \"allow\"\n"),
            cfgs,
        ),
        (
            "Cargo.toml",
            format!(
                "[workspace.lints.rust.x]\n{check_cfg} = [\"cfg({})\"]\n",
                "coverage"
            ),
            check_cfg,
        ),
    ]);
}

#[test]
fn refuses_the_harness_key_in_every_manifest() {
    let key = format!("{}{}", "har", "ness");
    assert_refused(vec![
        (
            "crates/a/Cargo.toml",
            format!("[[test]]\n{key} = false\n"),
            key.clone(),
        ),
        (
            "Cargo.toml",
            format!("[[bench]]\n\"{}{}\"=true\n", "HAR", "NESS"),
            key.clone(),
        ),
    ]);
    let manifest = [(
        "crates/a/Cargo.toml".to_owned(),
        format!("[[bench]]\n{key} = false\n"),
    )];
    assert_eq!(
        bypass_gate_text(
            &manifest,
            &format!("crates/a/Cargo.toml: {key} at {key} = false -- a benchmark's own main\n")
        ),
        Ok(()),
        "a reviewed key"
    );
    assert_eq!(
        bypass_gate_text(
            &[(
                "crates/a/src/lib.rs".to_owned(),
                format!("// {key} = false\n")
            )],
            ""
        ),
        Ok(()),
        "the word in Rust is no manifest key"
    );
}

#[test]
fn refuses_every_form_of_a_typescript_bypass() {
    let [_, _, _, v8, c8, istanbul, node, methods] = bypass_words();
    assert_refused(vec![
        (
            "apps/a/src/a.ts",
            format!("/* {} next */\n", v8.replace(' ', "  ")),
            v8.clone(),
        ),
        (
            "apps/a/src/a.ts",
            format!("/* {} start */\n", v8.replace(' ', "\t")),
            v8.clone(),
        ),
        (
            "apps/a/src/a.ts",
            format!("/* {} next */\n", v8.to_uppercase()),
            v8.clone(),
        ),
        (
            "apps/a/src/a.ts",
            format!("const a = 1;\n/* {}\n   next */\n", v8.replace(' ', "\n  ")),
            v8.clone(),
        ),
        ("apps/a/src/a.mts", format!("/* {node} next */\n"), node),
        (
            "apps/a/src/a.cts",
            format!("/* {istanbul} else */\n"),
            istanbul,
        ),
        ("apps/a/src/a.cjs", format!("// {c8} next\n"), c8),
        ("apps/a/src/a.jsx", format!("/* {v8} next */\n"), v8),
        (
            "apps/a/vitest.config.ts",
            format!("coverage: {{ {methods}: [\"run\"] }},\n"),
            methods,
        ),
        (
            "apps/a/src/a.test.jsx",
            format!("it.{}(\"x\", () => {{}});\n", "only"),
            format!(".{}(", "only"),
        ),
    ]);
}

#[test]
fn accepts_what_only_resembles_a_bypass() {
    let [fence, cfgs, check_cfg, v8, _, _, _, methods] = bypass_words();
    let attr = format!("{}_{}", "cfg", "attr");
    let accepted = [
        (
            "crates/a/src/lib.rs",
            format!("/// {fence}text\n/// {fence}\n"),
        ),
        (
            "crates/a/src/lib.rs",
            format!(
                "/// {fence}compile_fail,E0599\n// we {} nothing\n",
                "ignore"
            ),
        ),
        ("crates/a/src/lib.rs", format!("/* {v8} next */\n")),
        (
            "crates/a/src/lib.rs",
            format!(
                "let tool = {:?};\n// Miri checks it\nlet {}n = x_{};\nlet v = vec![{}];\n",
                "miri", "miri", "miri", "ignore"
            ),
        ),
        (
            "README.md",
            format!("#[{}(not({}), {})]\n", attr, "miri", "ignore"),
        ),
        (
            "apps/a/src/a.ts",
            "coverage: { provider: \"v8\" },\n".to_owned(),
        ),
        (
            "apps/a/src/a.ts",
            "const c8 = 1;\nconst nore = c8;\n".to_owned(),
        ),
        ("README.md", format!("/* {v8} next */ {cfgs} {check_cfg}\n")),
        ("apps/a/a.json", format!("{{\"{methods}\": []}}\n")),
    ];
    for (path, text) in accepted {
        assert_eq!(
            bypass_gate_text(&[(path.to_owned(), text.clone())], ""),
            Ok(()),
            "{path}: {text:?} is no bypass"
        );
    }
}

#[test]
fn refuses_a_package_script_running_stryker_otherwise() {
    let manifest = |scripts: serde_json::Value| {
        serde_json::json!({ "name": "p", "scripts": scripts }).to_string()
    };
    for accepted in [
        manifest(serde_json::json!({ "test:mutation": "stryker run stryker.config.json" })),
        manifest(serde_json::json!({ "check": "vitest run" })),
        r#"{"name":"p"}"#.to_owned(),
    ] {
        assert_eq!(
            super::stryker_scripts_gate("p/package.json", &accepted),
            Ok(()),
            "{accepted}"
        );
    }
    assert_eq!(
        super::stryker_scripts_gate(
            "p/package.json",
            &manifest(serde_json::json!({
                "a": "stryker run",
                "b": "stryker run stryker.conf.json",
                "c": "stryker run stryker.config.json --mutate src/a.ts",
                "d": "pnpm exec stryker run stryker.config.json",
                "e": "vitest run",
            }))
        ),
        failed(
            "p/package.json runs Stryker otherwise than `stryker run stryker.config.json`: \
             4 problems, listed above\n\
             `a`: stryker run\n\
             `b`: stryker run stryker.conf.json\n\
             `c`: stryker run stryker.config.json --mutate src/a.ts\n\
             `d`: pnpm exec stryker run stryker.config.json"
        ),
        "another configuration, another argument, another wrapper"
    );
    assert!(
        super::stryker_scripts_gate("p/package.json", "{").is_err_and(|failure| failure
            .summary
            .starts_with("p/package.json: invalid manifest: ")),
        "unreadable"
    );
    assert_eq!(
        super::stryker_scripts_gate("p/package.json", r#"{"scripts":{"a":1}}"#),
        failed(
            "p/package.json runs Stryker otherwise than `stryker run stryker.config.json`: \
             `a` is not a command"
        ),
        "a script that is no string is refused, not skipped"
    );
}

#[test]
fn the_stryker_step_reads_every_manifest() {
    let step = [Step {
        name: "Stryker".to_owned(),
        action: Action::StrykerConfigGate,
    }];
    assert_eq!(
        run_steps(&step, &mut passing(), &mut Vec::new()),
        Ok(()),
        "the fake passes"
    );
    for path in package_jsons() {
        let mut fake = passing();
        let _script = fake.files.insert(
            path.clone(),
            r#"{"scripts":{"m":"stryker run"}}"#.to_owned(),
        );
        assert_eq!(
            run_steps(&step, &mut fake, &mut Vec::new()),
            failed(format!(
                "Stryker: {path} runs Stryker otherwise than `stryker run stryker.config.json`: \
                 `m`: stryker run"
            )),
            "{path}"
        );
    }
    for missing in [
        "apps/roadmap/stryker.config.json",
        "apps/roadmap/package.json",
    ] {
        let mut lacking = passing();
        let removed = lacking.files.remove(missing);
        assert!(removed.is_some(), "{missing} removed");
        assert_eq!(
            run_steps(&step, &mut lacking, &mut Vec::new()),
            failed(format!("Stryker: cannot read {missing}: missing")),
            "{missing} cannot be read"
        );
    }
}

/// Runs [`super::bypass_gate`].
fn bypass_gate_text(sources: &[(String, String)], reviewed: &str) -> Result<(), Failure> {
    super::bypass_gate(sources, reviewed)
}

#[test]
fn fails_on_a_missing_or_failing_typescript_report() {
    let mut no_summary = passing();
    let removed = no_summary
        .files
        .remove("apps/playground/coverage/coverage-summary.json");
    assert!(removed.is_some(), "summary removed");
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut no_summary, &mut Vec::new()),
        failed(
            "TypeScript coverage and mutation reports of apps/playground: cannot read apps/playground/coverage/coverage-summary.json: missing"
        ),
        "missing summary"
    );
    let mut survivor = passing();
    let replaced = survivor.files.insert(
        "apps/roadmap/reports/mutation/mutation.json".to_owned(),
        one_mutant("Survived", "M", 4),
    );
    assert!(replaced.is_some(), "report replaced");
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut survivor, &mut Vec::new()),
        failed(
            "TypeScript coverage and mutation reports of apps/roadmap: apps/roadmap: mutants not killed: \
             apps/roadmap/x.ts:4 M Survived"
        ),
        "survivor"
    );
    let mut busy = passing();
    busy.broken = Some("fresh apps/playground/coverage".to_owned());
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut busy, &mut Vec::new()),
        failed("Fresh TypeScript report directories: cannot empty apps/playground/coverage: busy"),
        "a report directory that cannot be emptied"
    );
}

#[test]
fn knows_exactly_the_packages_and_crates_of_the_repository() {
    let packages = package_jsons();
    let members: Vec<(String, String)> = super::RUST_CRATES
        .iter()
        .map(|krate| (format!("{}/Cargo.toml", krate.dir), MEMBER.to_owned()))
        .collect();
    let mut crates = members.clone();
    crates.push(("Cargo.toml".to_owned(), "[workspace]".to_owned()));
    crates.push((
        format!("{}x/Cargo.toml", super::FIXTURES),
        "[package]\nversion = \"0.0.0\"\n".to_owned(),
    ));
    assert_eq!(
        membership_gate(&packages, &crates, &[".gitignore".to_owned()]),
        Ok(()),
        "known, a fixture of xtask apart"
    );
    assert_eq!(
        membership_gate(&packages, &crates, &[".gitignore".to_owned(), "bindings/wasm/.gitignore".to_owned()]),
        found("the manifests differ from what xtask knows", &["bindings/wasm/.gitignore: build outputs go to dist/ or target/, not hidden in a source folder".to_owned()]),
        "a nested .gitignore"
    );

    let mut more = packages.clone();
    more.push("apps/editor/package.json".to_owned());
    let mut own = members.clone();
    own[0].1 =
        "[package]\nversion = \"9.0.0\"\nrepository.workspace = true\n\n[lints]\nworkspace = true\n"
            .to_owned();
    own.push(("crates/b/Cargo.toml".to_owned(), MEMBER.to_owned()));
    assert_eq!(
        membership_gate(&more, &own, &[]),
        found(
            "the manifests differ from what xtask knows",
            &[
                "apps/editor/package.json is not a package of PACKAGES: its checks and reports would not run"
                    .to_owned(),
                "crates/b/Cargo.toml is not a crate of RUST_CRATES: its coverage would not be measured"
                    .to_owned(),
                "crates/engine-core/Cargo.toml does not take the workspace version".to_owned(),
            ]
        ),
        "an unknown package and crate, a crate with its own version"
    );
    let fewer: Vec<String> = packages
        .iter()
        .filter(|package| *package != "apps/roadmap/package.json")
        .cloned()
        .collect();
    assert_eq!(
        membership_gate(&fewer, &members[1..], &[]),
        found(
            "the manifests differ from what xtask knows",
            &[
                "apps/roadmap/package.json belongs to PACKAGES but is not tracked".to_owned(),
                "crates/engine-core/Cargo.toml belongs to RUST_CRATES but is not tracked"
                    .to_owned(),
            ]
        ),
        "a package and a crate that are gone"
    );
}

#[test]
fn reports_manifests_it_cannot_list() {
    let mut broken = passing();
    broken.broken = Some("list *Cargo.toml".to_owned());
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut broken, &mut Vec::new()),
        failed(
            "Packages and crates known to the checks: cannot list the *Cargo.toml files: `git ls-files -- *Cargo.toml` exited with code 128: fatal: not a git repository"
        ),
        "named"
    );
    let mut unlisted = passing();
    unlisted.broken = Some("list *package.json".to_owned());
    assert!(
        run_steps(&check_steps(CHANGES_MODE), &mut unlisted, &mut Vec::new())
            .is_err_and(|error| error.summary.ends_with("cannot list the *package.json files: `git ls-files -- *package.json` exited with code 128: fatal: not a git repository")),
        "named"
    );
}

#[test]
fn allows_strict_licenses_for_what_ships_and_wider_ones_for_tooling() {
    assert_eq!(license_gate(SHIPPED, TOOLING), Ok(()), "allowed");
    assert_eq!(
        license_gate(
            r#"{"MPL-2.0":[{"name":"axe-core"}],"MIT":[{"name":"react"},{"name":"react-dom"}]}"#,
            r#"{"GPL-3.0":[{"name":"bad"}],"LGPL-3.0-or-later":[{"name":"other-lgpl"},{"name":"@img/sharp-libvips-x"}],"CC0-1.0":[{"name":"data"}],"Apache-2.0":[{"name":"typescript"}]}"#
        ),
        failed(
            "licenses not allowed: 3 problems, listed above\n\
             axe-core is shipped under MPL-2.0\n\
             bad is under GPL-3.0\n\
             other-lgpl is under LGPL-3.0-or-later"
        ),
        "a tooling license shipped, a license allowed nowhere, an exception for its own package only"
    );
    assert_eq!(
        license_gate(
            SHIPPED,
            r#"{"MIT-0":[{"name":"@img/sharp-libvips-x"}],"Apache-2.0":[{"name":"typescript"}]}"#
        ),
        failed("licenses not allowed: @img/sharp-libvips-x is under MIT-0"),
        "an exception for its own license only"
    );
    assert!(
        license_gate("x", TOOLING).is_err_and(|error| error
            .summary
            .starts_with("invalid shipped license report: ")),
        "unreadable"
    );
    assert_eq!(
        license_gate(SHIPPED, "[]"),
        failed("invalid tooling license report: not an object"),
        "not an object"
    );
    for (shipped, all, why) in [
        (
            r#"{"MIT":{"x":{"name":"react"}}}"#,
            TOOLING,
            "invalid shipped license report: MIT does not list packages",
        ),
        (
            r#"{"MIT":[{"version":"1"}]}"#,
            TOOLING,
            "invalid shipped license report: a package under MIT has no name",
        ),
        (
            r#"{"MIT":["react"]}"#,
            TOOLING,
            "invalid shipped license report: a package under MIT has no name",
        ),
        ("{}", TOOLING, "invalid shipped license report: no package"),
        (
            r#"{"MIT":[{"name":"a"}]}"#,
            TOOLING,
            "invalid shipped license report: react, react-dom missing",
        ),
        (
            SHIPPED,
            r#"{"MIT":[{"name":"react"}]}"#,
            "invalid tooling license report: typescript missing",
        ),
    ] {
        assert_eq!(license_gate(shipped, all), failed(why), "{why}");
    }
}

#[test]
fn reports_license_lists_it_cannot_get() {
    let mut fake = passing();
    let removed = fake.files.remove(&format!("capture {}", LICENSE_LINES[0]));
    assert!(removed.is_some(), "report removed");
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut fake, &mut Vec::new()),
        failed(format!(
            "JavaScript licenses: cannot list licenses: `{}` exited with code 1: failed",
            LICENSE_LINES[0]
        )),
        "named"
    );
}

#[test]
fn system_runner_captures_output_and_failures() {
    let mut runner = SystemRunner {
        root: fixture("coverage-locality"),
    };
    let echo = Cmd {
        program: "cargo",
        args: vec!["--version".to_owned()],
        dir: Dir::Root,
    };
    assert!(
        runner
            .capture(&echo)
            .is_ok_and(|out| out.starts_with("cargo ")),
        "stdout captured"
    );
    let failing = Cmd {
        program: "cargo",
        args: vec!["no-such-command".to_owned()],
        dir: Dir::Root,
    };
    assert_eq!(
        runner.capture(&failing).map_err(|failure| failure.to_string()),
        Err("`cargo no-such-command` exited with code 101: error: no such command: `no-such-command`".to_owned()),
        "a failing command, its code and its own reason"
    );
    let unknown = Cmd {
        program: "git",
        args: vec!["log".to_owned(), "deadbeefdeadbeef..HEAD".to_owned()],
        dir: Dir::Root,
    };
    assert!(
        runner.capture(&unknown).is_err_and(|failure| {
            let text = failure.to_string();
            text.starts_with("`git log deadbeefdeadbeef..HEAD` exited with code 128: fatal: ")
                && text.matches("git log").count() == 1
        }),
        "git's own reason, the command named once"
    );
    let missing = Cmd {
        program: "no-such-program-xyz",
        args: vec![],
        dir: Dir::Root,
    };
    assert!(
        runner
            .capture(&missing)
            .is_err_and(|failure| matches!(failure.failed, Failed::NotStarted(_))),
        "a program that cannot start"
    );
    let binary = Cmd {
        program: "printf",
        args: vec!["\\377".to_owned()],
        dir: Dir::Root,
    };
    assert_eq!(
        runner
            .capture(&binary)
            .map_err(|failure| failure.to_string()),
        Err("`printf \\377` wrote output that is not UTF-8 text".to_owned()),
        "output that is not text"
    );
}

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
    assert_eq!(UNSAFE_FILES.len(), 3, "three audited files");
    let mut fourth = sources.clone();
    fourth.push(("crates/engine-fpenv/src/other.rs".to_owned(), lifting()));
    assert_eq!(
        unsafe_gate(&fourth),
        found(
            "the lint against unsafe code",
            &[
                "crates/engine-fpenv/src/other.rs lifts the lint against unsafe code".to_owned(),
                format!(
                    "crates/engine-fpenv/src/other.rs does not hold `{}`",
                    forbid.trim()
                ),
            ]
        ),
        "a fourth file, even in the crate of an audited one"
    );
    let mut no_assembly = sources.clone();
    for (path, text) in &mut no_assembly {
        if super::ASSEMBLY_FILES.contains(&path.as_str()) {
            *text = format!(
                "{}// unsafe {{ core::arch::asm!(\"nop\") }} in a comment\nconst A: &str = \"unsafe {{ core::arch::asm!() }}\";\n",
                lifting()
            );
        }
    }
    assert_eq!(
        unsafe_gate(&no_assembly),
        failed(
            "the lint against unsafe code: crates/engine-fpenv/src/word.rs is listed in \
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
            if super::ASSEMBLY_FILES.contains(&path.as_str()) {
                *text = format!("{}{code}", audited(path));
            }
        }
        let offset = audited(super::ASSEMBLY_FILES[0]).lines().count();
        assert_eq!(
            unsafe_gate(&other),
            failed(format!(
                "the lint against unsafe code: {}:{}: `unsafe` not followed by \
                 `core::arch::asm!`: an assembly file holds no other unsafe code, which Miri \
                 would not check",
                super::ASSEMBLY_FILES[0],
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
        "//! The crate.\nmacro_rules! peek {{\n    ($p:expr) => {{\n        // SAFETY: read.\n        {word} {{ core::ptr::read($p) }}\n    }};\n}}\n"
    );
    let assembly = super::ASSEMBLY_FILES[0];
    let refused = |path: &str, line: usize| {
        failed(format!(
            "the lint against unsafe code: {path}:{line}: `unsafe` in the crate of the assembly \
             file {assembly}: its code could run under that file's lifting, where Miri never \
             checks it"
        ))
    };
    assert_eq!(
        with("crates/engine-fpenv/src/lib.rs", &peek),
        refused("crates/engine-fpenv/src/lib.rs", 5),
        "a macro of the library root, used in the assembly file"
    );
    assert_eq!(
        with(
            "crates/engine-fpenv/src/lib_tests.rs",
            &format!("{forbid}{word} fn f() {{}}\n")
        ),
        refused("crates/engine-fpenv/src/lib_tests.rs", 2),
        "an unsafe function of another module"
    );
    assert_eq!(
        with(
            "crates/engine-fpenv/src/lib.rs",
            &format!("//! Not {word} but the assembly.\nconst A: &str = \"{word}\";\n")
        ),
        Ok(()),
        "the word in prose and strings"
    );
    assert_eq!(
        with(
            "crates/engine-core/src/lib.rs",
            &format!("{forbid}fn f(p: *const u8) -> u8 {{\n    {word} {{ *p }}\n}}\n")
        ),
        Ok(()),
        "another crate, whose forbid the compiler holds"
    );
}

#[test]
fn reports_rust_files_it_cannot_list() {
    for (name, action) in [
        (
            "Unsafe code confined to the audited files",
            Action::UnsafeGate,
        ),
        ("Test code only in tests/", Action::TestCodeGate),
        ("Test data declared", Action::TestDataGate),
        (
            "Rust mutation testing of the changes",
            Action::RustMutantsOfChanges {
                jobs: NonZeroUsize::MIN,
            },
        ),
    ] {
        let mut fake = passing();
        fake.broken = Some("list *.rs".to_owned());
        // A change to test code, which widens the mutation testing to the crate, whose
        // sources are then listed.
        let _changes = fake.files.insert(
            format!("capture {}", CHANGES.0),
            one_line_change("crates/engine-core/src/lib_tests.rs"),
        );
        let step = Step {
            name: name.to_owned(),
            action,
        };
        assert_eq!(
            run_steps(&[step], &mut fake, &mut Vec::new()),
            failed(format!(
                "{name}: cannot list the Rust files: `git ls-files -- *.rs` exited with code 128: fatal: not a git repository"
            )),
            "named"
        );
    }
}

#[test]
fn checks_the_tools_and_the_platform_first() {
    let present = |node: &str| -> Vec<Result<String, CommandFailure>> {
        super::TOOLS
            .iter()
            .map(|tool| {
                Ok(if tool.program == "node" {
                    node.to_owned()
                } else {
                    tool_answer(tool.program, tool.args.first().copied())
                })
            })
            .collect()
    };
    assert_eq!(
        preflight_gate(
            "macos",
            &present("v22.23.4\n"),
            "22.23.4\n",
            TOOLCHAIN,
            &CLEAN
        ),
        Ok(()),
        "the release of .node-version"
    );
    for (node, pinned) in [
        ("v22.24.0", "22.23.4"),
        ("v22.23.3", "22.23.4"),
        ("v24.1.0", "22.23.4"),
        ("garbage", "22.23.4"),
        ("v22.23.4", "22.23"),
    ] {
        assert_eq!(
            preflight_gate("linux", &present(node), pinned, TOOLCHAIN, &CLEAN),
            found(
                "the checks cannot run here",
                &[format!(
                    "Node.js {node} is installed, the checks need {pinned}: install the Node.js \
                     release of .node-version (fnm, nvm and volta read it)"
                )]
            ),
            "{node} against {pinned}"
        );
    }
    let mut missing = present("v22.23.4");
    missing[5] = Err(CommandFailure {
        line: "cargo mutants --version".to_owned(),
        failed: Failed::Ended {
            exit: Exit::Code(101_i32),
            reason: Some("error: no such command: `mutants`".to_owned()),
        },
    });
    assert_eq!(
        preflight_gate("windows", &missing, "22.23.4", TOOLCHAIN, &CLEAN),
        found("the checks cannot run here", &[
            "the checks run on macOS and Linux: on Windows, run them in WSL2".to_owned(),
            "`cargo mutants --version` exited with code 101: error: no such command: `mutants`; to install: cargo install --locked cargo-mutants@27.1.0".to_owned(),
        ]),
        "platform, and a missing tool with its own reason and how to install it"
    );
    assert_eq!(
        super::NODE_FLOOR,
        (22, 18),
        "the engines of every package.json"
    );
}

#[test]
fn requires_the_rust_release_of_the_toolchain_file() {
    let answers = |rustc: &str| -> Vec<Result<String, CommandFailure>> {
        super::TOOLS
            .iter()
            .map(|tool| {
                Ok(if tool.program == "rustc" {
                    rustc.to_owned()
                } else {
                    tool_answer(tool.program, tool.args.first().copied())
                })
            })
            .collect()
    };
    assert_eq!(
        preflight_gate(
            "linux",
            &answers("rustc 1.99.1 (58cf8f9c3 2026-10-13)\n"),
            "22.23.4",
            TOOLCHAIN,
            &CLEAN
        ),
        Ok(()),
        "the channel of rust-toolchain.toml"
    );
    let hint = "the release of rust-toolchain.toml, which rustup installs: \
                remove what overrides it (`rustup override unset`)";
    for (rustc, toolchain) in [
        ("rustc 1.98.0 (aaaaaaaaa 2026-09-01)", TOOLCHAIN),
        ("rustc 1.99.1-beta.2 (bbbbbbbbb 2026-10-01)", TOOLCHAIN),
        ("rustc 1.100.0-nightly (ccccccccc 2026-11-01)", TOOLCHAIN),
        (
            "rustc 1.99.1 (58cf8f9c3 2026-10-13)",
            "[toolchain]\nchannel = \"1.99.10\"\n",
        ),
        ("rustc", TOOLCHAIN),
    ] {
        let channel = super::toolchain_channel(toolchain).unwrap_or_default();
        assert_eq!(
            preflight_gate("linux", &answers(rustc), "22.23.4", toolchain, &CLEAN),
            found(
                "the checks cannot run here",
                &[format!(
                    "{rustc} is installed, the checks need {channel} of rust-toolchain.toml: {hint}"
                )]
            ),
            "{rustc} against {toolchain:?}"
        );
    }
    for toolchain in [
        "[toolchain]\nprofile = \"minimal\"\n",
        "[toolchain]\nchannel = 1.99.1\n",
        "[toolchain]\nchannels = \"1.99.1\"\n",
        "[toolchain]\nchannel = \"\"\n",
    ] {
        assert_eq!(super::toolchain_channel(toolchain), None, "{toolchain:?}");
        assert_eq!(
            preflight_gate(
                "linux",
                &answers("rustc 1.99.1 (58cf8f9c3 2026-10-13)"),
                "22.23.4",
                toolchain,
                &CLEAN
            ),
            found(
                "the checks cannot run here",
                &["rust-toolchain.toml must pin one release: channel = \"X.Y.Z\"".to_owned()]
            ),
            "{toolchain:?}"
        );
    }
    assert_eq!(super::toolchain_channel(TOOLCHAIN), Some("1.99.1"), "read");
    assert_eq!(
        super::toolchain_channel("[toolchain]\n  channel   =   \"1.99.1\"  \n"),
        Some("1.99.1"),
        "spaces around"
    );
    let without = super::command("rustc", Path::new("."));
    assert!(
        without
            .get_envs()
            .any(|(name, value)| name == "RUSTUP_TOOLCHAIN" && value.is_none()),
        "no command inherits the toolchain rustup chose for xtask itself"
    );
}

#[test]
fn refuses_what_changes_cargo_from_outside_the_repository() {
    let answers: Vec<Result<String, CommandFailure>> = super::TOOLS
        .iter()
        .map(|tool| Ok(tool_answer(tool.program, tool.args.first().copied())))
        .collect();
    let refused = [
        "RUSTFLAGS",
        "RUSTDOCFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
        "CARGO_ENCODED_RUSTDOCFLAGS",
        "CARGO_BUILD_TARGET",
        "CARGO_BUILD_RUSTFLAGS",
        "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS",
        "CARGO_PROFILE_RELEASE_OPT_LEVEL",
        "CARGO_UNSTABLE_BUILD_STD",
        "RUSTC",
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
        "RUSTDOC",
        "CLIPPY_CONF_DIR",
    ];
    let accepted = [
        "CARGO_TARGET_DIR",
        "CARGO_HOME",
        "CARGO_MANIFEST_DIR",
        "RUSTUP_TOOLCHAIN",
        "RUSTUP_HOME",
        "RUSTC_BOOTSTRAP_X",
        "XRUSTFLAGS",
        "rustflags",
        "CARGO_TARGET",
        "PATH",
    ];
    for name in refused {
        assert!(super::refused_variable(name), "{name}");
    }
    for name in accepted {
        assert!(!super::refused_variable(name), "{name}");
    }
    let hostile = super::Environment {
        variables: refused
            .iter()
            .chain(&accepted)
            .map(|name| (*name).to_owned())
            .collect(),
        cargo_configs: vec!["/home/x/.cargo/config.toml".to_owned()],
    };
    let mut problems: Vec<String> = refused
        .iter()
        .map(|name| {
            format!("{name} is set, which changes what cargo builds or how it lints: unset it")
        })
        .collect();
    problems.push(
        "/home/x/.cargo/config.toml configures every cargo command here: move it away (the \
         configuration of the repository is .cargo/config.toml)"
            .to_owned(),
    );
    assert_eq!(
        preflight_gate("linux", &answers, "22.23.4", TOOLCHAIN, &hostile),
        found("the checks cannot run here", &problems),
        "each by name"
    );
    let mut fake = passing();
    fake.environment.variables = vec!["RUSTFLAGS".to_owned()];
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut fake, &mut Vec::new()),
        failed(
            "Tools and platform: the checks cannot run here: RUSTFLAGS is set, which changes \
             what cargo builds or how it lints: unset it"
        ),
        "the check reads the environment of its runner"
    );
}

#[test]
fn lists_the_cargo_configurations_outside_the_repository() {
    let root = Path::new("/a/b/repo");
    let configs = |cargo_home: Option<&str>, home: Option<&str>| -> Vec<String> {
        super::cargo_config_candidates(
            root,
            cargo_home.map(OsString::from),
            home.map(OsString::from),
        )
        .iter()
        .map(|path| path.display().to_string())
        .collect()
    };
    let ancestors = [
        "/.cargo/config",
        "/.cargo/config.toml",
        "/a/.cargo/config",
        "/a/.cargo/config.toml",
        "/a/b/.cargo/config",
        "/a/b/.cargo/config.toml",
        "/a/b/repo/.cargo/config",
    ];
    let with = |home: &[&str]| -> Vec<String> {
        let mut all: Vec<String> = home
            .iter()
            .chain(&ancestors)
            .map(|path| (*path).to_owned())
            .collect();
        all.sort();
        all
    };
    assert_eq!(
        configs(Some("/c"), Some("/h")),
        with(&["/c/config", "/c/config.toml"]),
        "CARGO_HOME, and every directory from the root up, the repository's own file apart"
    );
    assert_eq!(
        configs(None, Some("/h")),
        with(&["/h/.cargo/config", "/h/.cargo/config.toml"]),
        "else the .cargo of the home directory"
    );
    assert_eq!(configs(None, None), with(&[]), "no home at all");
    assert_eq!(
        configs(Some("/a/.cargo"), None),
        with(&[]),
        "each file once"
    );

    let dir = scratch("cargo-configs");
    let repo = dir.join("repo");
    std::fs::create_dir_all(repo.join(".cargo")).expect("repository");
    std::fs::create_dir_all(dir.join(".cargo")).expect("above");
    for path in [
        dir.join(".cargo/config.toml"),
        repo.join(".cargo/config"),
        repo.join(".cargo/config.toml"),
    ] {
        std::fs::write(path, "").expect("configuration");
    }
    std::fs::create_dir_all(repo.join("x")).expect("a folder to walk out of");
    let environment = SystemRunner {
        root: repo.join("x/.."),
    }
    .environment();
    let missing = SystemRunner {
        root: dir.join("missing"),
    }
    .environment();
    let canonical = std::fs::canonicalize(&dir).expect("canonical");
    std::fs::remove_dir_all(&dir).expect("cleanup");
    let listed = |path: &str| {
        environment
            .cargo_configs
            .contains(&canonical.join(path).display().to_string())
    };
    assert!(listed(".cargo/config.toml"), "above: {environment:?}");
    assert!(
        listed("repo/.cargo/config"),
        "the other name: {environment:?}"
    );
    assert!(
        !listed("repo/.cargo/config.toml"),
        "its own: {environment:?}"
    );
    assert!(
        environment
            .variables
            .contains(&"CARGO_MANIFEST_DIR".to_owned()),
        "the names of the variables: {environment:?}"
    );
    assert!(
        missing
            .cargo_configs
            .contains(&dir.join(".cargo/config.toml").display().to_string()),
        "a root that cannot be resolved is walked up as it is: {missing:?}"
    );
}

#[test]
fn lists_the_checks_in_order() {
    let mut out = Vec::new();
    assert_eq!(
        run_cli(
            &["list".to_owned()],
            &mut passing(),
            &mut out,
            &mut Vec::new()
        ),
        0,
        "listed"
    );
    let text = String::from_utf8(out).expect("UTF-8");
    assert!(
        text.starts_with(
            " 1. Tools and platform\n 2. Working tree recorded\n 3. JavaScript dependencies\n"
        ),
        "{text}"
    );
    assert_eq!(
        text.lines().count(),
        check_steps(CHANGES_MODE).len(),
        "every check"
    );
    assert!(
        text.contains("Rust mutation testing of the changes\n"),
        "of the default check: {text}"
    );
    let mut full = Vec::new();
    let mut fake = passing();
    assert_eq!(
        run_cli(
            &["list".to_owned(), "--full".to_owned()],
            &mut fake,
            &mut full,
            &mut Vec::new()
        ),
        0,
        "listed"
    );
    let full = String::from_utf8(full).expect("UTF-8");
    assert!(
        full.contains(". Rust mutation testing\n") && !full.contains("of the changes"),
        "of the full check: {full}"
    );
    assert!(
        full.starts_with(
            " 1. Tools and platform\n 2. Working tree recorded\n 3. Rust formatting\n"
        ),
        "in the order of the full check: {full}"
    );
    assert!(fake.ran.is_empty(), "listing runs nothing");
    assert_eq!(
        run_cli(
            &["list".to_owned()],
            &mut passing(),
            &mut Broken,
            &mut Vec::new()
        ),
        1,
        "broken stdout"
    );
}

#[test]
fn stops_at_a_missing_tool() {
    let mut fake = passing();
    fake.broken = Some("cargo mutants --version".to_owned());
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut fake, &mut Vec::new()),
        failed(
            "Tools and platform: the checks cannot run here: cannot run `cargo mutants --version`: not found; to install: cargo install --locked cargo-mutants@27.1.0"
        ),
        "named, with how to install it"
    );
}

#[test]
fn requires_every_part_to_be_described() {
    let readmes: Vec<(String, String)> = READMES
        .iter()
        .map(|&(path, text)| (path.to_owned(), text.to_owned()))
        .collect();
    let mut parts: Vec<String> = PACKAGES
        .iter()
        .map(|package| package.dir.to_owned())
        .collect();
    assert_eq!(super::readme_gate(&parts, &readmes), Ok(()), "described");
    parts.push("crates/engine-core".to_owned());
    parts.push("lonely".to_owned());
    let without: Vec<(String, String)> = readmes
        .iter()
        .filter(|(path, _)| {
            !["tooling/oracle/README.md", "crates/README.md", "README.md"].contains(&path.as_str())
        })
        .cloned()
        .chain([
            (
                "crates/README.md".to_owned(),
                "| `engine-hash` |".to_owned(),
            ),
            (
                "README.md".to_owned(),
                "| `apps/` | | `bindings/` | | `tooling/` |".to_owned(),
            ),
        ])
        .collect();
    assert_eq!(
        super::readme_gate(&parts, &without),
        failed(
            "parts not described: 4 problems, listed above\n\
             tooling/oracle has no README.md\n\
             crates/README.md does not name engine-core\n\
             lonely is not inside a folder of parts (apps/, crates/ …)\n\
             README.md does not name crates/"
        ),
        "a package without a README, a crate the table forgets, a part outside any folder"
    );
    let unnamed: Vec<(String, String)> = readmes
        .iter()
        .filter(|(path, _)| path != "apps/README.md")
        .cloned()
        .collect();
    assert_eq!(
        super::readme_gate(&PACKAGES.map(|package| package.dir.to_owned()), &unnamed),
        found("parts not described", &["apps has no README.md".to_owned()]),
        "a folder without its README, said once for its two parts"
    );
}

#[test]
fn names_every_check_alike() {
    for step in check_steps(CHANGES_MODE) {
        assert!(
            step.name
                .starts_with(|first: char| first.is_ascii_uppercase()),
            "{} starts with a capital letter",
            step.name
        );
        assert!(
            !step.name.ends_with('.'),
            "{} has no final period",
            step.name
        );
    }
}

/// Whether a check step is a gate whose verdict names its step: the summary of a refusal
/// opens with the step's name, its first letter in lowercase, or with one clause of it
/// (`; `) for a step running two gates.
fn opens_with_its_step(step: &str, summary: &str) -> bool {
    let mut letters = step.chars();
    let lowered: String = letters
        .next()
        .map(|first| first.to_lowercase().chain(letters).collect())
        .unwrap_or_default();
    lowered
        .split("; ")
        .any(|clause| summary.starts_with(&format!("{clause}: ")))
}

/// Whether the verdict of a step doing `action` names its step ([`opens_with_its_step`]):
/// every action is classed here, with no wildcard, so that a new one does not compile
/// before it is. `false` for a step that is no gate, or for a gate whose verdict names
/// the register or the files it reads, written before the rule.
const fn names_its_step(action: &Action) -> bool {
    match action {
        Action::TestCodeGate
        | Action::TestDataGate
        | Action::UntestedTargetsGate
        | Action::PinGate
        | Action::LibraryGate
        | Action::CitationGate => true,
        Action::BypassGate
        | Action::CargoConfigGate
        | Action::DependencyGate
        | Action::EquivalentGate
        | Action::FloatMethodGate
        | Action::HistoryGate
        | Action::LicenseGate
        | Action::MathLayerGate
        | Action::MembershipGate
        | Action::PartsGate
        | Action::ReadmeGate
        | Action::StrykerConfigGate
        | Action::SupplyChainGate
        | Action::TypeScriptMutationGate { .. }
        | Action::TypeScriptReportsGate(_)
        | Action::TypeScriptScopeGate(_)
        | Action::UnsafeGate
        | Action::UnviableGate(_)
        | Action::UnviableReviewsCurrent
        | Action::ShardsUnviableGate(_)
        | Action::VersionGate
        | Action::VocabularyGate
        | Action::WorkflowGate
        | Action::Run(_)
        | Action::Coverage(_)
        | Action::CoverageLocality
        | Action::RustMutantsOfChanges { .. }
        | Action::Bless(_)
        | Action::MutationProofRecorded
        | Action::FreshDirs(_)
        | Action::FusedBuild
        | Action::TreeRecorded
        | Action::TreeUnchanged
        | Action::Preflight => false,
    }
}

#[test]
fn names_each_gate_verdict_after_its_step() {
    let refused = |result: Result<(), Failure>| result.expect_err("refused").summary;
    let rust = |text: &str| vec![("crates/engine-core/src/a.rs".to_owned(), text.to_owned())];
    let (real, _) = fixture_metadata();
    let shared = r#"{"workspace_root":"/r","packages":[{"name":"x","dependencies":[{"name":"engine-core","kind":"dev","path":"/r/crates/engine-core"}]}]}"#;
    let named = [
        (
            Action::TestCodeGate,
            vec![refused(super::test_code_gate(&rust(
                "#[cfg(test)]\nmod tests {}\n",
            )))],
        ),
        (
            Action::TestDataGate,
            vec![refused(super::test_data_gate(&rust(
                "const A: &str = include_str!(\"../../../spec/x.json\");\n",
            )))],
        ),
        (
            Action::UntestedTargetsGate,
            vec![refused(super::untested_targets_gate(
                &targets_metadata(&[("lib", "crates/x/src/lib.rs", true, false)]),
                &[],
            ))],
        ),
        (Action::PinGate, vec![refused(super::pin_gate(&real, ""))]),
        (
            Action::LibraryGate,
            vec![
                refused(super::library_gate(&real)),
                refused(super::shared_tests_gate(shared)),
            ],
        ),
        (Action::CitationGate, vec![refused(cite("\u{23}12 step 4"))]),
    ];
    let steps = check_steps(Mode::Full { jobs: FAKE_JOBS });
    let classed: Vec<&Action> = steps
        .iter()
        .map(|step| &step.action)
        .filter(|action| names_its_step(action))
        .collect();
    assert_eq!(
        classed,
        named.iter().map(|(action, _)| action).collect::<Vec<_>>(),
        "every gate whose verdict names its step is refused below, in the order of the steps"
    );
    for (action, summaries) in named {
        let step = steps
            .iter()
            .find(|step| step.action == action)
            .unwrap_or_else(|| panic!("{action:?} is a step"));
        for summary in summaries {
            assert!(
                opens_with_its_step(&step.name, &summary),
                "{}: {summary}",
                step.name
            );
        }
    }
    assert!(
        opens_with_its_step("A b; c d", "c d: x") && !opens_with_its_step("A b", "a b c: x"),
        "a clause, then a colon"
    );
}

#[test]
fn tells_changes_of_behavior() {
    for (message, expected) in [
        ("feat: add", true),
        ("fix(roadmap): repair", true),
        ("perf(engine-core): faster", true),
        ("docs!: drop a page", true),
        ("refactor(roadmap)!: rename", true),
        (
            "refactor: rename\n\nBREAKING CHANGE: the command is renamed",
            true,
        ),
        ("docs: explain", false),
        ("test(xtask): cover", false),
        ("build: bump", false),
        ("chore: tidy\n\nnot a BREAKING CHANGE: inline", false),
        ("feature: not a type", false),
        ("", false),
    ] {
        assert_eq!(super::changes_behavior(message), expected, "{message:?}");
    }
}

#[test]
fn checks_every_message_and_the_changelog_of_every_change() {
    assert_eq!(
        super::history_gate(HISTORY, HISTORY_FILES, HISTORY_SCOPED),
        Ok(()),
        "a clean history"
    );
    let messages = "\u{1e}cccccccccc\u{1f}fix: repair\n\u{1e}bbbbbbbbbb\u{1f}Update stuff\n\u{1e}aaaaaaaaaa\u{1f}feat: before the rule\n";
    let files = "\u{1e}cccccccccc\n\nsrc/a.rs\n\u{1e}bbbbbbbbbb\n\nsrc/b.rs\n";
    assert_eq!(
        super::history_gate(messages, files, ""),
        failed(
            "the history breaks the commit rules: 2 problems, listed above\n\
             commit ccccccc changes behavior but not CHANGELOG.md\n\
             commit bbbbbbb: invalid commit message: missing \"<type>: \" prefix"
        ),
        "a change without its entry, a malformed message; the commit before the rule is not held to it"
    );
    assert_eq!(super::history_gate("", "", ""), Ok(()), "no commit");
    let named = "\u{1e}cccccccccc\u{1f}docs(wasm): x\n\u{1e}bbbbbbbbbb\u{1f}docs(wasm): y\n";
    assert_eq!(
        super::history_gate(named, "", "cccccccccc\n"),
        Err(Failure {
            summary: format!(
                "the history breaks the commit rules: commit ccccccc: invalid commit message: \
                 unknown scope \"wasm\": name a crate, a package or a scope of xtask's \
                 COMMIT_SCOPE_EXTRAS ({})",
                super::commit_scopes().join(", ")
            ),
            details: vec![],
        }),
        "a scope held to the registries only after the scopes' boundary"
    );
    assert_eq!(
        super::history_gate("\u{1e}eeeeeeeeee\u{1f}# fix: hidden\nfix: shown\n", "", ""),
        found(
            "the history breaks the commit rules",
            &["commit eeeeeee: invalid commit message: unknown type \"# fix\"".to_owned()]
        ),
        "a recorded message is read as it is: a first line starting with # is its header"
    );
    assert_eq!(
        super::history_gate("\u{1e}dddddddddd", "\u{1e}dddddddddd\n", ""),
        found(
            "the history breaks the commit rules",
            &["commit ddddddd: invalid commit message: missing \"<type>: \" prefix".to_owned()]
        ),
        "a record without a message"
    );
    assert_eq!(super::CHANGELOG_SINCE.len(), 40, "a full commit hash");
}

#[test]
fn reads_the_history_or_says_why_it_cannot() {
    let mut fake = passing();
    let _removed = fake.files.remove(&format!("capture {}", HISTORY_LINES[0]));
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut fake, &mut Vec::new()),
        failed(
            "Commit messages, and a changelog entry for each change of behavior: cannot read the history: `git log --format=%x1e%H%x1f%B HEAD` exited with code 1: failed"
        ),
        "a shallow clone, or no git"
    );
}

#[test]
fn pushes_only_what_was_checked() {
    let head = "1111111111111111111111111111111111111111\n";
    let pushed = |commits: &[&str]| {
        commits
            .iter()
            .map(|&commit| commit.to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        super::pre_push_gate(
            head,
            "",
            "H a.rs\nH b.rs\n",
            &pushed(&["1111111111111111111111111111111111111111"])
        ),
        Ok(()),
        "the checked-out commit"
    );
    for (hidden, path) in [("h a.rs\n", "a.rs"), ("H b.rs\nS c d.rs\n", "c d.rs")] {
        assert_eq!(
            super::pre_push_gate(head, "", hidden, &[]),
            failed(format!(
                "the push cannot be checked: the working tree may differ from what is \
                 committed, which the check would include: `{path}` is hidden from git status \
                 (assume-unchanged or skip-worktree)"
            )),
            "{hidden}"
        );
    }
    assert_eq!(
        super::pre_push_gate(
            head,
            "",
            "",
            &pushed(&["0000000000000000000000000000000000000000"])
        ),
        Ok(()),
        "a deleted branch"
    );
    assert_eq!(
        super::pre_push_gate(head, "\n", "", &[]),
        Ok(()),
        "nothing pushed, nothing changed"
    );
    assert_eq!(
        super::pre_push_gate(
            head,
            " M src/a.rs\n?? notes.txt\n",
            "h hidden.rs\n",
            &pushed(&["2222222222222222222222222222222222222222"])
        ),
        failed(
            "the push cannot be checked: 2 problems, listed above\n\
             the working tree has changes not committed, which the check would include: M src/a.rs, ?? notes.txt\n\
             pushing 2222222222222222222222222222222222222222, but 1111111111111111111111111111111111111111 \
             is checked out: check out the pushed commit, then push again"
        ),
        "a dirty tree, another commit"
    );
}

#[test]
fn runs_the_full_check_before_pushing_a_tag() {
    let head = "1111111111111111111111111111111111111111";
    let mut tagged = passing();
    for (command, answer) in [
        ("git rev-parse HEAD".to_owned(), format!("{head}\n")),
        (
            "git status --porcelain --untracked-files=all --ignore-submodules=none".to_owned(),
            String::new(),
        ),
        (
            format!("git rev-parse --verify {head}^{{commit}}"),
            format!("{head}\n"),
        ),
    ] {
        let _answer = tagged.files.insert(format!("capture {command}"), answer);
    }
    let release = [
        "pre-push".to_owned(),
        head.to_owned(),
        "refs/heads/main".to_owned(),
        head.to_owned(),
        "refs/tags/v1.0.0".to_owned(),
    ];
    assert_eq!(
        run_cli(&release, &mut tagged, &mut Vec::new(), &mut Vec::new()),
        0,
        "a release"
    );
    assert!(
        tagged.ran.contains(
            &"cargo mutants --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --workspace --jobs 3 --timeout-multiplier 15 --minimum-test-timeout 60 -- -- --include-ignored".to_owned()
        ) && !tagged.ran.contains(&format!("capture {}", MERGE_BASE.0)),
        "a pushed tag runs every mutant: {:?}",
        tagged.ran
    );
}

#[test]
fn checks_a_push_then_runs_every_check() {
    let head = "1111111111111111111111111111111111111111";
    let mut fake = passing();
    let first = fake
        .files
        .insert("capture git rev-parse HEAD".to_owned(), format!("{head}\n"));
    assert!(first.is_none(), "fresh fake");
    assert_eq!(
        fake.files.get(&format!("capture {}", PUSH_STATUS.0)),
        Some(&String::new()),
        "a clean tree"
    );
    let peeled = fake.files.insert(
        format!("capture git rev-parse --verify {head}^{{commit}}"),
        format!("{head}\n"),
    );
    assert!(peeled.is_none(), "fresh fake");
    let mut out = Vec::new();
    let args = [
        "pre-push".to_owned(),
        head.to_owned(),
        "refs/heads/work".to_owned(),
    ];
    assert_eq!(
        run_cli(&args, &mut fake, &mut out, &mut Vec::new()),
        0,
        "clean push"
    );
    assert!(
        String::from_utf8(out)
            .expect("UTF-8")
            .ends_with("✔ all 89 checks passed in 2 min 13 s\n"),
        "then every check"
    );
    assert!(
        fake.ran.contains(&format!("capture {}", MERGE_BASE.0))
            && fake.ran.contains(
                &"cargo mutants --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --workspace --in-diff target/xtask/changes.diff --jobs 3 --timeout-multiplier 15 --minimum-test-timeout 60 -- -- --include-ignored"
                    .to_owned()
            ),
        "the default check: the hook mutates the changes"
    );
    let mut dirty = passing();
    let third = dirty
        .files
        .insert("capture git rev-parse HEAD".to_owned(), format!("{head}\n"));
    let fourth = dirty.files.insert(
        "capture git status --porcelain --untracked-files=all --ignore-submodules=none".to_owned(),
        " M a.rs\n".to_owned(),
    );
    let fifth = dirty.files.insert(
        format!("capture git rev-parse --verify {head}^{{commit}}"),
        format!("{head}\n"),
    );
    assert!(
        third.is_none() && fourth.is_some() && fifth.is_none(),
        "fresh fake, but its clean status"
    );
    let mut err = Vec::new();
    assert_eq!(
        run_cli(&args, &mut dirty, &mut Vec::new(), &mut err),
        1,
        "dirty tree"
    );
    assert!(
        dirty.ran.iter().all(|line| !line.contains("cargo fmt")),
        "no check runs"
    );
    assert_eq!(
        String::from_utf8(err).expect("UTF-8"),
        "error: the push cannot be checked: the working tree has changes not committed, which the check would include: M a.rs\n",
        "says what to commit"
    );
    let mut blind = passing();
    let mut blind_err = Vec::new();
    assert_eq!(
        run_cli(&args, &mut blind, &mut Vec::new(), &mut blind_err),
        1,
        "no git"
    );
    assert_eq!(
        String::from_utf8(blind_err).expect("UTF-8"),
        "error: cannot read the work tree: `git rev-parse HEAD` exited with code 1: failed\n",
        "says what it could not read"
    );
}

#[test]
fn refuses_cargo_tools_at_other_versions() {
    let answers: Vec<Result<String, CommandFailure>> = super::TOOLS
        .iter()
        .map(|tool| Ok(tool_answer(tool.program, tool.args.first().copied())))
        .collect();
    assert_eq!(
        preflight_gate("linux", &answers, "22.23.4", TOOLCHAIN, &CLEAN),
        Ok(()),
        "the pinned versions"
    );
    let mut older = answers;
    older[5] = Ok("cargo-mutants 26.0.0\n".to_owned());
    older[6] = Ok("cargo-deny".to_owned());
    assert_eq!(
        preflight_gate("linux", &older, "22.23.4", TOOLCHAIN, &CLEAN),
        found("the checks cannot run here", &[
            "cargo-mutants 26.0.0 is installed, the checks need cargo-mutants 27.1.0: \
             cargo install --locked cargo-mutants@27.1.0".to_owned(),
            "cargo-deny is installed, the checks need cargo-deny 0.20.2: cargo install --locked cargo-deny@0.20.2".to_owned(),
        ]),
        "another version, no version"
    );
}

#[test]
fn requires_a_workflow_that_pins_everything() {
    assert_eq!(
        super::workflow_gate(&[super::WORKFLOW.to_owned()], PINNED_WORKFLOW, "22.23.4\n"),
        Ok(()),
        "pinned"
    );
    let loose = PINNED_WORKFLOW
        .replace(
            "checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1",
            "checkout@v7",
        )
        .replace(
            "setup-node@949feb2413d6458794dcd2491c4babbbce0c15c1",
            "setup-node@949feb2413d6458794dcd2491c4babbbce0c15cz",
        )
        .replace(
            "taiki-e/install-action@cfdb7ed4ccc7c7dce37759eae89dc5674b469b39",
            "taiki-e/install-action",
        )
        .replace("cargo-mutants@27.1.0", "cargo-mutants")
        .replace("fetch-depth: 0", "fetch-depth: 1")
        .replace("node-version-file: .node-version", "node-version: 22")
        .replace("run: cargo xtask check --full", "run: cargo xtask check");
    assert_eq!(
        super::workflow_gate(&[super::WORKFLOW.to_owned()], &loose, "22"),
        failed(
            ".github/workflows/quality.yml can change CI without a commit: 9 problems, listed above\n\
             actions/checkout@v7 is not pinned to a full commit hash\n\
             taiki-e/install-action is not pinned to a full commit hash\n\
             actions/setup-node@949feb2413d6458794dcd2491c4babbbce0c15cz # v7.1.0 is not pinned to a full commit hash\n\
             .github/workflows/quality.yml lacks `tool: cargo-llvm-cov@0.9.1,cargo-mutants@27.1.0,cargo-deny@0.20.2`\n\
             .github/workflows/quality.yml lacks `node-version-file: .node-version`\n\
             .github/workflows/quality.yml lacks `fetch-depth: 0`\n\
             .github/workflows/quality.yml lacks `run: cargo xtask check --full --part ${{ matrix.part }}`\n\
             .github/workflows/quality.yml lacks `run: cargo xtask check --full --part final`\n\
             .node-version must pin one Node.js release, 22.18 or later (X.Y.Z): found \"22\""
        ),
        "every kind of loose pin"
    );
    let unkept = PINNED_WORKFLOW
        .replacen("runs-on: ubuntu-24.04", "runs-on: ubuntu-latest", 1)
        .replace("cache-on-failure: true", "cache-on-failure: false")
        .replace("            tooling/*/coverage\n", "")
        .replace("            target/coverage\n", "");
    assert_eq!(
        super::workflow_gate(&[super::WORKFLOW.to_owned()], &unkept, "22.23.4"),
        failed(
            ".github/workflows/quality.yml can change CI without a commit: 4 problems, listed above\n\
             `runs-on: ubuntu-latest` moves to a new OS release without a commit: name the release\n\
             .github/workflows/quality.yml lacks `cache-on-failure: true`\n\
             .github/workflows/quality.yml lacks `target/coverage`\n\
             .github/workflows/quality.yml lacks `tooling/*/coverage`"
        ),
        "a moving runner, a cache lost on failure, reports not uploaded"
    );
    assert_eq!(
        super::failure_uploads(),
        vec![
            "mutants.out",
            "target/coverage",
            "bindings/wasm/dist/engine.wasm",
            "tooling/*/coverage",
            "tooling/*/reports",
            "bindings/*/coverage",
            "bindings/*/reports",
            "apps/*/coverage",
            "apps/*/reports",
        ],
        "every report a failed run leaves, derived from where xtask reads them"
    );
    for (node, accepted) in [
        ("22.18.0", true),
        ("24.1.0\n", true),
        ("22.17.9", false),
        ("21.99.0", false),
        // As `node --version` writes it, which setup-node and the version managers read too.
        ("v22.23.4", true),
        ("22.x.1", false),
        ("22.23.4.1", false),
    ] {
        assert_eq!(
            super::workflow_gate(&[super::WORKFLOW.to_owned()], PINNED_WORKFLOW, node).is_ok(),
            accepted,
            "{node:?}"
        );
    }
}

#[test]
fn requires_an_audio_sink_for_firefox() {
    let gate =
        |workflow: &str| super::workflow_gate(&[super::WORKFLOW.to_owned()], workflow, "22.23.4");
    let lacks = failed(
        ".github/workflows/quality.yml can change CI without a commit: \
         .github/workflows/quality.yml lacks the audio sink, these lines in this order: \
         `sudo apt-get update` `sudo apt-get install --yes --no-install-recommends \
         pulseaudio=1:16.1+dfsg1-2ubuntu10.1` `pulseaudio --start --exit-idle-time=-1` \
         `pactl load-module module-null-sink sink_name=null` `pactl set-default-sink null` \
         `pactl get-default-sink | grep --quiet --line-regexp null || { echo \"::error::no audio \
         sink: Firefox cannot play live (the runner has no sound card)\"; exit 1; }`",
    );
    let guard = "          pactl get-default-sink | grep --quiet --line-regexp null || { echo \"::error::no audio sink: Firefox cannot play live (the runner has no sound card)\"; exit 1; }\n";
    for (case, workflow) in [
        (
            "no audio server",
            PINNED_WORKFLOW.replace("          pulseaudio --start --exit-idle-time=-1\n", ""),
        ),
        (
            "a package at whatever version the archive holds",
            PINNED_WORKFLOW.replace("pulseaudio=1:16.1+dfsg1-2ubuntu10.1", "pulseaudio"),
        ),
        (
            "with its recommends",
            PINNED_WORKFLOW.replace(" --no-install-recommends", ""),
        ),
        ("no guard", PINNED_WORKFLOW.replace(guard, "")),
        (
            "the sink chosen after the guard",
            PINNED_WORKFLOW
                .replace("          pactl set-default-sink null\n", "")
                .replace(
                    guard,
                    &format!("{guard}          pactl set-default-sink null\n"),
                ),
        ),
    ] {
        assert_eq!(gate(&workflow), lacks, "{case}");
    }
    assert_eq!(
        gate(&PINNED_WORKFLOW.replace("          sudo", "      sudo")),
        Ok(()),
        "lines read trimmed, as the other required lines"
    );
}

#[test]
fn explains_why_a_command_failed() {
    let reason = |stderr: &str| super::error_reason(stderr.as_bytes());
    assert_eq!(
        reason(
            "error: no such command: `mutants`\n\nhelp: view all installed commands with `cargo --list`\n"
        ),
        Some("error: no such command: `mutants`".to_owned()),
        "cargo: the error, not the help after it"
    );
    assert_eq!(
        reason(
            "info: syncing channel updates for 'nightly-2026-11-01'\nerror: toolchain 'nightly-2026-11-01' is not installed\n"
        ),
        Some("error: toolchain 'nightly-2026-11-01' is not installed".to_owned()),
        "rustup: the error, not the information before it"
    );
    assert_eq!(
        reason(
            "\n  fatal: ambiguous argument 'x..HEAD': unknown revision\nUse '--' to separate paths\n"
        ),
        Some("fatal: ambiguous argument 'x..HEAD': unknown revision".to_owned()),
        "git: the fatal line, trimmed"
    );
    assert_eq!(
        reason("\nsomething went wrong\nthen more\n"),
        Some("something went wrong".to_owned()),
        "otherwise the first line said"
    );
    assert_eq!(reason(" \n\n"), None, "nothing said");
    let failure = |exit: Exit, said: Option<&str>| {
        CommandFailure {
            line: "x y".to_owned(),
            failed: Failed::Ended {
                exit,
                reason: said.map(str::to_owned),
            },
        }
        .to_string()
    };
    assert_eq!(
        failure(Exit::Signal(11_i32), Some("core dumped")),
        "`x y` was stopped by signal 11 (SIGSEGV): core dumped",
        "a named signal and a reason"
    );
    assert_eq!(
        failure(Exit::Signal(31_i32), None),
        "`x y` was stopped by signal 31",
        "a signal without a portable name"
    );
    assert_eq!(
        failure(Exit::Code(2_i32), None),
        "`x y` exited with code 2",
        "a code alone"
    );
}

/// A fake whose work tree is clean at `head`, and where each of `objects` peels to its
/// commit.
fn pushing(head: &str, objects: &[(&str, &str)]) -> Fake {
    let mut fake = passing();
    for (command, answer) in
        core::iter::once(("git rev-parse HEAD".to_owned(), format!("{head}\n"))).chain(
            objects.iter().map(|(object, commit)| {
                (
                    format!("git rev-parse --verify {object}^{{commit}}"),
                    format!("{commit}\n"),
                )
            }),
        )
    {
        let fresh = fake.files.insert(format!("capture {command}"), answer);
        assert!(fresh.is_none(), "fresh fake");
    }
    fake
}

#[test]
fn compares_the_commits_pushed_not_their_tags() {
    let head = "1111111111111111111111111111111111111111";
    let tag = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let other = "2222222222222222222222222222222222222222";
    let deleted = "0000000000000000000000000000000000000000";
    let push = |fake: &mut Fake, objects: &[&str]| {
        let pushed: Vec<String> = objects.iter().map(|&object| object.to_owned()).collect();
        super::pre_push(fake, &pushed)
    };
    let mut on_head = pushing(head, &[(tag, head)]);
    assert_eq!(
        push(&mut on_head, &[tag, deleted]),
        Ok(()),
        "an annotated tag of the commit checked out, a deleted branch"
    );
    assert!(
        !on_head.ran.iter().any(|line| line.contains(deleted)),
        "a deleted branch is not resolved"
    );
    assert_eq!(
        push(&mut pushing(head, &[(tag, other)]), &[tag]),
        failed(format!(
            "the push cannot be checked: pushing {other}, but {head} is checked out: check out the pushed commit, then push again"
        )),
        "a tag of another commit, named by its commit"
    );
    assert_eq!(
        push(&mut pushing(head, &[]), &[tag]),
        failed(format!(
            "cannot resolve the pushed {tag}: `git rev-parse --verify {tag}^{{commit}}` exited with code 1: failed"
        )),
        "an object that is not a commit"
    );
}

#[test]
fn pushes_an_annotated_tag_of_the_commit_checked_out() {
    let repo = scratch("tag");
    let _init = hermetic_git(&repo, &["init", "--quiet"]);
    for message in ["first", "second"] {
        let _commit = hermetic_git(
            &repo,
            &["commit", "--quiet", "--allow-empty", "-m", message],
        );
    }
    let _here = hermetic_git(&repo, &["tag", "-a", "here", "-m", "on HEAD"]);
    let _before = hermetic_git(&repo, &["tag", "-a", "before", "-m", "on HEAD~1", "HEAD~1"]);
    let object = |name: &str| hermetic_git(&repo, &["rev-parse", name]).trim().to_owned();
    let (here, before, parent) = (object("here"), object("before"), object("HEAD~1"));
    assert_ne!(here, object("HEAD"), "a tag object, not the commit");
    let mut runner = SystemRunner { root: repo.clone() };
    let on_head = super::pre_push(&mut runner, core::slice::from_ref(&here));
    let elsewhere = super::pre_push(&mut runner, &[before]);
    let _hidden = hermetic_git(&repo, &["config", "status.showUntrackedFiles", "no"]);
    std::fs::create_dir_all(repo.join("src")).expect("folder");
    std::fs::write(repo.join("src/gain.rs"), "").expect("untracked file");
    let untracked = super::pre_push(&mut runner, &[here]);
    std::fs::remove_dir_all(&repo).expect("cleanup");
    assert_eq!(
        untracked,
        failed(
            "the push cannot be checked: the working tree has changes not committed, which the \
             check would include: ?? src/gain.rs"
        ),
        "a file never added, which the user's configuration hides from `git status`"
    );
    assert_eq!(on_head, Ok(()), "the tag of the commit checked out");
    assert!(
        elsewhere.is_err_and(|error| error.summary.contains(&format!("pushing {parent}, but "))),
        "the tag of another commit, named by the commit"
    );
}

#[test]
fn checks_the_history_git_really_writes() {
    let repo = scratch("history");
    let _init = hermetic_git(&repo, &["init", "--quiet"]);
    let commit = |message: &str, file: Option<&str>| {
        if let Some(path) = file {
            std::fs::write(repo.join(path), message).expect("file written");
            let _add = hermetic_git(&repo, &["add", path]);
        }
        let _commit = hermetic_git(
            &repo,
            &["commit", "--quiet", "--allow-empty", "-m", message],
        );
        hermetic_git(&repo, &["rev-parse", "HEAD"])
            .trim()
            .to_owned()
    };
    let _before = commit("fix: before the boundary", Some("a.rs"));
    let since = commit("fix: the boundary itself", Some("b.rs"));
    let _free = commit("docs(wasm): before the scopes' boundary", None);
    let scopes_since = commit("docs(wasm): the scopes' boundary itself", None);
    let _recorded = commit("fix: recorded\n\nWhy.", Some(CHANGELOG));
    let unrecorded = commit("fix: unrecorded", Some("c.rs"));
    let malformed = commit("Update stuff", None);
    let unscoped = commit("docs(wasm): a part named two ways", None);
    let mut runner = SystemRunner { root: repo.clone() };
    let [messages, files, scoped] = super::history_commands(&since, &scopes_since)
        .map(|cmd| runner.capture(&cmd).expect("git log"));
    std::fs::remove_dir_all(&repo).expect("cleanup");
    let short = |hash: &str| hash.chars().take(7).collect::<String>();
    assert_eq!(
        super::history_gate(&messages, &files, &scoped),
        failed(format!(
            "the history breaks the commit rules: 3 problems, listed above\n\
             commit {}: invalid commit message: unknown scope \"wasm\": name a crate, a \
             package or a scope of xtask's COMMIT_SCOPE_EXTRAS ({})\n\
             commit {}: invalid commit message: missing \"<type>: \" prefix\n\
             commit {} changes behavior but not CHANGELOG.md",
            short(&unscoped),
            super::commit_scopes().join(", "),
            short(&malformed),
            short(&unrecorded)
        )),
        "every message checked; the changelog rule from the commit after its boundary on, \
         the scopes from the commit after theirs"
    );
}

#[test]
fn shows_one_problem_inline_and_lists_several() {
    assert_eq!(super::verdict("context", vec![]), Ok(()), "no problem");
    assert_eq!(
        super::verdict("context", vec!["a".to_owned()]),
        Err(Failure {
            summary: "context: a".to_owned(),
            details: vec![],
        }),
        "one problem, in the summary line"
    );
    assert_eq!(
        super::verdict("context", vec!["a".to_owned(), "b".to_owned()]),
        Err(Failure {
            summary: "context: 2 problems, listed above".to_owned(),
            details: vec!["a".to_owned(), "b".to_owned()],
        }),
        "several problems, counted, then listed"
    );
}

#[test]
fn prints_the_details_of_a_failure_on_stdout_and_one_line_on_stderr() {
    let mut fake = passing();
    let unreviewed = fake.files.insert(
        UNVIABLE_REPORT.to_owned(),
        "crates/a/src/c.rs:1:1: replace g -> U with x\n".to_owned(),
    );
    assert!(unreviewed.is_some(), "report replaced");
    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert_eq!(
        run_cli(
            &["check".to_owned(), "--full".to_owned()],
            &mut fake,
            &mut out,
            &mut err
        ),
        1,
        "the check fails"
    );
    assert!(
        fake.ran.contains(
            &"cargo mutants --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --workspace --jobs 3 --timeout-multiplier 15 --minimum-test-timeout 60 -- -- --include-ignored".to_owned()
        ),
        "every mutant, in half the threads of the machine"
    );
    assert_eq!(
        String::from_utf8(err).expect("UTF-8"),
        "error: 1 of 87 checks failed, listed above\n",
        "one line on stderr, counting the failed steps"
    );
    assert!(
        String::from_utf8(out).expect("UTF-8").ends_with(&format!(
            "▶ [87/87] Working tree unchanged by the checks\n  done in 1.5 s\n\
             not reviewed: crates/a/src/c.rs: replace g -> U with x\n\
             stale review: crates/a/src/b.rs: replace f -> T with Default::default()\n\
             Rust unviable mutants reviewed: unviable mutants must be reviewed in \
             {UNVIABLE_REVIEWED}: 2 problems, listed above\n"
        )),
        "every step run, then the problems and the failed step on stdout"
    );
}

#[test]
fn aligns_the_usage_in_the_order_of_its_first_line() {
    let mut lines = USAGE.lines();
    let first = lines.next().expect("a first line");
    let listed: Vec<&str> = first
        .trim_start_matches("usage: cargo xtask <")
        .trim_end_matches('>')
        .split('|')
        .filter_map(|form| form.split_whitespace().next())
        .collect();
    let mut described = Vec::new();
    let mut columns = Vec::new();
    for line in lines {
        let body = line.trim_start();
        let indent = line.len() - body.len();
        if indent == 2 {
            described.push(body.split_whitespace().next().expect("a command"));
            let (_, description) = body
                .split_once("  ")
                .expect("a description after two spaces");
            columns.push(line.len() - description.trim_start().len());
        } else {
            columns.push(indent);
        }
    }
    assert_eq!(listed, described, "the rows follow the first line");
    assert!(
        columns.windows(2).all(|pair| pair[0] == pair[1]),
        "every description starts at the same column: {columns:?}"
    );
}

/// The usage error of `check` with other arguments than it takes.
const CHECK_USAGE: &str = "`check` takes no argument but `--full`, then `--part <part>`";

/// The usage error of `list` with other arguments than it takes.
const LIST_USAGE: &str = "`list` takes no argument but `--full`, then `--parts`";

#[test]
fn refuses_usage_errors_in_one_line() {
    for (args, reason) in [
        (vec![], "no command given"),
        (vec!["nope"], "unknown command \"nope\""),
        (vec!["nope", "x"], "unknown command \"nope\""),
        (vec!["check", "x"], CHECK_USAGE),
        (vec!["check", "--full", "x"], CHECK_USAGE),
        (vec!["check", "--part", "rust"], CHECK_USAGE),
        (vec!["check", "--full", "--part"], CHECK_USAGE),
        (vec!["check", "x", "--part", "rust"], CHECK_USAGE),
        (vec!["check", "--full", "x", "rust"], CHECK_USAGE),
        (vec!["check", "--full", "--part", "rust", "x"], CHECK_USAGE),
        (
            vec!["check", "--full", "--part", "mutation-8"],
            "unknown part \"mutation-8\" (`cargo xtask list --full --parts` lists them)",
        ),
        (
            vec!["check", "--full", "--part", "tooling/config"],
            "unknown part \"tooling/config\" (`cargo xtask list --full --parts` lists them)",
        ),
        (vec!["list", "x"], LIST_USAGE),
        (vec!["list", "--full", "--full"], LIST_USAGE),
        (vec!["list", "--parts"], LIST_USAGE),
        (vec!["list", "x", "--parts"], LIST_USAGE),
        (vec!["list", "--full", "--parts", "x"], LIST_USAGE),
        (vec!["bless", "x"], BLESS_USAGE),
        (vec!["bless", "--case-changed"], BLESS_USAGE),
        (
            vec!["bless", "--case-changed", "a", "--other", "b"],
            BLESS_USAGE,
        ),
        (
            vec!["bless", "--from-nothing", "--from-nothing"],
            BLESS_USAGE,
        ),
        (
            vec![
                "bless",
                "--battery-changed",
                "--case-changed",
                "a",
                "--battery-changed",
            ],
            BLESS_USAGE,
        ),
        (
            vec!["bless", "--case-changed", "a", "--case-changed"],
            BLESS_USAGE,
        ),
        (
            vec!["bless-benchmarks", "x"],
            "`bless-benchmarks` takes no argument",
        ),
        (vec!["help", "x"], "`help` takes no argument"),
        (vec!["commit-msg"], "`commit-msg` takes one file"),
        (
            vec!["pre-push", "abc"],
            "`pre-push` takes pairs of a commit and the remote ref it updates",
        ),
        (
            vec!["pre-push", "abc", "refs/heads/a", "def"],
            "`pre-push` takes pairs of a commit and the remote ref it updates",
        ),
        (vec!["commit-msg", "a", "b"], "`commit-msg` takes one file"),
    ] {
        let args: Vec<String> = args.into_iter().map(str::to_owned).collect();
        let (mut stdout, mut usage) = (Vec::new(), Vec::new());
        let mut fake = passing();
        assert_eq!(
            run_cli(&args, &mut fake, &mut stdout, &mut usage),
            2,
            "{args:?} is a usage error"
        );
        assert_eq!(
            String::from_utf8(usage).expect("UTF-8"),
            format!("error: {reason} (see: cargo xtask help)\n"),
            "{args:?}: one line on stderr, the reason and where to look"
        );
        assert!(stdout.is_empty(), "{args:?}: nothing on stdout");
        assert!(fake.ran.is_empty(), "{args:?}: nothing runs");
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt as _;
        let mut usage = Vec::new();
        assert_eq!(
            cli(
                &[OsString::from_vec(b"che\xffck".to_vec())],
                &mut passing(),
                &mut Vec::new(),
                &mut usage
            ),
            2,
            "an argument that is not UTF-8 is a usage error, not a panic"
        );
        assert_eq!(
            String::from_utf8(usage).expect("UTF-8"),
            "error: argument is not valid UTF-8: che\u{fffd}ck (see: cargo xtask help)\n",
            "named as well as it can be"
        );
    }
}

#[test]
fn wires_every_package_into_the_checks() {
    let steps = check_steps(CHANGES_MODE);
    let fresh: Vec<String> = steps
        .iter()
        .filter_map(|step| {
            if let Action::FreshDirs(paths) = &step.action {
                Some(paths.clone())
            } else {
                None
            }
        })
        .flatten()
        .collect();
    let [shipped, _] = super::license_commands();
    for package in PACKAGES {
        let checked = steps.iter().position(|step| {
            step.name == package.step
                && step.action
                    == Action::Run(Cmd {
                        program: "pnpm",
                        args: vec!["run".to_owned(), "check".to_owned()],
                        dir: Dir::Package(package.dir),
                    })
        });
        let gated = steps
            .iter()
            .position(|step| step.action == Action::TypeScriptReportsGate(package));
        assert!(
            checked.is_some() && checked < gated,
            "{}: its checks run, before the reports gate",
            package.dir
        );
        for report in package.reports() {
            assert!(fresh.contains(&report), "{report} is emptied first");
        }
        assert!(
            package_jsons().contains(&format!("{}/package.json", package.dir)),
            "{}: its manifest is versioned",
            package.dir
        );
        assert_eq!(
            shipped.args.contains(&format!("./{}", package.dir)),
            package.shipped,
            "{}: its production licenses are held to the shipped list if it ships",
            package.dir
        );
    }
}

#[test]
fn renders_install_hints_from_the_pins() {
    use super::{Install, NIGHTLY_TOOLCHAIN, Need, PINNED_TOOLS, TOOLS};
    for pin in PINNED_TOOLS {
        let tool = TOOLS
            .iter()
            .find(|tool| tool.need == Need::Pinned(pin))
            .expect("every pinned tool is checked");
        assert_eq!(
            tool.install.to_string(),
            format!("cargo install --locked {}@{}", pin.0, pin.1),
            "{}: installed at the version the checks need",
            pin.0
        );
    }
    let nightly = NIGHTLY_TOOLCHAIN.trim_start_matches('+');
    for tool in TOOLS
        .iter()
        .filter(|tool| tool.args.contains(&NIGHTLY_TOOLCHAIN))
    {
        assert_eq!(tool.need, Need::Present, "any build of the pinned nightly");
        assert!(
            tool.install.to_string().contains(nightly),
            "{:?}: installs the pinned nightly",
            tool.args
        );
    }
    assert_eq!(
        Install::Nightly(&["a", "b"]).to_string(),
        format!("rustup toolchain install {nightly} --component a --component b"),
        "a toolchain with its components"
    );
    assert_eq!(
        Install::NightlyComponents(&["a", "b"]).to_string(),
        format!("rustup component add --toolchain {nightly} a b"),
        "components of the toolchain"
    );
    assert_eq!(
        Install::Node.to_string(),
        "install the Node.js release of .node-version (fnm, nvm and volta read it)",
        "Node.js from its pin"
    );
    assert_eq!(
        Install::Text("install git").to_string(),
        "install git",
        "what to do, in words"
    );
}

#[test]
fn requires_every_package_to_support_the_node_floor() {
    for engines in [
        r#""engines": {"node": ">=22.17"}"#,
        r#""engines": {"node": ">=22.18.1"}"#,
        r#""engines": {}"#,
    ] {
        let manifest = format!(r#"{{"version": "0.2.0", {engines}}}"#);
        assert!(
            release_gate(CARGO, &roadmap_package(&manifest), LOG).is_err_and(|failure| failure
                .summary
                .contains(&format!("{ROADMAP_PACKAGE}: engines.node is "))),
            "{engines}"
        );
    }
    assert_eq!(
        release_gate(CARGO, &roadmap_package(r#"{"version": "0.2.0"}"#), LOG),
        found(
            "the versions and the changelog",
            &[format!(
                "{ROADMAP_PACKAGE}: engines.node is (missing), the packages support Node.js >=22.18 (NODE_FLOOR)"
            )]
        ),
        "no engines"
    );
}

#[test]
fn allows_tooling_every_license_allowed_for_what_ships() {
    for license in super::SHIPPED_LICENSES {
        assert_eq!(
            license_gate(
                &format!(r#"{{"{license}":[{{"name":"react"}},{{"name":"react-dom"}}]}}"#),
                &format!(
                    r#"{{"{license}":[{{"name":"react"}},{{"name":"react-dom"}},{{"name":"typescript"}}]}}"#
                )
            ),
            Ok(()),
            "{license}: a shipped package is in the full report too"
        );
    }
}

#[test]
fn reads_node_releases_alike_everywhere() {
    for (text, release) in [
        ("v22.18.0\n", Some((22, 18, 0))),
        ("22.23.4", Some((22, 23, 4))),
        ("v24.1.0", Some((24, 1, 0))),
        ("v22.18.0-nightly", None),
        ("v22.x", None),
        ("22.18", None),
        ("vv22.18.0", None),
        ("22.23.4.1", None),
    ] {
        assert_eq!(
            super::node_release(text).map(|version| (version.major, version.minor, version.patch)),
            release,
            "{text:?}"
        );
        let answers: Vec<Result<String, CommandFailure>> = super::TOOLS
            .iter()
            .map(|tool| {
                Ok(if tool.program == "node" {
                    text.to_owned()
                } else {
                    tool_answer(tool.program, tool.args.first().copied())
                })
            })
            .collect();
        assert_eq!(
            preflight_gate("linux", &answers, text, TOOLCHAIN, &CLEAN).is_ok(),
            super::workflow_gate(&[super::WORKFLOW.to_owned()], PINNED_WORKFLOW, text).is_ok(),
            "{text:?}: the same verdict from the preflight and the workflow gate"
        );
    }
}

#[test]
fn reports_readme_problems_whatever_the_membership() {
    let mut fake = passing();
    let extra = fake
        .files
        .insert("apps/editor/package.json".to_owned(), PACKAGE.to_owned());
    let removed = fake.files.remove("apps/README.md");
    assert!(extra.is_none() && removed.is_some(), "both problems set");
    let step = |action: &Action| -> Vec<Step> {
        check_steps(CHANGES_MODE)
            .into_iter()
            .filter(|step| step.action == *action)
            .collect()
    };
    assert_eq!(
        run_steps(&step(&Action::MembershipGate), &mut fake, &mut Vec::new()),
        failed(
            "Packages and crates known to the checks: the manifests differ from what xtask \
             knows: apps/editor/package.json is not a package of PACKAGES: its checks and \
             reports would not run"
        ),
        "the membership problem, under its own step"
    );
    assert_eq!(
        run_steps(&step(&Action::ReadmeGate), &mut fake, &mut Vec::new()),
        failed("Every part described in a README: parts not described: apps has no README.md"),
        "the README problems, not hidden by the membership one"
    );
}

#[test]
fn license_and_history_commands_are_exactly() {
    assert_eq!(
        super::license_commands().map(|cmd| cmd.line()),
        LICENSE_LINES.map(str::to_owned),
        "every shipped package filtered, then everything"
    );
    assert_eq!(
        super::history_commands(super::CHANGELOG_SINCE, super::SCOPES_SINCE).map(|cmd| cmd.line()),
        HISTORY_LINES.map(str::to_owned),
        "every message, the files after the boundary, the commits after the scopes' boundary"
    );
    assert!(
        super::license_commands()
            .iter()
            .chain(&super::history_commands(
                super::CHANGELOG_SINCE,
                super::SCOPES_SINCE
            ))
            .all(|cmd| cmd.dir == Dir::Root),
        "run at the root"
    );
}

#[test]
fn reviews_each_occurrence_of_a_mutant() {
    let report = "crates/a/src/b.rs:10:5: replace && with || in f\ncrates/a/src/b.rs:20:9: replace && with || in f\n";
    let once = "crates/a/src/b.rs: replace && with || in f -- a let chain\n";
    assert_eq!(
        unviable_gate(report, Extent::Complete, once),
        found(
            &format!("unviable mutants must be reviewed in {UNVIABLE_REVIEWED}"),
            &[
                "not reviewed: crates/a/src/b.rs: replace && with || in f (2 found, 1 reviewed)"
                    .to_owned()
            ]
        ),
        "the second occurrence is not reviewed"
    );
    let twice = format!("{once}crates/a/src/b.rs: replace && with || in f -- another let chain\n");
    assert_eq!(
        unviable_gate(report, Extent::Complete, &twice),
        Ok(()),
        "each occurrence reviewed"
    );
    let thrice = format!("{twice}{once}");
    assert_eq!(
        unviable_gate(report, Extent::Complete, &thrice),
        found(
            &format!("unviable mutants must be reviewed in {UNVIABLE_REVIEWED}"),
            &[
                "stale review: crates/a/src/b.rs: replace && with || in f (2 found, 3 reviewed)"
                    .to_owned()
            ]
        ),
        "a review too many"
    );
    let source = "// Stryker disable next-line EqualityOperator: same\nx < 1\n// Stryker disable next-line EqualityOperator: same\nx < 1\n";
    assert_eq!(
        equivalent_gate(
            &[("a.ts".to_owned(), source.to_owned())],
            "a.ts: EqualityOperator at x < 1 -- same\n"
        ),
        found(
            &format!("disabled mutants must be reviewed in {EQUIVALENT_REVIEWED}"),
            &["not reviewed: a.ts: EqualityOperator at x < 1 (2 found, 1 reviewed)".to_owned()]
        ),
        "the same disable twice in a file needs two reviews"
    );
}

#[test]
fn refuses_checks_that_change_the_working_tree() {
    let tree = |status: &str, diff: &str| super::TreeState {
        status: status.to_owned(),
        diff: diff.to_owned(),
        head: TREE_HEAD.1.to_owned(),
    };
    let dirty = tree(" M a.rs\n?? notes.txt\n", "diff a.rs\n");
    assert_eq!(
        super::tree_gate(Some(&dirty), &dirty.clone()),
        Ok(()),
        "a dirty tree the checks left alone"
    );
    assert_eq!(
        super::tree_gate(
            Some(&dirty),
            &tree(" M a.rs\n M Cargo.lock\n", "diff a.rs\n")
        ),
        found(
            "the checks changed the working tree",
            &[
                "` M Cargo.lock` appeared".to_owned(),
                "`?? notes.txt` went away".to_owned()
            ]
        ),
        "a rewritten lock file, a file removed"
    );
    assert_eq!(
        super::tree_gate(
            Some(&dirty),
            &tree(" M a.rs\n?? notes.txt\n", "other diff\n")
        ),
        failed(
            "the checks changed the working tree: the uncommitted changes of a tracked file changed"
        ),
        "a dirty file changed again"
    );
    for head in [
        "def4560000000000000000000000000000000000\nrefs/heads/main\n",
        "abc1230000000000000000000000000000000000\nrefs/heads/other\n",
        "abc1230000000000000000000000000000000000\nHEAD\n",
    ] {
        let moved = super::TreeState {
            head: head.to_owned(),
            ..dirty.clone()
        };
        assert_eq!(
            super::tree_gate(Some(&dirty), &moved),
            failed(format!(
                "the checks changed the working tree: what is checked out changed, from \
                 `abc1230000000000000000000000000000000000 refs/heads/main` to `{}`: the checks read another commit than the one \
                 recorded",
                head.split_whitespace().collect::<Vec<_>>().join(" ")
            )),
            "a commit made, a branch checked out or HEAD detached during the checks: {head:?}"
        );
    }
    assert_eq!(
        super::tree_gate(None, &dirty),
        failed("the working tree was not recorded before the checks"),
        "nothing to compare with"
    );
    let mut committed = passing();
    committed.rewrite = Some((
        "cargo deny --locked check".to_owned(),
        format!("capture {}", TREE_HEAD.0),
        "def4560000000000000000000000000000000000\nrefs/heads/main\n".to_owned(),
    ));
    assert!(
        run_steps(&check_steps(CHANGES_MODE), &mut committed, &mut Vec::new()).is_err_and(
            |failure| failure
                .summary
                .contains("what is checked out changed, from `abc1230000000000000000000000000000000000 refs/heads/main` to `def4560000000000000000000000000000000000 refs/heads/main`")
        ),
        "a commit made in the same checkout while the check runs fails it"
    );
    let mut rewritten = passing();
    rewritten.rewrite = Some((
        "cargo deny --locked check".to_owned(),
        format!("capture {}", TREE_STATUS.0),
        "?? notes.txt\n M Cargo.lock\n".to_owned(),
    ));
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut rewritten, &mut Vec::new()),
        failed(
            "Working tree unchanged by the checks: the checks changed the working tree: \
             ` M Cargo.lock` appeared"
        ),
        "a command that rewrites a tracked file fails the whole check, at its end"
    );
    let last = check_steps(CHANGES_MODE).split_off(check_steps(CHANGES_MODE).len() - 1);
    assert_eq!(
        run_steps(&last, &mut passing(), &mut Vec::new()),
        failed(
            "Working tree unchanged by the checks: the working tree was not recorded before \
             the checks"
        ),
        "the comparison alone"
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
        "crates/engine-core/src/lib.rs",
        "crates/engine-cli/src/main.rs",
        "crates/engine-cli/tests/binary.rs",
        "crates/engine-cli/build.rs",
        "crates/engine-cli/src/bin/other.rs",
        "bindings/wasm/src/engine.rs",
        "bindings/wasm/tests/web.rs",
    ] {
        sources.push((path.to_owned(), forbid.clone()));
    }
    for path in [
        "crates/engine-core/src/plan.rs",
        "crates/engine-cli/tests/data/fixture.rs",
        "bindings/wasm/src/lib.rs",
        "README.rs",
    ] {
        sources.push((path.to_owned(), "fn plain() {}\n".to_owned()));
    }
    assert_eq!(
        unsafe_gate(&sources),
        Ok(()),
        "every root forbids, modules inherit, the unsafe crate's root and files apart"
    );
    let mut missing = sources.clone();
    for (path, _) in [
        ("crates/engine-core/src/lib.rs", ""),
        ("crates/engine-cli/tests/binary.rs", ""),
        ("bindings/wasm/src/engine.rs", ""),
    ] {
        missing.retain(|(source, _)| source != path);
        missing.push((path.to_owned(), "fn plain() {}\n".to_owned()));
    }
    assert_eq!(
        unsafe_gate(&missing),
        found(
            "the lint against unsafe code",
            &[
                format!(
                    "crates/engine-core/src/lib.rs does not hold `{}`",
                    forbid.trim()
                ),
                format!(
                    "crates/engine-cli/tests/binary.rs does not hold `{}`",
                    forbid.trim()
                ),
                format!(
                    "bindings/wasm/src/engine.rs does not hold `{}`",
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
        probe.push(("crates/engine-core/src/m.rs".to_owned(), text.clone()));
        assert_eq!(
            unsafe_gate(&probe),
            found(
                "the lint against unsafe code",
                &["crates/engine-core/src/m.rs lifts the lint against unsafe code".to_owned()]
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
            let text = if *path == "bindings/wasm/src/ffi.rs" {
                format!("{}{declaration}", audited(path))
            } else {
                audited(path)
            };
            ((*path).to_owned(), text)
        })
        .collect();
    sources.push((
        "bindings/wasm/src/ffi_tests.rs".to_owned(),
        "fn pointer() {}\n".to_owned(),
    ));
    sources.push((
        "bindings/wasm/src/engine.rs".to_owned(),
        format!("{forbid}{declaration}").replace("ffi_tests", "engine_tests"),
    ));
    sources.push((
        "bindings/wasm/src/engine_tests.rs".to_owned(),
        forbid.clone(),
    ));
    assert_eq!(
        unsafe_gate(&sources),
        Ok(()),
        "the test module of the FFI file is under its lifting"
    );
    let mut safe_tests = sources.clone();
    for (path, text) in &mut safe_tests {
        if path == "bindings/wasm/src/engine_tests.rs" {
            *text = "fn plain() {}\n".to_owned();
        }
    }
    assert_eq!(
        unsafe_gate(&safe_tests),
        failed(format!(
            "the lint against unsafe code: bindings/wasm/src/engine_tests.rs does not hold `{}`",
            forbid.trim()
        )),
        "the test module of a safe file forbids it like any module"
    );
    let mut undeclared = sources;
    for (path, text) in &mut undeclared {
        if path == "bindings/wasm/src/ffi.rs" {
            *text = audited(path);
        }
    }
    assert_eq!(
        unsafe_gate(&undeclared),
        failed(format!(
            "the lint against unsafe code: bindings/wasm/src/ffi_tests.rs does not hold `{}`",
            forbid.trim()
        )),
        "a file the unsafe file does not declare is not its test module"
    );
}

#[test]
fn reads_the_lint_forbids_as_no_test_code() {
    let gate = |text: &str| {
        super::test_code_gate(&[("crates/engine-core/src/a.rs".to_owned(), text.to_owned())])
    };
    let context = "test code only in tests/ and declared *_tests.rs files";
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
        failed(format!(
            "{context}: crates/engine-core/src/a.rs:1: names `test` outside test code: tests \
             live in tests/ or in a *_tests.rs file declared `#[cfg(test)] #[path = \
             \"<name>_tests.rs\"] mod <module>;`"
        )),
        "another line"
    );
}

#[test]
fn refuses_a_test_file_that_loads_another() {
    let gate = |sources: &[(&str, &str)]| {
        let owned: Vec<(String, String)> = sources
            .iter()
            .map(|(path, text)| ((*path).to_owned(), (*text).to_owned()))
            .collect();
        super::test_code_gate(&owned)
    };
    let context = "test code only in tests/ and declared *_tests.rs files";
    let declared = "#[cfg(test)]\n#[path = \"plan_tests.rs\"]\nmod tests;\n";
    let inline = "the modules of a *_tests.rs file are inline";
    let loads = |what: &str, line: usize| {
        format!(
            "{context}: crates/engine-core/src/plan_tests.rs:{line}: {what}, which no rule tells \
             as test code: a *_tests.rs file writes its modules inline"
        )
    };
    for (text, expected) in [
        (
            "#[test]\nfn t() {}\nmod helper;\n",
            loads(
                "declares a module without a body, loaded from another file",
                3,
            ),
        ),
        (
            "pub(crate) mod  helper ;\n",
            loads(
                "declares a module without a body, loaded from another file",
                1,
            ),
        ),
        (
            "#[path = \"h.rs\"]\nmod h {}\n",
            loads("holds a `#[path]` attribute, which loads another file", 1),
        ),
        (
            "# [\n  path = \"h.rs\"]\nmod h {}\n",
            loads("holds a `#[path]` attribute, which loads another file", 2),
        ),
        (
            "#![path = \"h.rs\"]\n",
            loads("holds a `#[path]` attribute, which loads another file", 1),
        ),
        (
            "include!(\"h.rs\");\n",
            loads("names `include`, which loads another file", 1),
        ),
    ] {
        assert_eq!(
            gate(&[
                ("crates/engine-core/src/plan.rs", declared),
                ("crates/engine-core/src/plan_tests.rs", text),
            ]),
            failed(expected),
            "{text}"
        );
    }
    assert_eq!(
        gate(&[
            ("crates/engine-core/src/plan.rs", declared),
            (
                "crates/engine-core/src/plan_tests.rs",
                "mod inner {\n    #[test]\n    fn t() { let path = include_str!(\"x.json\"); }\n}\n// mod helper;\nconst M: &str = \"mod helper;\";\nfn f(path: &str) -> &str { path }\n#[derive(Debug)] struct S;\n",
            ),
        ]),
        Ok(()),
        "{inline}; a word `path`, a module in prose and test data are no loading"
    );
}

#[test]
fn refuses_test_code_outside_test_files() {
    let gate = |sources: &[(&str, &str)]| {
        let owned: Vec<(String, String)> = sources
            .iter()
            .map(|(path, text)| ((*path).to_owned(), (*text).to_owned()))
            .collect();
        super::test_code_gate(&owned)
    };
    let context = "test code only in tests/ and declared *_tests.rs files";
    let declared = "//! test, in prose\nconst A: &str = \"test\";\nconst B: &str = include_str!(\"x.json\");\n#[cfg(test)]\n#[path = \"plan_tests.rs\"]\nmod tests;\n";
    let tests = "#[test]\nfn f() { assert!(cfg!(test)); }\n";
    let fine = [
        ("crates/engine-core/src/plan.rs", declared),
        ("crates/engine-core/src/plan_tests.rs", tests),
        (
            "crates/engine-cli/tests/binary.rs",
            "#[cfg(test)]\nmod t { include!(\"x.rs\"); }\n",
        ),
        ("crates/engine-cli/tests/data_tests.rs", ""),
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
        ("#[cfg(any(test, doc))]\nfn f() {}\n", 1),
    ] {
        assert_eq!(
            gate(&[("crates/engine-core/src/a.rs", text)]),
            failed(format!(
                "{context}: crates/engine-core/src/a.rs:{line}: {names}"
            )),
            "{text}"
        );
    }
    assert_eq!(
        gate(&[(
            "crates/engine-core/src/a.rs",
            "fn f() {\n    #[test]\n    fn g() {}\n}\n"
        )]),
        failed(format!("{context}: crates/engine-core/src/a.rs:2: {names}")),
        "a test function"
    );
    assert_eq!(
        gate(&[("bindings/wasm/src/a.rs", "include!(\"b.rs\");\n")]),
        failed(format!(
            "{context}: bindings/wasm/src/a.rs:1: names `include`: code is brought in by `mod` \
             only, so that its path says what it is"
        )),
        "code included"
    );
    assert_eq!(
        gate(&[("crates/engine-core/src/plan_tests.rs", tests)]),
        failed(format!(
            "{context}: crates/engine-core/src/plan_tests.rs: a *_tests.rs file no file beside \
             it declares as test code"
        )),
        "an undeclared test file"
    );
    assert_eq!(
        gate(&[
            ("crates/engine-core/src/plan.rs", declared),
            ("crates/engine-core/src/other.rs", declared),
            ("crates/engine-core/src/plan_tests.rs", tests),
            ("crates/engine-core/src/sub/x.rs", declared),
        ]),
        found(
            context,
            &["crates/engine-core/src/plan_tests.rs: declared as test code 2 times".to_owned()]
        ),
        "a test file declared twice; one declared from another directory is another file"
    );
    let unknown = gate(&[
        ("crates/engine-core/src/plan.rs", declared),
        ("crates/engine-core/src/plan_tests.rs", tests),
        ("crates/engine-core/src/sub/x.rs", declared),
    ]);
    assert_eq!(
        unknown,
        Ok(()),
        "a declaration of a missing file is the compiler's to refuse"
    );
}

#[test]
fn refuses_test_data_a_crate_does_not_declare() {
    let context = "the tests of each crate read only its declared test data";
    let base = test_data_reader_sources();
    assert_eq!(super::test_data_gate(&base), Ok(()), "every root read");
    let with = |path: &str, text: &str| {
        let mut sources = base.clone();
        sources.push((path.to_owned(), text.to_owned()));
        super::test_data_gate(&sources)
    };
    for text in [
        "const A: &str = include_str!(\"../../../spec/render-hash-vectors.json\");\n",
        "const A: &[u8] = include_bytes!(\"./data/own.bin\");\n",
        "fn f() { let _ = Path::new(env!(\"CARGO_MANIFEST_DIR\")).join(\"src\"); }\n",
        "fn f() { let _ = env!(\"CARGO_PKG_NAME\"); let _ = std::env::temp_dir(); }\n",
        "// include_str!(\"../../../spec/x.json\")\nconst A: &str = \"include_str!(\\\"x\\\")\";\n",
    ] {
        assert_eq!(
            with("crates/engine-hash/src/more_tests.rs", text),
            Ok(()),
            "declared data, the crate's own files, other variables, prose: {text}"
        );
    }
    let unread = "crates/engine-hash/src/more_tests.rs:1: the tests of engine-hash read";
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
            with("crates/engine-hash/src/more_tests.rs", text),
            failed(format!(
                "{context}: {unread} `{read}`, which is not in its test_data"
            )),
            "{text}"
        );
    }
    for (text, word) in [
        ("const A: &str = include_str!(PATH);\n", "include_str"),
        ("const A: &str = include_bytes! (\"x\");\n", "include_bytes"),
        ("const A: &str = include_str!(\"x\" );\n", "include_str"),
        ("const A: &str = include_str!(\"x\n", "include_str"),
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
            with("crates/engine-hash/src/more_tests.rs", text),
            failed(format!(
                "{context}: crates/engine-hash/src/more_tests.rs:1: `{word}!` without a literal \
                 path the gate can follow"
            )),
            "{text}"
        );
    }
    assert_eq!(
        with(
            "crates/engine-hash/src/more_tests.rs",
            "const A: &str = include_str!(\"../../../../x\");\n"
        ),
        failed(format!(
            "{context}: crates/engine-hash/src/more_tests.rs:1: `../../../../x` leaves the \
             repository"
        )),
        "outside the repository"
    );
    let without: Vec<(String, String)> = base
        .iter()
        .filter(|(path, _)| path != "crates/engine-hash/tests/data.rs")
        .cloned()
        .collect();
    assert_eq!(
        super::test_data_gate(&without),
        failed(format!(
            "{context}: engine-hash: declares `spec/render-hash-vectors.json` as test data, \
             which no test of it reads"
        )),
        "a stale root"
    );
    assert_eq!(
        with("README.rs", "const A: &str = include_str!(\"x\");\n"),
        Ok(()),
        "a file of no crate"
    );
}

#[test]
fn refuses_the_reads_of_test_data_it_cannot_follow() {
    let context = "the tests of each crate read only its declared test data";
    let with = |path: &str, text: &str| {
        let mut sources = test_data_reader_sources();
        sources.push((path.to_owned(), text.to_owned()));
        super::test_data_gate(&sources)
    };
    let file = "crates/engine-hash/src/more_tests.rs";
    let manifest = format!("\"{}_{}_{}\"", "CARGO", "MANIFEST", "DIR");
    for (text, why) in [
        (
            format!("fn f() {{ let _ = std::env::var_os({manifest}); }}\n"),
            format!("`var_os({manifest})`, a directory read at run time the gate cannot follow"),
        ),
        (
            format!("fn f() {{ let _ = var ( {manifest} ); }}\n"),
            format!("`var({manifest})`, a directory read at run time the gate cannot follow"),
        ),
        (
            "fn f() { let _ = option_env!(\"X\"); }\n".to_owned(),
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
            failed(format!("{context}: {file}:1: {why}")),
            "{text}"
        );
    }
    for text in [
        "fn f() { let _ = std::env::var_os(CHILD); let _ = var(\"HOME\"); }\n".to_owned(),
        "fn f() { let _ = env!(\"CARGO_BIN_EXE_engine-cli\"); let _ = env!(\"CARGO_PKG_VERSION\"); }\n"
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
            "crates/engine-testkit/src/measure/mod.rs",
            "const V: &str = include_str!(\"../../../../spec/measurement-vectors.json\");\n"
        ),
        failed(format!(
            "{context}: crates/engine-testkit/src/measure/mod.rs:1: code that is not a test \
             reads `spec/measurement-vectors.json`, outside its crate: only tests read test data"
        )),
        "a library's read is no input any crate declares"
    );
}

#[test]
fn leaves_a_binary_entry_point_to_find_its_repository() {
    let context = "the tests of each crate read only its declared test data";
    let with = |path: &str, text: &str| {
        let mut sources = test_data_reader_sources();
        sources.push((path.to_owned(), text.to_owned()));
        super::test_data_gate(&sources)
    };
    let root =
        "fn main() { let _ = PathBuf::from(env!(\"CARGO_MANIFEST_DIR\")).join(\"../..\"); }\n";
    assert_eq!(
        with("crates/xtask/src/main.rs", root),
        Ok(()),
        "a binary's entry point, which finds the repository it runs in"
    );
    for path in [
        "crates/engine-hash/src/main.rs",
        "crates/xtask/src/bin/main.rs",
    ] {
        assert_eq!(
            with(path, root),
            failed(format!(
                "{context}: {path}:1: code that is not a test reads ``, outside its crate: only \
                 tests read test data"
            )),
            "{path}: not a binary's entry point"
        );
    }
    assert_eq!(
        with("crates/xtask/tests/main.rs", root),
        failed(format!(
            "{context}: crates/xtask/tests/main.rs:1: the tests of xtask read ``, which is not in \
             its test_data"
        )),
        "a test's main"
    );
}

#[test]
fn reads_only_the_one_form_of_a_test_declaration() {
    let declared = |path: &str, text: &str| super::test_declarations(path, text);
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
    for text in [
        "#[path = \"x_tests.rs\"]\nmod tests;\n",
        "#[cfg(not(test))]\n#[path = \"x_tests.rs\"]\nmod tests;\n",
        "#[cfg(test)]\n\n#[path = \"x_tests.rs\"]\nmod tests;\n",
        "#[cfg(test)]\n#[path = \"../x_tests.rs\"]\nmod tests;\n",
        "#[cfg(test)]\n#[path = \"d/x_tests.rs\"]\nmod tests;\n",
        "#[cfg(test)]\n#[path = \"_tests.rs\"]\nmod tests;\n",
        "#[cfg(test)]\n#[path = \"x.rs\"]\nmod tests;\n",
        "#[cfg(test)]\n#[path=\"x_tests.rs\"]\nmod tests;\n",
        "#[cfg(test)]\n#[path = \"x_tests.rs\"] mod tests;\n",
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

#[test]
fn requires_every_crate_to_inherit_the_workspace_lints() {
    let packages = package_jsons();
    let crates: Vec<(String, String)> = super::RUST_CRATES
        .iter()
        .map(|krate| {
            let text = if krate.name == "engine-core" {
                format!(
                    "[package]\nversion.workspace = true\nrepository.workspace = true\n\n\
                     [lints.rust]\n{}_{} = \"allow\"\n",
                    "unsafe", "code"
                )
            } else if krate.name == "engine-hash" {
                "[package]\nversion.workspace = true\nrepository.workspace = true\n\n[lints]\n\n\
                 workspace = true\n"
                    .to_owned()
            } else {
                MEMBER.to_owned()
            };
            (format!("{}/Cargo.toml", krate.dir), text)
        })
        .collect();
    assert_eq!(
        membership_gate(&packages, &crates, &[]),
        found(
            "the manifests differ from what xtask knows",
            &[
                "crates/engine-core/Cargo.toml does not inherit the workspace lints (`[lints]` then `workspace = true`)".to_owned(),
                "crates/engine-hash/Cargo.toml does not inherit the workspace lints (`[lints]` then `workspace = true`)".to_owned(),
            ]
        ),
        "lints of its own, or the inheritance away from its table"
    );
}

#[test]
fn requires_the_exact_cargo_configuration() {
    assert_eq!(
        super::cargo_config_gate(super::CARGO_CONFIG),
        Ok(()),
        "the alias"
    );
    for config in [
        super::CARGO_CONFIG.replace(" --locked", ""),
        format!(
            "{}[build]\nrustflags = [\"-A{}_{}\"]\n",
            super::CARGO_CONFIG,
            "unsafe",
            "code"
        ),
    ] {
        assert_eq!(
            super::cargo_config_gate(&config),
            Err(Failure {
                summary: ".cargo/config.toml differs from the configuration xtask expects (no \
                          build flag, a locked alias)"
                    .to_owned(),
                details: vec![
                    "expected:".to_owned(),
                    super::CARGO_CONFIG.to_owned(),
                    "found:".to_owned(),
                    config.clone(),
                ],
            }),
            "{config}"
        );
    }
}

#[test]
fn requires_every_direct_dependency_to_be_reviewed() {
    let reviewed = "x: serde_json (normal) -- reads JSON\n./: left-pad (devDependencies) -- old\n";
    let packages = vec![
        (
            "package.json".to_owned(),
            r#"{"devDependencies":{"left-pad":"1"}}"#.to_owned(),
        ),
        (
            "apps/a/package.json".to_owned(),
            r#"{"dependencies":{"@poyetik/engine-wasm":"workspace:*","react":"^19"},"peerDependencies":{"typescript":"~6"}}"#.to_owned(),
        ),
    ];
    let metadata = r#"{"packages":[{"name":"x","dependencies":[{"name":"engine-core","kind":null,"path":"/r"},{"name":"serde_json","kind":null},{"name":"rand","kind":"dev"}]},{"name":"y"}]}"#;
    assert_eq!(
        super::dependency_gate(metadata, &packages, reviewed),
        found(
            "direct dependencies must be reviewed in dependencies.txt",
            &[
                "not reviewed: .: left-pad (devDependencies)".to_owned(),
                "not reviewed: apps/a: react (dependencies)".to_owned(),
                "not reviewed: apps/a: typescript (peerDependencies)".to_owned(),
                "not reviewed: x: rand (dev)".to_owned(),
                "stale review: ./: left-pad (devDependencies)".to_owned(),
            ]
        ),
        "a new crate, an extra npm key in each field, a review of nothing; parts of the \
         workspace apart"
    );
    assert!(
        super::dependency_gate("[", &[], "")
            .is_err_and(|failure| failure.summary.starts_with("invalid cargo metadata: ")),
        "unreadable metadata"
    );
    assert_eq!(
        super::dependency_gate("{}", &[], ""),
        failed("cargo metadata lists no packages"),
        "metadata without packages"
    );
    assert_eq!(
        super::dependency_gate(r#"{"packages":[{"dependencies":[]}]}"#, &[], ""),
        failed("cargo metadata lists a package without a name"),
        "a package without a name is refused, never read as \"\""
    );
    assert!(
        super::dependency_gate(
            METADATA,
            &[("p/package.json".to_owned(), "{".to_owned())],
            ""
        )
        .is_err_and(|failure| failure
            .summary
            .starts_with("p/package.json: invalid package.json: ")),
        "an unreadable manifest"
    );
}

/// The summary of the math layer gate's failures.
const MATH_LAYER: &str = "box arithmetic only through the math layer (engine-core, \
                          engine-testkit only may depend on libm; box code names no raw float, \
                          libm, host or delivery, and computes no float; no iterator reduction \
                          outside the math layer; code comes in by `mod` alone)";

#[test]
fn refuses_libm_outside_the_math_layer_and_the_measurements() {
    let gate = |dependencies: &str| {
        super::math_layer_gate(
            &format!(
                r#"{{"packages":[{{"name":"engine-core","dependencies":[{{"name":"libm","kind":null}}]}},{{"name":"engine-testkit","dependencies":[{{"name":"libm","kind":null}}]}},{{"name":"engine-objects","dependencies":{dependencies}}}]}}"#
            ),
            &[],
        )
    };
    let refused = |name: &str| {
        Err(Failure::from(format!(
            "{MATH_LAYER}: {name} depends on libm: its arithmetic must go through the math \
             layer of engine-core"
        )))
    };
    assert_eq!(
        gate("[]"),
        Ok(()),
        "the math layer and the measurements may"
    );
    assert_eq!(
        gate(r#"[{"name":"libm","kind":"dev"}]"#),
        Ok(()),
        "a test's reference may"
    );
    assert_eq!(
        gate(r#"[{"name":"libm","kind":null}]"#),
        refused("engine-objects"),
        "a box crate may not"
    );
    assert_eq!(
        gate(r#"[{"name":"libm","kind":"build","rename":"math"}]"#),
        refused("engine-objects"),
        "not as a build dependency, not renamed"
    );
    assert_eq!(
        gate(r#"[{"name":"libmx","kind":null},{"name":"engine-core","kind":null,"path":"/r"}]"#),
        Ok(()),
        "another crate"
    );
    let two = super::math_layer_gate(
        r#"{"packages":[{"name":"a","dependencies":[{"name":"libm"}]},{"name":"b","dependencies":[{"name":"libm","kind":"normal"}]}]}"#,
        &[],
    );
    assert_eq!(
        two.map_err(|failure| (failure.summary, failure.details.len())),
        Err((format!("{MATH_LAYER}: 2 problems, listed above"), 2)),
        "every crate"
    );
    assert!(
        super::math_layer_gate("[", &[])
            .is_err_and(|failure| failure.summary.starts_with("invalid cargo metadata: ")),
        "unreadable metadata"
    );
    assert_eq!(
        super::math_layer_gate("{}", &[]),
        failed("cargo metadata lists no packages"),
        "metadata without packages"
    );
    assert_eq!(
        super::math_layer_gate(r#"{"packages":[{"dependencies":[{"name":"libm"}]}]}"#, &[]),
        failed("cargo metadata lists a package without a name"),
        "a package without a name is refused, never read as \"\""
    );
    assert_eq!(
        super::MATH_LAYER_SOURCES,
        [
            format!("{}*.rs", super::BOX_CODE),
            format!("{}*.rs", super::ENGINE_CODE)
        ],
        "the pathspecs of the box and engine directories"
    );
}

#[test]
fn refuses_raw_floats_libm_and_host_in_box_code() {
    let gate = |text: &str| {
        super::math_layer_gate(
            r#"{"packages":[]}"#,
            &[("crates/engine-objects/src/a.rs".to_owned(), text.to_owned())],
        )
    };
    let refused = |line: usize, word: &str| {
        Err(Failure::from(format!(
            "{MATH_LAYER}: crates/engine-objects/src/a.rs:{line}: box code names `{word}`: it \
             computes with the engine's sample types, through the math layer"
        )))
    };
    for word in super::BOX_FORBIDDEN_NAMES {
        assert_eq!(
            gate(&format!("fn a() {{}}\nlet x: {word} = y;")),
            refused(2, word),
            "{word}"
        );
    }
    assert_eq!(
        gate("let x = 1.5_f64;"),
        refused(1, "5_f64"),
        "a suffixed literal"
    );
    assert_eq!(
        gate("let x = 2f32;"),
        refused(1, "2f32"),
        "a suffix without separator"
    );
    assert_eq!(
        gate("use engine_core::host::Raw;"),
        refused(1, "host"),
        "the host module"
    );
    assert_eq!(gate("x as f64"), refused(1, "f64"), "a cast");
    // Prose and other words are not code: comments, strings, longer names.
    for allowed in [
        "// an f64 phase\nlet a = 1;",
        "/* f32\n libm */ let a = 1;",
        "/// in f64\nfn a() {}",
        r#"let s = "f64 \" host";"#,
        "let f64x = hostname + libm_like + f32_; let n = 1.0e-3;",
        "let x = Binary64::new(440.0);",
    ] {
        assert_eq!(gate(allowed), Ok(()), "{allowed}");
    }
    assert_eq!(
        gate("/* a comment */ f64"),
        refused(1, "f64"),
        "code after a block comment"
    );
    assert_eq!(
        gate("/* x */\n// y\nlet s = \"a\nb\";\nf64"),
        refused(5, "f64"),
        "lines counted through comments and strings"
    );
    // A sample's float taken out through the host's delivery, as a box could write it.
    assert_eq!(
        gate("let p = Delivered::deliver(self.phase);")
            .map_err(|failure| (failure.summary, failure.details)),
        Err((
            format!("{MATH_LAYER}: 2 problems, listed above"),
            vec![
                "crates/engine-objects/src/a.rs:1: box code names `Delivered`: it computes \
                 with the engine's sample types, through the math layer"
                    .to_owned(),
                "crates/engine-objects/src/a.rs:1: box code names `deliver`: it computes with \
                 the engine's sample types, through the math layer"
                    .to_owned(),
            ]
        )),
        "the delivery of samples to a host"
    );
    assert_eq!(
        super::math_layer_gate(
            r#"{"packages":[]}"#,
            &[
                (
                    "crates/engine-objects/src/osc.rs".to_owned(),
                    "#[cfg(test)]\n#[path = \"osc_tests.rs\"]\nmod tests;\n".to_owned()
                ),
                (
                    "crates/engine-objects/src/osc_tests.rs".to_owned(),
                    "let x: f64 = libm::sin(1.0);".to_owned()
                )
            ],
        ),
        Ok(()),
        "test files are not box code"
    );
}

#[test]
fn reads_char_literals_and_raw_strings_as_prose() {
    let gate = |text: &str| {
        super::math_layer_gate(
            r#"{"packages":[]}"#,
            &[("crates/engine-objects/src/a.rs".to_owned(), text.to_owned())],
        )
    };
    let refused = |line: usize, word: &str| {
        Err(Failure::from(format!(
            "{MATH_LAYER}: crates/engine-objects/src/a.rs:{line}: box code names `{word}`: it \
             computes with the engine's sample types, through the math layer"
        )))
    };
    // Literals that hold a quote: a char literal and raw strings are not strings to the end
    // of the file, and the code after them is read.
    for (text, line) in [
        ("let q = '\"'; let x: f64 = 2.0;", 1),
        ("let q = '\\''; let x: f64 = 2.0;", 1),
        ("let q = '\\\\'; let s = \"\"; f64", 1),
        ("let q = '\\u{22}'; f64", 1),
        ("fn a<'a>(x: &'a u8) {}\nf64", 2),
        ("let s = r\"\\\"; f64", 1),
        ("let s = r#\"a\"b\"#; let t = \"\";\nf64", 2),
        ("let s = br##\"a\"#b\n\"##; f64", 2),
        ("let s = cr#\"a\"#; f64", 1),
        ("let a = 1; /*/ f64 */ f64", 1),
        ("let a = 1; /**/ f64", 1),
        ("let q = '\\\"'; f64", 1),
        ("let a = ['\\x41','\"', f64];", 1),
        ("fn a<'f64>() {}", 1),
        ("let r#f64 = 1;", 1),
    ] {
        let found = gate(text);
        assert_eq!(found, refused(line, "f64"), "{text}");
    }
    for allowed in [
        "let q = '\"'; let s = \"f64\";",
        "let s = r#\"f64 \" libm\"#;",
        "let s = r\"f64\";",
        "let r#type = 1; let s = \"f64\";",
        "let s = \"unterminated f64",
        "let q = '\\",
        "/* unterminated f64",
        "let s = r#\"unterminated f64",
        "/* a/b f64 */ let a = 1;",
        "/* a* f64 */ let a = 1;",
    ] {
        assert_eq!(gate(allowed), Ok(()), "{allowed}");
    }
}

#[test]
fn refuses_sums_outside_the_math_layer() {
    let gate = |path: &str, text: &str| {
        super::math_layer_gate(r#"{"packages":[]}"#, &[(path.to_owned(), text.to_owned())])
    };
    let refused = |path: &str, line: usize, word: &str| {
        Err(Failure::from(format!(
            "{MATH_LAYER}: {path}:{line}: `{word}` is an iterator reduction outside the math \
             layer: an iterator sums samples only through ordered_sum, in the reduction order"
        )))
    };
    let (boxes, engine) = ("crates/engine-objects/src/a.rs", super::ENGINE_ROOT);
    for path in [boxes, engine] {
        for word in super::REDUCTION_NAMES {
            assert_eq!(
                gate(path, &format!("let a = 1;\nlet t = x.iter().{word}(y);")),
                refused(path, 2, word),
                "{path}: {word}"
            );
        }
        assert_eq!(
            gate(
                path,
                "let t = ordered_sum(x); // a fold, a sum\nlet s = \"sum\";"
            ),
            Ok(()),
            "{path}: the helper, prose and strings"
        );
    }
    assert_eq!(
        gate(
            engine,
            "#[expect(clippy::arithmetic_side_effects, reason = \"x\")]"
        ),
        Err(Failure::from(format!(
            "{MATH_LAYER}: {engine}:1: engine code names `arithmetic_side_effects`: it does \
             no arithmetic on a sample type outside the math layer"
        ))),
        "arithmetic on a generic sample type in the engine"
    );
    assert_eq!(
        gate(
            boxes,
            "#[expect(clippy::arithmetic_side_effects, reason = \"x\")]"
        ),
        Ok(()),
        "a box computes on its samples"
    );
    assert_eq!(
        gate(engine, "let x: f64 = host::Raw::raw(y);"),
        Ok(()),
        "the engine core names floats and its host module"
    );
    let anything = "let t = x.iter().fold(0, f).sum(); arithmetic_side_effects; f64";
    assert_eq!(
        gate("crates/engine-core/src/math.rs", anything),
        Ok(()),
        "the math layer"
    );
    for (declaring, exempt, name) in [
        (
            "crates/engine-core/src/plan.rs",
            "crates/engine-core/src/plan_tests.rs",
            "plan",
        ),
        (
            "crates/engine-objects/src/osc.rs",
            "crates/engine-objects/src/osc_tests.rs",
            "osc",
        ),
    ] {
        let declaration = format!(
            "{}\n#[cfg(test)]\n#[path = \"{name}_tests.rs\"]\nmod tests;\n",
            super::ENGINE_ARITHMETIC_FORBID
        );
        assert_eq!(
            super::math_layer_gate(
                r#"{"packages":[]}"#,
                &[
                    (declaring.to_owned(), declaration),
                    (exempt.to_owned(), anything.to_owned())
                ]
            ),
            Ok(()),
            "{exempt}: declared test code"
        );
    }
    assert_eq!(
        gate("crates/engine-patch/src/lib.rs", "let a = 1;"),
        Err(Failure::from(format!(
            "{MATH_LAYER}: crates/engine-patch/src/lib.rs: not a source the math layer gate \
             reads"
        ))),
        "a source of another crate"
    );
}

#[test]
fn requires_the_compiler_to_refuse_float_and_generic_arithmetic() {
    let none = r#"{"packages":[]}"#;
    let gate = |sources: &[(&str, &str)]| {
        let owned: Vec<(String, String)> = sources
            .iter()
            .map(|(path, text)| ((*path).to_owned(), (*text).to_owned()))
            .collect();
        super::math_layer_gate(none, &owned)
    };
    let (float, arithmetic) = (super::BOX_FLOAT_FORBID, super::ENGINE_ARITHMETIC_FORBID);
    assert_eq!(
        gate(&[
            (super::BOX_ROOT, &format!("#![no_std]\n{float}\n")),
            ("crates/engine-objects/src/osc.rs", "fn f() {}\n"),
            (
                "crates/engine-core/src/plan.rs",
                &format!("//! Plans.\n\n{arithmetic}\n")
            ),
            (super::ENGINE_ROOT, "mod plan;\n"),
            (
                super::MATH_LAYER_FILE,
                "#[expect(clippy::arithmetic_side_effects)]\n"
            ),
        ]),
        Ok(()),
        "each forbid where it belongs, the root and the math layer apart"
    );
    assert_eq!(
        gate(&[(
            super::BOX_ROOT,
            "#![no_std]\n#![forbid(clippy::float_arithmetic)]\n"
        )]),
        failed(format!(
            "{MATH_LAYER}: {} does not hold `{float}`",
            super::BOX_ROOT
        )),
        "the box root without its exact line"
    );
    assert_eq!(
        gate(&[("crates/engine-core/src/render.rs", "fn f() {}\n")]),
        failed(format!(
            "{MATH_LAYER}: crates/engine-core/src/render.rs does not hold `{arithmetic}`"
        )),
        "an engine module without its line"
    );
    for word in super::ENGINE_FORBIDDEN_NAMES {
        assert_eq!(
            gate(&[(
                super::ENGINE_ROOT,
                &format!("#![expect(clippy::{word}, reason = \"r\")]\n")
            )]),
            failed(format!(
                "{MATH_LAYER}: {}:1: engine code names `{word}`: it does no arithmetic on a \
                 sample type outside the math layer",
                super::ENGINE_ROOT
            )),
            "{word} in the root, which cannot forbid it"
        );
    }
}

#[test]
fn reads_every_source_of_box_and_engine_code() {
    let gate = |metadata: &str, sources: &[(&str, &str)]| {
        let owned: Vec<(String, String)> = sources
            .iter()
            .map(|(path, text)| ((*path).to_owned(), (*text).to_owned()))
            .collect();
        super::math_layer_gate(metadata, &owned)
    };
    let none = r#"{"packages":[]}"#;
    let declared = "#[cfg(test)]\n#[path = \"a_tests.rs\"]\nmod tests;\n";
    let words = |path: &str, line: usize, word: &str| {
        format!(
            "{path}:{line}: names `{word}` outside a test declaration: code comes into these \
             crates by `mod` alone, from files the gate reads"
        )
    };
    let boxes = "crates/engine-objects/src/a.rs";
    for (text, line, word) in [
        ("fn f() {}\ninclude!(\"../../x.rs\");\n", 2, "include"),
        ("#[path = \"other.rs\"]\nmod other;\n", 1, "path"),
        (
            "#[cfg_attr(test, path = \"a_tests.rs\")]\nmod tests;\n",
            1,
            "path",
        ),
        (
            "#[cfg(not(test))]\n#[path = \"a_tests.rs\"]\nmod tests;\n",
            2,
            "path",
        ),
        (
            "#[cfg(test)]\n#[path = \"../x/a_tests.rs\"]\nmod tests;\n",
            2,
            "path",
        ),
    ] {
        assert_eq!(
            gate(none, &[(boxes, text)]),
            failed(format!("{MATH_LAYER}: {}", words(boxes, line, word))),
            "{text}"
        );
    }
    assert_eq!(
        gate(none, &[(super::MATH_LAYER_FILE, "include!(\"x.rs\");\n")]),
        failed(format!(
            "{MATH_LAYER}: {}",
            words(super::MATH_LAYER_FILE, 1, "include")
        )),
        "the math layer brings no code in either"
    );
    let undeclared = "crates/engine-objects/src/a_tests.rs: a *_tests.rs file not declared \
                      once as test code beside it, `#[cfg(test)] #[path = \"<name>_tests.rs\"] \
                      mod <module>;`";
    assert_eq!(
        gate(
            none,
            &[
                (boxes, declared),
                ("crates/engine-objects/src/a_tests.rs", "f64")
            ]
        ),
        Ok(()),
        "a declared test file"
    );
    for sources in [
        vec![("crates/engine-objects/src/a_tests.rs", "f64")],
        vec![
            (boxes, declared),
            ("crates/engine-objects/src/b.rs", declared),
            ("crates/engine-objects/src/a_tests.rs", "f64"),
        ],
        vec![
            ("crates/engine-objects/src/sub/a.rs", declared),
            ("crates/engine-objects/src/a_tests.rs", "f64"),
        ],
    ] {
        assert_eq!(
            gate(none, &sources),
            failed(format!("{MATH_LAYER}: {undeclared}")),
            "not declared once beside it: {sources:?}"
        );
    }
    let build = |name: &str| {
        format!(
            r#"{{"packages":[{{"name":"{name}","dependencies":[],"targets":[{{"kind":["lib"]}},{{"kind":["custom-build"]}}]}}]}}"#
        )
    };
    for name in ["engine-objects", "engine-core"] {
        assert_eq!(
            gate(&build(name), &[]),
            failed(format!(
                "{MATH_LAYER}: {name} has a build script: code the math layer gate does not read"
            )),
            "{name}"
        );
    }
    assert_eq!(
        gate(&build("engine-cli"), &[]),
        Ok(()),
        "another crate may build"
    );
}

#[test]
fn reads_what_real_cargo_metadata_says_of_libm_and_build_scripts() {
    let (metadata, _) = fixture_metadata();
    assert_eq!(
        super::math_layer_gate(&metadata, &[]),
        found(
            MATH_LAYER,
            &[
                "engine-objects has a build script: code the math layer gate does not read"
                    .to_owned(),
                "engine-cli depends on libm: its arithmetic must go through the math layer of \
                 engine-core"
                    .to_owned(),
            ]
        ),
        "a dependency renamed, a build script; libm in the core and the measurements, and in \
         the tests of the boxes"
    );
}
#[test]
fn reviews_the_dependencies_real_cargo_metadata_lists() {
    let (metadata, _) = fixture_metadata();
    let reviewed = "engine-core: libm (normal) -- r\nengine-testkit: libm (normal) -- r\n\
                    engine-testkit: sha2 (normal) -- r\nengine-objects: libm (dev) -- r\n\
                    engine-cli: libm (normal) -- r\nengine-cli: serde (build) -- r\n\
                    engine-hash: serde_json (normal) -- r\nengine-hash: sha2 (normal) -- r\n";
    assert_eq!(
        super::dependency_gate(&metadata, &[], reviewed),
        Ok(()),
        "every kind, the crates of the workspace apart, a renamed one by its name"
    );
}
/// The context of the pin gate's verdict.
const PINNED: &str = "bit-defining dependencies pinned to one exact version";

/// What the pin gate says of a dependency `key` required as `req`.
fn unpinned(key: &str, req: &str) -> String {
    format!(
        "{key} is required as `{req}`: pin it to one version, `=X.Y.Z`, in its Cargo.toml, so \
         that an update, which can move bits, is decided alone (spec/engine-design.md, \
         decision 29)"
    )
}

/// What the pin gate says of a dependency `key` of the library not reviewed as bit-defining.
fn unmarked(key: &str) -> String {
    format!(
        "{key}: its code runs in the audio library's: review it in dependencies.txt with a \
         reason starting `bit-defining: `"
    )
}

#[test]
fn pins_the_bit_defining_dependencies_real_cargo_metadata_lists() {
    let (metadata, _) = fixture_metadata();
    let reviewed = "# engine-cli: libm (normal) -- bit-defining: in a comment\n\
                    engine-core: libm (normal) -- bit-defining: the math layer\n\
                    engine-testkit: libm (normal) -- the measurements\n\
                    engine-testkit: sha2 (normal) -- bit-defining: the fingerprints\n\
                    engine-hash: serde_json (normal) -- reads JSON\n\
                    engine-hash: sha2 (normal) -- bit-defining: the render hash\n";
    assert_eq!(
        super::pin_gate(&metadata, reviewed),
        found(
            PINNED,
            &[
                unpinned("engine-testkit: sha2 (normal)", "^0.11"),
                unmarked("engine-hash: serde_json (normal)"),
                unpinned("engine-hash: serde_json (normal)", "^1"),
            ]
        ),
        "a reviewed one unpinned, one of the library neither reviewed so nor pinned; the \
         library's tests, another crate's and a comment apart"
    );
}

#[test]
fn reads_one_exact_version_only() {
    let gate = |req: &str| {
        super::pin_gate(
            &format!(
                r#"{{"packages":[{{"name":"engine-core","dependencies":[{{"name":"libm","kind":"build","req":"{req}"}}]}}]}}"#
            ),
            "engine-core: libm (build) -- bit-defining: the math layer\n",
        )
    };
    assert_eq!(gate("=0.2.16"), Ok(()), "one version, a build dependency");
    assert_eq!(gate("=10.20.300"), Ok(()), "numbers of several digits");
    for req in [
        "0.2.16",
        "^0.2.16",
        ">=0.2.16",
        "=0.2",
        "=0.2.16.1",
        "=0.2.x",
        "=0..16",
        "=.2.16",
        "=0.2.",
        "=0.2.16-rc1",
        "= 0.2.16",
        "",
    ] {
        assert_eq!(
            gate(req),
            found(PINNED, &[unpinned("engine-core: libm (build)", req)]),
            "{req}"
        );
    }
    let missing = super::pin_gate(
        r#"{"packages":[{"name":"engine-core","dependencies":[{"name":"libm"}]}]}"#,
        "engine-core: libm (normal) -- reads\n",
    );
    assert_eq!(
        missing,
        found(
            PINNED,
            &[
                unmarked("engine-core: libm (normal)"),
                unpinned("engine-core: libm (normal)", "")
            ]
        ),
        "no kind, no requirement"
    );
    assert_eq!(
        super::pin_gate(
            r#"{"packages":[{"name":"engine-cli","dependencies":[{"name":"x","req":"1"}]}]}"#,
            "engine-cli: x (normal) --bit-defining: no space\nengine-cli: x (normal)  -- reads\n"
        ),
        Ok(()),
        "a crate outside the library, a reason not starting with the mark"
    );
    assert!(
        super::pin_gate("not json", "")
            .is_err_and(|failure| failure.summary.starts_with("invalid cargo metadata: ")),
        "the metadata's error"
    );
}

/// The context of the library gate's verdict.
const LIBRARY: &str =
    "the audio library depends only on its own crates, its tests also on its test crates";

/// What the library gate says of `library` depending on `name` (`kind`, at `path`).
fn outside_library(library: &str, name: &str, kind: &str, path: &str) -> String {
    format!(
        "{library} depends on {name} ({kind}, {path}): the audio library depends only on its \
         own crates (engine-core, engine-objects, engine-patch, engine-hash, engine-fpenv, \
         engine-wasm), its tests also on engine-testkit; move what it needs into the library, \
         or the dependency out of it"
    )
}

#[test]
fn keeps_the_library_apart_in_real_cargo_metadata() {
    let (metadata, root) = fixture_metadata();
    let cli = format!("{root}/crates/engine-cli");
    assert_eq!(
        super::library_gate(&metadata),
        found(
            LIBRARY,
            &[
                outside_library("engine-objects", "engine-cli", "normal", &cli),
                outside_library("engine-wasm", "engine-cli", "dev", &cli),
            ]
        ),
        "the native shell in the code and in the tests; the test bench in the tests, the \
         library's own crates and third-party crates allowed"
    );
}

#[test]
fn refuses_a_library_that_reaches_outside_itself() {
    let packages = |core: &str| {
        let others = r#"{"name":"engine-objects"},{"name":"engine-patch"},{"name":"engine-hash"},{"name":"engine-fpenv"},"#;
        format!(
            r#"{{"packages":[{others}{{"name":"engine-wasm"}},{{"name":"engine-core","dependencies":[{core}]}}]}}"#
        )
    };
    let dependency = |name: &str, kind: &str, path: &str| {
        format!(r#"{{"name":"{name}","kind":{kind},"path":"{path}"}}"#)
    };
    let gate = |core: &str| super::library_gate(&packages(core));
    for allowed in [
        dependency("engine-hash", "null", "/r/crates/engine-hash"),
        dependency("engine-wasm", "\"build\"", "/r/bindings/wasm"),
        dependency("engine-testkit", "\"dev\"", "/r/crates/engine-testkit"),
        r#"{"name":"serde","kind":null}"#.to_owned(),
    ] {
        assert_eq!(gate(&allowed), Ok(()), "{allowed}");
    }
    for (name, kind, path, said) in [
        (
            "engine-testkit",
            "null",
            "/r/crates/engine-testkit",
            "normal",
        ),
        (
            "engine-testkit",
            "\"build\"",
            "/r/crates/engine-testkit",
            "build",
        ),
        (
            "engine-alloc-counter",
            "\"dev\"",
            "/r/crates/engine-alloc-counter",
            "dev",
        ),
        ("xtask", "null", "/r/crates/xtask", "normal"),
        ("engine-hash", "null", "/r/apps/engine-hash", "normal"),
        ("engine-hash", "null", "/r/crates/engine-hashx", "normal"),
        (
            "engine-testkit",
            "\"dev\"",
            "/r/tooling/engine-testkit",
            "dev",
        ),
        ("player", "null", "/r/apps/player", "normal"),
    ] {
        assert_eq!(
            gate(&dependency(name, kind, path)),
            found(LIBRARY, &[outside_library("engine-core", name, said, path)]),
            "{name} {kind} {path}"
        );
    }
    assert_eq!(
        super::library_gate(r#"{"packages":[{"name":"engine-core"}]}"#),
        found(
            LIBRARY,
            &[
                "engine-objects",
                "engine-patch",
                "engine-hash",
                "engine-fpenv",
                "engine-wasm"
            ]
            .map(|name| {
                format!("{name}: not in the cargo metadata, so its dependencies cannot be checked")
            })
        ),
        "a crate of the library missing"
    );
    assert!(
        super::library_gate("{}")
            .is_err_and(|failure| failure.summary == "cargo metadata lists no packages"),
        "the metadata's error"
    );
}

/// The context of the citation gate's verdict.
const CITATIONS: &str = "every roadmap step cited exists on the board";

/// A board of two cards: #12 of three steps, #3 of one.
const BOARD: &str =
    r#"{"version":1,"cards":[{"id":12,"tasks":[{},{},{}]},{"id":3,"tasks":[{"title":"x"}]}]}"#;

/// The citation gate on the board [`BOARD`] and one file, `x.md`, holding `text`. A card is
/// written `\u{23}N` in this file, so that the gate, reading it, sees no citation here.
fn cite(text: &str) -> Result<(), Failure> {
    super::citation_gate(&[
        (super::ROADMAP_BOARD.to_owned(), BOARD.to_owned()),
        ("x.md".to_owned(), text.to_owned()),
    ])
}

#[test]
fn accepts_the_steps_a_board_has() {
    for text in [
        "\u{23}12 step 3",
        "see \u{23}12 steps 1 and 2.",
        "\u{23}12, step 1",
        "(\u{23}12 (step 2 of it)",
        "\u{23}12 steps 1, 2, and 3",
        "\u{23}12 steps 1, 2 and 3",
        "\u{23}12 steps 1 to 3",
        "\u{23}12 steps 2 or 3; \u{23}3 step 1",
        "**Step.** \u{23}12 step 1 (with step 3). **Order:** 3, before step 2.",
        "  **Step.** Engine: \u{23}12 step 3; format: \u{23}3 step 1, the port at step 1.",
        "**Step.** \u{23}12 step 1; substep 9, x-step 9.",
        "card \u{23}1 (entry 1) step 9",
        "C\u{23}12 step 9, &\u{23}12 step 9, x\u{23}12 step 9",
        "step 9 of the plan, steps 9 and 10",
        "\u{23}12, ( step 9",
        "\u{23}99, \u{23}12 Steps 9, \u{23}12 stepping 9, \u{23}12 step",
        "",
    ] {
        assert_eq!(cite(text), Ok(()), "{text}");
    }
    assert_eq!(
        super::citation_gate(&[(super::ROADMAP_BOARD.to_owned(), BOARD.to_owned())]),
        Ok(()),
        "the board alone"
    );
}

#[test]
fn refuses_a_step_the_board_does_not_have() {
    let past = |step: u64| {
        format!(
            "x.md:1: cites \u{23}12 step {step}, but \u{23}12 has 3 steps: cite one of them, \
             or add it (`pnpm roadmap step add 12 …`)"
        )
    };
    for (text, problems) in [
        ("\u{23}12 step 4", vec![past(4)]),
        ("\u{23}12 step 0", vec![past(0)]),
        ("\u{23}12 steps 2 to 5", vec![past(4), past(5)]),
        ("\u{23}12 steps 1, 4 and 9", vec![past(4), past(9)]),
        ("\u{23}12 steps 1, and 4", vec![past(4)]),
        ("\u{23}12 steps 1 or 4", vec![past(4)]),
        (
            "\u{23}12 step 18446744073709551615",
            vec![past(18_446_744_073_709_551_615)],
        ),
        (
            "\u{23}7 step 1",
            vec!["x.md:1: cites \u{23}7 step 1, but the board has no card \u{23}7".to_owned()],
        ),
        ("**Step.** \u{23}12 step 1 (with step 4).", vec![past(4)]),
        ("**Step.** \u{23}12 step 1 (step 4).", vec![past(4)]),
        (
            "**Step.** \u{23}3 step 1, then \u{23}12: at step 4.",
            vec![past(4)],
        ),
        ("\u{23}12 step 3 \u{23}12 step 4 step 5", vec![past(4)]),
        ("\u{23}12  step 4", vec![past(4)]),
        ("\u{23}12,step 4", vec![past(4)]),
        ("\u{23}12step 4", vec![past(4)]),
    ] {
        let problems: Vec<String> = problems;
        assert_eq!(cite(text), found(CITATIONS, &problems), "{text}");
    }
    assert_eq!(
        cite("\u{23}12 step 1\n\n\u{23}12 step 4\n"),
        found(CITATIONS, &[past(4).replace("x.md:1:", "x.md:3:")]),
        "the line named"
    );
}

#[test]
fn refuses_a_citation_it_cannot_read() {
    let at = |problem: &str| format!("x.md:1: {problem}");
    for (text, problem) in [
        ("\u{23}12 step one", "a step without its number"),
        ("\u{23}12, steps x", "a step without its number"),
        (
            "\u{23}12 step 99999999999999999999",
            "a step without its number",
        ),
        (
            "\u{23}12 steps 3 to 1",
            "the range of steps 3 to 1 does not rise",
        ),
        (
            "\u{23}12 steps 2 to 2",
            "the range of steps 2 to 2 does not rise",
        ),
        (
            "\u{23}12 steps 1-3",
            "a range of steps from 1 written with a dash: write `1 to N`",
        ),
        (
            "\u{23}12 steps 1 and 2–3",
            "a range of steps from 2 written with a dash: write `2 to N`",
        ),
        (
            "**Step.** step 2, then \u{23}12 step 1.",
            "a step cited without its card: write `\u{23}N step 2`",
        ),
        (
            "**Step.** Engine block.",
            "a **Step.** line citing no roadmap step: write `\u{23}N step M`",
        ),
        (
            "**Step.** \u{23}12 steps 3 to 1.",
            "the range of steps 3 to 1 does not rise",
        ),
        (
            "see \u{23}12 item 6",
            "\u{23}12 item 6 cites no step: write `\u{23}12 step 6`",
        ),
        (
            "\u{23}12 items 6 and 7",
            "\u{23}12 items 6 cites no step: write `\u{23}12 step 6`",
        ),
        (
            "\u{23}12, task 3",
            "\u{23}12 task 3 cites no step: write `\u{23}12 step 3`",
        ),
        (
            "(\u{23}12 (tasks 2 to 3",
            "\u{23}12 tasks 2 cites no step: write `\u{23}12 step 2`",
        ),
    ] {
        assert_eq!(cite(text), found(CITATIONS, &[at(problem)]), "{text}");
    }
    assert_eq!(cite("\u{23}12 steps 1-x"), Ok(()), "a dash, then no number");
    for text in [
        "\u{23}12 itemized 6",
        "\u{23}12 items x",
        "\u{23}12 item",
        "\u{23}12 tasks of it",
        "every item 6",
    ] {
        assert_eq!(cite(text), Ok(()), "not a citation: {text}");
    }
}

#[test]
fn reads_the_board_as_the_roadmap_writes_it() {
    let gate = |board: &str| {
        super::citation_gate(&[(super::ROADMAP_BOARD.to_owned(), board.to_owned())])
            .map_err(|failure| failure.summary)
    };
    assert!(
        gate("{").is_err_and(|summary| summary
            .starts_with("cannot read the roadmap board apps/roadmap/board.json: ")),
        "not JSON"
    );
    for board in ["{}", r#"{"cards":{}}"#] {
        assert_eq!(
            gate(board),
            Err("apps/roadmap/board.json lists no cards (see: pnpm roadmap validate)".to_owned()),
            "{board}"
        );
    }
    for board in [
        r#"{"cards":[{"tasks":[]}]}"#,
        r#"{"cards":[{"id":"12","tasks":[]}]}"#,
        r#"{"cards":[{"id":12}]}"#,
        r#"{"cards":[{"id":12,"tasks":{}}]}"#,
    ] {
        assert_eq!(
            gate(board),
            Err(
                "apps/roadmap/board.json holds a card without its number or its steps (see: \
                 pnpm roadmap validate)"
                    .to_owned()
            ),
            "{board}"
        );
    }
    assert_eq!(
        super::citation_gate(&[("x.md".to_owned(), String::new())])
            .map_err(|failure| failure.summary),
        Err("cannot read the roadmap board apps/roadmap/board.json".to_owned()),
        "no board"
    );
    assert_eq!(
        gate(r#"{"cards":[{"id":12,"tasks":[]}]}"#),
        Ok(()),
        "a card without steps"
    );
}

#[test]
fn refuses_workflows_that_could_pass_without_checking() {
    let gate = |workflows: &[&str], workflow: &str| {
        let workflows: Vec<String> = workflows.iter().map(|path| (*path).to_owned()).collect();
        super::workflow_gate(&workflows, workflow, "22.23.4")
    };
    let only = [super::WORKFLOW];
    let refused = |workflow: &str, problem: &str| {
        assert_eq!(
            gate(&only, workflow),
            failed(format!(
                ".github/workflows/quality.yml can change CI without a commit: {problem}"
            )),
            "{problem}"
        );
    };
    refused(
        &PINNED_WORKFLOW.replace(
            "      - uses: Swatinem/rust-cache@d1527c8f67ef9af357290475d4af4b0fab8f12e0 # v2.9.3",
            "      -   uses: Swatinem/rust-cache@main",
        ),
        "Swatinem/rust-cache@main is not pinned to a full commit hash",
    );
    refused(
        &PINNED_WORKFLOW.replace(
            "      - uses: Swatinem/rust-cache@d1527c8f67ef9af357290475d4af4b0fab8f12e0 # v2.9.3",
            "      - { uses: Swatinem/rust-cache@main }",
        ),
        "Swatinem/rust-cache@main is not pinned to a full commit hash",
    );
    refused(
        &PINNED_WORKFLOW.replace(
            "        run: cargo xtask check --full --part final\n",
            "        run: cargo xtask check --full --part final\n        continue-on-error: true\n",
        ),
        "`continue-on-error: true`: `continue-on-error` is not allowed",
    );
    refused(
        &PINNED_WORKFLOW.replace(
            "      - name: The final part\n",
            "      - name: The final part\n        if: false\n",
        ),
        "`if: false`: the only conditions are `if: failure()` and `if: github.event_name != \'push\' || contains(github.event.head_commit.message, \'[ci]\')`",
    );
    refused(
        &PINNED_WORKFLOW.replace("nightly-2026-11-01", "nightly-2020-01-01"),
        "`nightly-2020-01-01` is not the pinned nightly, nightly-2026-11-01",
    );
    refused(
        &PINNED_WORKFLOW.replace(
            "        run: cargo xtask check --full --part final\n",
            "        run: cargo xtask check --full --part final\n        env:\n          RUSTFLAGS: -Aunsafe\n",
        ),
        "`RUSTFLAGS: -Aunsafe`: `RUSTFLAGS` is not allowed",
    );
    refused(
        &PINNED_WORKFLOW.replace("          persist-credentials: false\n", ""),
        ".github/workflows/quality.yml lacks `persist-credentials: false`",
    );
    assert_eq!(
        gate(
            &only,
            &PINNED_WORKFLOW
                .replace("  pull_request:\n", "  pull_request_target:\n")
                .replace("  contents: read", "  contents: write")
        ),
        failed(
            ".github/workflows/quality.yml can change CI without a commit: 3 problems, listed above\n\
             `pull_request_target:`: `pull_request_target` is not allowed\n\
             .github/workflows/quality.yml lacks the triggers `on:   push:   pull_request:   workflow_dispatch:`\n\
             .github/workflows/quality.yml lacks the permissions `permissions:   contents: read`"
        ),
        "a trigger running fork code with the token, a token that writes"
    );
    assert_eq!(
        gate(
            &only,
            &format!(
                "{PINNED_WORKFLOW}  other:\n    {CI_CONDITION}\n    permissions:\n      contents: write\n"
            )
        ),
        failed(
            ".github/workflows/quality.yml can change CI without a commit: .github/workflows/quality.yml \
             declares triggers and permissions once, for the whole workflow"
        ),
        "a job that widens the permissions"
    );
    assert_eq!(
        gate(
            &[super::WORKFLOW, ".github/workflows/other.yml"],
            PINNED_WORKFLOW
        ),
        failed(
            ".github/workflows/quality.yml can change CI without a commit: \
             .github/workflows/other.yml: the only workflow is .github/workflows/quality.yml, \
             which xtask checks"
        ),
        "a second workflow, which nothing checks"
    );
}

#[test]
fn requires_a_week_before_a_release_and_libm_without_features() {
    let files = |dependabot: &str, pnpm: &str, deny: &str| {
        [dependabot, pnpm, deny].map(str::to_owned).to_vec()
    };
    assert_eq!(
        super::supply_chain_gate(&files(DEPENDABOT, "minimumReleaseAge: 10080\n", DENY)),
        Ok(()),
        "every setting"
    );
    assert_eq!(
        super::supply_chain_gate(&files(
            &DEPENDABOT.replacen("    cooldown:\n      default-days: 7\n", "", 1),
            "minimumReleaseAge: 1440\n",
            &DENY.replace("exact = true\n", "")
        )),
        found(
            "the supply-chain settings",
            &[
                ".github/dependabot.yml (cargo) lacks `cooldown: default-days: 7`".to_owned(),
                "pnpm-workspace.yaml lacks `minimumReleaseAge: 10080`".to_owned(),
                "deny.toml lacks `[[bans.features]] crate = \"libm\" exact = true allow = []`"
                    .to_owned(),
            ]
        ),
        "an ecosystem without its cooldown, a shorter age, features allowed"
    );
    assert_eq!(
        super::supply_chain_gate(&files("version: 2\n", "minimumReleaseAge: 10080\n", DENY)),
        failed("the supply-chain settings: .github/dependabot.yml configures nothing"),
        "no ecosystem"
    );
}

#[test]
fn requires_every_crate_to_take_the_workspace_repository() {
    let packages = package_jsons();
    let crates: Vec<(String, String)> = super::RUST_CRATES
        .iter()
        .map(|krate| {
            let text = if krate.name == "engine-core" {
                MEMBER.replace(
                    "repository.workspace = true",
                    "repository = \"https://example.invalid/poyetik\"",
                )
            } else {
                MEMBER.to_owned()
            };
            (format!("{}/Cargo.toml", krate.dir), text)
        })
        .collect();
    assert_eq!(
        membership_gate(&packages, &crates, &[]),
        found(
            "the manifests differ from what xtask knows",
            &["crates/engine-core/Cargo.toml does not take the workspace repository".to_owned()]
        ),
        "a repository of its own"
    );
}

#[test]
fn empties_stale_build_outputs_before_the_builds_that_write_them() {
    let steps = check_steps(CHANGES_MODE);
    assert_eq!(
        super::build_outputs(),
        vec!["bindings/wasm/dist".to_owned()],
        "the outputs of tsc, which never removes a stale one"
    );
    for package in PACKAGES {
        for output in package.outputs {
            let path = format!("{}/{output}", package.dir);
            let emptied = steps.iter().position(
                |step| matches!(&step.action, Action::FreshDirs(paths) if paths.contains(&path)),
            );
            let built = steps.iter().position(|step| step.name == package.step);
            assert!(
                emptied.is_some() && emptied < built,
                "{path} is emptied before {} runs",
                package.step
            );
        }
    }
}

#[test]
fn runs_miri_on_every_crate_of_the_audited_files() {
    assert_eq!(
        super::unsafe_crates(),
        vec!["engine-alloc-counter", "engine-wasm"],
        "the crates of the audited files, in the order of RUST_CRATES, but the assembly one"
    );
    assert_eq!(
        super::ASSEMBLY_FILES,
        ["crates/engine-fpenv/src/word.rs"],
        "the one file Miri cannot run"
    );
    assert!(
        super::ASSEMBLY_FILES
            .iter()
            .all(|file| UNSAFE_FILES.contains(file)),
        "an audited file"
    );
    assert_eq!(
        super::miri_command().line(),
        format!(
            "cargo +nightly-2026-11-01 {} test --package engine-alloc-counter --package \
             engine-wasm --locked -- --include-ignored",
            "miri"
        ),
        "Miri on each of them, ignored tests included"
    );
    assert!(
        check_steps(CHANGES_MODE)
            .iter()
            .any(|step| step.action == Action::Run(super::miri_command())),
        "a step of the check"
    );
}

#[test]
fn empties_the_coverage_build_before_every_measurement() {
    let mut fake = passing();
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut fake, &mut Vec::new()),
        Ok(()),
        "all green"
    );
    let clean = Coverage::clean_command().line();
    let measurements: Vec<usize> = fake
        .ran
        .iter()
        .enumerate()
        .filter(|(_, line)| line.contains("llvm-cov --manifest-path"))
        .map(|(index, _)| index)
        .collect();
    assert_eq!(
        measurements.len(),
        13 + 1,
        "every crate and binary measured, and the locality fixture"
    );
    for index in measurements {
        assert_eq!(
            fake.ran.get(index.wrapping_sub(1)),
            Some(&clean),
            "a measurement right after a clean coverage build"
        );
    }
}

#[test]
fn stops_when_the_coverage_build_cannot_be_emptied() {
    let mut fake = passing();
    fake.broken = Some(Coverage::clean_command().line());
    let failure =
        run_steps(&check_steps(CHANGES_MODE), &mut fake, &mut Vec::new()).expect_err("stops");
    assert!(
        failure.summary.contains("not found"),
        "the reason the clean failed: {}",
        failure.summary
    );
    assert!(
        fake.ran
            .iter()
            .all(|line| !line.contains("llvm-cov --manifest-path")),
        "nothing is measured on a build that could not be emptied"
    );
}

#[test]
fn fails_when_a_supply_chain_setting_cannot_be_read() {
    let mut fake = passing();
    let (path, _) = super::SUPPLY_CHAIN_SETTINGS[1];
    let _removed = fake.files.remove(path);
    let failure =
        run_steps(&check_steps(CHANGES_MODE), &mut fake, &mut Vec::new()).expect_err("stops");
    assert!(
        failure.summary.contains(path),
        "names the file: {}",
        failure.summary
    );
}

#[test]
fn fails_when_the_crates_cannot_be_listed() {
    let mut fake = passing();
    let _removed = fake
        .files
        .remove(&format!("capture {}", super::metadata_command().line()));
    let failure =
        run_steps(&check_steps(CHANGES_MODE), &mut fake, &mut Vec::new()).expect_err("stops");
    assert!(
        failure.summary.contains("cannot list the crates"),
        "says what it could not do: {}",
        failure.summary
    );
}

#[test]
fn requires_the_fused_build_to_have_run_with_its_option() {
    assert_eq!(
        super::fused_gate(&format!("a\n{}\nb", super::FUSED_PROOF)),
        Ok(()),
        "the proof line"
    );
    for output in [
        String::new(),
        "test result: ok. 81 passed".to_owned(),
        format!("{} extra", super::FUSED_PROOF),
        super::FUSED_PROOF.replace("... ok", "... FAILED"),
    ] {
        assert_eq!(
            super::fused_gate(&output),
            failed(format!(
                "the fused test build ran without its option: `{}` is missing from its output",
                super::FUSED_PROOF
            )),
            "{output:?}"
        );
    }
    let command = super::fused_command().line();
    assert!(
        command.contains(r#"--config build.rustflags=["--cfg", "poyetik_math=\"fused\""]"#)
            && command.contains("--target-dir target/fused")
            && command.contains("--package engine-core --package engine-objects"),
        "{command}"
    );
    let steps: Vec<Step> = check_steps(CHANGES_MODE)
        .into_iter()
        .filter(|step| step.action == Action::FusedBuild)
        .collect();
    assert_eq!(steps.len(), 1, "one fused build");
    let mut broken = passing();
    let _removed = broken
        .files
        .remove(&format!("capture {}", super::fused_command().line()));
    let failure = run_steps(&steps, &mut broken, &mut Vec::new()).expect_err("stops");
    assert!(
        failure
            .summary
            .contains(": the fused test build failed: `cargo test --locked"),
        "{}",
        failure.summary
    );
    let mut silent = passing();
    let _replaced = silent.files.insert(
        format!("capture {}", super::fused_command().line()),
        "test result: ok.\n".to_owned(),
    );
    assert!(
        run_steps(&steps, &mut silent, &mut Vec::new()).is_err(),
        "a build without the option fails"
    );
}

#[test]
fn the_math_layer_gate_fails_when_the_crates_cannot_be_listed() {
    let steps: Vec<Step> = check_steps(CHANGES_MODE)
        .into_iter()
        .filter(|step| step.action == Action::MathLayerGate)
        .collect();
    assert_eq!(steps.len(), 1, "one math layer gate");
    let mut fake = passing();
    let _removed = fake
        .files
        .remove(&format!("capture {}", super::metadata_command().line()));
    let failure = run_steps(&steps, &mut fake, &mut Vec::new()).expect_err("stops");
    assert!(
        failure.summary.contains("cannot list the crates"),
        "says what it could not do: {}",
        failure.summary
    );
    assert_eq!(
        run_steps(&steps, &mut passing(), &mut Vec::new()),
        Ok(()),
        "the fake's metadata has no libm"
    );
}

#[test]
fn refuses_a_second_place_declaring_triggers() {
    let workflow =
        format!("{PINNED_WORKFLOW}  other:\n    {CI_CONDITION}\n    on:\n      schedule:\n");
    assert_eq!(
        super::workflow_gate(&[super::WORKFLOW.to_owned()], &workflow, "22.23.4"),
        failed(
            ".github/workflows/quality.yml can change CI without a commit: .github/workflows/quality.yml \
             declares triggers and permissions once, for the whole workflow"
        ),
        "triggers declared twice"
    );
}

#[test]
fn matches_the_globs_of_stryker_mutate() {
    for (glob, path, expected) in [
        ("src/**/*.ts", "src/a.ts", true),
        ("src/**/*.ts", "src/a/b/c.ts", true),
        ("src/**/*.ts", "src/a.tsx", false),
        ("src/**/*.ts", "lib/a.ts", false),
        ("src/**/*.ts", "src", false),
        ("csp.ts", "csp.ts", true),
        ("csp.ts", "src/csp.ts", false),
        ("csp.ts", "csp.tsx", false),
        ("*.ts", "a.ts", true),
        ("*.ts", "a/b.ts", false),
        ("src/*/x.ts", "src/a/x.ts", true),
        ("src/*/x.ts", "src/a/b/x.ts", false),
        ("a*b*c.ts", "abxbc.ts", true),
        ("a*b*c.ts", "ab.ts", false),
        ("a*b*c.ts", "acb.ts", false),
        ("a*b*c.ts", "axc.ts", false),
        ("a*a", "a", false),
        ("a*a", "aa", true),
        ("**", "a/b", true),
        ("**/x.ts", "x.ts", true),
        ("e2e/**", "e2e/a/b.ts", true),
    ] {
        let pattern: Vec<&str> = glob.split('/').collect();
        let segments: Vec<&str> = path.split('/').collect();
        assert_eq!(
            super::glob_matches(&pattern, &segments),
            expected,
            "{glob} against {path}"
        );
    }
}

/// A package `p` of the scope gate: its Stryker configuration mutates `mutate`, its
/// coverage summary measures `measured` (absolute paths, below `/r/`) and its mutation
/// report lists each of them.
fn scope(dir: &'static str, mutate: &[&str], measured: &[&str]) -> super::Scope {
    let files: serde_json::Map<String, serde_json::Value> = measured
        .iter()
        .chain(&["total"])
        .map(|file| ((*file).to_owned(), serde_json::json!({})))
        .collect();
    let reported: serde_json::Map<String, serde_json::Value> = measured
        .iter()
        .filter_map(|file| file.strip_prefix(&format!("/r/{dir}/")))
        .map(|file| (file.to_owned(), serde_json::json!({ "mutants": [] })))
        .collect();
    super::Scope {
        dir,
        stryker: serde_json::json!({ "mutate": mutate }).to_string(),
        summary: serde_json::Value::Object(files).to_string(),
        mutation: serde_json::json!({ "files": reported }).to_string(),
    }
}

#[test]
fn requires_every_source_mutated_in_the_mutation_report() {
    let tracked: Vec<String> = ["p/src/a.ts", "p/src/b.ts", "p/src/c.ts", "p/src/env.d.ts"]
        .map(str::to_owned)
        .to_vec();
    let statements = |count: u64| serde_json::json!({ "statements": { "total": count } });
    let summary = serde_json::json!({
        "/r/p/src/a.ts": statements(4),
        "/r/p/src/b.ts": statements(1),
        "/r/p/src/c.ts": {},
        "/r/p/src/env.d.ts": statements(0),
        "total": statements(5),
    })
    .to_string();
    let only_a = super::Scope {
        dir: "p",
        stryker: serde_json::json!({ "mutate": ["src/**/*.ts"] }).to_string(),
        summary,
        mutation: r#"{"files":{"src/a.ts":{"mutants":[]}}}"#.to_owned(),
    };
    let context = "TypeScript sources outside coverage or mutation testing must be reviewed in \
                   proof-bypasses.txt";
    assert_eq!(
        super::scope_gate(&tracked, core::slice::from_ref(&only_a), ""),
        failed(format!(
            "{context}: 2 problems, listed above\n\
             not reviewed: p/src/b.ts: mutated, no mutant reported\n\
             not reviewed: p/src/c.ts: mutated, no mutant reported"
        )),
        "a source with statements, or whose statements are not counted, must be reported; \
         one without any (declarations only) need not"
    );
    let reviewed = "p/src/b.ts: mutated, no mutant reported -- constants only\n\
                    p/src/c.ts: mutated, no mutant reported -- constants only\n";
    assert_eq!(
        super::scope_gate(&tracked, core::slice::from_ref(&only_a), reviewed),
        Ok(()),
        "reviewed"
    );
    assert_eq!(
        super::bypass_gate(&[], reviewed),
        failed(
            "bypasses of coverage or mutation testing must be reviewed in proof-bypasses.txt: \
             2 problems, listed above\n\
             stale review: p/src/b.ts: mutated, no mutant reported (a file of no package)\n\
             stale review: p/src/c.ts: mutated, no mutant reported (a file of no package)"
        ),
        "a scope review of a file in no package of the checks: no scope gate reads it"
    );
    let packaged = "apps/roadmap/src/b.ts: mutated, no mutant reported -- constants only\n\
                    # p/src/b.ts: mutated, no mutant reported\n";
    assert_eq!(
        super::bypass_gate(&[], packaged),
        Ok(()),
        "a scope review of a package's file is left to its scope gate; a comment is no review"
    );
    let lookalike = "apps/roadmapx/src/b.ts: not measured -- r\napps/roadmap: not measured -- r\n";
    assert_eq!(
        super::bypass_gate(&[], lookalike),
        failed(
            "bypasses of coverage or mutation testing must be reviewed in proof-bypasses.txt: \
             2 problems, listed above\n\
             stale review: apps/roadmapx/src/b.ts: not measured (a file of no package)\n\
             stale review: apps/roadmap: not measured (a file of no package)"
        ),
        "below a package's directory, not beside it nor the directory itself"
    );
    for (mutation, problem) in [
        ("{", "invalid mutation report: "),
        ("{}", "the mutation report lists no `files`"),
    ] {
        let unreadable = super::Scope {
            mutation: mutation.to_owned(),
            ..only_a.clone()
        };
        assert!(
            super::scope_gate(&tracked, &[unreadable], "")
                .is_err_and(|failure| failure.summary.contains(&format!(": p: {problem}"))),
            "{mutation}"
        );
    }
}

#[test]
fn requires_every_typescript_source_measured_and_mutated_or_reviewed() {
    let tracked: Vec<String> = [
        "p/src/a.ts",
        "p/src/b.astro",
        "p/src/c.ts",
        "p/test/a.test.ts",
        "p/test/helper.ts",
        "p/e2e/x.spec.ts",
        "p/e2e/helper.ts",
        "p/vite.config.ts",
        "q/src/z.ts",
    ]
    .map(str::to_owned)
    .to_vec();
    let incomplete = scope(
        "p",
        &["src/**/*.ts", "{a,b}.ts", "lib/*.ts"],
        &[
            "/r/p/src/a.ts",
            "/r/p/src/b.astro",
            "/r/p/test/a.test.ts",
            "/r/q/src/z.ts",
        ],
    );
    let other = scope("q", &["src/**/*.ts"], &["/r/q/src/z.ts"]);
    let context = "TypeScript sources outside coverage or mutation testing must be reviewed in \
                   proof-bypasses.txt";
    assert_eq!(
        super::scope_gate(&tracked, &[incomplete, other.clone()], ""),
        failed(format!(
            "{context}: 8 problems, listed above\n\
             p: the coverage measures /r/p/test/a.test.ts, which is not a tracked source of the package\n\
             p: the coverage measures /r/q/src/z.ts, which is not a tracked source of the package\n\
             p/stryker.config.json: `{{a,b}}.ts` in mutate: only `*` and `**` are understood\n\
             p/src/c.ts is mutated but not measured: the coverage `include` misses it\n\
             p/stryker.config.json: `lib/*.ts` in mutate matches no source\n\
             not reviewed: p/e2e/helper.ts: not measured\n\
             not reviewed: p/src/b.astro: measured, not mutated\n\
             not reviewed: p/vite.config.ts: not measured"
        )),
        "tests apart, every source measured and mutated or an occurrence; q is complete"
    );
    let reviewed = "# reviews\n\
                    p/e2e/helper.ts: not measured -- a reason\n\
                    p/src/b.astro: measured, not mutated -- a reason\n\
                    p/vite.config.ts: not measured -- a reason\n\
                    p/gone.ts: not measured -- a reason\n\
                    crates/a/src/lib.rs: a token at a line -- the bypass gate's\n";
    let complete = scope(
        "p",
        &["src/**/*.ts"],
        &["/r/p/src/a.ts", "/r/p/src/b.astro", "/r/p/src/c.ts"],
    );
    assert_eq!(
        super::scope_gate(&tracked, &[complete, other.clone()], reviewed),
        failed(format!("{context}: stale review: p/gone.ts: not measured")),
        "reviewed occurrences pass, a review of nothing is stale, the other reviews are the \
         bypass gate's"
    );
    let other_package = format!("{reviewed}q/src/gone.ts: not measured -- q's review\n");
    let complete_alone = scope(
        "p",
        &["src/**/*.ts"],
        &["/r/p/src/a.ts", "/r/p/src/b.astro", "/r/p/src/c.ts"],
    );
    assert_eq!(
        super::scope_gate(
            &tracked,
            core::slice::from_ref(&complete_alone),
            &other_package
        ),
        failed(format!("{context}: stale review: p/gone.ts: not measured")),
        "a package checked alone leaves the reviews of another package to its own gate"
    );
    assert_eq!(
        super::scope_gate(&tracked, &[complete_alone, other.clone()], &other_package),
        failed(format!(
            "{context}: 2 problems, listed above\n\
             stale review: p/gone.ts: not measured\n\
             stale review: q/src/gone.ts: not measured"
        )),
        "each package checks the reviews of its own files"
    );
    let rust_only = "crates/a/src/lib.rs: a token at a line -- the bypass gate's\n";
    assert_eq!(
        super::bypass_gate(&[], rust_only),
        failed(
            "bypasses of coverage or mutation testing must be reviewed in proof-bypasses.txt: \
             stale review: crates/a/src/lib.rs: a token at a line"
        ),
        "the bypass gate's own reviews"
    );
    let unreadable = super::Scope {
        dir: "q",
        stryker: "{".to_owned(),
        summary: String::new(),
        mutation: String::new(),
    };
    assert!(
        super::scope_gate(&tracked, &[unreadable], "").is_err_and(|failure| failure
            .summary
            .contains(": q: invalid Stryker configuration: ")),
        "an unreadable Stryker configuration"
    );
    let no_summary = super::Scope {
        summary: "[".to_owned(),
        ..other
    };
    assert!(
        super::scope_gate(&tracked, &[no_summary], "")
            .is_err_and(|failure| failure.summary.contains(": q: invalid coverage summary: ")),
        "an unreadable coverage summary"
    );
}

#[test]
fn fails_when_the_scope_gate_cannot_read_its_inputs() {
    let step = [Step {
        name: "Scope".to_owned(),
        action: Action::TypeScriptScopeGate(PACKAGES[4]),
    }];
    assert_eq!(
        run_steps(&step, &mut passing(), &mut Vec::new()),
        Ok(()),
        "the fake passes"
    );
    let mut fake = passing();
    fake.broken = Some("list *.ts *.tsx *.mts *.cts *.js *.jsx *.mjs *.cjs *.astro".to_owned());
    assert_eq!(
        run_steps(&step, &mut fake, &mut Vec::new()),
        failed(
            "Scope: cannot list the TypeScript sources: `git ls-files -- *.ts *.tsx *.mts *.cts \
             *.js *.jsx *.mjs *.cjs *.astro` exited with code 128: fatal: not a git repository"
        ),
        "the sources cannot be listed"
    );
    for missing in [
        "apps/roadmap/stryker.config.json",
        "apps/roadmap/coverage/coverage-summary.json",
        "apps/roadmap/reports/mutation/mutation.json",
        super::BYPASSES_REVIEWED,
    ] {
        let mut lacking = passing();
        let removed = lacking.files.remove(missing);
        assert!(removed.is_some(), "{missing} removed");
        assert_eq!(
            run_steps(&step, &mut lacking, &mut Vec::new()),
            failed(format!("Scope: cannot read {missing}: missing")),
            "{missing} cannot be read"
        );
    }
}

#[test]
fn reads_the_prose_of_each_language() {
    let rust = "/// A doc.\nlet a = \"text {name} here\"; // trailing\nlet b = 'x'; let c = '\\''; fn f<'a>() {}\n/* block\n   two */ let d = r#\"raw \"quoted\" text\"#;\nlet e = \"word\"; let f = r\"plain raw\"; let g = br\"two bytes\";\nlet h = \"a\\\nb\";\n";
    assert_eq!(
        super::code_prose(rust, super::Language::Rust),
        [
            (1, "/// A doc."),
            (2, "text  here"),
            (2, "// trailing"),
            (4, " block"),
            (5, "   two "),
            (5, "raw \"quoted\" text"),
            (6, "plain raw"),
            (6, "two bytes"),
            (7, "a"),
            (8, "b"),
        ]
        .map(|(line, text)| (line, text.to_owned()))
        .to_vec(),
        "comments and literals, placeholders, single words, characters and lifetimes apart"
    );
    let script = "const a = 'single \\'quoted\\''; // note\nconst b = `tem ${node.x} plate ${{ a: 1 }.a}`;\nconst c = `two\n${\n0} lines`;\nconst d = \"it's\"; /* unclosed";
    assert_eq!(
        super::code_prose(script, super::Language::Script),
        [
            (1, "single 'quoted'"),
            (1, "// note"),
            (2, "tem   plate  "),
            (3, "two"),
            (5, " lines"),
            (6, "it's"),
            (6, " unclosed"),
        ]
        .map(|(line, text)| (line, text.to_owned()))
        .to_vec(),
        "every quote, substitutions apart, line numbers kept"
    );
    assert_eq!(
        super::markdown_prose("# Title\n\n```sh\nnode x\n```\ntext\n"),
        [(1, "# Title"), (2, ""), (6, "text")]
            .map(|(line, text)| (line, text.to_owned()))
            .to_vec(),
        "fenced code apart"
    );
    assert_eq!(
        super::without_code("a `node` [link](node.md) (node) b"),
        "a        [link]          (node) b",
        "code spans and link targets apart"
    );
    let json = "{\n  \"description\": \"a long render\", \"key\": \"word\",\n  \"say \\\"x\\\" here\" : [\"two words\", \"esc\\\\aped \\\"q\\\"\"],\n  \"open\": \"to the end\n}\n";
    assert_eq!(
        super::json_prose(json),
        [
            (2, "a long render"),
            (3, "two words"),
            (3, "esc\\aped \"q\""),
            (4, "to the end"),
        ]
        .map(|(line, text)| (line, text.to_owned()))
        .to_vec(),
        "string values, keys and single words apart, escapes read, an unclosed string to the \
         end of its line"
    );
    assert_eq!(super::language("spec/b.json"), Some(super::Language::Json));
    assert_eq!(super::language("a/b.tsx"), Some(super::Language::Script));
    assert_eq!(super::language("a/b.txt"), None);
    assert_eq!(super::language("Makefile"), None);
}

#[test]
fn finds_only_words_of_prose() {
    for (text, expected) in [
        ("the node runs", vec![4]),
        ("node.", vec![0]),
        ("a node's id", vec![2]),
        ("nodes", vec![]),
        ("anode", vec![]),
        ("node_id", vec![]),
        ("engine-node", vec![]),
        (".node-version", vec![]),
        ("/engines/node", vec![]),
        ("Node.js node:fs", vec![]),
        ("\"node\" 'node'", vec![]),
        ("\"node", vec![1]),
        ("node: a box", vec![0]),
        ("@node #node $node ~node \\node", vec![]),
    ] {
        let term = if text.starts_with("Node") {
            "Node"
        } else {
            "node"
        };
        assert_eq!(super::occurrences(text, term), expected, "{text}");
    }
}

/// The vocabulary of the fake repository, whose prose it all accepts.
const FAKE_WORDS: &str = "forbid node -> box -- the glossary\n";

/// The vocabulary of the gate's tests.
const WORDS: &str = "# words\n\n\
    forbid node -> box -- the glossary\n\
    forbid object -> box in crates/ -- the glossary\n\
    forbid tap block -> render quantum -- the glossary\n\
    allow node as \"engine node\" -- Web Audio\n\
    allow node in web/ -- Web Audio\n\
    allow object as \"JSON object\" -- JSON\n";

#[test]
fn refuses_the_synonyms_the_glossary_replaces() {
    let sources: Vec<(String, String)> = [
        (
            "README.md",
            "A node, an engine node.\n\n```\nnode\n```\nA `node`, a tap block.\nAn engine node, a node.\n",
        ),
        (
            "crates/a/src/lib.rs",
            "/// An object, a JSON object.\nfn f() { let node = \"the node\"; }\n",
        ),
        (
            "apps/a/b.ts",
            "// an object here is fine\nconst node = 'node';\n",
        ),
        ("web/c.ts", "/** A node. */\n"),
        ("notes.txt", "node\n"),
    ]
    .map(|(path, text)| (path.to_owned(), text.to_owned()))
    .to_vec();
    let context = "words the glossary replaces (reviewed in vocabulary.txt)";
    assert_eq!(
        super::vocabulary_gate(&sources, WORDS),
        failed(format!(
            "{context}: 5 problems, listed above\n\
             README.md:1: \"node\": say \"box\" (the glossary)\n\
             README.md:6: \"tap block\": say \"render quantum\" (the glossary)\n\
             README.md:7: \"node\": say \"box\" (the glossary)\n\
             crates/a/src/lib.rs:1: \"object\": say \"box\" (the glossary)\n\
             crates/a/src/lib.rs:2: \"node\": say \"box\" (the glossary)"
        )),
        "prose only, each rule where it applies, allowed contexts apart"
    );
    let clean: Vec<(String, String)> = [(
        "web/c.ts",
        "/** A node: an engine node, a JSON object. */\n",
    )]
    .map(|(path, text)| (path.to_owned(), text.to_owned()))
    .to_vec();
    assert_eq!(
        super::vocabulary_gate(
            &clean,
            &format!("{WORDS}allow box in x/ -- r\nallow node in other/ -- r\nforbid broken\n")
        ),
        failed(format!(
            "{context}: 4 problems, listed above\n\
             vocabulary.txt:11: write `forbid <term> -> <instead>[ in <path>…] -- <reason>`, `allow <term> in <path> -- <reason>` or `allow <term> as \"<phrase>\" -- <reason>`\n\
             stale: allow object as \"JSON object\": it allows nothing\n\
             allow box in x/: no rule refuses \"box\"\n\
             stale: allow node in other/: it allows nothing"
        )),
        "a malformed line, an allowance of nothing refused, allowances that allow nothing"
    );
    for malformed in [
        "forbid a -- r",
        "allow a -- r",
        "allow a as b -- r",
        "allow a as \"b -- r",
        "permit a in b -- r",
        "forbid a -> b",
    ] {
        assert!(
            super::vocabulary_gate(&[], malformed)
                .is_err_and(|failure| failure.summary.contains("vocabulary.txt:1: write")),
            "{malformed}"
        );
    }
    assert!(super::within("web/c.ts", "web/"), "a directory");
    assert!(super::within("web", "web"), "a file");
    assert!(
        !super::within("web/c.ts", "web"),
        "a file is not a directory"
    );
}

#[test]
fn reads_the_prose_files_and_the_vocabulary() {
    let step = [Step {
        name: "Words".to_owned(),
        action: Action::VocabularyGate,
    }];
    let mut fake = passing();
    let replaced = fake
        .files
        .insert("README.md".to_owned(), "A node.\n".to_owned());
    assert!(replaced.is_some(), "README replaced");
    for (path, text) in [
        ("spec/v.json", "{\"description\": \"a node, then\"}\n"),
        ("conformance/cases/c.json", "{\"note\": \"one node\"}\n"),
    ] {
        let added = fake.files.insert(path.to_owned(), text.to_owned());
        assert!(added.is_none(), "fresh fake");
    }
    let board = fake.files.insert(
        super::ROADMAP_BOARD.to_owned(),
        "{\"title\": \"a node\"}\n".to_owned(),
    );
    assert!(board.is_some(), "the board replaced");
    assert_eq!(
        run_steps(&step, &mut fake, &mut Vec::new()),
        Err(Failure {
            summary: "Words: words the glossary replaces (reviewed in vocabulary.txt): 3 \
                      problems, listed above"
                .to_owned(),
            details: [
                "README.md:1: \"node\": say \"box\" (the glossary)",
                "conformance/cases/c.json:1: \"node\": say \"box\" (the glossary)",
                "spec/v.json:1: \"node\": say \"box\" (the glossary)",
            ]
            .map(str::to_owned)
            .to_vec(),
        }),
        "the tracked prose against the vocabulary, the JSON of the specifications and the \
         corpus included, the board's apart"
    );
    let mut lacking = passing();
    let removed = lacking.files.remove(super::VOCABULARY);
    assert!(removed.is_some(), "vocabulary removed");
    assert_eq!(
        run_steps(&step, &mut lacking, &mut Vec::new()),
        failed("Words: cannot read vocabulary.txt: missing"),
        "no vocabulary"
    );
    let mut unlisted = passing();
    unlisted.broken = Some(
        "list *.md *.rs *.ts *.tsx *.mts *.js *.mjs spec/*.json conformance/*.json".to_owned(),
    );
    assert!(
        run_steps(&step, &mut unlisted, &mut Vec::new()).is_err_and(|failure| failure
            .summary
            .starts_with("Words: cannot list the prose files")),
        "the files cannot be listed"
    );
}

#[test]
fn reads_odd_sources_without_losing_prose() {
    let lines = |text: &str, language| -> Vec<String> {
        super::code_prose(text, language)
            .into_iter()
            .map(|(line, prose)| format!("{line}:{prose}"))
            .collect()
    };
    let rust = "let a = \"\"; let r#type = ar\"one two\"; x ` y\nlet b = \"{a b} } c\";";
    assert_eq!(
        lines(rust, super::Language::Rust),
        ["1:one two", "2:  } c"],
        "an empty literal, a raw identifier, an `r` ending a name, a backquote; in a Rust \
         literal, what braces hold is a placeholder or code (JSON), never prose"
    );
    let script = "const a = r\"one two\"; const b = 'a {b} c'; const c = `cost $5 and '$x'`; const d = 'tab\\there';";
    assert_eq!(
        lines(script, super::Language::Script),
        ["1:one two", "1:a {b} c", "1:cost $5 and '$x'", "1:tab here"],
        "no raw strings, braces kept, a dollar sign that starts no substitution, an escape"
    );
    let quotes = "let q = '\\\"'; // one\nlet d = '\"'; // two\n'\\x41'\"'tail word\"\n_r\"a\\tb c\" ar\"d\\te f\" br\"g\\th i\"";
    assert_eq!(
        lines(quotes, super::Language::Rust),
        [
            "1:// one",
            "2:// two",
            "3:'tail word",
            "4:a b c",
            "4:d e f",
            "4:g\\th i"
        ],
        "character literals holding a quote, an escape of several characters, an `r` ending \
         a name (after `_` or a letter), a raw byte string"
    );
    for (text, language, expected) in [
        (
            "r\"open to the end",
            super::Language::Rust,
            "1:open to the end",
        ),
        (
            "\"open to the end",
            super::Language::Rust,
            "1:open to the end",
        ),
        ("`open ${to the end", super::Language::Script, "1:open  "),
        ("'\\", super::Language::Rust, ""),
    ] {
        assert_eq!(
            lines(text, language).join("|"),
            expected,
            "an unterminated literal: {text}"
        );
    }
    assert_eq!(
        super::without_code("[a](x`y) `a](b` c"),
        format!("[a]{}c", " ".repeat(13)),
        "a backquote in a link target, a bracket in a code span"
    );
}

#[test]
fn accepts_exactly_the_conditions_decided() {
    for condition in super::WORKFLOW_CONDITIONS {
        let workflow = PINNED_WORKFLOW.replace(
            "    runs-on: ubuntu-24.04\n",
            &format!("    {condition}\n    runs-on: ubuntu-24.04\n"),
        );
        assert!(
            workflow.contains(condition),
            "the fixture holds the condition"
        );
        assert_eq!(
            super::workflow_gate(&[super::WORKFLOW.to_owned()], &workflow, "22.23.4"),
            Ok(()),
            "{condition}"
        );
    }
    for variant in [
        "if: github.event_name != 'push' || contains(github.event.head_commit.message, 'ci')",
        "if: false",
        "if: github.event_name == 'pull_request'",
    ] {
        let workflow = PINNED_WORKFLOW.replace(
            "    runs-on: ubuntu-24.04\n",
            &format!("    {variant}\n    runs-on: ubuntu-24.04\n"),
        );
        assert!(
            super::workflow_gate(&[super::WORKFLOW.to_owned()], &workflow, "22.23.4").is_err(),
            "{variant}"
        );
    }
}

#[test]
fn keeps_a_run_skipped_by_the_ci_condition_from_cancelling_a_check() {
    let [_, condition] = super::WORKFLOW_CONDITIONS;
    let group = super::workflow_concurrency()
        .into_iter()
        .nth(1)
        .unwrap_or_default();
    assert!(
        group.contains(&format!(
            "({}) && ",
            condition.strip_prefix("if: ").unwrap_or_default()
        )),
        "the group follows the condition of the check: {group}"
    );
    let gate =
        |workflow: &str| super::workflow_gate(&[super::WORKFLOW.to_owned()], workflow, "22.23.4");
    let lacks = |line: &str| {
        failed(format!(
            ".github/workflows/quality.yml can change CI without a commit: \
             .github/workflows/quality.yml lacks the concurrency `{line}`"
        ))
    };
    let expected = lacks(&super::workflow_concurrency().join(" "));
    let unconditional = PINNED_WORKFLOW.replace(&group, "  group: quality-${{ github.ref }}");
    assert_ne!(
        unconditional, PINNED_WORKFLOW,
        "the fixture holds the group"
    );
    assert_eq!(
        gate(&unconditional),
        expected,
        "a run the condition skips would join the group of the check and cancel it"
    );
    assert_eq!(
        gate(&PINNED_WORKFLOW.replace("  cancel-in-progress: true\n", "")),
        expected,
        "a superseded check is cancelled"
    );
    assert_eq!(
        gate(&PINNED_WORKFLOW.replace("\nconcurrency:\n", "\n  concurrency:\n")),
        expected,
        "for the whole workflow"
    );
}

#[test]
fn confines_the_ci_marker_to_a_line_of_its_own() {
    let ok = "ci: rerun the check\n\nAsked by the maintainer.\n\n[ci]\n";
    assert_eq!(commit_msg_gate(ok), Ok(()), "alone on a line");
    assert_eq!(
        super::ci_marker("docs: x\n\n  [ci]  \n"),
        None,
        "spaces around it"
    );
    assert_eq!(
        super::ci_marker("docs: x\n\nno marker at all\n"),
        None,
        "none"
    );
    let expected = Some(
        "invalid commit message: [ci] asks CI to run; write it alone on a line, or not at all"
            .to_owned(),
    );
    for message in [
        "docs: x\n\n[CI]\n",
        "docs: x\n\n [Ci] \n",
        "docs: x\n\n[cı]\n",
    ] {
        assert_eq!(
            super::ci_marker(message),
            None,
            "GitHub reads the marker in any case: {message:?}"
        );
    }
    for message in [
        "docs: x\n\nA push runs CI when it says [ci] in its message.\n",
        "docs: run [ci] now\n",
        "docs: x\n\n[ci] please\n",
        "docs: x\n\nA push runs CI when it says [CI] in its message.\n",
        "docs: run [Ci] now\n",
        "docs: x\n\n[cI] please\n",
        "docs: x\n\nsee [cı] here\n",
    ] {
        assert_eq!(super::ci_marker(message), expected, "{message:?}");
        assert!(commit_msg_gate(message).is_err(), "{message:?}");
    }
    let record = |hash: &str, message: &str| format!("\u{1e}{hash}\u{1f}{message}");
    let sentence = "docs: x\n\nIt runs with [ci] in the message.\n";
    assert!(
        super::history_gate(
            &record("1111111111111111111111111111111111111111", sentence),
            "",
            ""
        )
        .is_err(),
        "the history refuses it too"
    );
    assert_eq!(
        super::history_gate(&record(super::CI_MARKER_EXEMPT[0], sentence), "", ""),
        Ok(()),
        "except the published commit that predates the rule"
    );
}

#[test]
fn accepts_the_synthetic_merge_of_a_pull_request_only_as_the_commit_checked_out() {
    let (head, base) = ("a".repeat(40), "0123456789abcdef0123456789abcdef01234567");
    let merge = format!("Merge {head} into {base}\n");
    let record = |hash: &str, message: &str| format!("\u{1e}{hash}\u{1f}{message}");
    for message in [merge.clone(), format!("{merge}\n")] {
        let merged = format!("{}{HISTORY}", record("ffffffffff", &message));
        assert_eq!(
            super::history_gate(&merged, HISTORY_FILES, ""),
            Ok(()),
            "the merge GitHub checks out for a pull request, at HEAD: {message:?}"
        );
    }
    let refused = |hash: &str| {
        found(
            "the history breaks the commit rules",
            &[format!(
                "commit {hash}: invalid commit message: missing \"<type>: \" prefix"
            )],
        )
    };
    let below = format!(
        "{}{}{HISTORY}",
        record("cccccccccc", "docs: x\n"),
        record("ffffffffff", &merge)
    );
    assert_eq!(
        super::history_gate(&below, HISTORY_FILES, ""),
        refused("fffffff"),
        "anywhere but HEAD, the same message is refused"
    );
    for message in [
        "Merge branch 'x' into main\n".to_owned(),
        format!("Merge {head} into {}\n", base.get(..39).unwrap_or_default()),
        format!("Merge {} into {base}\n", head.to_uppercase()),
        format!("Merge {head} into {base}\n\nmore\n"),
        format!("Merge {head} into {base}g\n"),
        format!("Merge {head} into {base} \n"),
        format!("Merge {head}  into {base}\n"),
        format!("merge {head} into {base}\n"),
    ] {
        assert_eq!(
            super::history_gate(&record("ffffffffff", &message), "", ""),
            refused("fffffff"),
            "{message:?}"
        );
    }
    assert!(
        commit_msg_gate(&merge).is_err(),
        "a commit written here never has that message"
    );
}

#[test]
fn makes_a_message_safe_to_show() {
    use super::{MAX_MESSAGE_CHARS, safe_message};
    assert_eq!(
        safe_message("commit abc: unknown type \"x\""),
        "commit abc: unknown type \"x\"",
        "plain text is unchanged, quotes included"
    );
    let hostile = "a\nb\rc\td\u{7f}e\u{85}f\u{202e}g\u{2066}h\u{200f}i\u{200e}i\u{61c}j\u{2028}k\u{2029}l\u{202a}m\u{2069}n\u{1b}";
    assert_eq!(
        safe_message(hostile),
        "a\\u{a}b\\u{d}c\\u{9}d\\u{7f}e\\u{85}f\\u{202e}g\\u{2066}h\\u{200f}i\\u{200e}i\\u{61c}j\\u{2028}k\\u{2029}l\\u{202a}m\\u{2069}n\\u{1b}",
        "control, bidi and line-separator characters are escaped"
    );
    assert_eq!(
        safe_message("é ü 音 \u{202f}\u{206a}\u{2065}"),
        "é ü 音 \u{202f}\u{206a}\u{2065}",
        "other text is kept, the neighbours of the escaped ranges included"
    );
    assert_eq!(safe_message(""), "", "nothing to show");
    let at_limit = "x".repeat(MAX_MESSAGE_CHARS);
    assert_eq!(safe_message(&at_limit), at_limit, "exactly at the limit");
    assert_eq!(
        safe_message(&format!("{at_limit}y")),
        format!("{}…", "x".repeat(MAX_MESSAGE_CHARS - 1)),
        "one character more: cut, and marked"
    );
    assert_eq!(
        safe_message(&"\n".repeat(MAX_MESSAGE_CHARS)),
        format!("{}…", "\\u{a}".repeat((MAX_MESSAGE_CHARS - 1) / 5)),
        "whole escapes only"
    );
    assert_eq!(
        MAX_MESSAGE_CHARS, 1_000,
        "the limit of engine_patch::safe_message"
    );
}

/// Whether `text` is lines of at most `max` characters, none holding a control character.
fn safe_lines(text: &[u8], max: usize) -> bool {
    String::from_utf8(text.to_vec()).is_ok_and(|text| {
        text.lines()
            .all(|line| line.chars().count() <= max && !line.chars().any(char::is_control))
    })
}

#[test]
fn shows_untrusted_text_on_one_bounded_line() {
    let line_max = "error: ".len() + super::MAX_MESSAGE_CHARS;
    let args = ["commit-msg".to_owned(), "COMMIT_EDITMSG".to_owned()];
    for (message, shown) in [
        (
            "\u{1b}]0;owned\u{7} feat: x\n".to_owned(),
            "error: invalid commit message: unknown type \"\\u{1b}]0;owned\\u{7} feat\"\n"
                .to_owned(),
        ),
        (
            format!("{}: y\n", "x".repeat(200_000)),
            format!(
                "error: {}…\n",
                format!(
                    "invalid commit message: unknown type \"{}",
                    "x".repeat(1_000)
                )
                .chars()
                .take(super::MAX_MESSAGE_CHARS - 1)
                .collect::<String>()
            ),
        ),
    ] {
        let mut fake = passing();
        let _replaced = fake.files.insert("COMMIT_EDITMSG".to_owned(), message);
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(run_cli(&args, &mut fake, &mut out, &mut err), 1, "refused");
        assert_eq!(String::from_utf8(err).expect("UTF-8"), shown, "one line");
        assert_eq!(
            String::from_utf8(out).expect("UTF-8"),
            "expected: <type>(<scope>)!: <summary>, type one of feat, fix, docs, test, refactor, \
             perf, build, ci, chore, style, revert\n",
            "the details"
        );
    }
    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert_eq!(
        run_cli(
            &["\u{1b}[2J\nerror: forged".to_owned()],
            &mut passing(),
            &mut out,
            &mut err
        ),
        2,
        "a usage error"
    );
    assert_eq!(
        String::from_utf8(err).expect("UTF-8"),
        "error: unknown command \"\\u{1b}[2J\\u{a}error: forged\" (see: cargo xtask help)\n",
        "one line, the argument escaped"
    );
    assert!(out.is_empty(), "nothing on stdout");

    let mut hostile = passing();
    let _replaced = hostile.files.insert(
        super::CARGO_CONFIG_FILE.to_owned(),
        format!(
            "[build]\u{1b}[8m\nrustflags = [\"{}\"]\n",
            "x".repeat(5_000)
        ),
    );
    let (mut check_out, mut check_err) = (Vec::new(), Vec::new());
    assert_eq!(
        run_cli(
            &["check".to_owned()],
            &mut hostile,
            &mut check_out,
            &mut check_err
        ),
        1,
        "a configuration refused"
    );
    let details = String::from_utf8(check_out.clone()).expect("UTF-8");
    assert!(
        details.contains("\nfound:\n[build]\\u{1b}[8m\nrustflags = [\"xxx"),
        "each line of a detail on its own line, escaped: {details}"
    );
    assert!(
        safe_lines(&check_out, super::MAX_MESSAGE_CHARS),
        "bounded: {details}"
    );
    assert!(safe_lines(&check_err, line_max), "one bounded error line");
}

#[test]
fn starts_commands_with_no_input_and_git_untranslated() {
    let locale = |program: &str| {
        super::command(program, Path::new("."))
            .get_envs()
            .find(|(name, _)| *name == "LC_ALL")
            .map(|(_, value)| value.map(OsString::from))
    };
    assert_eq!(
        locale("git"),
        Some(Some(OsString::from("C"))),
        "git's messages in English, which error_reason reads"
    );
    assert_eq!(locale("cargo"), None, "the other commands keep the locale");
    let mut runner = SystemRunner {
        root: fixture("coverage-locality"),
    };
    let reads = Cmd::new(
        "sh",
        &["-c", "if read line; then echo read; else echo none; fi"],
        Dir::Root,
    );
    assert_eq!(
        runner.capture(&reads),
        Ok("none\n".to_owned()),
        "a command waiting for input finds none, instead of blocking"
    );
    let terminal = Cmd::new("sh", &["-c", "test ! -t 0"], Dir::Root);
    assert_eq!(
        runner.run(&terminal),
        Ok(()),
        "a command run, its output on the terminal, does not read the terminal"
    );
}

#[test]
fn reads_the_changes_as_they_are() {
    let repo = scratch("textconv");
    let _init = hermetic_git(&repo, &["init", "--quiet"]);
    std::fs::write(repo.join("a.rs"), "fn f() {}\n").expect("source");
    let _add = hermetic_git(&repo, &["add", "a.rs"]);
    let _commit = hermetic_git(&repo, &["commit", "--quiet", "-m", "x"]);
    std::fs::write(repo.join("a.rs"), "fn g() {}\n").expect("changed");
    std::fs::write(repo.join(".gitattributes"), "*.rs diff=upper\n").expect("attributes");
    for (key, value) in [
        ("diff.upper.textconv", "tr a-z A-Z <"),
        ("diff.external", "false"),
    ] {
        let _set = hermetic_git(&repo, &["config", key, value]);
    }
    let mut runner = SystemRunner { root: repo.clone() };
    let state = super::tree_state(&mut runner);
    std::fs::remove_dir_all(&repo).expect("cleanup");
    let diff = state.expect("read").diff;
    assert!(
        diff.contains("-fn f() {}\n+fn g() {}\n"),
        "neither an external diff nor a text conversion: {diff}"
    );
}

#[test]
fn knows_a_deletion_by_its_all_zero_object_name() {
    for (object, deletion) in [
        ("0".repeat(40), true),
        ("0".repeat(64), true),
        (String::new(), false),
        ("0".repeat(39), false),
        ("0".repeat(41), false),
        ("0".repeat(63), false),
        (format!("{}1", "0".repeat(39)), false),
    ] {
        assert_eq!(super::is_deletion(&object), deletion, "{object:?}");
    }
}

#[test]
fn builds_the_engine_package_before_every_package() {
    assert_eq!(
        super::ENGINE_PACKAGE.dir,
        "bindings/wasm",
        "the engine in the browser"
    );
    assert!(
        super::WASM_MODULE.starts_with("bindings/wasm/dist/"),
        "its build writes the module"
    );
    for (mode, first_user) in [
        (Mode::Full { jobs: FAKE_JOBS }, PACKAGES[0].step),
        (CHANGES_MODE, "Playground checks"),
    ] {
        let steps = check_steps(mode);
        let built = steps.iter().position(|step| {
            step.action
                == Action::Run(Cmd::new(
                    "pnpm",
                    &["run", "build"],
                    Dir::Package("bindings/wasm"),
                ))
        });
        let emptied = steps
            .iter()
            .position(|step| step.action == Action::FreshDirs(super::build_outputs()));
        let first = steps.iter().position(|step| step.name == first_user);
        assert!(
            emptied < built && built < first && emptied.is_some(),
            "{mode:?}: built after its outputs are emptied, before {first_user}"
        );
    }
    assert_eq!(
        PACKAGES[3].needs,
        [super::Setup::EngineBuild],
        "the playground uses it"
    );
}

/// The parts of the full check, by name, in order: the matrix of the CI workflow, then the
/// final part.
const PART_NAMES: [&str; 18] = [
    "rust",
    "coverage",
    "mutation-0",
    "mutation-1",
    "mutation-2",
    "mutation-3",
    "mutation-4",
    "mutation-5",
    "mutation-6",
    "mutation-7",
    "miri",
    "gates",
    "tooling-config",
    "tooling-oracle",
    "bindings-wasm",
    "apps-playground",
    "apps-roadmap",
    "final",
];

/// The steps of the part named `name` of the full check, with the fake's jobs.
fn part_named(name: &str) -> Vec<Step> {
    let part = super::full_parts()
        .into_iter()
        .find(|part| part.name() == name)
        .expect("a part of that name");
    super::part_steps(part, FAKE_JOBS)
}

/// Every part of the full check, named, with its steps.
fn every_part() -> Vec<(String, Vec<Step>)> {
    PART_NAMES
        .iter()
        .map(|name| ((*name).to_owned(), part_named(name)))
        .collect()
}

/// The steps of the full check, with the fake's jobs.
fn full_check() -> Vec<Step> {
    check_steps(Mode::Full { jobs: FAKE_JOBS })
}

#[test]
fn splits_the_full_check_into_named_parts() {
    assert_eq!(super::MUTATION_SHARDS, 8, "eight mutation shards");
    assert_eq!(
        super::full_parts()
            .into_iter()
            .map(super::Part::name)
            .collect::<Vec<_>>(),
        PART_NAMES,
        "the parts, in order, the final one last"
    );
    assert_eq!(
        super::Shard::all()
            .map(super::Shard::index)
            .collect::<Vec<_>>(),
        [0, 1, 2, 3, 4, 5, 6, 7],
        "every shard, as cargo-mutants counts them"
    );
    for name in PART_NAMES {
        let steps = part_named(name);
        let names: Vec<&str> = steps.iter().map(|step| step.name.as_str()).collect();
        assert_eq!(
            names.get(..2),
            Some(&["Tools and platform", "Working tree recorded"][..]),
            "{name}: the tools and the tree first"
        );
        assert_eq!(
            names.last(),
            Some(&"Working tree unchanged by the checks"),
            "{name}: the tree checked last"
        );
    }
}

#[test]
fn runs_each_part_of_the_full_check_as_its_own_job() {
    let lines = |name: &str| command_lines(&part_named(name));
    let frame = |inner: &[&str]| {
        let mut all = vec!["preflight".to_owned(), "tree recorded".to_owned()];
        all.extend(inner.iter().map(|line| (*line).to_owned()));
        all.push("tree unchanged".to_owned());
        all
    };
    assert_eq!(
        lines("mutation-3"),
        frame(&[
            "fresh mutants.out",
            "Root: cargo mutants --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --workspace --shard 3/8 --sharding=round-robin --jobs 3 --timeout-multiplier 15 --minimum-test-timeout 60 -- -- --include-ignored",
        ]),
        "one shard of the mutants, round-robin"
    );
    assert_eq!(
        part_named("mutation-3")
            .get(3)
            .map(|step| step.name.as_str()),
        Some("Rust mutation testing (shard 3/8)"),
        "named after its shard"
    );
    assert_eq!(
        lines("final"),
        frame(&[
            "unviable gate over mutants.out/shards/unviable-mutation-0/unviable.txt mutants.out/shards/unviable-mutation-1/unviable.txt mutants.out/shards/unviable-mutation-2/unviable.txt mutants.out/shards/unviable-mutation-3/unviable.txt mutants.out/shards/unviable-mutation-4/unviable.txt mutants.out/shards/unviable-mutation-5/unviable.txt mutants.out/shards/unviable-mutation-6/unviable.txt mutants.out/shards/unviable-mutation-7/unviable.txt",
        ]),
        "the unviable mutants of every shard"
    );
    assert_eq!(
        lines("miri"),
        frame(&[&format!(
            "Root: cargo +nightly-2026-11-01 {} test --package engine-alloc-counter --package engine-wasm --locked -- --include-ignored",
            "miri"
        )]),
        "Miri alone"
    );
    assert_eq!(
        lines("apps-playground"),
        frame(&[
            "Root: pnpm install --frozen-lockfile",
            "fresh tooling/config/coverage tooling/config/reports/mutation tooling/oracle/coverage tooling/oracle/reports/mutation bindings/wasm/coverage bindings/wasm/reports/mutation apps/playground/coverage apps/playground/reports/mutation apps/roadmap/coverage apps/roadmap/reports/mutation",
            "fresh bindings/wasm/dist",
            "Package(\"bindings/wasm\"): pnpm run build",
            "Package(\"apps/playground\"): pnpm run check",
            "Package(\"apps/playground\"): pnpm exec stryker run stryker.config.json --incremental --incrementalFile reports/stryker-incremental.json --force",
            "typescript reports gate of apps/playground",
            "typescript scope gate of apps/playground",
        ]),
        "the playground on the engine package's build, without its checks"
    );
    assert_eq!(
        lines("tooling-oracle").get(2..5).map(<[String]>::to_vec),
        Some(
            [
                "fresh target/conformance",
                "Root: cargo run --package engine-cli --locked --quiet -- export conformance target/conformance",
                "Root: pnpm install --frozen-lockfile",
            ]
            .map(str::to_owned)
            .to_vec()
        ),
        "the oracle on the exported conformance files"
    );
    assert_eq!(
        lines("tooling-config").len(),
        3 + 4 + 3,
        "the frame, the JavaScript preparation (and no other) and the package's four steps"
    );
    let gates = lines("gates");
    assert!(
        gates.contains(&"parts gate".to_owned())
            && gates.contains(&"workflow gate".to_owned())
            && gates.contains(&"Root: pnpm install --frozen-lockfile".to_owned())
            && !gates.iter().any(|line| line.contains("export conformance")),
        "the gates, with the JavaScript dependencies their license gate reads"
    );
    assert_eq!(
        super::preparation_steps(),
        [
            "Tools and platform",
            "Working tree recorded",
            "Fresh Rust mutation report",
            "Fresh conformance export directory",
            "Conformance export (native WAV files)",
            "JavaScript dependencies",
            "Fresh TypeScript report directories",
            "Fresh TypeScript build outputs",
            "Engine WASM build, for the packages that use it",
            "Working tree unchanged by the checks",
        ]
        .map(str::to_owned)
        .to_vec(),
        "only preparation runs in several parts, never a proof"
    );
}

#[test]
fn checks_the_parts_of_the_full_check_as_a_step() {
    assert_eq!(
        super::parts_of_the_full_check(super::full_parts()),
        Ok(()),
        "every part"
    );
    let without_miri: Vec<super::Part> = super::full_parts()
        .into_iter()
        .filter(|part| *part != super::Part::Miri)
        .collect();
    assert_eq!(
        super::parts_of_the_full_check(without_miri),
        parts_refused(&["`Unsafe code under Miri` runs in no part"]),
        "a part left out"
    );
}

#[test]
fn the_parts_run_every_step_of_the_full_check_once() {
    let preparation = super::preparation_steps();
    assert_eq!(
        super::parts_gate(&full_check(), &every_part(), &preparation),
        Ok(()),
        "the real parts"
    );
    let mut steps_run = 0;
    for (_, steps) in every_part() {
        steps_run += steps.len();
    }
    assert_eq!(
        steps_run,
        18 * 3 + (87 - 3 - 7 - 2) + 8 * 2 + 1 + 2 + 6 * 3 + 1,
        "each part's frame (three steps); every other step of the 87 once, but the seven \
         preparations, the mutation run and its unviable gate; in each of the eight shards, \
         the report emptied and the shard's run; the gate over the shards; the conformance \
         export (two steps) for the oracle; the JavaScript preparation (three steps) for the \
         gates and each package; the engine build for the playground"
    );
}

/// The failure of the parts gate with `problems`.
fn parts_refused(problems: &[&str]) -> Result<(), Failure> {
    found(
        "the parts of the full check do not run each of its steps once",
        &problems
            .iter()
            .map(|problem| (*problem).to_owned())
            .collect::<Vec<_>>(),
    )
}

/// A step that runs `program`.
fn plain(name: &str, program: &'static str, args: &[&str]) -> Step {
    Step {
        name: name.to_owned(),
        action: Action::Run(Cmd::new(program, args, Dir::Root)),
    }
}

/// The steps `a`, `b` and `c`: a full check of three steps.
fn abc() -> [Step; 3] {
    [
        plain("a", "a", &[]),
        plain("b", "b", &[]),
        plain("c", "c", &[]),
    ]
}

/// The parts gate on `parts` of the full check [`abc`], with `preparation`.
fn gate(parts: &[(&str, Vec<Step>)], preparation: &[&str]) -> Result<(), Failure> {
    let parts: Vec<(String, Vec<Step>)> = parts
        .iter()
        .map(|(name, steps)| ((*name).to_owned(), steps.clone()))
        .collect();
    let preparation: Vec<String> = preparation.iter().map(|name| (*name).to_owned()).collect();
    super::parts_gate(&abc(), &parts, &preparation)
}

#[test]
fn refuses_parts_that_miss_or_repeat_a_step() {
    let [first, second, third] = abc();
    assert_eq!(
        gate(
            &[
                ("x", vec![first.clone(), second.clone()]),
                ("y", vec![third.clone()])
            ],
            &[]
        ),
        Ok(()),
        "each step once"
    );
    assert_eq!(
        gate(
            &[("x", vec![first.clone()]), ("y", vec![third.clone()])],
            &[]
        ),
        parts_refused(&["`b` runs in no part"]),
        "a step missing"
    );
    assert_eq!(
        gate(
            &[
                ("x", vec![first.clone(), second.clone()]),
                ("y", vec![second.clone(), third.clone()])
            ],
            &[]
        ),
        parts_refused(&["`b` runs in 2 parts"]),
        "a proof run twice"
    );
    assert_eq!(
        gate(
            &[
                ("x", vec![first.clone(), second.clone()]),
                ("y", vec![second.clone(), third.clone()])
            ],
            &["b"]
        ),
        Ok(()),
        "a preparation, run where it is needed"
    );
    assert_eq!(
        gate(
            &[("x", vec![first.clone()]), ("y", vec![third.clone()])],
            &["b"]
        ),
        parts_refused(&["`b` runs in no part"]),
        "a preparation runs somewhere"
    );
    assert_eq!(
        gate(
            &[
                ("x", vec![second.clone(), first.clone()]),
                ("y", vec![third.clone()])
            ],
            &[]
        ),
        parts_refused(&["x: `a` runs out of the order of the full check"]),
        "out of order"
    );
    assert_eq!(
        gate(
            &[
                ("x", vec![first.clone(), first, second]),
                ("y", vec![third])
            ],
            &["a"]
        ),
        parts_refused(&["x: `a` runs out of the order of the full check"]),
        "twice in one part"
    );
}

#[test]
fn refuses_parts_with_steps_or_names_of_their_own() {
    let [first, second, third] = abc();
    let renamed = Step {
        name: "B".to_owned(),
        ..second.clone()
    };
    let changed = plain("b", "b", &["--more"]);
    assert_eq!(
        gate(
            &[
                ("x", vec![first.clone(), renamed, changed]),
                ("y", vec![third.clone()])
            ],
            &[]
        ),
        parts_refused(&[
            "x: `B` is no step of the full check",
            "x: `b` is no step of the full check",
            "`b` runs in no part",
        ]),
        "a step of a part is a step of the full check, name and action"
    );
    assert_eq!(
        gate(&[("x", vec![first, second]), ("x", vec![third])], &[]),
        parts_refused(&["two parts are named x"]),
        "unique part names"
    );
}

/// A shard `value` of the mutation step `m`, named `name`, as a step of a part.
fn shard(name: &str, value: &str, sharding: &str) -> Step {
    plain(
        name,
        "cargo",
        &[
            "mutants",
            "--workspace",
            "--shard",
            value,
            sharding,
            "--",
            "x",
        ],
    )
}

/// The unviable-mutant gate over the reports of the parts `parts`.
fn across(parts: &[&str]) -> Step {
    Step {
        name: "u (every shard)".to_owned(),
        action: Action::ShardsUnviableGate(
            parts
                .iter()
                .map(|part| super::shard_unviable_report(part))
                .collect(),
        ),
    }
}

/// A full check of a mutation step `m` and its unviable-mutant gate `u`.
fn mutation_check() -> [Step; 2] {
    [
        plain("m", "cargo", &["mutants", "--workspace", "--", "x"]),
        Step {
            name: "u".to_owned(),
            action: Action::UnviableGate(Extent::Complete),
        },
    ]
}

/// Round-robin sharding, as cargo-mutants reads it.
const ROUND_ROBIN: &str = "--sharding=round-robin";

/// The parts gate on `parts` of [`mutation_check`], without preparation.
fn shards_gate(parts: Vec<(&str, Vec<Step>)>) -> Result<(), Failure> {
    let parts: Vec<(String, Vec<Step>)> = parts
        .into_iter()
        .map(|(name, steps)| (name.to_owned(), steps))
        .collect();
    super::parts_gate(&mutation_check(), &parts, &[])
}

/// The parts gate on two shard parts `s0` and `s1` running `first` and `second`, and a
/// final part reading the reports of `reports`.
fn two(first: Step, second: Step, reports: &[&str]) -> Result<(), Failure> {
    shards_gate(vec![
        ("s0", vec![first]),
        ("s1", vec![second]),
        ("f", vec![across(reports)]),
    ])
}

#[test]
fn refuses_shards_that_do_not_split_the_mutants_exactly() {
    let rr = ROUND_ROBIN;
    let gate = shards_gate;
    let full = mutation_check();
    assert_eq!(
        two(
            shard("m (shard 0/2)", "0/2", rr),
            shard("m (shard 1/2)", "1/2", rr),
            &["s0", "s1"]
        ),
        Ok(()),
        "two shards of two, their reports read by the final part"
    );
    assert_eq!(
        super::shard_unviable_report("s0"),
        "mutants.out/shards/unviable-s0/unviable.txt",
        "where the final job downloads the artifact of a shard"
    );
    assert_eq!(
        two(
            shard("m (shard 0/2)", "0/2", rr),
            shard("m (shard 0/2)", "0/2", rr),
            &["s0", "s1"]
        ),
        parts_refused(&[
            "shard 0/2 of `m` runs in 2 parts: s0, s1",
            "shard 1/2 of `m` runs in no part",
        ]),
        "a shard twice, another never"
    );
    assert_eq!(
        two(
            shard("m (shard 0/2)", "0/2", rr),
            shard("m (shard 2/2)", "2/2", rr),
            &["s0", "s1"]
        ),
        parts_refused(&[
            "s1: shard 2/2 of `m` is no shard",
            "shard 1/2 of `m` runs in no part",
        ]),
        "an index out of range"
    );
    assert_eq!(
        two(
            shard("m (shard 0/2)", "0/2", rr),
            shard("m (shard 1/3)", "1/3", rr),
            &["s0", "s1"]
        ),
        parts_refused(&["`m` is split into shards of different counts"]),
        "two counts"
    );
    assert_eq!(
        gate(vec![
            ("s0", vec![shard("m (shard 0/1)", "0/1", rr)]),
            ("w", vec![full[0].clone()]),
            ("f", vec![across(&["s0"])]),
        ]),
        parts_refused(&["`m` runs both whole (w) and in shards"]),
        "whole and in shards"
    );
    assert_eq!(
        gate(vec![
            (
                "s",
                vec![
                    shard("m (shard 0/2)", "0/2", rr),
                    shard("m (shard 1/2)", "1/2", rr)
                ]
            ),
            ("f", vec![across(&["s", "s"])]),
        ]),
        parts_refused(&[
            "s: runs a second shard, whose report would replace the first",
            "s: `m (shard 1/2)` runs out of the order of the full check",
        ]),
        "one shard per part, since each writes the same report"
    );
}

#[test]
fn refuses_a_final_part_that_reads_not_every_shard() {
    let rr = ROUND_ROBIN;
    let gate = shards_gate;
    let full = mutation_check();
    assert_eq!(
        two(
            shard("m (shard 0/2)", "0/2", rr),
            shard("m (shard 1/2)", "1/2", rr),
            &["s0"]
        ),
        parts_refused(&[
            "the unviable-mutant gate over the shards reads mutants.out/shards/unviable-s0/unviable.txt, \
             not the reports of the shards: mutants.out/shards/unviable-s0/unviable.txt \
             mutants.out/shards/unviable-s1/unviable.txt"
        ]),
        "a shard's unviable mutants unread"
    );
    assert_eq!(
        gate(vec![
            ("s0", vec![shard("m (shard 0/1)", "0/1", rr)]),
            ("f", vec![across(&["s0"])]),
            ("g", vec![across(&["s0"])]),
        ]),
        parts_refused(&["`u` runs in 2 parts"]),
        "the gate over the shards once"
    );
    assert_eq!(
        gate(vec![
            ("s0", vec![shard("m (shard 0/1)", "0/1", rr)]),
            ("f", vec![full[1].clone()]),
        ]),
        Ok(()),
        "the gate run whole stands for itself (the full check, in one part)"
    );
    assert_eq!(
        gate(vec![("s0", vec![shard("m (shard 0/1)", "0/1", rr)])]),
        parts_refused(&["`u` runs in no part"]),
        "the unviable mutants of the shards unread"
    );
}

#[test]
fn refuses_shards_written_otherwise_than_cargo_mutants_reads_them() {
    let rr = ROUND_ROBIN;
    let gate = shards_gate;
    let full = mutation_check();
    for (value, sharding) in [
        ("+0/1", rr),
        ("00/1", rr),
        ("0/01", rr),
        ("0-1", rr),
        ("x/1", rr),
        ("0/x", rr),
        ("0/1", "--sharding=slice"),
    ] {
        let named = format!("m (shard {value})");
        assert_eq!(
            gate(vec![
                ("s0", vec![shard(&named, value, sharding)]),
                ("f", vec![full[1].clone()]),
            ]),
            parts_refused(&[
                &format!("s0: `{named}` is no step of the full check"),
                "`m` runs in no part",
            ]),
            "{value} {sharding}: only the canonical shard of cargo-mutants, round-robin"
        );
    }
    let cut = plain(
        "m (shard 0/1)",
        "cargo",
        &["mutants", "--workspace", "--shard", "0/1"],
    );
    let ended = plain("m (shard 0/1)", "cargo", &["mutants", "--shard"]);
    for step in [cut, ended] {
        assert!(
            gate(vec![("s0", vec![step]), ("f", vec![full[1].clone()])]).is_err(),
            "a command cut short is no shard"
        );
    }
    let misnamed = shard("m (shard 1/1)", "0/1", rr);
    assert_eq!(
        gate(vec![("s0", vec![misnamed]), ("f", vec![full[1].clone()])]),
        parts_refused(&[
            "s0: `m (shard 1/1)` is no step of the full check",
            "`m` runs in no part",
        ]),
        "named after the shard it runs"
    );
    let misnamed_gate = Step {
        name: "u".to_owned(),
        ..across(&["s0"])
    };
    let partial = Step {
        name: "u (every shard)".to_owned(),
        action: Action::UnviableGate(Extent::Partial),
    };
    assert_eq!(
        gate(vec![
            ("s0", vec![shard("m (shard 0/1)", "0/1", rr)]),
            ("f", vec![misnamed_gate, partial]),
        ]),
        parts_refused(&[
            "f: `u` is no step of the full check",
            "f: `u (every shard)` is no step of the full check",
            "`u` runs in no part",
        ]),
        "the gate over the shards is named after the complete gate, and stands for no other"
    );
}

#[test]
fn reviews_the_unviable_mutants_of_every_shard_together() {
    let reports: Vec<String> = ["s0", "s1"]
        .iter()
        .map(|part| super::shard_unviable_report(part))
        .collect();
    let step = [Step {
        name: "Shards".to_owned(),
        action: Action::ShardsUnviableGate(reports.clone()),
    }];
    let with = |first: &str, second: &str, reviewed: &str| {
        let mut fake = Fake::default();
        for (path, text) in reports.iter().zip([first, second]) {
            let _new = fake.files.insert(path.clone(), text.to_owned());
        }
        let _new = fake
            .files
            .insert(UNVIABLE_REVIEWED.to_owned(), reviewed.to_owned());
        run_steps(&step, &mut fake, &mut Vec::new())
    };
    let one = "crates/a/src/b.rs:3:5: replace f -> T with Default::default()";
    let other = "crates/a/src/c.rs:4:1: replace g -> U with Default::default()\n";
    let reviews = "crates/a/src/b.rs: replace f -> T with Default::default() -- T has no Default\n\
                   crates/a/src/c.rs: replace g -> U with Default::default() -- U has none\n";
    assert_eq!(
        with(one, other, reviews),
        Ok(()),
        "each shard's own, reviewed"
    );
    assert_eq!(
        with(one, "", reviews),
        failed(
            "Shards: unviable mutants must be reviewed in mutants-unviable.txt: stale review: \
             crates/a/src/c.rs: replace g -> U with Default::default()"
        ),
        "a review no shard needs is stale, as after a complete run"
    );
    assert_eq!(
        with(one, one, reviews),
        failed(
            "Shards: unviable mutants must be reviewed in mutants-unviable.txt: 2 problems, \
             listed above\n\
             not reviewed: crates/a/src/b.rs: replace f -> T with Default::default() \
             (2 found, 1 reviewed)\n\
             stale review: crates/a/src/c.rs: replace g -> U with Default::default()"
        ),
        "the shards counted together, a report without a final newline kept apart"
    );
    let mut lacking = Fake::default();
    let _new = lacking.files.insert(reports[0].clone(), one.to_owned());
    assert_eq!(
        run_steps(&step, &mut lacking, &mut Vec::new()),
        failed(format!("Shards: cannot read {}: missing", reports[1])),
        "a shard that wrote no report"
    );
}

#[test]
fn lists_and_runs_the_parts_of_the_full_check() {
    let args =
        |words: &[&str]| -> Vec<String> { words.iter().map(|word| (*word).to_owned()).collect() };
    let mut listed = Vec::new();
    assert_eq!(
        run_cli(
            &args(&["list", "--full", "--parts"]),
            &mut passing(),
            &mut listed,
            &mut Vec::new()
        ),
        0,
        "listed"
    );
    assert_eq!(
        String::from_utf8(listed).expect("UTF-8"),
        format!("{}\n", PART_NAMES.join("\n")),
        "one part per line"
    );
    let mut fake = passing();
    let mut progress = Vec::new();
    assert_eq!(
        run_cli(
            &args(&["check", "--full", "--part", "miri"]),
            &mut fake,
            &mut progress,
            &mut Vec::new()
        ),
        0,
        "the part passes"
    );
    assert!(
        String::from_utf8(progress).expect("UTF-8").ends_with(
            "▶ [4/4] Working tree unchanged by the checks\n  done in 1.5 s\n✔ all 4 checks \
                 passed in 6.0 s\n"
        ),
        "its own steps only"
    );
    let mut failing = passing();
    failing.fail = Some(super::miri_command().line());
    let mut err = Vec::new();
    let mut shown = Vec::new();
    let code = run_cli(
        &args(&["check", "--full", "--part", "miri"]),
        &mut failing,
        &mut shown,
        &mut err,
    );
    let text = String::from_utf8(shown).expect("UTF-8");
    assert_eq!(code, 1, "the part fails");
    assert!(
        text.contains("▶ [4/4] Working tree unchanged by the checks\n"),
        "every step runs whatever fails, as in the full check: {text}"
    );
    assert_eq!(
        String::from_utf8(err).expect("UTF-8"),
        "error: 1 of 4 checks failed, listed above\n",
        "every failure counted"
    );
}

#[test]
fn requires_the_workflow_to_run_every_part_of_the_full_check() {
    let gate =
        |workflow: &str| super::workflow_gate(&[super::WORKFLOW.to_owned()], workflow, "22.23.4");
    let lacks = |line: &str| {
        failed(format!(
            ".github/workflows/quality.yml can change CI without a commit: \
             .github/workflows/quality.yml lacks `{line}`"
        ))
    };
    let matrix = concat!(
        "part: [rust, coverage, mutation-0, mutation-1, mutation-2, mutation-3, mutation-4, ",
        "mutation-5, mutation-6, mutation-7, ",
        "miri",
        ", gates, tooling-config, tooling-oracle, bindings-wasm, apps-playground, apps-roadmap]"
    );
    assert_eq!(
        gate(&PINNED_WORKFLOW.replace(", mutation-7", "")),
        lacks(matrix),
        "a shard missing from the matrix"
    );
    assert_eq!(
        gate(&PINNED_WORKFLOW.replace("apps-roadmap]", "apps-roadmap, final]")),
        lacks(matrix),
        "the final part, run before the shards it reads"
    );
    for (line, instead) in [
        ("fail-fast: false", "fail-fast: true"),
        (
            "run: cargo xtask check --full --part ${{ matrix.part }}",
            "run: cargo xtask check --full --part rust",
        ),
        ("name: unviable-${{ matrix.part }}", "name: unviable"),
        (
            "path: mutants.out/unviable.txt",
            "path: mutants.out/caught.txt",
        ),
        ("needs: check", "needs: other"),
        ("pattern: unviable-*", "pattern: unviable-mutation-0"),
        ("path: mutants.out/shards", "path: mutants.out"),
        (
            "run: cargo xtask check --full --part final",
            "run: cargo xtask check --full --part gates",
        ),
    ] {
        let changed = PINNED_WORKFLOW.replace(line, instead);
        assert_ne!(changed, PINNED_WORKFLOW, "the fixture holds {line}");
        assert_eq!(gate(&changed), lacks(line), "{instead}");
    }
    assert_eq!(
        gate(&PINNED_WORKFLOW.replace(
            "        part: [",
            concat!(
                "        exclude:\n          - part: ",
                "miri",
                "\n        part: ["
            )
        )),
        failed(
            ".github/workflows/quality.yml can change CI without a commit: \
             `exclude:`: `exclude:` is not allowed"
        ),
        "a part excluded from the matrix"
    );
    assert_eq!(
        super::workflow_parts().get(1),
        Some(&matrix.to_owned()),
        "the matrix, derived from the parts"
    );
}

#[test]
fn requires_the_ci_condition_on_every_job() {
    let gate =
        |workflow: &str| super::workflow_gate(&[super::WORKFLOW.to_owned()], workflow, "22.23.4");
    let refused = |problem: &str| {
        failed(format!(
            ".github/workflows/quality.yml can change CI without a commit: {problem}"
        ))
    };
    let unconditioned = PINNED_WORKFLOW.replacen(&format!("    {CI_CONDITION}\n"), "", 1);
    assert_eq!(
        gate(&unconditioned),
        refused(&format!(
            "job `check` lacks `{CI_CONDITION}`, which every job holds"
        )),
        "a part a push would run without being asked"
    );
    let last = PINNED_WORKFLOW
        .rfind(&format!("    {CI_CONDITION}\n"))
        .expect("the final job's condition");
    let mut final_unconditioned = PINNED_WORKFLOW.to_owned();
    final_unconditioned.replace_range(last..last + CI_CONDITION.len() + 5, "");
    assert_eq!(
        gate(&final_unconditioned),
        refused(&format!(
            "job `final` lacks `{CI_CONDITION}`, which every job holds"
        )),
        "the final job too"
    );
    assert_eq!(
        gate(&PINNED_WORKFLOW.replacen(
            &format!("    {CI_CONDITION}\n"),
            &format!("      {CI_CONDITION}\n"),
            1
        )),
        refused(&format!(
            "job `check` lacks `{CI_CONDITION}`, which every job holds"
        )),
        "a condition deeper than the job's keys is not the job's"
    );
    assert_eq!(
        gate(&PINNED_WORKFLOW.replace("  final:\n", "  final: # the last\n")),
        refused("`final: # the last`: a job is written `  <name>:`"),
        "a job header the gate does not read is refused, not skipped"
    );
    assert_eq!(
        gate(&PINNED_WORKFLOW.replace("  final:\n", "  :\n")),
        refused("`:`: a job is written `  <name>:`"),
        "a job without a name"
    );
    assert_eq!(
        gate(&unconditioned.replace("jobs:\n", &format!("jobs:\n    {CI_CONDITION}\n"))),
        refused(&format!(
            "job `check` lacks `{CI_CONDITION}`, which every job holds"
        )),
        "a condition before any job is no job's"
    );
    let named = |name: &str| {
        gate(&format!(
            "{PINNED_WORKFLOW}  {name}:\n    {CI_CONDITION}\n    runs-on: ubuntu-24.04\n"
        ))
    };
    assert_eq!(
        named("a-b_C9"),
        Ok(()),
        "a job named with letters, digits, - and _"
    );
    assert_eq!(
        named("a.b"),
        refused("`a.b:`: a job is written `  <name>:`"),
        "any other character"
    );
    assert_eq!(
        gate(&final_unconditioned.replace("  final:\n", "\n  final:\n")),
        refused(&format!(
            "job `final` lacks `{CI_CONDITION}`, which every job holds"
        )),
        "a blank line does not end the jobs"
    );
    assert_eq!(
        gate(&format!(
            "{PINNED_WORKFLOW}defaults:\n  run:\n    shell: bash\n"
        )),
        Ok(()),
        "a top-level key ends the jobs"
    );
    let commented = PINNED_WORKFLOW
        .replace("  final:\n", "  # The final job.\n\n  final:\n")
        .replace("jobs:\n", "jobs:\n # one space\n");
    assert_eq!(
        gate(&commented),
        Ok(()),
        "comments and blank lines between jobs"
    );
}

/// A commit, a tree and a base of the proof record tests: full object names.
const PROVEN: [&str; 3] = [
    "1111111111111111111111111111111111111111",
    "2222222222222222222222222222222222222222",
    "3333333333333333333333333333333333333333",
];

/// The tools and the host the fake answers with.
fn fake_tools() -> super::ProofTools {
    super::ProofTools {
        mutants: "cargo-mutants 27.1.0".to_owned(),
        toolchain: "1.99.1".to_owned(),
        host: "aarch64-apple-darwin".to_owned(),
        compiler: "58cf8f9c35b355199727627d07ae34a4e2846d67 LLVM 23.1.1".to_owned(),
        linker: "Apple clang version 14.0.3 (clang-1403.0.22.14.1)".to_owned(),
        git: "git version 2.39.2 (Apple Git-143)".to_owned(),
    }
}

/// A record of the fake's tools and host for `base`, proving [`PROVEN`]'s commit.
fn proof_record(base: &str) -> super::ProofRecord {
    let [commit, tree, _] = PROVEN;
    super::ProofRecord {
        commit: commit.to_owned(),
        tree: tree.to_owned(),
        base: base.to_owned(),
        tools: fake_tools(),
    }
}

#[test]
fn reads_exactly_the_proof_records_it_writes() {
    let record = proof_record(PROVEN[2]);
    let text = record.text();
    assert_eq!(
        text,
        format!(
            "poyetik rust-mutants-proven 4\ncommit {}\ntree {}\nbase {}\ncargo-mutants cargo-mutants 27.1.0\ntoolchain 1.99.1\nhost aarch64-apple-darwin\ncompiler 58cf8f9c35b355199727627d07ae34a4e2846d67 LLVM 23.1.1\nlinker Apple clang version 14.0.3 (clang-1403.0.22.14.1)\ngit git version 2.39.2 (Apple Git-143)\n",
            PROVEN[0], PROVEN[1], PROVEN[2]
        ),
        "one field per line, after the tag"
    );
    assert_eq!(
        super::ProofRecord::parse(&text),
        Ok(record.clone()),
        "round trip"
    );
    let long = super::ProofRecord {
        commit: "a".repeat(64),
        ..record
    };
    assert_eq!(
        super::ProofRecord::parse(&long.text()),
        Ok(long),
        "SHA-256 object names"
    );
    let refused = |malformed: &str| super::ProofRecord::parse(malformed).err();
    for (malformed, reason) in [
        (
            text.trim_end().to_owned(),
            "it does not end with a line break",
        ),
        (
            text.replacen("commit ", "comit ", 1),
            "it has no `commit` line where expected",
        ),
        (
            text.replacen("toolchain 1.99.1", "toolchain ", 1),
            "it has no `toolchain` line where expected",
        ),
        (
            text.replacen("toolchain 1.99.1", "toolchain  ", 1),
            "it has no `toolchain` line where expected",
        ),
        (
            text.replacen("host aarch64-apple-darwin\n", "", 1),
            "it has no `host` line where expected",
        ),
        (
            text.replacen("compiler 58cf", "compile 58cf", 1),
            "it has no `compiler` line where expected",
        ),
        (
            text.replacen(
                "linker Apple clang version 14.0.3 (clang-1403.0.22.14.1)",
                "linker ",
                1,
            ),
            "it has no `linker` line where expected",
        ),
        (
            text.replacen("tree ", "treex", 1),
            "it has no `tree` line where expected",
        ),
        (format!("{text}extra\n"), "it has a line more"),
        (
            text.replacen(PROVEN[0], &"1".repeat(39), 1),
            "its commit is not an object name",
        ),
        (
            text.replacen(PROVEN[1], &"G".repeat(40), 1),
            "its tree is not an object name",
        ),
        (
            text.replacen(PROVEN[2], &"A".repeat(40), 1),
            "its base is not an object name",
        ),
        (
            text.replacen(PROVEN[2], &"3".repeat(41), 1),
            "its base is not an object name",
        ),
        (
            "poyetik rust-mutants-proven 4\ncommit x\n".to_owned(),
            "it has no `tree` line where expected",
        ),
    ] {
        assert_eq!(
            refused(&malformed),
            Some(reason.to_owned()),
            "{malformed:?}"
        );
    }
    for name in ["g", "/", "`", "@"] {
        assert!(
            refused(&text.replacen('1', name, 1)).is_some(),
            "{name} is no hexadecimal digit"
        );
    }
}

#[test]
fn names_the_format_of_a_proof_record_of_another_format() {
    let text = proof_record(PROVEN[2]).text();
    for (malformed, reason) in [
        (
            text.replacen("proven 4", "proven 2", 1),
            "it was written in format 2; this xtask reads format 4",
        ),
        (
            text.replacen("proven 4", "proven 3", 1),
            "it was written in format 3; this xtask reads format 4",
        ),
        (
            text.replacen("proven 4", "proven 5", 1),
            "it was written in format 5; this xtask reads format 4",
        ),
        (
            text.replacen("proven 4", "proven 10", 1),
            "it was written in format 10; this xtask reads format 4",
        ),
        (
            text.replacen("proven 4", "proven 04", 1),
            "its first line is not `poyetik rust-mutants-proven 4`",
        ),
        (
            text.replacen("proven 4", "proven +4", 1),
            "its first line is not `poyetik rust-mutants-proven 4`",
        ),
        (
            text.replacen("proven 4", "proven 4x", 1),
            "its first line is not `poyetik rust-mutants-proven 4`",
        ),
        (
            text.replacen("proven 4", "proven ", 1),
            "its first line is not `poyetik rust-mutants-proven 4`",
        ),
        (
            text.replacen("proven 4", "proven -2", 1),
            "its first line is not `poyetik rust-mutants-proven 4`",
        ),
        (
            text.replacen("proven 4", "proven  4", 1),
            "its first line is not `poyetik rust-mutants-proven 4`",
        ),
        (
            text.replacen("proven 4", "proven4", 1),
            "its first line is not `poyetik rust-mutants-proven 4`",
        ),
        (
            text.replacen("poyetik rust-mutants-proven 4", "other 2", 1),
            "its first line is not `poyetik rust-mutants-proven 4`",
        ),
    ] {
        assert_eq!(
            super::ProofRecord::parse(&malformed).err(),
            Some(reason.to_owned()),
            "{malformed:?}"
        );
    }
}

/// The path of the proof record of the base `base`, below the fake's common directory.
fn record_path(base: &str) -> String {
    format!(".git/poyetik/rust-mutants-proven/{base}")
}

/// The command reading the tree of the proven commit.
fn proven_tree_command() -> String {
    format!(
        "capture git rev-parse --verify --quiet {}^{{tree}}",
        PROVEN[0]
    )
}

/// The command reading the changes since `since`.
fn diff_since(since: &str) -> String {
    format!(
        "capture git diff --no-color --no-ext-diff --no-textconv --no-renames --src-prefix=a/ --dst-prefix=b/ {since}"
    )
}

/// The command listing the untracked files, for the changes since a proof record.
const UNTRACKED: &str = "capture git ls-files -z --others --exclude-standard";

/// The changes since the base of the proof record tests: a line of the engine core and a
/// line of the render hash.
fn since_base() -> String {
    format!(
        "{}{}",
        one_line_change("crates/engine-core/src/plan.rs"),
        one_line_change("crates/engine-hash/src/lib.rs")
    )
}

/// Cargo metadata where the render hash depends on the engine core when `linked`, and on
/// no crate of the workspace otherwise; the boxes always depend on the engine core.
fn closure_metadata(linked: bool) -> String {
    let hash = if linked {
        r#"[{"name":"engine-core","kind":null,"path":"/r/crates/engine-core"},{"name":"sha2","kind":null}]"#
    } else {
        r#"[{"name":"sha2","kind":null}]"#
    };
    format!(
        r#"{{"packages":[{{"name":"engine-hash","dependencies":{hash}}},{{"name":"engine-objects","dependencies":[{{"name":"engine-core","kind":null,"path":"/r/crates/engine-core"}},{{"name":"engine-testkit","kind":"dev","path":"/r/crates/engine-testkit"}}]}},{{"name":"engine-core","dependencies":[]}},{{"name":"serde","dependencies":[]}}]}}"#
    )
}

/// A passing fake whose base is [`PROVEN`]'s, holding `record` as its proof record, the
/// tree of the proven commit, `since_commit` as the changes since it, no untracked file,
/// [`since_base`] as the changes since the base, and [`closure_metadata`] unlinked.
fn proven_fake(record: &super::ProofRecord, since_commit: &str) -> Fake {
    let mut fake = passing();
    for (key, value) in [
        (record_path(PROVEN[2]), record.text()),
        (proven_tree_command(), format!("{}\n", PROVEN[1])),
        (diff_since(PROVEN[0]), since_commit.to_owned()),
        (diff_since(PROVEN[2]), since_base()),
        (UNTRACKED.to_owned(), String::new()),
        (
            format!("capture {}", MERGE_BASE.0),
            format!("{}\n", PROVEN[2]),
        ),
        (
            format!("capture {}", super::metadata_command().line()),
            closure_metadata(false),
        ),
    ] {
        drop(fake.files.insert(key, value));
    }
    fake
}

/// Runs the mutation of the changes on `fake`, and returns what it said.
fn mutate_changes(fake: &mut Fake) -> String {
    let mut out = Vec::new();
    assert_eq!(
        run_steps(&rust_mutation_of_changes(), fake, &mut out),
        Ok(()),
        "mutated"
    );
    String::from_utf8(out).expect("UTF-8")
}

/// What the default check says of a crate a record still proves.
fn proven_line(name: &str) -> String {
    format!(
        "  {name}: proven by an earlier check at {}, unchanged since\n",
        PROVEN[0]
    )
}

#[test]
fn skips_only_the_crates_a_valid_proof_record_still_proves() {
    let plan = one_line_change("crates/engine-core/src/plan.rs");
    let mut fake = proven_fake(&proof_record(PROVEN[2]), &plan);
    let said = mutate_changes(&mut fake);
    assert_eq!(
        fake.files.get(CHANGES_DIFF).cloned(),
        Some(plan.clone()),
        "the changes since the base of the crate changed since the record; the render hash, \
         unchanged since and depending on nothing changed, left out"
    );
    assert!(
        said.contains(&format!(
            "  the changes since {} (merge base with @{{upstream}})\n",
            PROVEN[2]
        )) && said.contains(&proven_line("engine-hash"))
            && !said.contains(&proven_line("engine-core")),
        "said: {said}"
    );
    assert_eq!(
        fake.files.get(super::MEASURED_BASE).cloned(),
        Some(format!("{}\n", PROVEN[2])),
        "the base noted for the record of this run"
    );
    let mut linked = proven_fake(&proof_record(PROVEN[2]), &plan);
    drop(linked.files.insert(
        format!("capture {}", super::metadata_command().line()),
        closure_metadata(true),
    ));
    let linked_said = mutate_changes(&mut linked);
    assert_eq!(
        linked.files.get(CHANGES_DIFF).cloned(),
        Some(since_base()),
        "a crate whose dependency changed since the record is mutated as without it"
    );
    assert!(
        !linked_said.contains("proven by an earlier check"),
        "linked_said: {linked_said}"
    );
    // A line changed since the base, its neighbor since the record: the crate's changes
    // since the base are mutated whole, not only what changed since the record.
    let mut neighbor = proven_fake(&proof_record(PROVEN[2]), &plan);
    let two = "diff --git a/crates/engine-core/src/plan.rs b/crates/engine-core/src/plan.rs\n--- a/crates/engine-core/src/plan.rs\n+++ b/crates/engine-core/src/plan.rs\n@@ -1 +1 @@\n-a\n+b\n@@ -9 +9 @@\n-c\n+d\n";
    drop(neighbor.files.insert(diff_since(PROVEN[2]), two.to_owned()));
    let _said = mutate_changes(&mut neighbor);
    assert_eq!(
        neighbor.files.get(CHANGES_DIFF).map(String::as_str),
        Some(two),
        "every change of the crate since the base"
    );
}

#[test]
fn mutates_nothing_when_every_change_is_proven() {
    for since_commit in [
        String::new(),
        one_line_change("apps/roadmap/board.json"),
        one_line_change("crates/engine-cli/src/lib.rs"),
    ] {
        let mut fake = proven_fake(&proof_record(PROVEN[2]), &since_commit);
        drop(fake.files.remove(UNVIABLE_REPORT));
        let said = mutate_changes(&mut fake);
        assert!(
            said.contains(&proven_line("engine-core"))
                && said.contains(&proven_line("engine-hash"))
                && said.contains("  every change is proven: no mutant to test\n"),
            "{since_commit}: said: {said}"
        );
        assert!(
            !fake
                .ran
                .iter()
                .any(|line| line.starts_with("cargo mutants")),
            "{since_commit}: nothing mutated"
        );
    }
}

#[test]
fn widens_a_crate_that_changed_since_the_record_as_without_it() {
    // Test code changed since the base, before the record: proven, not widened again.
    let mut before = proven_fake(&proof_record(PROVEN[2]), "");
    drop(before.files.insert(
        diff_since(PROVEN[2]),
        format!(
            "{}{}",
            since_base(),
            one_line_change("crates/engine-hash/src/lib_tests.rs")
        ),
    ));
    let said = mutate_changes(&mut before);
    assert!(
        !said.contains("the test code of engine-hash changed"),
        "said: {said}"
    );
    // The same, with the render hash changed since the record: widened whole.
    let mut after = proven_fake(
        &proof_record(PROVEN[2]),
        &one_line_change("crates/engine-hash/src/lib.rs"),
    );
    drop(after.files.insert(
        diff_since(PROVEN[2]),
        format!(
            "{}{}",
            since_base(),
            one_line_change("crates/engine-hash/src/lib_tests.rs")
        ),
    ));
    drop(after.files.insert(
        "crates/engine-hash/src/lib.rs".to_owned(),
        "fn h() {}\n".to_owned(),
    ));
    let after_said = mutate_changes(&mut after);
    assert!(
        after_said
            .contains("  the test code of engine-hash changed: every mutant of engine-hash\n")
            && after_said.contains(&proven_line("engine-core")),
        "after_said: {after_said}"
    );
    assert_eq!(
        after.files.get(CHANGES_DIFF).cloned(),
        Some(super::new_file_diff(
            "crates/engine-hash/src/lib.rs",
            "fn h() {}\n"
        )),
        "the render hash widened, its source once and whole, the engine core left out"
    );
    // Its test data changed since the record, or an untracked file under it appeared.
    let mut data = proven_fake(
        &proof_record(PROVEN[2]),
        &one_line_change("spec/render-hash-vectors.json"),
    );
    assert!(
        !mutate_changes(&mut data).contains(&proven_line("engine-hash")),
        "test data changed since the record"
    );
    let mut untracked = proven_fake(&proof_record(PROVEN[2]), "");
    drop(untracked.files.insert(
        UNTRACKED.to_owned(),
        "conformance/new.json\0crates/engine-hash/notes.txt\0".to_owned(),
    ));
    let untracked_said = mutate_changes(&mut untracked);
    assert!(
        !untracked_said.contains(&proven_line("engine-hash"))
            && untracked_said.contains(&proven_line("engine-core")),
        "an untracked file in a crate: untracked_said: {untracked_said}"
    );
}

#[test]
fn mutates_every_crate_not_proven_whole_when_what_every_test_uses_changed() {
    // What every test depends on changed since the base, before the record: every crate
    // not proven, whole; the proven ones left out.
    let mut everything = proven_fake(
        &proof_record(PROVEN[2]),
        &one_line_change("crates/engine-hash/src/lib.rs"),
    );
    drop(everything.files.insert(
        diff_since(PROVEN[2]),
        format!("{}{}", since_base(), one_line_change("Cargo.lock")),
    ));
    drop(everything.files.insert(
        "crates/engine-hash/src/lib.rs".to_owned(),
        "fn h() {}\n".to_owned(),
    ));
    drop(everything.files.insert(
        "crates/engine-core/src/plan.rs".to_owned(),
        "fn p() {}\n".to_owned(),
    ));
    let everything_said = mutate_changes(&mut everything);
    let diff = everything
        .files
        .get(CHANGES_DIFF)
        .cloned()
        .unwrap_or_default();
    let proven: String = [
        "engine-core",
        "engine-objects",
        "engine-patch",
        "engine-alloc-counter",
        "engine-fpenv",
        "engine-battery",
        "engine-testkit",
        "engine-cli",
        "engine-wasm",
        "xtask",
    ]
    .iter()
    .map(|name| proven_line(name))
    .collect();
    assert_eq!(
        everything_said,
        format!(
            "▶ [1/2] Rust mutation testing of the changes\n  the changes since {} (merge base \
             with @{{upstream}})\n  Cargo.lock changed and cannot be narrowed (`git show {}:Cargo.lock` \
             exited with code 1: failed): every Rust mutant but the crates proven below\n{proven}  done in 1.5 s\n▶ [2/2] Rust unviable mutants reviewed\n  done in \
             1.5 s\n✔ all 2 checks passed in 3.0 s\n",
            PROVEN[2], PROVEN[2]
        ),
        "every crate proven named, the render hash mutated whole"
    );
    assert_eq!(
        diff,
        format!(
            "{}{}",
            one_line_change("Cargo.lock"),
            super::new_file_diff("crates/engine-hash/src/lib.rs", "fn h() {}\n")
        ),
        "every crate is tested whole or proven: git keeps only what no crate holds"
    );
}

#[test]
fn names_each_proven_crate_the_check_would_otherwise_have_tested() {
    // The shared test crate changed since the base, before the record: the boxes, which
    // only it widens, are proven, and said so; the render hash, out of scope, is not named.
    let mut fake = proven_fake(&proof_record(PROVEN[2]), "");
    drop(fake.files.insert(
        diff_since(PROVEN[2]),
        one_line_change("crates/engine-testkit/src/lib.rs"),
    ));
    drop(fake.files.remove(UNVIABLE_REPORT));
    let said = mutate_changes(&mut fake);
    assert_eq!(
        said,
        format!(
            "▶ [1/2] Rust mutation testing of the changes\n  the changes since {} (merge base \
             with @{{upstream}})\n{}{}  every change is proven: no mutant to test\n  done in \
             1.5 s\n▶ [2/2] Rust unviable mutants reviewed\n  done in 1.5 s\n✔ all 2 checks \
             passed in 3.0 s\n",
            PROVEN[2],
            proven_line("engine-objects"),
            proven_line("engine-testkit")
        ),
        "the crates in scope only through the widening are named"
    );
}

#[test]
fn reuses_no_record_of_a_crate_whose_quoted_path_changed() {
    let mut fake = proven_fake(&proof_record(PROVEN[2]), "");
    drop(fake.files.insert(
        format!(
            "capture git diff --no-color --name-only -z --no-renames {}",
            PROVEN[0]
        ),
        "crates/engine-hash/tests/caf\u{e9}.json\0".to_owned(),
    ));
    let said = mutate_changes(&mut fake);
    assert!(
        !said.contains(&proven_line("engine-hash")) && said.contains(&proven_line("engine-core")),
        "a path git would quote is a change of its crate: {said}"
    );
}

/// Checks that `fake` ignores its proof record for `reason`, saying so, and mutates the
/// changes since the base as without it.
fn ignored(fake: &mut Fake, reason: &str) {
    let said = mutate_changes(fake);
    assert!(
        said.contains(&format!(
            "  ignored the mutation proof record {}: {reason}\n",
            record_path(PROVEN[2])
        )) && !said.contains("proven by an earlier check"),
        "{reason}: said, then nothing proven: {said}"
    );
    assert_eq!(
        fake.files.get(CHANGES_DIFF).cloned(),
        Some(since_base()),
        "{reason}: the diff since the base"
    );
}

#[test]
fn ignores_a_proof_record_it_cannot_trust() {
    let other = |edit: &dyn Fn(&mut super::ProofRecord)| {
        let mut record = proof_record(PROVEN[2]);
        edit(&mut record);
        proven_fake(&record, "")
    };
    ignored(
        &mut other(&|record| record.base = "4".repeat(40)),
        &format!("it is of the base {}", "4".repeat(40)),
    );
    ignored(
        &mut other(&|record| record.tools.mutants = "cargo-mutants 27.0.0".to_owned()),
        "it was made with the cargo-mutants cargo-mutants 27.0.0, not cargo-mutants 27.1.0",
    );
    ignored(
        &mut other(&|record| record.tools.toolchain = "1.98.0".to_owned()),
        "it was made with the toolchain 1.98.0, not 1.99.1",
    );
    ignored(
        &mut other(&|record| record.tools.host = "x86_64-apple-darwin".to_owned()),
        "it was made with the host x86_64-apple-darwin, not aarch64-apple-darwin",
    );
    let mut gone = proven_fake(&proof_record(PROVEN[2]), "");
    drop(gone.files.remove(&proven_tree_command()));
    ignored(
        &mut gone,
        &format!(
            "git cannot find the tree of its commit {0}: `git rev-parse --verify --quiet {0}^{{tree}}` exited with code 1: failed",
            PROVEN[0]
        ),
    );
    let mut retreed = proven_fake(&proof_record(PROVEN[2]), "");
    drop(
        retreed
            .files
            .insert(proven_tree_command(), format!("{}\n", PROVEN[2])),
    );
    ignored(
        &mut retreed,
        &format!("git has no commit {} of tree {}", PROVEN[0], PROVEN[1]),
    );
    let mut malformed = proven_fake(&proof_record(PROVEN[2]), "");
    drop(
        malformed
            .files
            .insert(record_path(PROVEN[2]), "nonsense\n".to_owned()),
    );
    ignored(
        &mut malformed,
        "its first line is not `poyetik rust-mutants-proven 4`",
    );
    let mut unreadable = proven_fake(&proof_record(PROVEN[2]), "");
    unreadable.broken = Some(format!("read {}", record_path(PROVEN[2])));
    ignored(&mut unreadable, "cannot read it: not UTF-8");
    let mut no_tools = proven_fake(&proof_record(PROVEN[2]), "");
    no_tools.broken = Some("cargo mutants --version".to_owned());
    ignored(
        &mut no_tools,
        "cannot read the version of cargo-mutants: cannot run `cargo mutants --version`: not found",
    );
    let mut no_host = proven_fake(&proof_record(PROVEN[2]), "");
    no_host.broken = Some("rustc -vV".to_owned());
    ignored(
        &mut no_host,
        "cannot read the host of rustc: cannot run `rustc -vV`: not found",
    );
    let mut hostless = proven_fake(&proof_record(PROVEN[2]), "");
    drop(hostless.files.insert(
        format!("capture {}", HOST.0),
        "rustc 1.99.1\nhost: \n".to_owned(),
    ));
    ignored(&mut hostless, "`rustc -vV` names no host");
    let mut absent = proven_fake(&proof_record(PROVEN[2]), "");
    drop(absent.files.remove(&record_path(PROVEN[2])));
    assert!(
        !mutate_changes(&mut absent).contains("proof record"),
        "no record: nothing to say"
    );
}

#[test]
fn ignores_a_proof_record_when_its_rules_or_shared_inputs_changed() {
    for (path, what, tracked) in [
        ("crates/xtask/src/lib.rs", "the mutation rules", true),
        ("mutants-unviable.txt", "the mutation rules", true),
        (".cargo/config.toml", "the mutation rules", true),
        (".cargo/mutants.toml", "the mutation rules", false),
        (
            "Cargo.lock",
            "and cannot be narrowed (`git show 1111111111111111111111111111111111111111:Cargo.lock` \
             exited with code 1: failed), what every test depends on",
            true,
        ),
        ("rust-toolchain.toml", "what every test depends on", true),
        ("Cargo.toml", "what every test depends on", true),
    ] {
        let mut fake = if tracked {
            proven_fake(&proof_record(PROVEN[2]), &one_line_change(path))
        } else {
            let mut untracked = proven_fake(&proof_record(PROVEN[2]), "");
            drop(
                untracked
                    .files
                    .insert(UNTRACKED.to_owned(), format!("{path}\0")),
            );
            untracked
        };
        let said = mutate_changes(&mut fake);
        let comma = if what.starts_with("and ") { " " } else { ", " };
        assert!(
            said.contains(&format!(
                "  ignored the mutation proof record of {}: {path} changed since{comma}{what}\n",
                PROVEN[0]
            )) && !said.contains("proven by an earlier check"),
            "{path}: said: {said}"
        );
        assert_eq!(
            fake.files.get(CHANGES_DIFF).cloned(),
            Some(since_base()),
            "{path}: as without a record"
        );
    }
    let mut tests = proven_fake(
        &proof_record(PROVEN[2]),
        &one_line_change("crates/xtask/src/lib_tests.rs"),
    );
    let said = mutate_changes(&mut tests);
    assert!(
        !said.contains("ignored the mutation proof record")
            && said.contains(&proven_line("engine-core")),
        "xtask's tests are not its rules: said: {said}"
    );
}

#[test]
fn falls_back_without_a_common_directory() {
    let mut fake = proven_fake(&proof_record(PROVEN[2]), "");
    drop(fake.files.remove(&format!("capture {}", COMMON_DIR.0)));
    let said = mutate_changes(&mut fake);
    assert!(
        said.contains(
            "  no mutation proof record: cannot find git's common directory: `git rev-parse --git-common-dir` exited with code 1: failed\n"
        ),
        "said: {said}"
    );
    assert_eq!(
        fake.files.get(CHANGES_DIFF).cloned(),
        Some(since_base()),
        "the diff since the base"
    );
    let mut unwritable = passing();
    unwritable.broken = Some(format!("write {}", super::MEASURED_BASE));
    assert_eq!(
        run_steps(
            &rust_mutation_of_changes(),
            &mut unwritable,
            &mut Vec::new()
        ),
        failed(format!(
            "Rust mutation testing of the changes: cannot write {}: read-only",
            super::MEASURED_BASE
        )),
        "the base measured cannot be noted"
    );
    for (broken, expected) in [
        (
            UNTRACKED
                .strip_prefix("capture ")
                .unwrap_or_default()
                .to_owned(),
            "cannot list the untracked files: cannot run `git ls-files -z --others --exclude-standard`: not found",
        ),
        (
            super::metadata_command().line(),
            "cannot list the crates: cannot run `cargo metadata --format-version 1 --no-deps \
             --locked`: not found",
        ),
    ] {
        let mut broken_fake = proven_fake(&proof_record(PROVEN[2]), "");
        broken_fake.broken = Some(broken.clone());
        assert_eq!(
            run_steps(
                &rust_mutation_of_changes(),
                &mut broken_fake,
                &mut Vec::new()
            ),
            failed(format!("Rust mutation testing of the changes: {expected}")),
            "{broken}: the record cannot be weighed"
        );
    }
    let mut invalid = proven_fake(&proof_record(PROVEN[2]), "");
    drop(invalid.files.insert(
        format!("capture {}", super::metadata_command().line()),
        "{}".to_owned(),
    ));
    assert_eq!(
        run_steps(&rust_mutation_of_changes(), &mut invalid, &mut Vec::new()),
        failed("Rust mutation testing of the changes: cargo metadata lists no packages"),
        "metadata without packages"
    );
}

/// The steps that record the proof: the tree recorded, then the record.
fn proof_steps() -> Vec<Step> {
    vec![
        Step {
            name: "Working tree recorded".to_owned(),
            action: Action::TreeRecorded,
        },
        Step {
            name: "Rust mutation proof recorded for the next default check".to_owned(),
            action: Action::MutationProofRecorded,
        },
    ]
}

/// A passing fake that measured [`PROVEN`]'s base.
fn measured() -> Fake {
    let mut fake = passing();
    drop(
        fake.files
            .insert(super::MEASURED_BASE.to_owned(), format!("{}\n", PROVEN[2])),
    );
    fake
}

/// The commit and tree the fake checks out, full object names.
const CHECKED_OUT: [&str; 2] = [
    "abc1230000000000000000000000000000000000",
    "def4560000000000000000000000000000000000",
];

#[test]
fn records_the_proof_of_a_clean_tree() {
    let mut fake = measured();
    let mut out = Vec::new();
    assert_eq!(
        run_steps(&proof_steps(), &mut fake, &mut out),
        Ok(()),
        "recorded"
    );
    let record = super::ProofRecord {
        commit: CHECKED_OUT[0].to_owned(),
        tree: CHECKED_OUT[1].to_owned(),
        base: PROVEN[2].to_owned(),
        tools: fake_tools(),
    };
    assert_eq!(
        fake.files.get(&record_path(PROVEN[2])).cloned(),
        Some(format!(
            "poyetik rust-mutants-proven 4\ncommit {}\ntree {}\nbase {}\ncargo-mutants cargo-mutants 27.1.0\ntoolchain 1.99.1\nhost aarch64-apple-darwin\ncompiler 58cf8f9c35b355199727627d07ae34a4e2846d67 LLVM 23.1.1\nlinker Apple clang version 14.0.3 (clang-1403.0.22.14.1)\ngit git version 2.39.2 (Apple Git-143)\n",
            CHECKED_OUT[0], CHECKED_OUT[1], PROVEN[2]
        )),
        "the commit and tree checked out, the base measured, the tools, the host"
    );
    assert_eq!(
        super::ProofRecord::parse(
            &fake
                .files
                .get(&record_path(PROVEN[2]))
                .cloned()
                .unwrap_or_default()
        ),
        Ok(record),
        "readable as written"
    );
    assert!(
        String::from_utf8(out).expect("UTF-8").contains(&format!(
            "  recorded: the mutants of {} are proven against {}\n",
            CHECKED_OUT[0], PROVEN[2]
        )),
        "said"
    );
}

#[test]
fn uses_the_proof_record_it_wrote() {
    let mut fake = measured();
    assert_eq!(
        run_steps(&proof_steps(), &mut fake, &mut Vec::new()),
        Ok(()),
        "recorded"
    );
    for (key, value) in [
        (
            format!(
                "capture git rev-parse --verify --quiet {}^{{tree}}",
                CHECKED_OUT[0]
            ),
            format!("{}\n", CHECKED_OUT[1]),
        ),
        (diff_since(CHECKED_OUT[0]), String::new()),
        (diff_since(PROVEN[2]), since_base()),
        (UNTRACKED.to_owned(), String::new()),
        (
            format!("capture {}", MERGE_BASE.0),
            format!("{}\n", PROVEN[2]),
        ),
    ] {
        drop(fake.files.insert(key, value));
    }
    let said = mutate_changes(&mut fake);
    assert!(
        said.contains(&format!(
            "  engine-core: proven by an earlier check at {}, unchanged since\n",
            CHECKED_OUT[0]
        )) && said.contains("  every change is proven: no mutant to test\n"),
        "the record written is the record read: {said}"
    );
}

#[test]
fn records_no_proof_of_a_tree_not_clean() {
    for (status, hidden, reason) in [
        (
            " M crates/a/src/b.rs\n",
            "",
            "` M crates/a/src/b.rs` is not committed",
        ),
        (
            "?? crates/a/src/new.rs\n",
            "",
            "`?? crates/a/src/new.rs` is not committed",
        ),
        (
            "?? conformance/x.json\n",
            "",
            "`?? conformance/x.json` is not committed",
        ),
        ("?? \"a b.rs\"\n", "", "`?? \"a b.rs\"` is not committed"),
        (
            "?? .cargo/mutants.toml\n",
            "",
            "`?? .cargo/mutants.toml` is not committed",
        ),
        (
            "",
            "H Cargo.toml\nh crates/engine-core/src/plan.rs\n",
            "`crates/engine-core/src/plan.rs` is hidden from git status (assume-unchanged or skip-worktree)",
        ),
        (
            "",
            "S crates/engine-core/src/plan.rs\n",
            "`crates/engine-core/src/plan.rs` is hidden from git status (assume-unchanged or skip-worktree)",
        ),
    ] {
        let mut dirty = measured();
        for (command, answer) in [(PUSH_STATUS.0, status), (HIDDEN.0, hidden)] {
            drop(
                dirty
                    .files
                    .insert(format!("capture {command}"), answer.to_owned()),
            );
        }
        let mut out = Vec::new();
        assert_eq!(
            run_steps(&proof_steps(), &mut dirty, &mut out),
            Ok(()),
            "{status}{hidden}"
        );
        assert!(
            !dirty.ran.iter().any(|line| line.starts_with("write")),
            "{status}{hidden}: nothing recorded"
        );
        assert_eq!(
            String::from_utf8(out).expect("UTF-8"),
            format!(
                "▶ [1/2] Working tree recorded\n  done in 1.5 s\n▶ [2/2] Rust mutation proof recorded for the next default check\n  not recorded: the tree is not clean: {reason}\n  done in 1.5 s\n✔ all 2 checks passed in 3.0 s\n"
            ),
            "{status}{hidden}: said"
        );
    }
}

#[test]
fn records_no_proof_of_a_tree_that_changed() {
    let changed_while = "  not recorded: the tree changed while the checks ran, or was not \
                         recorded at their start\n";
    let mut unrecorded = measured();
    let mut out = Vec::new();
    assert_eq!(
        run_steps(&proof_steps().split_off(1), &mut unrecorded, &mut out),
        Ok(()),
        "no state of the tree"
    );
    assert!(
        !unrecorded.ran.iter().any(|line| line.starts_with("write"))
            && String::from_utf8(out)
                .expect("UTF-8")
                .contains(changed_while),
        "nothing recorded without the tree's state at the start"
    );
    let mut moved = measured();
    moved.rewrite = Some((
        "git checkout other".to_owned(),
        format!("capture {}", TREE_HEAD.0),
        "fed9870000000000000000000000000000000000\nrefs/heads/other\n".to_owned(),
    ));
    let mut steps = proof_steps();
    steps.insert(
        1,
        Step {
            name: "move".to_owned(),
            action: Action::Run(Cmd::new("git", &["checkout", "other"], Dir::Root)),
        },
    );
    let mut moved_out = Vec::new();
    assert_eq!(
        run_steps(&steps, &mut moved, &mut moved_out),
        Ok(()),
        "moved"
    );
    assert!(
        !moved.ran.iter().any(|line| line.starts_with("write"))
            && String::from_utf8(moved_out)
                .expect("UTF-8")
                .contains(changed_while),
        "nothing recorded when what is checked out changed"
    );
}

/// Runs the proof steps on `fake`, which must succeed without writing a record, and returns
/// what the record step said.
fn not_recorded(fake: &mut Fake) -> String {
    let mut out = Vec::new();
    assert_eq!(
        run_steps(&proof_steps(), fake, &mut out),
        Ok(()),
        "the record is optional"
    );
    assert!(
        !fake.files.contains_key(&record_path(PROVEN[2])),
        "nothing recorded"
    );
    let said = String::from_utf8(out).expect("UTF-8");
    said.lines()
        .find(|line| line.starts_with("  not recorded: "))
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn records_nothing_when_it_cannot_read_or_write() {
    for (broken, reason) in [
        (
            "git log -1 --format=%T HEAD".to_owned(),
            "cannot read the tree checked out: cannot run `git log -1 --format=%T HEAD`: not found"
                .to_owned(),
        ),
        (
            "cargo mutants --version".to_owned(),
            "cannot read the version of cargo-mutants: cannot run `cargo mutants --version`: not found"
                .to_owned(),
        ),
        (
            "rustc -vV".to_owned(),
            "cannot read the host of rustc: cannot run `rustc -vV`: not found".to_owned(),
        ),
        (
            "git ls-files -v".to_owned(),
            "cannot read the work tree: cannot run `git ls-files -v`: not found".to_owned(),
        ),
        (
            PUSH_STATUS.0.to_owned(),
            format!("cannot read the work tree: cannot run `{}`: not found", PUSH_STATUS.0),
        ),
        (
            format!("write {}", record_path(PROVEN[2])),
            format!("cannot write {}: read-only", record_path(PROVEN[2])),
        ),
    ] {
        let mut fake = measured();
        fake.broken = Some(broken.clone());
        assert_eq!(
            not_recorded(&mut fake),
            format!("  not recorded: {reason}"),
            "{broken}"
        );
    }
    let mut no_base = passing();
    assert_eq!(
        not_recorded(&mut no_base),
        format!(
            "  not recorded: cannot read {}: missing",
            super::MEASURED_BASE
        ),
        "no base measured"
    );
    let mut no_host = measured();
    drop(
        no_host
            .files
            .insert(format!("capture {}", HOST.0), "release: 1\n".to_owned()),
    );
    assert_eq!(
        not_recorded(&mut no_host),
        "  not recorded: `rustc -vV` names no host",
        "no host"
    );
    let mut no_channel = measured();
    drop(no_channel.files.insert(
        super::RUST_TOOLCHAIN_FILE.to_owned(),
        "[toolchain]\n".to_owned(),
    ));
    assert_eq!(
        not_recorded(&mut no_channel),
        "  not recorded: rust-toolchain.toml pins no channel",
        "no channel"
    );
    let mut no_common = measured();
    drop(no_common.files.remove(&format!("capture {}", COMMON_DIR.0)));
    assert_eq!(
        not_recorded(&mut no_common),
        format!(
            "  not recorded: cannot find git's common directory: `{}` exited with code 1: failed",
            COMMON_DIR.0
        ),
        "no common directory"
    );
}

#[test]
fn fails_to_record_what_it_would_not_read_back() {
    let step = "Rust mutation proof recorded for the next default check";
    let mut empty_base = measured();
    drop(
        empty_base
            .files
            .insert(super::MEASURED_BASE.to_owned(), "\n".to_owned()),
    );
    assert_eq!(
        run_steps(&proof_steps(), &mut empty_base, &mut Vec::new()),
        failed(format!(
            "{step}: cannot record the mutation proof: it has no `base` line where expected"
        )),
        "an empty base is never written"
    );
    assert!(
        !empty_base
            .ran
            .iter()
            .any(|line| line.starts_with("write .git/poyetik/")),
        "nothing written"
    );
    let mut spaced = measured();
    drop(spaced.files.insert(
        format!("capture {}", HOST.0),
        "commit-hash: 1\nhost: a b\nhost: c\nLLVM version: 2\n".to_owned(),
    ));
    assert_eq!(
        run_steps(&proof_steps(), &mut spaced, &mut Vec::new()),
        Ok(()),
        "a host with a space is written as it is, and reads back"
    );
    assert!(
        spaced
            .files
            .get(&record_path(PROVEN[2]))
            .is_some_and(|text| text.contains("\nhost a b\n")),
        "recorded"
    );
}

#[test]
fn reads_the_crates_each_crate_compiles_for_its_tests() {
    let closures = super::crate_closures(&closure_metadata(true)).expect("valid metadata");
    let names = |name: &str| -> Vec<&str> {
        closures
            .get(name)
            .map(|closure| closure.iter().copied().collect())
            .unwrap_or_default()
    };
    assert_eq!(
        names("engine-objects"),
        ["engine-core", "engine-objects", "engine-testkit"],
        "its dependencies of every kind, and itself"
    );
    assert_eq!(
        names("engine-hash"),
        ["engine-core", "engine-hash"],
        "linked"
    );
    assert_eq!(
        names("xtask"),
        ["xtask"],
        "a crate the metadata does not list"
    );
    let deep = r#"{"packages":[{"name":"engine-cli","dependencies":[{"name":"engine-hash","path":"/r"}]},{"name":"engine-hash","dependencies":[{"name":"engine-core","path":"/r"},{"name":"other","path":"/r"}]}]}"#;
    let deep_closures = super::crate_closures(deep).expect("valid metadata");
    assert_eq!(
        deep_closures
            .get("engine-cli")
            .map(|closure| closure.iter().copied().collect::<Vec<_>>()),
        Some(vec!["engine-cli", "engine-core", "engine-hash"]),
        "at any depth, crates outside the workspace apart"
    );
    assert!(
        super::crate_closures("not json")
            .is_err_and(|failure| failure.summary.starts_with("invalid cargo metadata: ")),
        "not JSON"
    );
}

#[test]
fn reads_the_crates_real_cargo_metadata_lists() {
    let (metadata, _) = fixture_metadata();
    let closures = super::crate_closures(&metadata).expect("valid metadata");
    let of = |name: &str| {
        closures
            .get(name)
            .map(|closure| closure.iter().copied().collect::<Vec<_>>())
    };
    assert_eq!(
        of("engine-objects"),
        Some(vec![
            "engine-alloc-counter",
            "engine-cli",
            "engine-core",
            "engine-objects",
            "engine-testkit"
        ]),
        "normal and development dependencies, at any depth"
    );
    assert_eq!(
        of("engine-fpenv"),
        Some(vec![
            "engine-alloc-counter",
            "engine-core",
            "engine-fpenv",
            "engine-testkit"
        ]),
        "a development dependency's own"
    );
    assert_eq!(
        of("xtask"),
        Some(vec!["xtask"]),
        "a crate the metadata does not list"
    );
}
#[test]
fn shares_proof_records_between_the_worktrees_of_real_git() {
    let dir = scratch("proof-record");
    let main = dir.join("main");
    std::fs::create_dir_all(&main).expect("main worktree");
    let _init = hermetic_git(&main, &["init", "--quiet", "--initial-branch=main"]);
    std::fs::write(main.join("a.rs"), "fn a() {}\n").expect("a source");
    let _add = hermetic_git(&main, &["add", "a.rs"]);
    let _commit = hermetic_git(&main, &["commit", "--quiet", "-m", "first"]);
    let commit = hermetic_git(&main, &["rev-parse", "HEAD"])
        .trim()
        .to_owned();
    let tree = hermetic_git(&main, &["log", "-1", "--format=%T", "HEAD"])
        .trim()
        .to_owned();
    let linked = dir.join("linked");
    let _worktree = hermetic_git(
        &main,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "other",
            linked.to_str().expect("UTF-8"),
        ],
    );
    let record = super::ProofRecord {
        commit: commit.clone(),
        tree,
        base: commit.clone(),
        tools: fake_tools(),
    };
    // Written from the linked worktree, read from the main one: one common directory.
    let mut from_linked = SystemRunner { root: linked };
    let path = super::proof_record_path(&mut from_linked, &commit).expect("common directory");
    from_linked
        .write(&path, &record.text())
        .expect("the record is written");
    let mut from_main = SystemRunner { root: main.clone() };
    let main_path = super::proof_record_path(&mut from_main, &commit).expect("common directory");
    let read = from_main.read(&main_path).expect("the record is read");
    assert_eq!(
        super::ProofRecord::parse(&read),
        Ok(record),
        "the same record"
    );
    // Real git names the proven commit's tree, and nothing for a commit it does not have.
    let peel = |runner: &mut SystemRunner, object: &str| {
        runner
            .capture(&Cmd::new(
                "git",
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    &format!("{object}^{{tree}}"),
                ],
                Dir::Root,
            ))
            .map(|named| named.trim().to_owned())
            .ok()
    };
    assert_eq!(
        peel(&mut from_main, &commit),
        Some(
            hermetic_git(&main, &["log", "-1", "--format=%T", "HEAD"])
                .trim()
                .to_owned()
        ),
        "the tree of a commit git has"
    );
    assert_eq!(
        peel(&mut from_main, &"1".repeat(40)),
        None,
        "nothing for a commit git does not have"
    );
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

/// The Cargo metadata of a workspace at `/r` of one crate whose targets are these: kind,
/// root (relative to `/r`), whether it is tested, whether its doc tests are.
fn targets_metadata(targets: &[(&str, &str, bool, bool)]) -> String {
    let targets: Vec<String> = targets
        .iter()
        .map(|(kind, root, test, doctest)| {
            format!(
                r#"{{"kind":["{kind}"],"src_path":"/r/{root}","test":{test},"doctest":{doctest}}}"#
            )
        })
        .collect();
    format!(
        r#"{{"workspace_root":"/r","packages":[{{"name":"x","manifest_path":"/r/crates/x/Cargo.toml","targets":[{}]}}]}}"#,
        targets.join(",")
    )
}

/// The failure of the untested targets gate with `problems`.
fn untested_refused(problems: &[&str]) -> Result<(), Failure> {
    found(
        "tests turned off only where no test can hide, doc tests kept",
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
        ("custom-build", "crates/x/build.rs", false, false),
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
        ("crates/x/build.rs".to_owned(), "fn main() {}\n".to_owned()),
    ];
    assert_eq!(
        super::untested_targets_gate(&metadata, &sources),
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
    ];
    assert_eq!(
        super::untested_targets_gate(&metadata, &sources),
        untested_refused(&[
            "crates/x/src/main.rs:4: declares a module, but its target turns its tests off \
             (`test = false`): only a root that is all its code may, so that no test hides in it",
            "crates/x/src/lib.rs:2: declares a module, but its target turns its tests off \
             (`test = false`): only a root that is all its code may, so that no test hides in it",
            "crates/x/src/bin/b.rs: the root of a target without tests, not among the Rust \
             files read",
        ]),
        "a test module declared, a library's modules, a root never read"
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
        super::untested_targets_gate(metadata, &sources),
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
    for kind in ["lib", "rlib", "proc-macro"] {
        let metadata = targets_metadata(&[(kind, "crates/x/src/lib.rs", true, false)]);
        assert_eq!(
            super::untested_targets_gate(&metadata, &[]),
            untested_refused(&[
                "crates/x/src/lib.rs: its doc tests are turned off (`doctest = false`): they \
                 would never run"
            ]),
            "{kind}"
        );
    }
    let cdylib = targets_metadata(&[("cdylib", "crates/x/src/lib.rs", true, false)]);
    assert_eq!(
        super::untested_targets_gate(&cdylib, &[]),
        Ok(()),
        "a C library alone has no doc tests"
    );
}

#[test]
fn refuses_metadata_it_cannot_read_whole() {
    let gate = |metadata: &str| super::untested_targets_gate(metadata, &[]);
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
    let metadata = SystemRunner { root: dir }
        .capture(&super::metadata_command())
        .expect("cargo metadata");
    let sources = [
        ("src/lib.rs".to_owned(), "//! A.\n".to_owned()),
        (
            "src/main.rs".to_owned(),
            "mod b;\nfn main() {}\n".to_owned(),
        ),
    ];
    assert_eq!(
        super::untested_targets_gate(&metadata, &sources),
        untested_refused(&[
            "src/lib.rs: its doc tests are turned off (`doctest = false`): they would never run",
            "src/main.rs:1: declares a module, but its target turns its tests off (`test = \
             false`): only a root that is all its code may, so that no test hides in it",
        ]),
        "both settings as cargo reports them"
    );
}

#[test]
fn tells_the_files_of_a_crate_by_its_directory() {
    let core = RustCrate {
        name: "engine-core",
        dir: "crates/engine-core",
        binary: false,
        test_data: &[],
        roles: &[],
    };
    for (path, held) in [
        ("crates/engine-core/src/lib.rs", Some("src/lib.rs")),
        ("crates/engine-core/", Some("")),
        ("crates/engine-core", None),
        ("crates/engine-core-x/a.rs", None),
        ("crates/engine-corea.rs", None),
        ("x/crates/engine-core/a.rs", None),
        ("crates/engine-cor/a.rs", None),
    ] {
        assert_eq!(core.holds(path), held, "{path}");
    }
}

#[test]
fn takes_for_shared_test_code_every_crate_only_tests_use() {
    let (real, _) = fixture_metadata();
    assert_eq!(
        super::shared_tests_gate(&real),
        Ok(()),
        "the crates real cargo metadata reaches only through tests"
    );
    assert_eq!(super::shared_tests_gate(METADATA), Ok(()), "the fake's");
    let context = "every crate only tests use is shared test code";
    let package = |name: &str, dependencies: &[(&str, &str)]| {
        let listed: Vec<String> = dependencies
            .iter()
            .map(|(dependency, kind)| {
                format!(
                    r#"{{"name":"{dependency}","kind":{kind},"path":"/r/crates/{dependency}"}}"#
                )
            })
            .collect();
        format!(
            r#"{{"name":"{name}","dependencies":[{}]}}"#,
            listed.join(",")
        )
    };
    let metadata = |packages: &[String]| format!(r#"{{"packages":[{}]}}"#, packages.join(","));
    assert_eq!(
        super::shared_tests_gate(&metadata(&[
            package("engine-objects", &[("engine-testkit", "\"dev\"")]),
            package("engine-core", &[("engine-alloc-counter", "null")]),
            package("engine-testkit", &[("engine-alloc-counter", "null")]),
            package("engine-cli", &[("engine-testkit", "null")]),
        ])),
        Ok(()),
        "shared test code that code depends on too widens more, never less"
    );
    assert_eq!(
        super::shared_tests_gate(&metadata(&[
            package(
                "engine-objects",
                &[("engine-testkit", "\"dev\""), ("engine-hash", "\"dev\"")]
            ),
            package("engine-testkit", &[("engine-alloc-counter", "null")]),
        ])),
        failed(format!(
            "{context}: engine-hash: only tests reach it, so a change to it can weaken the \
             tests of every crate using it: give it CrateRole::SharedTests"
        )),
        "a crate only tests reach, not taken for shared test code"
    );
    assert_eq!(
        super::shared_tests_gate(&metadata(&[
            package("engine-objects", &[("engine-testkit", "\"dev\"")]),
            package("engine-testkit", &[("engine-alloc-counter", "\"build\"")]),
            package("other", &[("engine-testkit", "\"dev\"")]),
        ])),
        Ok(()),
        "through a crate of shared test code, any kind"
    );
    assert_eq!(
        super::shared_tests_gate(&metadata(&[
            package("engine-objects", &[("engine-hash", "\"dev\"")]),
            package("engine-hash", &[("engine-patch", "null")]),
            package("engine-wasm", &[("engine-patch", "\"dev\"")]),
        ])),
        found(
            context,
            &[
                "engine-hash: only tests reach it, so a change to it can weaken the tests of \
                 every crate using it: give it CrateRole::SharedTests"
                    .to_owned(),
                "engine-patch: only tests reach it, so a change to it can weaken the tests of \
                 every crate using it: give it CrateRole::SharedTests"
                    .to_owned(),
            ]
        ),
        "reached through a crate only tests use, at any depth"
    );
    assert_eq!(
        super::shared_tests_gate(
            r#"{"packages":[{"name":"engine-objects","dependencies":[{"name":"engine-hash","kind":"dev","path":"/elsewhere/engine-hash"},{"name":"other","kind":"dev","path":"/r/crates/engine-patch"},{"name":"engine-testkit","kind":"dev","path":"/r/crates/engine-testkit"}]},{"name":"engine-testkit","dependencies":[{"name":"engine-alloc-counter","kind":null,"path":"/r/crates/engine-alloc-counter"}]}]}"#
        ),
        Ok(()),
        "a dependency is a crate of the workspace by its name and its directory both"
    );
    assert_eq!(
        super::shared_tests_gate(&metadata(&[
            package("engine-objects", &[("engine-hash", "\"dev\"")]),
            package("engine-core", &[("engine-patch", "null")]),
        ])),
        failed(format!(
            "{context}: engine-hash: only tests reach it, so a change to it can weaken the \
             tests of every crate using it: give it CrateRole::SharedTests"
        )),
        "code depending on one crate takes no other out of the set"
    );
}

#[test]
fn derives_each_list_of_crates_from_their_roles() {
    assert_eq!(
        super::crates_in(super::CrateRole::SharedTests),
        ["engine-alloc-counter", "engine-battery", "engine-testkit"],
        "the counting allocator, the fingerprint battery and the test kit"
    );
    for krate in super::RUST_CRATES {
        for role in krate.roles {
            assert!(
                super::crates_in(*role).contains(&krate.name),
                "{} holds {role:?}",
                krate.name
            );
        }
    }
}

#[test]
fn widens_each_kind_of_shared_input_as_far_as_it_reaches() {
    use super::Widening;
    for (path, expected) in [
        ("Cargo.toml", Some(Widening::Everything)),
        ("rust-toolchain.toml", Some(Widening::Everything)),
        ("Cargo.lock", Some(Widening::Lock)),
        (
            "crates/engine-core/Cargo.toml",
            Some(Widening::Manifest("engine-core")),
        ),
        (
            "bindings/wasm/Cargo.toml",
            Some(Widening::Manifest("engine-wasm")),
        ),
        (
            "crates/engine-testkit/Cargo.toml",
            Some(Widening::Manifest("engine-testkit")),
        ),
        (
            "crates/engine-testkit/src/lib.rs",
            Some(Widening::SharedTests("engine-testkit")),
        ),
        (
            "crates/engine-alloc-counter/src/lib.rs",
            Some(Widening::SharedTests("engine-alloc-counter")),
        ),
        ("tooling/x/Cargo.toml", Some(Widening::Everything)),
        ("doc/Cargo.lock", Some(Widening::Everything)),
        ("apps/a/rust-toolchain.toml", Some(Widening::Everything)),
        ("crates/xtask/tests/fixtures/workspace/Cargo.toml", None),
        (
            "crates/xtask/tests/fixtures/workspace/crates/engine-core/Cargo.toml",
            None,
        ),
        ("crates/engine-core/src/Cargo.lock", None),
        ("crates/engine-core/src/lib.rs", None),
        ("Cargo.toml.orig", None),
        ("crates/engine-testkits/a.rs", None),
        ("doc/crates/engine-testkit/a.rs", None),
    ] {
        assert_eq!(super::widening(path), expected, "{path}");
    }
}

/// A registry package of a lock file: name, version, checksum, dependencies.
type Registered<'text> = (&'text str, &'text str, &'text str, &'text [&'text str]);

/// A lock file as Cargo writes it: every crate of the workspace at 0.1.0, with the
/// dependencies `workspace` gives it (none for the others), then the `registry` packages.
fn lock_text(workspace: &[(&str, &[&str])], registry: &[Registered<'_>]) -> String {
    let dependencies = |names: &[&str]| {
        if names.is_empty() {
            String::new()
        } else {
            let mut list = "dependencies = [\n".to_owned();
            for name in names {
                list.push_str(" \"");
                list.push_str(name);
                list.push_str("\",\n");
            }
            list.push_str("]\n");
            list
        }
    };
    let mut text = "# This file is automatically @generated by Cargo.\n# It is not intended for manual editing.\nversion = 4\n".to_owned();
    for krate in super::RUST_CRATES {
        let names = workspace
            .iter()
            .find(|(name, _)| *name == krate.name)
            .map_or(&[][..], |(_, names)| *names);
        let _written = write!(
            text,
            "\n[[package]]\nname = \"{}\"\nversion = \"0.1.0\"\n{}",
            krate.name,
            dependencies(names)
        );
    }
    for (name, version, checksum, names) in registry {
        let _written = write!(
            text,
            "\n[[package]]\nname = \"{name}\"\nversion = \"{version}\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"{checksum}\"\n{}",
            dependencies(names)
        );
    }
    text
}

/// What the workspace of the lock tests depends on.
const LOCKED_WORKSPACE: [(&str, &[&str]); 4] = [
    ("engine-core", &["libm"]),
    ("engine-objects", &["engine-core"]),
    ("engine-testkit", &["engine-core", "proptest"]),
    ("xtask", &["serde_json"]),
];

/// The registry packages of the lock tests.
const LOCKED_REGISTRY: [Registered<'static>; 5] = [
    ("libm", "0.2.16", "c1", &[]),
    ("proptest", "1.0.0", "c2", &["rand"]),
    ("rand", "0.8.0", "c3", &[]),
    ("serde_json", "1.0.0", "c4", &["itoa"]),
    ("itoa", "1.0.0", "c5", &[]),
];

/// The crates [`super::lock_widened`] finds between the lock of the tests and the one with
/// `registry` instead.
fn widened_by(registry: &[Registered<'_>]) -> Result<Vec<&'static str>, String> {
    super::lock_widened(
        &lock_text(&LOCKED_WORKSPACE, &LOCKED_REGISTRY),
        &lock_text(&LOCKED_WORKSPACE, registry),
    )
    .map(|crates| crates.into_iter().collect())
}

#[test]
fn widens_to_the_crates_whose_build_reaches_a_changed_lock_entry() {
    let mut registry = LOCKED_REGISTRY.to_vec();
    assert_eq!(widened_by(&registry), Ok(Vec::new()), "nothing changed");
    registry[0] = ("libm", "0.2.16", "c9", &[]);
    assert_eq!(
        widened_by(&registry),
        Ok(vec!["engine-core", "engine-objects", "engine-testkit"]),
        "a checksum: every crate reaching it, however deep"
    );
    let mut registry2 = LOCKED_REGISTRY.to_vec();
    registry2[4] = ("itoa", "1.0.1", "c5", &[]);
    assert_eq!(
        widened_by(&registry2),
        Ok(vec!["xtask"]),
        "a version, reached through another package"
    );
    let mut registry3 = LOCKED_REGISTRY.to_vec();
    registry3[1] = ("proptest", "1.0.0", "c2", &[]);
    let _removed = registry3.remove(2);
    assert_eq!(
        widened_by(&registry3),
        Ok(vec!["engine-testkit"]),
        "a package removed: the one that reached it changed"
    );
    let mut registry4 = LOCKED_REGISTRY.to_vec();
    registry4.push(("unused", "1.0.0", "c6", &[]));
    assert_eq!(
        widened_by(&registry4),
        Ok(Vec::new()),
        "a package no crate reaches"
    );
    let workspace = [
        ("engine-core", &["libm"][..]),
        ("engine-objects", &["engine-core"]),
        ("engine-testkit", &["engine-core", "proptest"]),
        ("xtask", &["serde_json", "itoa"]),
    ];
    assert_eq!(
        super::lock_widened(
            &lock_text(&LOCKED_WORKSPACE, &LOCKED_REGISTRY),
            &lock_text(&workspace, &LOCKED_REGISTRY)
        )
        .map(|crates| crates.into_iter().collect::<Vec<_>>()),
        Ok(vec!["xtask"]),
        "a crate's own dependencies"
    );
}

#[test]
fn follows_a_lock_dependency_by_name_version_and_source() {
    let two = [
        ("serde_json", "1.0.0", "c4", &["itoa 1.0.0"][..]),
        ("itoa", "1.0.0", "c5", &[]),
        ("itoa", "2.0.0", "c7", &[]),
    ];
    let before = lock_text(&[("xtask", &["serde_json"])], &two);
    let mut changed = two;
    changed[2] = ("itoa", "2.0.0", "c8", &[]);
    assert_eq!(
        super::lock_widened(&before, &lock_text(&[("xtask", &["serde_json"])], &changed))
            .map(|crates| crates.into_iter().collect::<Vec<_>>()),
        Ok(Vec::new()),
        "the version named, not the other"
    );
    changed[1] = ("itoa", "1.0.0", "c9", &[]);
    assert_eq!(
        super::lock_widened(&before, &lock_text(&[("xtask", &["serde_json"])], &changed))
            .map(|crates| crates.into_iter().collect::<Vec<_>>()),
        Ok(vec!["xtask"]),
        "the version named"
    );
    let sourced = [
        (
            "serde_json",
            "1.0.0",
            "c4",
            &["itoa 1.0.0 (registry+https://github.com/rust-lang/crates.io-index)"][..],
        ),
        ("itoa", "1.0.0", "c5", &[]),
    ];
    let lock = lock_text(&[("xtask", &["serde_json"])], &sourced);
    assert_eq!(
        super::lock_widened(&lock, &lock).map(|crates| crates.len()),
        Ok(0),
        "a source named"
    );
    for dependencies in [
        &["itoa"][..],
        &["itoa 3.0.0"],
        &["itoa 1.0.0 (git+https://example.invalid)"],
        &["itoa 1.0.0 registry"],
        &["missing"],
    ] {
        let unresolved = lock_text(
            &[("xtask", &["serde_json"])],
            &[
                ("serde_json", "1.0.0", "c4", dependencies),
                ("itoa", "1.0.0", "c5", &[]),
                ("itoa", "2.0.0", "c7", &[]),
            ],
        );
        assert_eq!(
            super::lock_widened(&unresolved, &unresolved),
            Err(format!(
                "a dependency, `{}`, names no one entry",
                dependencies.join("")
            )),
            "{dependencies:?}: ambiguous, missing or malformed"
        );
    }
}

#[test]
fn reads_a_lock_file_only_as_cargo_writes_it() {
    let lock = lock_text(&LOCKED_WORKSPACE, &LOCKED_REGISTRY);
    assert!(super::lock_entries(&lock).is_ok(), "as written");
    assert_eq!(
        super::lock_entries(&lock).map(|entries| entries.len()),
        Ok(16),
        "every crate of the workspace and every package"
    );
    let first = "it does not start with `version = 4` after its comments";
    for (malformed, reason) in [
        (lock.replacen("version = 4", "version = 3", 1), first),
        (lock.replacen("version = 4\n", "", 1), first),
        (
            lock.replacen(
                "\n[[package]]\nname = \"libm\"",
                "\n[[package]]\nnamee = \"libm\"",
                1,
            ),
            "line 63 is not `name = \"…\"`",
        ),
        (
            lock.replacen("version = \"0.2.16\"", "version = 0.2.16", 1),
            "line 64 is not `version = \"…\"`",
        ),
        (
            lock.replacen("checksum = \"c1\"", "checksum = c1", 1),
            "line 66 is not `checksum = \"…\"`",
        ),
        (
            lock.replacen("checksum = \"c1\"", "checksum = \"c1\"\nyanked = true", 1),
            "line 67 is not the blank line ending an entry",
        ),
        (
            lock.replacen(" \"libm\",", "  \"libm\",", 1),
            "line 9 is not ` \"<dependency>\",`",
        ),
        (
            lock.replacen(" \"libm\",", " \"libm\"", 1),
            "line 9 is not ` \"<dependency>\",`",
        ),
        (
            lock.replacen(" \"libm\",\n]", " \"libm\",", 1),
            "line 10 is not ` \"<dependency>\",`",
        ),
        (
            format!(
                "{lock}\n[[package]]\nname = \"a\"\nversion = \"1\"\ndependencies = [\n \"b\",\n"
            ),
            "it ends in a list of dependencies",
        ),
        (
            lock.replacen(
                "\n\n[[package]]\nname = \"libm\"",
                "\n[[package]]\nname = \"libm\"",
                1,
            ),
            "line 61 is not the blank line ending an entry",
        ),
        (
            format!("{lock}\n[metadata]\n"),
            "line 98 is not `[[package]]`",
        ),
        (
            format!(
                "{lock}\n[[package]]\nname = \"libm\"\nversion = \"0.2.16\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"c1\"\n"
            ),
            "the entry of line 98 repeats an earlier one",
        ),
        (
            format!("{lock}\n[[package]]\nname = \"a\"\n"),
            "it ends where `version = \"…\"` is expected",
        ),
        (
            format!("{lock}\n[[package]]\n"),
            "it ends where `name = \"…\"` is expected",
        ),
        (
            lock.replacen(
                "source = \"registry+https://github.com/rust-lang/crates.io-index\"",
                "source = registry",
                1,
            ),
            "line 65 is not `source = \"…\"`",
        ),
    ] {
        assert_eq!(
            super::lock_entries(&malformed).err(),
            Some(reason.to_owned()),
            "{reason}"
        );
        assert_eq!(
            super::lock_widened(&lock, &malformed),
            Err(reason.to_owned()),
            "{reason}: after"
        );
        assert_eq!(
            super::lock_widened(&malformed, &lock),
            Err(format!("its earlier version: {reason}")),
            "{reason}: before"
        );
    }
}

#[test]
fn needs_one_entry_of_each_crate_of_the_workspace() {
    let lock = lock_text(&LOCKED_WORKSPACE, &LOCKED_REGISTRY);
    let without = lock.replacen(
        "\n[[package]]\nname = \"engine-cli\"\nversion = \"0.1.0\"\n",
        "",
        1,
    );
    assert!(super::lock_entries(&without).is_ok(), "still a lock file");
    assert_eq!(
        super::lock_widened(&lock, &without),
        Err("it does not hold exactly one entry of engine-cli".to_owned()),
        "a crate of the workspace missing"
    );
}

/// A passing fake whose base holds [`LOCKED_REGISTRY`]'s lock file and whose tree holds
/// `registry`'s, every crate of the workspace with one source file, and `changes` as the
/// changes since the base.
fn locked_fake(registry: &[Registered<'_>], changes: &str) -> Fake {
    let mut fake = passing();
    for (key, value) in [
        (format!("capture {}", CHANGES.0), changes.to_owned()),
        (
            format!("capture git show {}:Cargo.lock", MERGE_BASE.1.trim()),
            lock_text(&LOCKED_WORKSPACE, &LOCKED_REGISTRY),
        ),
        (
            "Cargo.lock".to_owned(),
            lock_text(&LOCKED_WORKSPACE, registry),
        ),
        (
            format!("capture {}", super::metadata_command().line()),
            closure_metadata(false),
        ),
    ] {
        drop(fake.files.insert(key, value));
    }
    for krate in super::RUST_CRATES {
        drop(fake.files.insert(
            format!("{}/src/lib.rs", krate.dir),
            "fn a() {}\n".to_owned(),
        ));
    }
    fake
}

/// The diff of [`locked_fake`]'s change, unless it is a file of a crate of `whole`, then
/// the source of each crate of `whole`, added whole, in the order of the workspace.
fn whole_crates(change: &str, whole: &[&str]) -> String {
    let mut diff = if super::RUST_CRATES
        .iter()
        .any(|krate| whole.contains(&krate.name) && change.contains(&format!(" b/{}/", krate.dir)))
    {
        String::new()
    } else {
        change.to_owned()
    };
    for krate in super::RUST_CRATES {
        if whole.contains(&krate.name) {
            diff.push_str(&super::new_file_diff(
                &format!("{}/src/lib.rs", krate.dir),
                "fn a() {}\n",
            ));
        }
    }
    diff
}

#[test]
fn mutates_the_crates_a_change_of_the_lock_file_reaches() {
    let mut registry = LOCKED_REGISTRY.to_vec();
    registry[4] = ("itoa", "1.0.1", "c5", &[]);
    let change = one_line_change("Cargo.lock");
    let mut fake = locked_fake(&registry, &change);
    let said = mutate_changes(&mut fake);
    assert_eq!(
        fake.files.get(CHANGES_DIFF).cloned(),
        Some(whole_crates(&change, &["xtask"])),
        "xtask alone, whole"
    );
    assert!(
        said.contains("  Cargo.lock changed: every mutant of xtask\n")
            && !said.contains("every Rust mutant"),
        "said: {said}"
    );
    let mut same = locked_fake(&LOCKED_REGISTRY, &change);
    let _quiet = mutate_changes(&mut same);
    assert_eq!(
        same.files.get(CHANGES_DIFF).cloned(),
        Some(change.clone()),
        "an entry-for-entry identical lock widens nothing"
    );
    let mut unreadable = locked_fake(&registry, &change);
    drop(unreadable.files.remove("Cargo.lock"));
    assert!(
        mutate_changes(&mut unreadable).contains(
            "  Cargo.lock changed and cannot be narrowed (cannot read it: missing): every Rust \
             mutant\n"
        ),
        "a lock file that cannot be read here widens to everything, saying why"
    );
    let mut malformed = locked_fake(&registry, &change);
    drop(
        malformed
            .files
            .insert("Cargo.lock".to_owned(), "nonsense\n".to_owned()),
    );
    assert!(
        mutate_changes(&mut malformed).contains(
            "  Cargo.lock changed and cannot be narrowed (it does not start with `version = 4` \
             after its comments): every Rust mutant\n"
        ),
        "a lock file not as Cargo writes it widens to everything, saying where"
    );
}

#[test]
fn mutates_the_crates_that_build_a_changed_manifest_or_the_shared_test_crate() {
    for (path, whole) in [
        (
            "crates/engine-core/Cargo.toml",
            &["engine-core", "engine-objects"][..],
        ),
        ("crates/engine-hash/Cargo.toml", &["engine-hash"]),
        (
            "crates/engine-testkit/src/runner.rs",
            &["engine-objects", "engine-testkit"],
        ),
    ] {
        let change = one_line_change(path);
        let mut fake = locked_fake(&LOCKED_REGISTRY, &change);
        let said = mutate_changes(&mut fake);
        assert_eq!(
            fake.files.get(CHANGES_DIFF).cloned(),
            Some(whole_crates(&change, whole)),
            "{path}"
        );
        for name in whole {
            assert!(
                said.contains(&format!("  {path} changed: every mutant of {name}\n")),
                "{path}: said of {name}: {said}"
            );
        }
    }
    let fixture = one_line_change("crates/xtask/tests/fixtures/workspace/Cargo.toml");
    let mut fake = locked_fake(&LOCKED_REGISTRY, &fixture);
    let said = mutate_changes(&mut fake);
    assert_eq!(
        fake.files.get(CHANGES_DIFF).cloned(),
        Some(whole_crates(&fixture, &["xtask"])),
        "a fixture manifest is xtask's test code"
    );
    assert!(
        said.contains("  the test code of xtask changed: every mutant of xtask\n"),
        "said: {said}"
    );
}

#[test]
fn keeps_the_crates_a_record_proves_that_a_lock_change_does_not_reach() {
    let mut registry = LOCKED_REGISTRY.to_vec();
    registry[4] = ("itoa", "1.0.1", "c5", &[]);
    let mut fake = proven_fake(&proof_record(PROVEN[2]), &one_line_change("Cargo.lock"));
    for (key, value) in [
        (
            format!("capture git show {}:Cargo.lock", PROVEN[0]),
            lock_text(&LOCKED_WORKSPACE, &LOCKED_REGISTRY),
        ),
        (
            "Cargo.lock".to_owned(),
            lock_text(&LOCKED_WORKSPACE, &registry),
        ),
    ] {
        drop(fake.files.insert(key, value));
    }
    let said = mutate_changes(&mut fake);
    assert!(
        said.contains(&proven_line("engine-core"))
            && said.contains(&proven_line("engine-hash"))
            && !said.contains("ignored the mutation proof record"),
        "the record still proves what the change does not reach: {said}"
    );
    let mut xtask = proven_fake(&proof_record(PROVEN[2]), &one_line_change("Cargo.lock"));
    let mut reaching = LOCKED_REGISTRY.to_vec();
    reaching[0] = ("libm", "0.2.16", "c9", &[]);
    for (key, value) in [
        (
            format!("capture git show {}:Cargo.lock", PROVEN[0]),
            lock_text(&LOCKED_WORKSPACE, &LOCKED_REGISTRY),
        ),
        (
            "Cargo.lock".to_owned(),
            lock_text(&LOCKED_WORKSPACE, &reaching),
        ),
    ] {
        drop(xtask.files.insert(key, value));
    }
    let reached = mutate_changes(&mut xtask);
    assert!(
        !reached.contains(&proven_line("engine-core"))
            && reached.contains(&proven_line("engine-hash")),
        "the engine core reaches the changed entry, the render hash does not: {reached}"
    );
}

#[test]
fn ignores_a_proof_record_of_another_compiler_or_linker() {
    let other = |edit: &dyn Fn(&mut super::ProofRecord)| {
        let mut record = proof_record(PROVEN[2]);
        edit(&mut record);
        proven_fake(&record, "")
    };
    ignored(
        &mut other(&|record| record.tools.compiler = "0000 LLVM 23.1.1".to_owned()),
        "it was made with the compiler 0000 LLVM 23.1.1, not \
         58cf8f9c35b355199727627d07ae34a4e2846d67 LLVM 23.1.1",
    );
    ignored(
        &mut other(&|record| record.tools.linker = "Apple clang version 17.0.0".to_owned()),
        "it was made with the linker Apple clang version 17.0.0, not Apple clang version 14.0.3 \
         (clang-1403.0.22.14.1)",
    );
    for (missing, key) in [
        (
            "commit-hash: 58cf8f9c35b355199727627d07ae34a4e2846d67\n",
            "commit-hash",
        ),
        ("LLVM version: 23.1.1\n", "LLVM version"),
    ] {
        let mut unbuilt = proven_fake(&proof_record(PROVEN[2]), "");
        drop(unbuilt.files.insert(
            format!("capture {}", HOST.0),
            HOST.1.replacen(missing, "", 1),
        ));
        ignored(&mut unbuilt, &format!("`rustc -vV` names no {key}"));
    }
    let mut no_linker = proven_fake(&proof_record(PROVEN[2]), "");
    no_linker.broken = Some(LINKER.0.to_owned());
    ignored(
        &mut no_linker,
        "cannot read the version of the linker driver: cannot run `cc --version`: not found",
    );
    ignored(
        &mut other(&|record| record.tools.git = "git version 2.30.0".to_owned()),
        "it was made with the git git version 2.30.0, not git version 2.39.2 (Apple Git-143)",
    );
    let mut no_git = proven_fake(&proof_record(PROVEN[2]), "");
    no_git.broken = Some("git --version".to_owned());
    ignored(
        &mut no_git,
        "cannot read the version of git: cannot run `git --version`: not found",
    );
    for silent in ["", "\nclang\n", "  \n"] {
        let mut unnamed = proven_fake(&proof_record(PROVEN[2]), "");
        drop(
            unnamed
                .files
                .insert(format!("capture {}", LINKER.0), silent.to_owned()),
        );
        ignored(&mut unnamed, "`cc --version` names no version");
    }
}
