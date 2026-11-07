//! Tests of the check pipeline, with a fake runner.

extern crate alloc;

use alloc::collections::BTreeMap;
use std::io;
use std::path::Path;

use super::{
    Action, CARGO_TOML, CHANGELOG, COMMIT_HEADER_MAX, COVERAGE_REPORT, Cmd, Dir, PACKAGE_JSONS,
    PORTABILITY_TARGETS, Runner, Step, SystemRunner, UNVIABLE_REPORT, UNVIABLE_REVIEWED, USAGE,
    Version, bless_step, check_steps, cli, commit_msg_gate, coverage_gate, release_gate, run_steps,
    unviable_gate,
};

/// Records commands and answers with scripted results.
#[derive(Debug, Default)]
struct Fake {
    ran: Vec<String>,
    fail: Option<String>,
    broken: Option<String>,
    files: BTreeMap<String, String>,
}

impl Runner for Fake {
    fn run(&mut self, cmd: &Cmd) -> io::Result<bool> {
        let line = format!("{} {}", cmd.program, cmd.args.join(" "));
        self.ran.push(line.clone());
        if self.broken.as_deref() == Some(line.as_str()) {
            return Err(io::Error::other("not found"));
        }
        Ok(self.fail.as_deref() != Some(line.as_str()))
    }

    fn read(&mut self, path: &str) -> io::Result<String> {
        self.ran.push(format!("read {path}"));
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::other("missing"))
    }
}

fn report(lines: (u64, u64), functions: (u64, u64), branches: (u64, u64)) -> String {
    format!(
        r#"{{"data":[{{"totals":{{"lines":{{"count":{},"covered":{}}},"functions":{{"count":{},"covered":{}}},"branches":{{"count":{},"covered":{}}}}}}}]}}"#,
        lines.0, lines.1, functions.0, functions.1, branches.0, branches.1
    )
}

fn full_report() -> String {
    report((10, 10), (3, 3), (4, 4))
}

fn passing() -> Fake {
    let mut fake = Fake::default();
    let previous = fake.files.insert(COVERAGE_REPORT.to_owned(), full_report());
    assert!(previous.is_none(), "fresh fake");
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
    for (path, text) in [
        (CARGO_TOML, CARGO),
        (PACKAGE_JSONS[0], PACKAGE),
        (PACKAGE_JSONS[1], PACKAGE),
        (PACKAGE_JSONS[2], PACKAGE),
        (PACKAGE_JSONS[3], PACKAGE),
        (PACKAGE_JSONS[4], PACKAGE),
        (CHANGELOG, LOG),
        ("COMMIT_EDITMSG", "feat(core): add the plan\n"),
    ] {
        let earlier = fake.files.insert(path.to_owned(), text.to_owned());
        assert!(earlier.is_none(), "fresh fake");
    }
    fake
}

const CARGO: &str = "[workspace]\nmembers = []\n\n[workspace.package]\nedition = \"2024\"\nversion = \"0.2.0\"\n\n[other]\nversion = \"9.9.9\"\n";
const PACKAGE: &str = r#"{"name": "x", "version": "0.2.0"}"#;
/// Both workspace manifests with the same content.
fn packages(text: &str) -> Vec<(&'static str, String)> {
    PACKAGE_JSONS
        .iter()
        .map(|&path| (path, text.to_owned()))
        .collect()
}

/// Every manifest correct except the roadmap one, with `text`.
fn roadmap_package(text: &str) -> Vec<(&'static str, String)> {
    vec![
        (PACKAGE_JSONS[0], PACKAGE.to_owned()),
        (PACKAGE_JSONS[1], text.to_owned()),
        (PACKAGE_JSONS[2], PACKAGE.to_owned()),
        (PACKAGE_JSONS[3], PACKAGE.to_owned()),
        (PACKAGE_JSONS[4], PACKAGE.to_owned()),
    ]
}

const LOG: &str = "# Changelog\n\nIntro.\n\n## [Unreleased]\n\n### Added\n\n- x\n\n## [0.2.0] - 2026-11-07\n\n## [0.1.10] - 2026-10-01\n\n## [0.1.2] - 2026-09-30\n";

fn command_lines(steps: &[Step]) -> Vec<String> {
    steps
        .iter()
        .map(|step| match &step.action {
            Action::Run(cmd) => format!("{:?}: {} {}", cmd.dir, cmd.program, cmd.args.join(" ")),
            Action::CoverageGate(path) => format!("gate {path}"),
            Action::UnviableGate => "unviable gate".to_owned(),
            Action::ReleaseGate => "release gate".to_owned(),
        })
        .collect()
}

