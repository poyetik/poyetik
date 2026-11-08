//! Tests of the check pipeline, with a fake runner.

extern crate alloc;

use alloc::collections::BTreeMap;
use std::io;
use std::path::Path;
use std::process::Command;

use super::{
    Action, BestEffort, CARGO_TOML, CHANGELOG, COMMIT_HEADER_MAX, COVERAGE_REPORT, Cmd, Dir,
    EQUIVALENT_REVIEWED, PACKAGE_JSONS, PORTABILITY_TARGETS, Runner, Step, SystemRunner,
    TYPESCRIPT_PACKAGES, UNVIABLE_REPORT, UNVIABLE_REVIEWED, USAGE, Version, bless_step,
    check_steps, cli, commit_msg_gate, coverage_gate, equivalent_gate, membership_gate,
    release_gate, run_steps, ts_coverage_gate, ts_mutation_gate, unviable_gate,
};

/// Records commands and answers with scripted results.
#[derive(Debug, Default)]
struct Fake {
    ran: Vec<String>,
    fail: Option<String>,
    broken: Option<String>,
    killed: Option<String>,
    files: BTreeMap<String, String>,
}

impl Runner for Fake {
    fn run(&mut self, cmd: &Cmd) -> io::Result<Option<i32>> {
        let line = cmd.line();
        self.ran.push(line.clone());
        if self.broken.as_deref() == Some(line.as_str()) {
            return Err(io::Error::other("not found"));
        }
        if self.killed.as_deref() == Some(line.as_str()) {
            return Ok(None);
        }
        // A failing command exits with 3: a code that is neither 0 nor 1, so that the
        // message is seen to report the real one.
        Ok(Some(if self.fail.as_deref() == Some(line.as_str()) {
            3_i32
        } else {
            0_i32
        }))
    }

