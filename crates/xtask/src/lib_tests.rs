//! Tests of the check pipeline, with a fake runner.

extern crate alloc;

use alloc::collections::BTreeMap;
use core::num::NonZeroUsize;
use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::process::Command;

use super::{
    Action, BestEffort, CARGO_TOML, CHANGELOG, CHANGES_DIFF, COMMIT_HEADER_MAX, Cmd,
    CommandFailure, Coverage, Dir, EQUIVALENT_REVIEWED, Exit, Extent, Failed, Failure, MUTANTS_OUT,
    Measured, Mode, OnFailure, PACKAGES, PORTABILITY_TARGETS, Runner, RustCrate, Step,
    SystemRunner, UNSAFE_FILES, UNVIABLE_REPORT, UNVIABLE_REVIEWED, USAGE, Version, bless_step,
    check_steps, cli, commit_msg_gate, coverage_gate, equivalent_gate, license_gate,
    membership_gate, package_jsons, preflight_gate, release_gate, run_steps, run_steps_then,
    ts_coverage_gate, ts_mutation_gate, unsafe_gate, unviable_gate,
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
    "abc123\nrefs/heads/main\n",
);

/// A Dependabot configuration whose two ecosystems wait a week.
const DEPENDABOT: &str = "version: 2\nupdates:\n  - package-ecosystem: cargo\n    cooldown:\n      default-days: 7\n  - package-ecosystem: npm\n    cooldown:\n      default-days: 7\n";

/// A cargo-deny configuration holding libm to no feature.
const DENY: &str = "[bans]\n\n[[bans.features]]\ncrate = \"libm\"\nexact = true\nallow = []\n";

/// The Cargo metadata of a workspace whose crate `x` depends on another crate of the
/// workspace and on one third-party crate.
const METADATA: &str = r#"{"packages":[{"name":"x","dependencies":[{"name":"engine-core","kind":null,"path":"/r/crates/engine-core"},{"name":"serde_json","kind":null}]}]}"#;

/// The two license listings, written out: the production dependencies of what ships
/// (the engine package, the playground and the roadmap), then everything.
const LICENSE_LINES: [&str; 2] = [
    "pnpm licenses list --json --prod --filter ./bindings/wasm --filter ./apps/playground --filter ./apps/roadmap",
    "pnpm licenses list --json --recursive",
];

/// The two history listings, written out: every message, then the files touched by each
/// commit after the changelog boundary.
const HISTORY_LINES: [&str; 2] = [
    "git log --format=%x1e%H%x1f%B HEAD",
    "git log --format=%x1e%H --name-only 8182fda87a1c631295548ff52eeb4ea08335fa36..HEAD",
];

/// The threads of the fake's machine: six, so that the jobs of a full check (three) differ
/// from it.
const FAKE_PARALLELISM: NonZeroUsize = NonZeroUsize::new(6).expect("six is not zero");

/// The jobs of the fake's checks: half of [`FAKE_PARALLELISM`].
const FAKE_JOBS: NonZeroUsize = NonZeroUsize::new(3).expect("three is not zero");

/// The default check on the fake's machine.
const CHANGES_MODE: Mode = Mode::Changes { jobs: FAKE_JOBS };

/// What the fake's git answers about its tree and its changes.
const GIT_ANSWERS: [(&str, &str); 6] = [
    TREE_STATUS,
    TREE_DIFF,
    TREE_HEAD,
    MERGE_BASE,
    CHANGES,
    NEW_RUST_FILES,
];

/// How the fake finds the base of the changes, and what it answers: the upstream resolves.
const MERGE_BASE: (&str, &str) = ("git merge-base HEAD @{upstream}", "abc123\n");

/// How the fake reads the changes since [`MERGE_BASE`], and what it answers: one changed
/// line of a Rust file.
const CHANGES: (&str, &str) = (
    "git diff --no-color --no-ext-diff --no-textconv --no-renames --src-prefix=a/ --dst-prefix=b/ abc123",
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
}

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

    fn read(&mut self, path: &str) -> io::Result<String> {
        self.ran.push(format!("read {path}"));
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
            .filter(|path| {
                patterns
                    .iter()
                    .filter_map(|pattern| pattern.strip_prefix('*'))
                    .any(|suffix| path.ends_with(suffix))
            })
            .cloned()
            .collect())
    }
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

fn passing() -> Fake {
    let mut fake = Fake::default();
    coverage_reports(&mut fake);
    let report = "crates/a/src/b.rs:3:5: replace f -> T with Default::default()\n";
    let reviewed =
        "crates/a/src/b.rs: replace f -> T with Default::default() -- T has no Default\n";
    let first = fake
        .files
        .insert(UNVIABLE_REPORT.to_owned(), report.to_owned());
    let second = fake
        .files
        .insert(UNVIABLE_REVIEWED.to_owned(), reviewed.to_owned());
    assert!(first.is_none() && second.is_none(), "fresh fake");
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
        let previous_unsafe = fake.files.insert(path.to_owned(), lifting());
        assert!(previous_unsafe.is_none(), "fresh fake");
    }
    let [messages, files] = HISTORY_LINES;
    let logged = fake
        .files
        .insert(format!("capture {messages}"), HISTORY.to_owned());
    let listed = fake
        .files
        .insert(format!("capture {files}"), HISTORY_FILES.to_owned());
    assert!(logged.is_none() && listed.is_none(), "fresh fake");
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
        ("COMMIT_EDITMSG".to_owned(), "feat(core): add the plan\n"),
        (super::CARGO_CONFIG_FILE.to_owned(), super::CARGO_CONFIG),
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