#[test]
fn lists_every_check_in_order() {
    let steps = check_steps();
    assert_eq!(
        command_lines(&steps),
        vec![
            "Root: cargo fmt --all --check",
            "Root: cargo clippy --workspace --all-targets --locked",
            "Root: cargo doc --workspace --no-deps --document-private-items --locked",
            "Root: cargo test --workspace --locked",
            "Root: cargo build --package engine-core --package engine-objects --package engine-patch --package engine-hash --locked --target aarch64-apple-darwin",
            "Root: cargo build --package engine-core --package engine-objects --package engine-patch --package engine-hash --locked --target x86_64-unknown-linux-gnu",
            "Root: cargo build --package engine-core --package engine-objects --package engine-patch --package engine-hash --locked --target wasm32-unknown-unknown",
            "Root: cargo build --package engine-core --package engine-objects --package engine-patch --package engine-hash --locked --target thumbv7em-none-eabihf",
            "Root: cargo +nightly-2026-11-01 llvm-cov --workspace --locked --branch --json --summary-only --output-path target/coverage.json",
            "gate target/coverage.json",
            "Root: cargo mutants --workspace --cap-lints=true",
            "unviable gate",
            "Root: cargo +nightly-2026-11-01 miri test --package engine-wasm --locked",
            "Root: cargo run --package engine-cli --locked --quiet -- check conformance",
            "Root: cargo deny --locked check",
            "release gate",
            "Root: pnpm install --frozen-lockfile",
            "EngineWasm: pnpm run check",
            "Playground: pnpm run check",
            "Roadmap: pnpm run check",
        ],
        "the full check pipeline"
    );
    assert_eq!(
        steps.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        vec![
            "Rust formatting",
            "Rust lints",
            "Rust documentation",
            "Rust tests",
            "engine portability (aarch64-apple-darwin)",
            "engine portability (x86_64-unknown-linux-gnu)",
            "engine portability (wasm32-unknown-unknown)",
            "engine portability (thumbv7em-none-eabihf)",
            "Rust coverage report",
            "Rust coverage: 100% lines, functions and branches",
            "Rust mutation testing",
            "Rust unviable mutants reviewed",
            "Rust FFI under Miri",
            "Conformance (native)",
            "Rust supply chain",
            "Versions and changelog",
            "JavaScript dependencies",
            "Engine WASM checks",
            "Playground checks",
            "Roadmap checks",
        ],
        "step names"
    );
    assert_eq!(PORTABILITY_TARGETS.len(), 4, "four portability targets");
}

#[test]
fn resolves_directories() {
    let root = Path::new("/repo");
    assert_eq!(Dir::Root.under(root), Path::new("/repo"), "root");
    assert_eq!(
        Dir::Roadmap.under(root),
        Path::new("/repo/apps/roadmap"),
        "roadmap"
    );
    assert_eq!(
        Dir::EngineWasm.under(root),
        Path::new("/repo/bindings/wasm"),
        "engine-wasm"
    );
    assert_eq!(
        Dir::Playground.under(root),
        Path::new("/repo/apps/playground"),
        "playground"
    );
}

#[test]
fn accepts_full_coverage() {
    assert_eq!(coverage_gate(&full_report()), Ok(()), "100% everywhere");
    assert_eq!(
        coverage_gate(&report((0, 0), (0, 0), (0, 0))),
        Ok(()),
        "nothing to cover"
    );
}

#[test]
fn rejects_incomplete_coverage() {
    assert_eq!(
        coverage_gate(&report((10, 9), (3, 3), (4, 4))),
        Err("coverage is not 100%: lines 9/10".to_owned()),
        "a missed line"
    );
    assert_eq!(
        coverage_gate(&report((10, 10), (3, 2), (4, 3))),
        Err("coverage is not 100%: functions 2/3, branches 3/4".to_owned()),
        "every incomplete kind is named"
    );
}