    fn read(&mut self, path: &str) -> io::Result<String> {
        self.ran.push(format!("read {path}"));
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::other("missing"))
    }

    fn fresh_dir(&mut self, path: &str) -> io::Result<()> {
        let line = format!("fresh {path}");
        self.ran.push(line.clone());
        if self.broken.as_deref() == Some(line.as_str()) {
            return Err(io::Error::other("busy"));
        }
        Ok(())
    }

    /// Matches the pathspecs this crate uses: `*<suffix>`.
    fn tracked(&mut self, patterns: &[&str]) -> io::Result<Vec<String>> {
        let line = format!("list {}", patterns.join(" "));
        self.ran.push(line.clone());
        if self.broken.as_deref() == Some(line.as_str()) {
            return Err(io::Error::other("no git"));
        }
        Ok(self
            .files
            .keys()
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

fn report(lines: (u64, u64), functions: (u64, u64), branches: (u64, u64)) -> String {
    format!(
        r#"{{"data":[{{"totals":{{"lines":{{"count":{},"covered":{}}},"functions":{{"count":{},"covered":{}}},"branches":{{"count":{},"covered":{}}}}}}}]}}"#,
        lines.0, lines.1, functions.0, functions.1, branches.0, branches.1
    )
}

fn full_report() -> String {
    report((10, 10), (3, 3), (4, 4))
}

/// A Vitest coverage summary at 100%.
const TS_COVERED: &str = r#"{"total":{"lines":{"total":4,"covered":4},"statements":{"total":5,"covered":5},"functions":{"total":2,"covered":2},"branches":{"total":3,"covered":3}}}"#;

/// A Stryker report where every mutant was killed (or timed out, or was disabled).
const TS_KILLED: &str = r#"{"files":{"a.ts":{"mutants":[{"status":"Killed"},{"status":"Timeout"}]},"b.ts":{"mutants":[{"status":"Ignored"}]}}}"#;

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
    let source = "const a = 1;\n// Stryker disable next-line EqualityOperator: same result\nwhile (a < 2) {}\n";
    let equivalent = "x/a.ts: EqualityOperator at while (a < 2) {} -- same result\n";
    for package in TYPESCRIPT_PACKAGES {
        let summary = fake.files.insert(
            format!("{package}/coverage/coverage-summary.json"),
            TS_COVERED.to_owned(),
        );
        let mutation = fake.files.insert(
            format!("{package}/reports/mutation/mutation.json"),
            TS_KILLED.to_owned(),
        );
        assert!(summary.is_none() && mutation.is_none(), "fresh fake");
    }
    let third = fake.files.insert("x/a.ts".to_owned(), source.to_owned());
    let fourth = fake
        .files
        .insert(EQUIVALENT_REVIEWED.to_owned(), equivalent.to_owned());
    assert!(third.is_none() && fourth.is_none(), "fresh fake");
    for (path, text) in [
        (CARGO_TOML, CARGO),
        (PACKAGE_JSONS[0], PACKAGE),
        (PACKAGE_JSONS[1], PACKAGE),
        (PACKAGE_JSONS[2], PACKAGE),
        (PACKAGE_JSONS[3], PACKAGE),
        (PACKAGE_JSONS[4], PACKAGE),
        (PACKAGE_JSONS[5], PACKAGE),
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
        (PACKAGE_JSONS[5], PACKAGE.to_owned()),
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
            Action::EquivalentGate => "equivalent gate".to_owned(),
            Action::ReleaseGate => "release gate".to_owned(),
            Action::FreshDir(path) => format!("fresh {path}"),
            Action::FreshDirs(paths) => format!("fresh {}", paths.join(" ")),
            Action::TypeScriptReportsGate => "typescript reports gate".to_owned(),
            Action::MembershipGate => "membership gate".to_owned(),
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
            "Root: cargo mutants --workspace --cap-lints=true --gitignore=true",
            "unviable gate",
            "equivalent gate",
            "Root: cargo +nightly-2026-11-01 miri test --package engine-wasm --locked",
            "Root: cargo run --package engine-cli --locked --quiet -- check conformance",
            "fresh target/conformance",
            "Root: cargo run --package engine-cli --locked --quiet -- export conformance target/conformance",
            "Root: cargo deny --locked check",
            "membership gate",
            "release gate",
            "Root: pnpm install --frozen-lockfile",
            "fresh tooling/oracle/coverage tooling/oracle/reports/mutation bindings/wasm/coverage bindings/wasm/reports/mutation apps/playground/coverage apps/playground/reports/mutation apps/roadmap/coverage apps/roadmap/reports/mutation",
            "Oracle: pnpm run check",
            "EngineWasm: pnpm run check",
            "Playground: pnpm run check",
            "Roadmap: pnpm run check",
            "typescript reports gate",
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
            "TypeScript equivalent mutants reviewed",
            "Rust FFI under Miri",
            "Conformance (native)",
            "Fresh conformance export directory",
            "Conformance export (native WAV files)",
            "Rust supply chain",
            "Packages and crates known to the checks",
            "Versions and changelog",
            "JavaScript dependencies",
            "Fresh TypeScript report directories",
            "Conformance oracle (independent, TypeScript)",
            "Engine WASM checks",
            "Playground checks",
            "Roadmap checks",
            "TypeScript coverage and mutation reports",
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
    assert_eq!(
        Dir::Oracle.under(root),
        Path::new("/repo/tooling/oracle"),
        "oracle"
    );
}

#[test]
fn accepts_full_coverage() {
    assert_eq!(coverage_gate(&full_report()), Ok(()), "100% everywhere");
}

#[test]
fn rejects_coverage_that_measured_nothing() {
    assert_eq!(
        coverage_gate(&report((0, 0), (0, 0), (0, 0))),
        Err(
            "coverage is not 100%: lines not measured, functions not measured, branches not measured"
                .to_owned()
        ),
        "an empty report proves nothing"
    );
    assert_eq!(
        coverage_gate(&report((10, 10), (3, 3), (0, 0))),
        Err("coverage is not 100%: branches not measured".to_owned()),
        "branch instrumentation missing"
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
        check_steps().len() + 7 + 10 + 7 + 2,
        "every step ran; the unviable gate reads two files, the equivalent gate lists the \
         sources and reads two files, the release gate eight, the fresh step empties seven \
         more directories, the TypeScript gate reads eight reports instead of one action, \
         the membership gate lists twice and reads the root Cargo.toml"
    );
    for path in [
        CARGO_TOML,
        PACKAGE_JSONS[0],
        PACKAGE_JSONS[1],
        PACKAGE_JSONS[2],
        PACKAGE_JSONS[3],
        PACKAGE_JSONS[4],
        PACKAGE_JSONS[5],
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
        text.starts_with("▶ [1/27] Rust formatting\n"),
        "numbered progress: {text}"
    );
    assert!(
        text.contains("▶ [26/27] Roadmap checks\n"),
        "last step announced: {text}"
    );
    assert!(
        text.ends_with("✔ all 27 checks passed\n"),
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
        Err("✘ Rust tests: `cargo test --workspace --locked` exited with code 3".to_owned()),
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
        .insert(COVERAGE_REPORT.to_owned(), report((2, 1), (1, 1), (1, 1)));
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
    assert_eq!(
        cli(&["check".to_owned()], &mut passing(), &mut out, &mut err),
        0,
        "check passes"
    );
    assert!(err.is_empty(), "no error output");

    let mut failing = passing();
    failing.fail = Some("cargo deny --locked check".to_owned());
    let mut err2 = Vec::new();
    assert_eq!(
        cli(
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
        "✘ Rust supply chain: `cargo deny --locked check` exited with code 3\n",
        "the command and its exit code on stderr"
    );

    let mut killed = passing();
    killed.killed = Some("cargo deny --locked check".to_owned());
    let mut err3 = Vec::new();
    assert_eq!(
        cli(
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
        "✘ Rust supply chain: `cargo deny --locked check` was stopped by a signal\n",
        "explained"
    );

    let mut help = Vec::new();
    assert_eq!(
        cli(
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
        cli(
            &["help".to_owned()],
            &mut passing(),
            &mut Broken,
            &mut Vec::new()
        ),
        1,
        "broken stdout"
    );

    for args in [
        vec![],
        vec!["nope".to_owned()],
        vec!["check".to_owned(), "x".to_owned()],
    ] {
        let mut usage = Vec::new();
        assert_eq!(
            cli(&args, &mut passing(), &mut Vec::new(), &mut usage),
            2,
            "{args:?} is a usage error"
        );
        assert_eq!(usage, format!("{USAGE}\n").into_bytes(), "usage on stderr");
    }
    assert_eq!(
        cli(&[], &mut passing(), &mut Vec::new(), &mut Broken),
        2,
        "broken stderr still tells"
    );
    assert_eq!(
        cli(
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
    assert_eq!(closed.write(b"abc").map_err(|e| e.kind()), Ok(3), "write");
    assert_eq!(closed.flush().map_err(|e| e.kind()), Ok(()), "flush");
    let mut full = BestEffort(Pipe(io::ErrorKind::StorageFull));
    assert_eq!(
        full.write(b"abc").map_err(|e| e.kind()),
        Err(io::ErrorKind::StorageFull),
        "write"
    );
    assert_eq!(
        full.flush().map_err(|e| e.kind()),
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
    assert_eq!(
        runner.run(&ok).ok(),
        Some(Some(0_i32)),
        "a successful command"
    );
    let bad = Cmd {
        program: "cargo",
        args: vec!["no-such-command".to_owned()],
        dir: Dir::Root,
    };
    assert_eq!(
        runner.run(&bad).ok(),
        Some(Some(101_i32)),
        "a failing command and its code"
    );
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
    #[cfg(unix)]
    {
        // A directory whose contents cannot be removed, but which exists: emptying it
        // fails, even though creating it again would succeed.
        use std::os::unix::fs::PermissionsExt;
        let locked = root.join("locked");
        std::fs::create_dir_all(locked.join("inner")).expect("locked directory");
        std::fs::write(locked.join("inner/file"), b"").expect("file inside");
        let set = |mode: u32| {
            std::fs::set_permissions(locked.join("inner"), std::fs::Permissions::from_mode(mode))
                .expect("permissions");
        };
        set(0o555);
        let result = runner.fresh_dir("locked/inner");
        set(0o755);
        assert!(
            result.is_err(),
            "a directory that cannot be emptied is an error"
        );
    }
    std::fs::remove_dir_all(&root).expect("cleanup");
}

#[test]
fn reports_a_directory_that_cannot_be_emptied() {
    let mut fake = passing();
    fake.broken = Some("fresh target/conformance".to_owned());
    assert_eq!(
        run_steps(&check_steps(), &mut fake, &mut Vec::new()),
        Err(
            "✘ Fresh conformance export directory: cannot empty target/conformance: busy"
                .to_owned()
        ),
        "named and explained"
    );
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
    let error = equivalent_gate(&source(disabled), "b.ts: X at y -- old\nno reason here\n")
        .expect_err("unreviewed, stale and malformed");
    assert_eq!(
        error,
        format!(
            "disabled mutants must be reviewed in {EQUIVALENT_REVIEWED}:\n{}",
            [
                "review without a reason: no reason here",
                "not reviewed: a.ts: EqualityOperator at i < n",
                "stale review: b.ts: X at y",
            ]
            .join("\n")
        ),
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
            Err(format!(
                "disabled mutants must be reviewed in {EQUIVALENT_REVIEWED}:\na.ts:{line}: write `// Stryker disable next-line <Mutator>: <reason>` above the line"
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
        run_steps(&check_steps(), &mut no_list, &mut Vec::new()),
        Err(
            "✘ TypeScript equivalent mutants reviewed: cannot list the TypeScript files: no git"
                .to_owned()
        ),
        "no listing"
    );
    let mut no_reviews = passing();
    let removed = no_reviews.files.remove(EQUIVALENT_REVIEWED);
    assert!(removed.is_some(), "reviews removed");
    assert_eq!(
        run_steps(&check_steps(), &mut no_reviews, &mut Vec::new()),
        Err(format!(
            "✘ TypeScript equivalent mutants reviewed: cannot read {EQUIVALENT_REVIEWED}: missing"
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
    ] {
        std::fs::write(root.join(file), text).expect("source file");
    }
    std::fs::write(root.join("untracked.ts"), "u").expect("untracked file");
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .is_ok_and(|output| output.status.success())
    };
    assert!(git(&["init", "--quiet"]), "git init");
    assert!(git(&["add", "src", "d.ts"]), "git add");
    let files = SystemRunner { root: root.clone() }.tracked(&["*.ts", "*.tsx"]);
    std::fs::remove_dir_all(&root).expect("cleanup");
    assert_eq!(
        files.expect("git lists the files"),
        vec![
            "d.ts".to_owned(),
            "src/a.ts".to_owned(),
            "src/b.tsx".to_owned()
        ],
        "tracked TypeScript only"
    );
    let outside = std::env::temp_dir().join(format!("xtask-no-git-{}", std::process::id()));
    std::fs::create_dir_all(&outside).expect("temporary directory");
    let error = SystemRunner {
        root: outside.clone(),
    }
    .tracked(&["*.ts", "*.tsx"]);
    std::fs::remove_dir_all(&outside).expect("cleanup");
    assert!(error.is_err(), "not a repository");
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
    assert_eq!(
        cli(&args, &mut passing(), &mut out, &mut err),
        0,
        "a valid message"
    );
    assert!(out.is_empty() && err.is_empty(), "silent on success");

    let mut bad = passing();
    let replaced = bad
        .files
        .insert("COMMIT_EDITMSG".to_owned(), "wip".to_owned());
    assert!(replaced.is_some(), "message replaced");
    let mut bad_err = Vec::new();
    assert_eq!(
        cli(&args, &mut bad, &mut Vec::new(), &mut bad_err),
        1,
        "an invalid message"
    );
    assert!(
        String::from_utf8(bad_err).is_ok_and(|e| e.starts_with("invalid commit message: ")),
        "explained on stderr"
    );

    let mut missing_err = Vec::new();
    let missing = ["commit-msg".to_owned(), "nowhere".to_owned()];
    assert_eq!(
        cli(&missing, &mut passing(), &mut Vec::new(), &mut missing_err),
        1,
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
        PACKAGE_JSONS[5],
        CHANGELOG,
    ] {
        let mut without = passing();
        let removed = without.files.remove(path);
        assert!(removed.is_some(), "{path} removed");
        // The release step alone: the membership gate before it also notices a removal.
        let release: Vec<Step> = check_steps()
            .into_iter()
            .filter(|step| step.action == Action::ReleaseGate)
            .collect();
        assert_eq!(
            run_steps(&release, &mut without, &mut Vec::new()),
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
    assert_eq!(
        cli(&["bless".to_owned()], &mut fake, &mut out, &mut Vec::new()),
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
    assert!(
        ts_coverage_gate("p", "nope")
            .is_err_and(|e| e.starts_with("p: invalid coverage summary: ")),
        "unreadable"
    );
}

#[test]
fn accepts_only_typescript_packages_without_surviving_mutants() {
    assert_eq!(ts_mutation_gate("p", TS_KILLED), Ok(()), "all killed");
    assert_eq!(
        ts_mutation_gate(
            "p",
            r#"{"files":{"a.ts":{"mutants":[{"status":"Survived"},{"status":"NoCoverage"},{"status":"NoCoverage"},{"status":"Killed"}]}}}"#
        ),
        Err("p: 1 mutant(s) survived, 2 without coverage".to_owned()),
        "survivors counted"
    );
    assert_eq!(
        ts_mutation_gate(
            "p",
            r#"{"files":{"a.ts":{"mutants":[{"status":"Survived"}]}}}"#
        ),
        Err("p: 1 mutant(s) survived, 0 without coverage".to_owned()),
        "a survivor alone"
    );
    assert_eq!(
        ts_mutation_gate(
            "p",
            r#"{"files":{"a.ts":{"mutants":[{"status":"NoCoverage"}]}}}"#
        ),
        Err("p: 0 mutant(s) survived, 1 without coverage".to_owned()),
        "an uncovered mutant alone"
    );
    for empty in [r#"{"files":{}}"#, r#"{"files":{"a.ts":{}}}"#, "{}"] {
        assert_eq!(
            ts_mutation_gate("p", empty),
            Err("p: no mutant in the mutation report".to_owned()),
            "{empty}"
        );
    }
    assert!(
        ts_mutation_gate("p", "[").is_err_and(|e| e.starts_with("p: invalid mutation report: ")),
        "unreadable"
    );
}

#[test]
fn fails_on_a_missing_or_failing_typescript_report() {
    let mut no_summary = passing();
    let removed = no_summary
        .files
        .remove("apps/playground/coverage/coverage-summary.json");
    assert!(removed.is_some(), "summary removed");
    assert_eq!(
        run_steps(&check_steps(), &mut no_summary, &mut Vec::new()),
        Err("✘ TypeScript coverage and mutation reports: cannot read apps/playground/coverage/coverage-summary.json: missing".to_owned()),
        "missing summary"
    );
    let mut survivor = passing();
    let replaced = survivor.files.insert(
        "apps/roadmap/reports/mutation/mutation.json".to_owned(),
        r#"{"files":{"a.ts":{"mutants":[{"status":"Survived"}]}}}"#.to_owned(),
    );
    assert!(replaced.is_some(), "report replaced");
    assert_eq!(
        run_steps(&check_steps(), &mut survivor, &mut Vec::new()),
        Err("✘ TypeScript coverage and mutation reports: apps/roadmap: 1 mutant(s) survived, 0 without coverage".to_owned()),
        "survivor"
    );
    let mut busy = passing();
    busy.broken = Some("fresh apps/playground/coverage".to_owned());
    assert_eq!(
        run_steps(&check_steps(), &mut busy, &mut Vec::new()),
        Err(
            "✘ Fresh TypeScript report directories: cannot empty apps/playground/coverage: busy"
                .to_owned()
        ),
        "a report directory that cannot be emptied"
    );
}

#[test]
fn knows_exactly_the_packages_and_crates_of_the_repository() {
    let packages: Vec<String> = PACKAGE_JSONS
        .iter()
        .map(|path| (*path).to_owned())
        .collect();
    let crates = vec![
        ("Cargo.toml".to_owned(), "[workspace]".to_owned()),
        (
            "crates/a/Cargo.toml".to_owned(),
            "[package]\nversion.workspace = true\n".to_owned(),
        ),
    ];
    assert_eq!(membership_gate(&packages, &crates), Ok(()), "known");

    let mut more = packages.clone();
    more.push("apps/editor/package.json".to_owned());
    let fewer: Vec<String> = packages
        .iter()
        .filter(|p| *p != "apps/roadmap/package.json")
        .cloned()
        .collect();
    let own = vec![(
        "crates/b/Cargo.toml".to_owned(),
        "[package]\nversion = \"9.0.0\"\n".to_owned(),
    )];
    assert_eq!(
        membership_gate(&more, &own),
        Err("apps/editor/package.json is not in PACKAGE_JSONS; apps/editor is not in TYPESCRIPT_PACKAGES: its checks and reports are not run; crates/b/Cargo.toml does not take the workspace version".to_owned()),
        "unknown package and crate with its own version"
    );
    assert_eq!(
        membership_gate(&fewer, &[]),
        Err("apps/roadmap/package.json is in PACKAGE_JSONS but not tracked; apps/roadmap is in TYPESCRIPT_PACKAGES but has no tracked package.json".to_owned()),
        "a package that is gone"
    );
}

#[test]
fn reports_manifests_it_cannot_list() {
    let mut broken = passing();
    broken.broken = Some("list *Cargo.toml".to_owned());
    assert_eq!(
        run_steps(&check_steps(), &mut broken, &mut Vec::new()),
        Err(
            "✘ Packages and crates known to the checks: cannot list the *Cargo.toml files: no git"
                .to_owned()
        ),
        "named"
    );
    let mut unlisted = passing();
    unlisted.broken = Some("list *package.json".to_owned());
    assert!(
        run_steps(&check_steps(), &mut unlisted, &mut Vec::new())
            .is_err_and(|e| e.ends_with("cannot list the *package.json files: no git")),
        "named"
    );
}
