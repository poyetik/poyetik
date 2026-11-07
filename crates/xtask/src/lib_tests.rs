//! Tests of the check pipeline, with a fake runner.

extern crate alloc;

use alloc::collections::BTreeMap;
use std::io;
use std::path::Path;

use super::{
    Action, COVERAGE_REPORT, Cmd, Dir, PORTABILITY_TARGETS, Runner, Step, SystemRunner,
    UNVIABLE_REPORT, UNVIABLE_REVIEWED, USAGE, check_steps, cli, coverage_gate, run_steps,
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
    fake
}

fn command_lines(steps: &[Step]) -> Vec<String> {
    steps
        .iter()
        .map(|step| match &step.action {
            Action::Run(cmd) => format!("{:?}: {} {}", cmd.dir, cmd.program, cmd.args.join(" ")),
            Action::CoverageGate(path) => format!("gate {path}"),
            Action::UnviableGate => "unviable gate".to_owned(),
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
            "Root: cargo test --workspace --locked",
            "Root: cargo build --package engine-core --locked --target aarch64-apple-darwin",
            "Root: cargo build --package engine-core --locked --target x86_64-unknown-linux-gnu",
            "Root: cargo build --package engine-core --locked --target wasm32-unknown-unknown",
            "Root: cargo build --package engine-core --locked --target thumbv7em-none-eabihf",
            "Root: cargo +nightly-2026-11-01 llvm-cov --workspace --locked --branch --json --summary-only --output-path target/coverage.json",
            "gate target/coverage.json",
            "Root: cargo mutants --workspace --cap-lints=true",
            "unviable gate",
            "Root: cargo deny --locked check",
            "Roadmap: pnpm install --frozen-lockfile",
            "Roadmap: pnpm run check",
        ],
        "the full check pipeline"
    );
    assert_eq!(
        steps.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        vec![
            "Rust formatting",
            "Rust lints",
            "Rust tests",
            "engine-core portability (aarch64-apple-darwin)",
            "engine-core portability (x86_64-unknown-linux-gnu)",
            "engine-core portability (wasm32-unknown-unknown)",
            "engine-core portability (thumbv7em-none-eabihf)",
            "Rust coverage report",
            "Rust coverage: 100% lines, functions and branches",
            "Rust mutation testing",
            "Rust unviable mutants reviewed",
            "Rust supply chain",
            "Roadmap dependencies",
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
        Path::new("/repo/roadmap"),
        "roadmap"
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
        check_steps().len() + 1,
        "every step ran, the gate reads two files"
    );
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
        text.starts_with("▶ [1/14] Rust formatting\n"),
        "numbered progress: {text}"
    );
    assert!(
        text.contains("▶ [14/14] Roadmap checks\n"),
        "last step announced: {text}"
    );
    assert!(
        text.ends_with("✔ all 14 checks passed\n"),
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
    assert_eq!(fake.ran.len(), 3, "nothing runs after the failure");
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