/// A workflow that pins everything, as the workflow gate requires.
const PINNED_WORKFLOW: &str = "on:\n  push:\n  pull_request:\n  workflow_dispatch:\n\npermissions:\n  contents: read\n\nconcurrency:\n  group: ${{ (github.event_name != 'push' || contains(github.event.head_commit.message, '[ci]')) && format('quality-{0}', github.ref) || format('skipped-{0}', github.run_id) }}\n  cancel-in-progress: true\n\njobs:\n  check:\n    runs-on: ubuntu-24.04\n    steps:\n      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1\n        with:\n          fetch-depth: 0\n          persist-credentials: false\n      - name: Install Rust toolchains\n        run: rustup toolchain install nightly-2026-11-01 --profile minimal\n      - uses: taiki-e/install-action@cfdb7ed4ccc7c7dce37759eae89dc5674b469b39\n        with:\n          tool: cargo-llvm-cov@0.9.1,cargo-mutants@27.1.0,cargo-deny@0.20.2\n      - uses: actions/setup-node@949feb2413d6458794dcd2491c4babbbce0c15c1 # v7.1.0\n        with:\n          node-version-file: .node-version\n      - uses: Swatinem/rust-cache@d1527c8f67ef9af357290475d4af4b0fab8f12e0 # v2.9.3\n        with:\n          cache-on-failure: true\n      - name: Every quality check\n        run: cargo xtask check --full\n      - uses: actions/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9 # v7.0.2\n        if: failure()\n        with:\n          path: |\n            mutants.out\n            target/coverage\n            bindings/wasm/dist/engine.wasm\n            apps/*/coverage\n            apps/*/reports\n            bindings/*/coverage\n            bindings/*/reports\n            tooling/*/coverage\n            tooling/*/reports\n";

/// A history whose two commits follow the convention, the change of behavior with its
/// changelog entry, as `git log` writes it for the history gate.
const HISTORY: &str =
    "\u{1e}bbbbbbbbbb\u{1f}feat(core): add the plan\n\nWhy.\n\u{1e}aaaaaaaaaa\u{1f}docs: start\n";

/// The files the commits of [`HISTORY`] touched.
const HISTORY_FILES: &str =
    "\u{1e}bbbbbbbbbb\n\nCHANGELOG.md\ncrates/a.rs\n\u{1e}aaaaaaaaaa\n\nREADME.md\n";

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
         `engine-testkit` `engine-cli` `xtask`",
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
            Action::EquivalentGate => "equivalent gate".to_owned(),
            Action::VersionGate => "version gate".to_owned(),
            Action::ReadmeGate => "readme gate".to_owned(),
            Action::VocabularyGate => "vocabulary gate".to_owned(),
            Action::FreshDirs(paths) => format!("fresh {}", paths.join(" ")),
            Action::TypeScriptReportsGate => "typescript reports gate".to_owned(),
            Action::StrykerConfigGate => "stryker config gate".to_owned(),
            Action::TypeScriptScopeGate => "typescript scope gate".to_owned(),
            Action::TypeScriptMutationGate { package, extent } => {
                format!("typescript mutation gate of {} ({extent:?})", package.dir)
            }
            Action::BypassGate => "bypass gate".to_owned(),
            Action::MembershipGate => "membership gate".to_owned(),
            Action::LicenseGate => "license gate".to_owned(),
            Action::UnsafeGate => "unsafe gate".to_owned(),
            Action::CargoConfigGate => "cargo config gate".to_owned(),
            Action::DependencyGate => "dependency gate".to_owned(),
            Action::SupplyChainGate => "supply chain gate".to_owned(),
            Action::Preflight => "preflight".to_owned(),
            Action::HistoryGate => "history gate".to_owned(),
            Action::WorkflowGate => "workflow gate".to_owned(),
            Action::TreeRecorded => "tree recorded".to_owned(),
            Action::TreeUnchanged => "tree unchanged".to_owned(),
        })
        .collect()
}