#[test]
fn rejects_unreadable_reports() {
    assert!(
        coverage_gate("not json").is_err_and(|e| e.starts_with("invalid coverage report: ")),
        "not JSON"
    );
    assert_eq!(
        coverage_gate(r#"{"data":[{"totals":{"lines":{"count":1,"covered":1}}}]}"#),
        Err("coverage report has no functions total".to_owned()),
        "a missing total"
    );
    assert_eq!(
        coverage_gate(r#"{"data":[{"totals":{"lines":{"count":1}}}]}"#),
        Err("coverage report has no lines total".to_owned()),
        "a missing field"
    );
}

#[test]
fn runs_every_step_and_reports_success() {
    let mut fake = passing();
    let mut out = Vec::new();
    assert_eq!(
        run_steps(&check_steps(), &mut fake, &mut out),
        Ok(()),
        "all green"
    );
    assert_eq!(
        fake.ran.len(),
        check_steps().len() + 7,
        "every step ran; the unviable gate reads two files, the release gate seven"
    );
    for path in [
        CARGO_TOML,
        PACKAGE_JSONS[0],
        PACKAGE_JSONS[1],
        PACKAGE_JSONS[2],
        PACKAGE_JSONS[3],
        PACKAGE_JSONS[4],
        CHANGELOG,
    ] {
        assert!(fake.ran.contains(&format!("read {path}")), "{path} read");
    }
    assert!(
        fake.ran.contains(&format!("read {UNVIABLE_REVIEWED}")),
        "reviews read"
    );
    assert!(
        fake.ran.contains(&format!("read {COVERAGE_REPORT}")),
        "coverage report read"
    );
    let text = String::from_utf8(out).expect("utf-8 output");
    assert!(
        text.starts_with("▶ [1/20] Rust formatting\n"),
        "numbered progress: {text}"
    );
    assert!(
        text.contains("▶ [20/20] Roadmap checks\n"),
        "last step announced: {text}"
    );
    assert!(
        text.ends_with("✔ all 20 checks passed\n"),
        "summary: {text}"
    );
}

#[test]
fn stops_at_the_first_failing_command() {
    let mut fake = passing();
    fake.fail = Some("cargo test --workspace --locked".to_owned());
    let mut out = Vec::new();
    assert_eq!(
        run_steps(&check_steps(), &mut fake, &mut out),
        Err("✘ Rust tests: command failed".to_owned()),
        "failure is named"
    );
    assert_eq!(fake.ran.len(), 4, "nothing runs after the failure");
}

#[test]
fn reports_commands_that_cannot_start() {
    let mut fake = passing();
    fake.broken = Some("cargo fmt --all --check".to_owned());
    assert_eq!(
        run_steps(&check_steps(), &mut fake, &mut Vec::new()),
        Err("✘ Rust formatting: cannot run cargo: not found".to_owned()),
        "start failure is named"
    );
}

#[test]
fn fails_on_missing_or_incomplete_coverage() {
    let mut missing = Fake::default();
    assert_eq!(
        run_steps(&check_steps(), &mut missing, &mut Vec::new()),
        Err(format!(
            "✘ Rust coverage: 100% lines, functions and branches: cannot read {COVERAGE_REPORT}: missing"
        )),
        "missing report"
    );
    let mut partial = Fake::default();
    let previous = partial
        .files
        .insert(COVERAGE_REPORT.to_owned(), report((2, 1), (1, 1), (0, 0)));
    assert!(previous.is_none(), "fresh fake");
    assert_eq!(
        run_steps(&check_steps(), &mut partial, &mut Vec::new()),
        Err(
            "✘ Rust coverage: 100% lines, functions and branches: coverage is not 100%: lines 1/2"
                .to_owned()
        ),
        "incomplete coverage"
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
        run_steps(&check_steps(), &mut passing(), &mut Broken),
        Err("closed".to_owned()),
        "cannot even announce a step"
    );
    let one = vec![check_steps().remove(0)];
    let mut limited = Limited(1);
    assert_eq!(
        run_steps(&one, &mut passing(), &mut limited),
        Err("closed".to_owned()),
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
    assert!(
        cli(&["check".to_owned()], &mut passing(), &mut out, &mut err),
        "check passes"
    );
    assert!(err.is_empty(), "no error output");

    let mut failing = passing();
    failing.fail = Some("cargo deny --locked check".to_owned());
    let mut err2 = Vec::new();
    assert!(
        !cli(
            &["check".to_owned()],
            &mut failing,
            &mut Vec::new(),
            &mut err2
        ),
        "check fails"
    );
    assert_eq!(
        err2, b"\xE2\x9C\x98 Rust supply chain: command failed\n",
        "failure on stderr"
    );

    let mut help = Vec::new();
    assert!(
        cli(
            &["help".to_owned()],
            &mut passing(),
            &mut help,
            &mut Vec::new()
        ),
        "help succeeds"
    );
    assert_eq!(help, format!("{USAGE}\n").into_bytes(), "help on stdout");
    assert!(
        !cli(
            &["help".to_owned()],
            &mut passing(),
            &mut Broken,
            &mut Vec::new()
        ),
        "broken stdout"
    );

    for args in [
        vec![],
        vec!["nope".to_owned()],
        vec!["check".to_owned(), "x".to_owned()],
    ] {
        let mut usage = Vec::new();
        assert!(
            !cli(&args, &mut passing(), &mut Vec::new(), &mut usage),
            "{args:?} is rejected"
        );
        assert_eq!(usage, format!("{USAGE}\n").into_bytes(), "usage on stderr");
    }
    assert!(
        !cli(&[], &mut passing(), &mut Vec::new(), &mut Broken),
        "broken stderr still fails"
    );
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
    assert_eq!(runner.run(&ok).ok(), Some(true), "a successful command");
    let bad = Cmd {
        program: "cargo",
        args: vec!["no-such-command".to_owned()],
        dir: Dir::Root,
    };
    assert_eq!(runner.run(&bad).ok(), Some(false), "a failing command");
    let missing = Cmd {
        program: "no-such-program-xyz",
        args: vec![],
        dir: Dir::Root,
    };
    assert!(runner.run(&missing).is_err(), "a program that cannot start");
    assert!(
        runner
            .read("Cargo.toml")
            .is_ok_and(|text| text.contains("[workspace]")),
        "reads below the root"
    );
    assert!(runner.read("no-such-file").is_err(), "missing file");
}

#[test]
fn accepts_exactly_the_reviewed_unviable_mutants() {
    let report = "crates/a/src/b.rs:3:5: replace f -> T with Default::default()\n\ncrates/a/src/c.rs:10:1: replace g -> U with x\n";
    let reviewed = "# reviewed mutants\n\ncrates/a/src/c.rs: replace g -> U with x -- U has no x\n  crates/a/src/b.rs: replace f -> T with Default::default() -- T has no Default  \n";
    assert_eq!(
        unviable_gate(report, reviewed),
        Ok(()),
        "same set, lines ignored"
    );
    assert_eq!(unviable_gate("", ""), Ok(()), "nothing unviable");
}

#[test]
fn rejects_unreviewed_stale_and_malformed_entries() {
    let error = unviable_gate(
        "crates/a/src/b.rs:3:5: replace f -> T with y\nnot a mutant line\ncrates/a/src/b.rs:9:9\n",
        "crates/a/src/z.rs: replace z -> Z with w -- old\ncrates/a/src/q.rs: replace q -> Q with w --  \nno separator\n",
    );
    assert_eq!(
        error,
        Err(format!(
            "unviable mutants must be reviewed in {UNVIABLE_REVIEWED}:\n{}",
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
    assert_eq!(
        run_steps(&check_steps(), &mut no_report, &mut Vec::new()),
        Err(format!(
            "✘ Rust unviable mutants reviewed: cannot read {UNVIABLE_REPORT}: missing"
        )),
        "missing report"
    );
    let mut no_reviews = passing();
    let removed_reviews = no_reviews.files.remove(UNVIABLE_REVIEWED);
    assert!(removed_reviews.is_some(), "reviews removed");
    assert_eq!(
        run_steps(&check_steps(), &mut no_reviews, &mut Vec::new()),
        Err(format!(
            "✘ Rust unviable mutants reviewed: cannot read {UNVIABLE_REVIEWED}: missing"
        )),
        "missing reviews"
    );
    let mut unreviewed = passing();
    let _previous = unreviewed
        .files
        .insert(UNVIABLE_REVIEWED.to_owned(), String::new());
    assert!(
        run_steps(&check_steps(), &mut unreviewed, &mut Vec::new())
            .is_err_and(|e| e.contains("not reviewed: crates/a/src/b.rs")),
        "unreviewed mutant"
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
        release_gate(CARGO, &roadmap_package(r#"{"version": "0.3.0"}"#), LOG),
        Err(format!(
            "{}: version 0.3.0 differs from the workspace version 0.2.0",
            PACKAGE_JSONS[1]
        )),
        "different versions"
    );
    assert_eq!(
        release_gate(CARGO, &roadmap_package("not json"), LOG),
        Err(format!(
            "{}: version (missing) differs from the workspace version 0.2.0",
            PACKAGE_JSONS[1]
        )),
        "unreadable package"
    );
    assert_eq!(
        release_gate(CARGO, &packages(r#"{"version": "0.1.0"}"#), LOG),
        Err(PACKAGE_JSONS
            .iter()
            .map(|path| format!("{path}: version 0.1.0 differs from the workspace version 0.2.0"))
            .collect::<Vec<_>>()
            .join("\n")),
        "every manifest is checked"
    );
    assert_eq!(
        release_gate("[workspace]\n", &packages(r#"{"version": 1}"#), LOG),
        Err(format!(
            "{CARGO_TOML}: [workspace.package] version must be X.Y.Z"
        )),
        "no workspace version, and none in the package either"
    );
    let bad = CARGO.replace("0.2.0\"\n\n[other]", "0.2\"\n\n[other]");
    assert_eq!(
        release_gate(&bad, &packages(&PACKAGE.replace("0.2.0", "0.2")), LOG),
        Err(format!(
            "{CARGO_TOML}: [workspace.package] version must be X.Y.Z"
        )),
        "not X.Y.Z"
    );
    let old = CARGO.replace("0.2.0\"\n\n[other]", "0.1.9\"\n\n[other]");
    assert_eq!(
        release_gate(&old, &packages(&PACKAGE.replace("0.2.0", "0.1.9")), LOG),
        Err("version 0.1.9 is older than the latest release 0.2.0".to_owned()),
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
        Err(format!(
            "{CHANGELOG}: must start with \"# Changelog\"\n{CHANGELOG}: the first section must be \"## [Unreleased]\""
        )),
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
            Err(format!("{CHANGELOG}: malformed heading \"{heading}\"")),
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
            Err(format!(
                "{CHANGELOG}: releases must be in strictly decreasing order"
            )),
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
        "Merge branch 'main' into feature",
        "Revert \"feat: add the plan\"",
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
        Err(format!(
            "invalid commit message: {reason}\nexpected: <type>(<scope>)!: <summary>, type one of feat, fix, docs, test, refactor, perf, build, ci, chore, style, revert"
        ))
    };
    let long = format!("feat: {}", "x".repeat(COMMIT_HEADER_MAX - 5));
    let cases: [(&str, String); 11] = [
        ("", "missing \"<type>: \" prefix".to_owned()),
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
    assert!(
        cli(&args, &mut passing(), &mut out, &mut err),
        "a valid message"
    );
    assert!(out.is_empty() && err.is_empty(), "silent on success");

    let mut bad = passing();
    let replaced = bad
        .files
        .insert("COMMIT_EDITMSG".to_owned(), "wip".to_owned());
    assert!(replaced.is_some(), "message replaced");
    let mut bad_err = Vec::new();
    assert!(
        !cli(&args, &mut bad, &mut Vec::new(), &mut bad_err),
        "an invalid message"
    );
    assert!(
        String::from_utf8(bad_err).is_ok_and(|e| e.starts_with("invalid commit message: ")),
        "explained on stderr"
    );

    let mut missing_err = Vec::new();
    let missing = ["commit-msg".to_owned(), "nowhere".to_owned()];
    assert!(
        !cli(&missing, &mut passing(), &mut Vec::new(), &mut missing_err),
        "a missing file"
    );
    assert_eq!(
        missing_err, b"cannot read nowhere: missing\n",
        "named on stderr"
    );
}

#[test]
fn release_gate_runs_on_the_real_files() {
    let mut fake = passing();
    let replaced = fake.files.insert(CHANGELOG.to_owned(), "nope".to_owned());
    assert!(replaced.is_some(), "changelog replaced");
    assert!(
        run_steps(&check_steps(), &mut fake, &mut Vec::new())
            .is_err_and(|e| e.starts_with("✘ Versions and changelog: ")),
        "release gate failure is named"
    );
    for path in [
        CARGO_TOML,
        PACKAGE_JSONS[0],
        PACKAGE_JSONS[1],
        PACKAGE_JSONS[2],
        PACKAGE_JSONS[3],
        PACKAGE_JSONS[4],
        CHANGELOG,
    ] {
        let mut without = passing();
        let removed = without.files.remove(path);
        assert!(removed.is_some(), "{path} removed");
        assert_eq!(
            run_steps(&check_steps(), &mut without, &mut Vec::new()),
            Err(format!(
                "✘ Versions and changelog: cannot read {path}: missing"
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
    assert!(
        cli(&["bless".to_owned()], &mut fake, &mut out, &mut Vec::new()),
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