#[test]
fn lists_every_check_in_order() {
    let steps = check_steps(CHANGES_MODE);
    assert_eq!(
        command_lines(&steps),
        vec![
            "preflight",
            "tree recorded",
            "Root: cargo fmt --all --check",
            "Root: cargo clippy --workspace --all-targets --locked",
            "Root: cargo doc --workspace --no-deps --document-private-items --locked",
            "Root: cargo test --workspace --locked -- --include-ignored",
            "Root: cargo build --package engine-core --package engine-objects --package engine-patch --package engine-hash --locked --target aarch64-apple-darwin",
            "Root: cargo build --package engine-core --package engine-objects --package engine-patch --package engine-hash --locked --target x86_64-unknown-linux-gnu",
            "Root: cargo build --package engine-core --package engine-objects --package engine-patch --package engine-hash --locked --target wasm32-unknown-unknown",
            "Root: cargo build --package engine-core --package engine-objects --package engine-patch --package engine-hash --locked --target thumbv7em-none-eabihf",
            "fresh target/coverage",
            "coverage locality",
            "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/engine-core/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/engine-core.json -- --include-ignored",
            "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/engine-objects/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/engine-objects.json -- --include-ignored",
            "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/engine-patch/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/engine-patch.json -- --include-ignored",
            "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/engine-hash/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/engine-hash.json -- --include-ignored",
            "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/engine-alloc-counter/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/engine-alloc-counter.json -- --include-ignored",
            "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/engine-testkit/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/engine-testkit.json -- --include-ignored",
            "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/engine-cli/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/engine-cli.json -- --include-ignored",
            "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path bindings/wasm/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/engine-wasm.json -- --include-ignored",
            "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/xtask/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/xtask.json -- --include-ignored",
            "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/engine-cli/Cargo.toml --locked --branch --json --summary-only --output-path target/coverage/engine-cli-main.json -- --include-ignored",
            "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/xtask/Cargo.toml --locked --branch --json --summary-only --output-path target/coverage/xtask-main.json -- --include-ignored",
            "fresh mutants.out",
            "rust mutants of the changes (3 jobs)",
            "unviable gate (Partial)",
            "equivalent gate",
            "bypass gate",
            "unsafe gate",
            "cargo config gate",
            &format!(
                "Root: cargo +nightly-2026-11-01 {} test --package engine-alloc-counter --package engine-wasm --locked -- --include-ignored",
                "miri"
            ),
            "Root: cargo run --package engine-cli --locked --quiet -- check conformance",
            "fresh target/conformance",
            "Root: cargo run --package engine-cli --locked --quiet -- export conformance target/conformance",
            "Root: cargo deny --locked check",
            "dependency gate",
            "supply chain gate",
            "membership gate",
            "readme gate",
            "vocabulary gate",
            "version gate",
            "history gate",
            "workflow gate",
            "Root: pnpm install --frozen-lockfile",
            "Root: pnpm audit",
            "license gate",
            "stryker config gate",
            "fresh tooling/config/coverage tooling/config/reports/mutation tooling/oracle/coverage tooling/oracle/reports/mutation bindings/wasm/coverage bindings/wasm/reports/mutation apps/playground/coverage apps/playground/reports/mutation apps/roadmap/coverage apps/roadmap/reports/mutation",
            "fresh bindings/wasm/dist",
            "Package(\"tooling/config\"): pnpm run check",
            "Package(\"tooling/config\"): pnpm exec stryker run stryker.config.json --incremental --incrementalFile reports/stryker-incremental.json",
            "Package(\"tooling/oracle\"): pnpm run check",
            "Package(\"tooling/oracle\"): pnpm exec stryker run stryker.config.json --incremental --incrementalFile reports/stryker-incremental.json",
            "Package(\"bindings/wasm\"): pnpm run check",
            "Package(\"bindings/wasm\"): pnpm exec stryker run stryker.config.json --incremental --incrementalFile reports/stryker-incremental.json",
            "Package(\"apps/playground\"): pnpm run check",
            "Package(\"apps/playground\"): pnpm exec stryker run stryker.config.json --incremental --incrementalFile reports/stryker-incremental.json",
            "Package(\"apps/roadmap\"): pnpm run check",
            "Package(\"apps/roadmap\"): pnpm exec stryker run stryker.config.json --incremental --incrementalFile reports/stryker-incremental.json",
            "typescript reports gate",
            "typescript scope gate",
            "tree unchanged",
        ],
        "the default check pipeline"
    );
    assert_eq!(PORTABILITY_TARGETS.len(), 4, "four portability targets");
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
            "Rust formatting",
            "Rust lints",
            "Rust documentation",
            "Rust tests",
            "Engine portability (aarch64-apple-darwin)",
            "Engine portability (x86_64-unknown-linux-gnu)",
            "Engine portability (wasm32-unknown-unknown)",
            "Engine portability (thumbv7em-none-eabihf)",
            "Fresh Rust coverage reports",
            "Rust coverage counts a crate's own unit tests only",
            "Rust coverage of engine-core: 100% by its own unit tests",
            "Rust coverage of engine-objects: 100% by its own unit tests",
            "Rust coverage of engine-patch: 100% by its own unit tests",
            "Rust coverage of engine-hash: 100% by its own unit tests",
            "Rust coverage of engine-alloc-counter: 100% by its own unit tests",
            "Rust coverage of engine-testkit: 100% by its own unit tests",
            "Rust coverage of engine-cli: 100% by its own unit tests",
            "Rust coverage of engine-wasm: 100% by its own unit tests",
            "Rust coverage of xtask: 100% by its own unit tests",
            "Rust coverage of the engine-cli binary: 100% by its tests",
            "Rust coverage of the xtask binary: 100% by its tests",
            "Fresh Rust mutation report",
            "Rust mutation testing of the changes",
            "Rust unviable mutants reviewed",
            "TypeScript equivalent mutants reviewed",
            "Every bypass of coverage or mutation testing reviewed",
            "Unsafe code confined to the audited files",
            "Cargo configuration: no build flag, a locked alias",
            "Unsafe code under Miri",
            "Conformance (native)",
            "Fresh conformance export directory",
            "Conformance export (native WAV files)",
            "Rust supply chain",
            "Every direct dependency reviewed",
            "Supply-chain settings: a week before a release, libm without features",
            "Packages and crates known to the checks",
            "Every part described in a README",
            "The words of the glossary, not their synonyms",
            "One version everywhere, changelog well formed",
            "Commit messages, and a changelog entry for each change of behavior",
            "CI workflow pinned",
            "JavaScript dependencies",
            "JavaScript vulnerabilities",
            "JavaScript licenses",
            "Stryker configurations hide no mutant",
            "Fresh TypeScript report directories",
            "Fresh TypeScript build outputs",
            "Shared TypeScript policy (each rule proven to reject what it forbids)",
            "Mutation testing of tooling/config (what changed since the last run)",
            "Conformance oracle (independent, TypeScript)",
            "Mutation testing of tooling/oracle (what changed since the last run)",
            "Engine WASM checks",
            "Mutation testing of bindings/wasm (what changed since the last run)",
            "Playground checks",
            "Mutation testing of apps/playground (what changed since the last run)",
            "Roadmap checks",
            "Mutation testing of apps/roadmap (what changed since the last run)",
            "TypeScript coverage and mutation reports",
            "Every TypeScript source measured and mutated, or reviewed",
            "Working tree unchanged by the checks",
        ],
        "step names"
    );
}

#[test]
fn the_full_check_differs_only_by_its_mutation_testing() {
    let jobs = NonZeroUsize::new(3).expect("three is not zero");
    let changes = check_steps(CHANGES_MODE);
    let full = check_steps(Mode::Full { jobs });
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
            "Root: cargo mutants --cap-lints=true --gitignore=true --cargo-arg=--locked --workspace --jobs 3 -- -- --include-ignored".to_owned(),
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
            "capture git merge-base HEAD @{upstream}",
            "capture git diff --no-color --no-ext-diff --no-textconv --no-renames --src-prefix=a/ --dst-prefix=b/ abc123",
            "capture git ls-files -z --others --exclude-standard -- *.rs",
            "write target/xtask/changes.diff",
            "cargo mutants --cap-lints=true --gitignore=true --cargo-arg=--locked --workspace --jobs 3 --in-diff target/xtask/changes.diff -- -- --include-ignored",
            "read mutants.out/unviable.txt",
            "read mutants-unviable.txt",
        ],
        "the diff since the merge base is written, then mutated"
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
        "▶ [1/2] Rust mutation testing of the changes\n  the changes since abc123 (merge base with @{upstream})\n▶ [2/2] Rust unviable mutants reviewed\n✔ all 2 checks passed\n",
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
                "abc123\n".to_owned(),
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
            Some(format!(
                "{}{}",
                one_line_change(test_code),
                super::new_file_diff("crates/engine-core/src/lib.rs", "fn a() {}\n")
            )),
            "{test_code}: every source of its crate added whole, its test code and other crates apart"
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
fn mutates_everything_when_what_every_test_uses_changed() {
    for path in [
        "Cargo.toml",
        "Cargo.lock",
        "crates/engine-core/Cargo.toml",
        "rust-toolchain.toml",
        "crates/engine-testkit/src/lib.rs",
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
                &"cargo mutants --cap-lints=true --gitignore=true --cargo-arg=--locked --workspace --jobs 3 -- -- --include-ignored".to_owned()
            ) && !fake.ran.iter().any(|line| line.starts_with("write")),
            "{path}: every mutant of the workspace, no diff: {:?}",
            fake.ran
        );
        assert!(
            String::from_utf8(out)
                .expect("UTF-8")
                .contains(&format!("  {path} changed: every Rust mutant\n")),
            "{path}: said"
        );
    }
    for path in [
        "Cargo.toml.orig",
        "crates/engine-core/Cargo.toml.orig",
        "crates/engine-testkits/a.rs",
        "doc/Cargo.lock.md",
        "doc/crates/engine-testkit/a.rs",
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
            fake.ran.iter().any(|line| line
                .ends_with("--in-diff target/xtask/changes.diff -- -- --include-ignored")),
            "{path}: only the changes"
        );
    }
    let mut broken = passing();
    let _changes = broken.files.insert(
        format!("capture {}", CHANGES.0),
        one_line_change("Cargo.lock"),
    );
    broken.fail =
        Some("cargo mutants --cap-lints=true --gitignore=true --cargo-arg=--locked --workspace --jobs 3 -- -- --include-ignored".to_owned());
    assert_eq!(
        run_steps(&rust_mutation_of_changes(), &mut broken, &mut Vec::new()),
        failed(
            "Rust mutation testing of the changes: `cargo mutants --cap-lints=true --gitignore=true --cargo-arg=--locked --workspace --jobs 3 -- -- --include-ignored` exited with code 3"
        ),
        "a surviving mutant of the workspace fails"
    );
    assert!(
        super::RUST_CRATES
            .iter()
            .any(|krate| krate.name == "engine-testkit" && krate.dir == "crates/engine-testkit"),
        "the shared test crate is where the escalation looks"
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
        !fake
            .ran
            .iter()
            .any(|line| line.starts_with("cargo mutants") || line.starts_with("write")),
        "nothing written, nothing mutated: {:?}",
        fake.ran
    );
    assert!(
        String::from_utf8(out).expect("UTF-8").contains(
            "  no change since abc123 (merge base with @{upstream}): no mutant to test\n"
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
            "cargo mutants --cap-lints=true --gitignore=true --cargo-arg=--locked --workspace --jobs 3 --in-diff target/xtask/changes.diff -- -- --include-ignored",
            "cannot run `cargo mutants --cap-lints=true --gitignore=true --cargo-arg=--locked --workspace --jobs 3 --in-diff target/xtask/changes.diff -- -- --include-ignored`: not found".to_owned(),
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
            "Root: cargo mutants --cap-lints=true --gitignore=true --cargo-arg=--locked --package engine-core --jobs 3 -- -- --include-ignored",
            "unviable gate (Partial)",
        ],
        "a crate"
    );
    assert_eq!(
        lines("crates/xtask/src/lib.rs"),
        vec![
            "fresh mutants.out",
            "Root: cargo mutants --cap-lints=true --gitignore=true --cargo-arg=--locked --package xtask --file crates/xtask/src/lib.rs --jobs 3 -- -- --include-ignored",
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
            "cargo mutants --cap-lints=true --gitignore=true --cargo-arg=--locked --package engine-core --jobs 3 -- -- --include-ignored",
            "read mutants.out/unviable.txt",
            "read mutants-unviable.txt",
        ],
        "in half the threads of the machine"
    );
    assert!(
        String::from_utf8(out)
            .expect("UTF-8")
            .ends_with("✔ all 3 checks passed\n"),
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
    let (tools, commands, emptied, listed, captured, written) = (10, 49, 14, 11, 14, 1);
    let read = 2 + 12 + 2 + 7 + 19 + 2 + 1 + 7 + 3 + 10 + 10 + 20 + 8 + 2 + 11 + 16 + 16;
    assert_eq!(
        fake.ran.len(),
        tools + commands + emptied + listed + captured + written + read,
        "the preflight asks ten tools; 49 commands run (formatting, lints, documentation, \
         tests, four portability builds, twelve coverage measurements each after emptying \
         the coverage build, the mutants of the changes, Miri, the \
         conformance check and export, deny, the JavaScript install, audit, and five package \
         checks each followed by its Stryker run); fourteen directories are emptied; git lists \
         eleven kinds of files; the licenses and the history are captured twice each, the state \
         of the tree (before and after) four times, the Cargo metadata, the merge base, the changes and the \
         new Rust files once; the diff is written once; and the gates read .node-version and the toolchain file for the preflight, twelve coverage reports, \
         the unviable report and reviews, six TypeScript sources and their reviews, the eight \
         sources, the ten Cargo manifests and the register of the bypass gate, the two unsafe files, the Cargo \
         configuration, the six package manifests and the register of the dependency gate, the three \
         supply-chain configurations, ten crate manifests, ten READMEs, the changelog, ten READMEs, two Rust and six TypeScript sources and the vocabulary of the vocabulary gate, eight version files, the workflow and \
         .node-version, five Stryker configurations and the six package manifests, the six \
         TypeScript sources and ten reports of the reports gate, and the five Stryker \
         configurations, five coverage summaries, five mutation reports and the register of \
         the scope gate"
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
        text.starts_with("▶ [1/62] Tools and platform\n"),
        "numbered progress: {text}"
    );
    assert!(
        text.contains("▶ [58/62] Roadmap checks\n"),
        "last step announced: {text}"
    );
    assert!(
        text.ends_with("✔ all 62 checks passed\n"),
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
        fake.ran.len(),
        10 + 2 + 3 + 4,
        "the preflight asks ten tools and reads .node-version and the toolchain file, the tree is recorded by three \
         captures; nothing runs after the failure"
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
        "▶ [1/5] ONE\n▶ [2/5] TWO\n▶ [3/5] THREE\n▶ [4/5] FOUR\n▶ [5/5] FIVE\n",
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
            .ends_with("▶ [5/5] FIVE\n✔ all 5 checks passed\n"),
        "passed"
    );
}

#[test]
fn stops_at_the_first_failure_unless_asked_to_run_the_rest() {
    let mut fake = failing_twice();
    assert_eq!(
        run_steps_then(&five_steps(), &mut fake, &mut Vec::new(), OnFailure::Stop),
        failed("TWO: `x two` exited with code 3"),
        "the first failure alone"
    );
    assert_eq!(fake.ran, ["x one", "x two"], "nothing after it");
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
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut runner = SystemRunner { root };
    let ok = Cmd {
        program: "cargo",
        args: vec!["--version".to_owned()],
        dir: Dir::Root,
    };
    assert_eq!(runner.run(&ok), Ok(()), "a successful command");
    let bad = Cmd {
        program: "cargo",
        args: vec!["no-such-command".to_owned()],
        dir: Dir::Root,
    };
    assert_eq!(
        runner.run(&bad).map_err(|failure| failure.failed),
        Err(Failed::Ended {
            exit: Exit::Code(101_i32),
            reason: None
        }),
        "a failing command and its code, its error output left on the terminal"
    );
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
    #[cfg(unix)]
    {
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
        "build(v2): bump the format",
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
    let cases: [(&str, String); 14] = [
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

#[test]
fn bless_renders_and_rewrites_the_references() {
    let step = bless_step();
    assert_eq!(step.name, "Bless conformance references", "name");
    assert_eq!(
        command_lines(&[step]),
        vec!["Root: cargo run --package engine-cli --locked --quiet -- bless conformance"],
        "runs engine-cli bless"
    );
    let mut fake = passing();
    let mut out = Vec::new();
    assert_eq!(
        run_cli(&["bless".to_owned()], &mut fake, &mut out, &mut Vec::new()),
        0,
        "bless succeeds"
    );
    assert_eq!(
        fake.ran,
        vec!["cargo run --package engine-cli --locked --quiet -- bless conformance"],
        "only bless runs"
    );
    assert_eq!(
        String::from_utf8(out).expect("utf-8"),
        "▶ [1/1] Bless conformance references\n✔ all 1 checks passed\n",
        "progress"
    );
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
            format!("[workspace.lints.rust.x]\n{check_cfg} = [\"cfg(coverage)\"]\n"),
            check_cfg,
        ),
    ]);
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
            "TypeScript coverage and mutation reports: cannot read apps/playground/coverage/coverage-summary.json: missing"
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
            "TypeScript coverage and mutation reports: apps/roadmap: mutants not killed: \
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
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut runner = SystemRunner { root };
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
        .map(|path| ((*path).to_owned(), lifting()))
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
fn reports_rust_files_it_cannot_list() {
    let mut fake = passing();
    fake.broken = Some("list *.rs".to_owned());
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut fake, &mut Vec::new()),
        failed(
            "Unsafe code confined to the audited files: cannot list the Rust files: `git ls-files -- *.rs` exited with code 128: fatal: not a git repository"
        ),
        "named"
    );
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
    let real = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(super::RUST_TOOLCHAIN_FILE),
    )
    .expect("the toolchain file");
    assert!(
        super::toolchain_channel(&real).is_some(),
        "the real file is read"
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
            " 1. Tools and platform\n 2. Working tree recorded\n 3. Rust formatting\n"
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
        super::history_gate(HISTORY, HISTORY_FILES),
        Ok(()),
        "a clean history"
    );
    let messages = "\u{1e}cccccccccc\u{1f}fix: repair\n\u{1e}bbbbbbbbbb\u{1f}Update stuff\n\u{1e}aaaaaaaaaa\u{1f}feat: before the rule\n";
    let files = "\u{1e}cccccccccc\n\nsrc/a.rs\n\u{1e}bbbbbbbbbb\n\nsrc/b.rs\n";
    assert_eq!(
        super::history_gate(messages, files),
        failed(
            "the history breaks the commit rules: 2 problems, listed above\n\
             commit ccccccc changes behavior but not CHANGELOG.md\n\
             commit bbbbbbb: invalid commit message: missing \"<type>: \" prefix"
        ),
        "a change without its entry, a malformed message; the commit before the rule is not held to it"
    );
    assert_eq!(super::history_gate("", ""), Ok(()), "no commit");
    assert_eq!(
        super::history_gate("\u{1e}eeeeeeeeee\u{1f}# fix: hidden\nfix: shown\n", ""),
        found(
            "the history breaks the commit rules",
            &["commit eeeeeee: invalid commit message: unknown type \"# fix\"".to_owned()]
        ),
        "a recorded message is read as it is: a first line starting with # is its header"
    );
    assert_eq!(
        super::history_gate("\u{1e}dddddddddd", "\u{1e}dddddddddd\n"),
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
            &pushed(&["1111111111111111111111111111111111111111"])
        ),
        Ok(()),
        "the checked-out commit"
    );
    assert_eq!(
        super::pre_push_gate(
            head,
            "",
            &pushed(&["0000000000000000000000000000000000000000"])
        ),
        Ok(()),
        "a deleted branch"
    );
    assert_eq!(
        super::pre_push_gate(head, "\n", &[]),
        Ok(()),
        "nothing pushed, nothing changed"
    );
    assert_eq!(
        super::pre_push_gate(
            head,
            " M src/a.rs\n?? notes.txt\n",
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
            &"cargo mutants --cap-lints=true --gitignore=true --cargo-arg=--locked --workspace --jobs 3 -- -- --include-ignored".to_owned()
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
    let second = fake.files.insert(
        "capture git status --porcelain --untracked-files=all --ignore-submodules=none".to_owned(),
        String::new(),
    );
    assert!(first.is_none() && second.is_none(), "fresh fake");
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
            .ends_with("✔ all 62 checks passed\n"),
        "then every check"
    );
    assert!(
        fake.ran.contains(&format!("capture {}", MERGE_BASE.0))
            && fake.ran.contains(
                &"cargo mutants --cap-lints=true --gitignore=true --cargo-arg=--locked --workspace --jobs 3 --in-diff target/xtask/changes.diff -- -- --include-ignored"
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
        third.is_none() && fourth.is_none() && fifth.is_none(),
        "fresh fake"
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
            ".github/workflows/quality.yml can change CI without a commit: 8 problems, listed above\n\
             actions/checkout@v7 is not pinned to a full commit hash\n\
             taiki-e/install-action is not pinned to a full commit hash\n\
             actions/setup-node@949feb2413d6458794dcd2491c4babbbce0c15cz # v7.1.0 is not pinned to a full commit hash\n\
             .github/workflows/quality.yml lacks `tool: cargo-llvm-cov@0.9.1,cargo-mutants@27.1.0,cargo-deny@0.20.2`\n\
             .github/workflows/quality.yml lacks `node-version-file: .node-version`\n\
             .github/workflows/quality.yml lacks `fetch-depth: 0`\n\
             .github/workflows/quality.yml lacks `run: cargo xtask check --full`\n\
             .node-version must pin one Node.js release, 22.18 or later (X.Y.Z): found \"22\""
        ),
        "every kind of loose pin"
    );
    let unkept = PINNED_WORKFLOW
        .replace("runs-on: ubuntu-24.04", "runs-on: ubuntu-latest")
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
fn the_real_workflow_pins_everything() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let read = |path: &str| std::fs::read_to_string(root.join(path)).expect("readable");
    assert_eq!(
        super::workflow_gate(
            &[super::WORKFLOW.to_owned()],
            &read(super::WORKFLOW),
            &read(super::NODE_VERSION_FILE)
        ),
        Ok(()),
        "the workflow of the repository"
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
    for (command, answer) in [
        ("git rev-parse HEAD".to_owned(), format!("{head}\n")),
        (
            "git status --porcelain --untracked-files=all --ignore-submodules=none".to_owned(),
            String::new(),
        ),
    ]
    .into_iter()
    .chain(objects.iter().map(|(object, commit)| {
        (
            format!("git rev-parse --verify {object}^{{commit}}"),
            format!("{commit}\n"),
        )
    })) {
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
    let _recorded = commit("fix: recorded\n\nWhy.", Some(CHANGELOG));
    let unrecorded = commit("fix: unrecorded", Some("c.rs"));
    let malformed = commit("Update stuff", None);
    let mut runner = SystemRunner { root: repo.clone() };
    let [messages, files] =
        super::history_commands(&since).map(|cmd| runner.capture(&cmd).expect("git log"));
    std::fs::remove_dir_all(&repo).expect("cleanup");
    let short = |hash: &str| hash.chars().take(7).collect::<String>();
    assert_eq!(
        super::history_gate(&messages, &files),
        failed(format!(
            "the history breaks the commit rules: 2 problems, listed above\n\
             commit {}: invalid commit message: missing \"<type>: \" prefix\n\
             commit {} changes behavior but not CHANGELOG.md",
            short(&malformed),
            short(&unrecorded)
        )),
        "every message checked; the changelog rule from the commit after the boundary on"
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
            &"cargo mutants --cap-lints=true --gitignore=true --cargo-arg=--locked --workspace --jobs 3 -- -- --include-ignored".to_owned()
        ),
        "every mutant, in half the threads of the machine"
    );
    assert_eq!(
        String::from_utf8(err).expect("UTF-8"),
        "error: 1 of 62 checks failed, listed above\n",
        "one line on stderr, counting the failed steps"
    );
    assert!(
        String::from_utf8(out).expect("UTF-8").ends_with(&format!(
            "▶ [62/62] Working tree unchanged by the checks\n\
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

#[test]
fn refuses_usage_errors_in_one_line() {
    for (args, reason) in [
        (vec![], "no command given"),
        (vec!["nope"], "unknown command \"nope\""),
        (vec!["nope", "x"], "unknown command \"nope\""),
        (vec!["check", "x"], "`check` takes no argument but `--full`"),
        (
            vec!["check", "--full", "x"],
            "`check` takes no argument but `--full`",
        ),
        (vec!["list", "x"], "`list` takes no argument but `--full`"),
        (
            vec!["list", "--full", "--full"],
            "`list` takes no argument but `--full`",
        ),
        (vec!["bless", "x"], "`bless` takes no argument"),
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
            .position(|step| step.action == Action::TypeScriptReportsGate);
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
        super::history_commands(super::CHANGELOG_SINCE).map(|cmd| cmd.line()),
        HISTORY_LINES.map(str::to_owned),
        "every message, then the files after the boundary"
    );
    assert!(
        super::license_commands()
            .iter()
            .chain(&super::history_commands(super::CHANGELOG_SINCE))
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
        "def456\nrefs/heads/main\n",
        "abc123\nrefs/heads/other\n",
        "abc123\nHEAD\n",
    ] {
        let moved = super::TreeState {
            head: head.to_owned(),
            ..dirty.clone()
        };
        assert_eq!(
            super::tree_gate(Some(&dirty), &moved),
            failed(format!(
                "the checks changed the working tree: what is checked out changed, from \
                 `abc123 refs/heads/main` to `{}`: the checks read another commit than the one \
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
        "def456\nrefs/heads/main\n".to_owned(),
    ));
    assert!(
        run_steps(&check_steps(CHANGES_MODE), &mut committed, &mut Vec::new()).is_err_and(
            |failure| failure
                .summary
                .contains("what is checked out changed, from `abc123 refs/heads/main` to `def456 refs/heads/main`")
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
        .map(|path| ((*path).to_owned(), lifting()))
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
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    assert_eq!(
        super::cargo_config_gate(
            &std::fs::read_to_string(root.join(super::CARGO_CONFIG_FILE)).expect("readable")
        ),
        Ok(()),
        "the configuration of the repository"
    );
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

#[test]
fn reviews_the_real_dependencies() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut runner = SystemRunner { root: root.clone() };
    let metadata = runner
        .capture(&super::metadata_command())
        .expect("cargo metadata");
    let read = |path: &str| std::fs::read_to_string(root.join(path)).expect("readable");
    let packages: Vec<(String, String)> = package_jsons()
        .into_iter()
        .map(|path| {
            let text = read(&path);
            (path, text)
        })
        .collect();
    assert_eq!(
        super::dependency_gate(&metadata, &packages, &read(super::DEPENDENCIES_REVIEWED)),
        Ok(()),
        "every direct dependency of the repository"
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
            "        run: cargo xtask check --full\n",
            "        run: cargo xtask check --full\n        continue-on-error: true\n",
        ),
        "`continue-on-error: true`: `continue-on-error` is not allowed",
    );
    refused(
        &PINNED_WORKFLOW.replace(
            "      - name: Every quality check\n",
            "      - name: Every quality check\n        if: false\n",
        ),
        "`if: false`: the only conditions are `if: failure()` and `if: github.event_name != \'push\' || contains(github.event.head_commit.message, \'[ci]\')`",
    );
    refused(
        &PINNED_WORKFLOW.replace("nightly-2026-11-01", "nightly-2020-01-01"),
        "`nightly-2020-01-01` is not the pinned nightly, nightly-2026-11-01",
    );
    refused(
        &PINNED_WORKFLOW.replace(
            "        run: cargo xtask check --full\n",
            "        run: cargo xtask check --full\n        env:\n          RUSTFLAGS: -Aunsafe\n",
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
            &format!("{PINNED_WORKFLOW}  other:\n    permissions:\n      contents: write\n")
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
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let real: Vec<String> = super::SUPPLY_CHAIN_SETTINGS
        .iter()
        .map(|(path, _)| std::fs::read_to_string(root.join(path)).expect("readable"))
        .collect();
    assert_eq!(
        super::supply_chain_gate(&real),
        Ok(()),
        "the repository's settings"
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
        "the crates of the audited files, in the order of RUST_CRATES"
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
        11 + 1,
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
fn refuses_a_second_place_declaring_triggers() {
    let workflow = format!("{PINNED_WORKFLOW}  other:\n    on:\n      schedule:\n");
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
        Ok(()),
        "a scope review, left to the scope gate"
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
    assert_eq!(
        super::bypass_gate(&[], reviewed),
        failed(
            "bypasses of coverage or mutation testing must be reviewed in proof-bypasses.txt: \
             stale review: crates/a/src/lib.rs: a token at a line"
        ),
        "the bypass gate leaves the scope reviews to the scope gate"
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
        action: Action::TypeScriptScopeGate,
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
    assert_eq!(
        run_steps(&step, &mut fake, &mut Vec::new()),
        failed(
            "Words: words the glossary replaces (reviewed in vocabulary.txt): README.md:1: \
             \"node\": say \"box\" (the glossary)"
        ),
        "the tracked prose against the vocabulary"
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
    unlisted.broken = Some("list *.md *.rs *.ts *.tsx *.mts *.js *.mjs".to_owned());
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
            ""
        )
        .is_err(),
        "the history refuses it too"
    );
    assert_eq!(
        super::history_gate(&record(super::CI_MARKER_EXEMPT[0], sentence), ""),
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
            super::history_gate(&merged, HISTORY_FILES),
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
        super::history_gate(&below, HISTORY_FILES),
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
            super::history_gate(&record("ffffffffff", &message), ""),
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
        root: Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."),
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
