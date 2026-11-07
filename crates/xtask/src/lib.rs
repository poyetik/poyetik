//! Project automation for poyetik.
//!
//! `cargo xtask check` runs every quality check of the repository, in order, and stops
//! at the first failure. CI and the pre-push hook run exactly this command, so a green
//! local check means a green CI.

#![forbid(unsafe_code)]

extern crate alloc;

use alloc::collections::BTreeSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Pinned nightly toolchain, used only to measure branch coverage and to run Miri.
pub const NIGHTLY_TOOLCHAIN: &str = "+nightly-2026-11-01";

/// Where the coverage report is written.
pub const COVERAGE_REPORT: &str = "target/coverage.json";

/// Mutants that cargo-mutants could not build, as reported by the last run.
pub const UNVIABLE_REPORT: &str = "mutants.out/unviable.txt";

/// Reviewed unviable mutants, each with the reason it is legitimately unbuildable.
pub const UNVIABLE_REVIEWED: &str = "mutants-unviable.txt";

/// Targets the engine core must build for: proof that it depends on no environment.
pub const PORTABILITY_TARGETS: [&str; 4] = [
    "aarch64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "wasm32-unknown-unknown",
    "thumbv7em-none-eabihf",
];

/// Directory a command runs in, relative to the repository root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    /// The repository root.
    Root,
    /// The `apps/roadmap/` package.
    Roadmap,
    /// The `bindings/wasm/` package.
    EngineWasm,
}

impl Dir {
    /// The directory, below `root`.
    #[must_use]
    pub fn under(self, root: &Path) -> PathBuf {
        match self {
            Self::Root => root.to_path_buf(),
            Self::Roadmap => root.join("apps").join("roadmap"),
            Self::EngineWasm => root.join("bindings").join("wasm"),
        }
    }
}

/// An external command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cmd {
    /// Program to run.
    pub program: &'static str,
    /// Its arguments.
    pub args: Vec<String>,
    /// Where it runs.
    pub dir: Dir,
}

/// What a step does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Run an external command; the step fails if it exits with an error.
    Run(Cmd),
    /// Require 100% line, function and branch coverage in the report at this path.
    CoverageGate(&'static str),
    /// Require every unviable mutant to be reviewed, and every review to be current.
    UnviableGate,
    /// Require one project version everywhere, consistent with a well-formed changelog.
    ReleaseGate,
}

/// One named check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// What is checked, shown while running.
    pub name: String,
    /// How it is checked.
    pub action: Action,
}

/// Builds a command step.
fn run(name: &str, program: &'static str, args: &[&str], dir: Dir) -> Step {
    Step {
        name: name.to_owned(),
        action: Action::Run(Cmd {
            program,
            args: args.iter().map(|&arg| arg.to_owned()).collect(),
            dir,
        }),
    }
}

/// Formatting, lints, documentation and tests.
fn static_steps() -> Vec<Step> {
    vec![
        run(
            "Rust formatting",
            "cargo",
            &["fmt", "--all", "--check"],
            Dir::Root,
        ),
        run(
            "Rust lints",
            "cargo",
            &["clippy", "--workspace", "--all-targets", "--locked"],
            Dir::Root,
        ),
        run(
            "Rust documentation",
            "cargo",
            &[
                "doc",
                "--workspace",
                "--no-deps",
                "--document-private-items",
                "--locked",
            ],
            Dir::Root,
        ),
        run(
            "Rust tests",
            "cargo",
            &["test", "--workspace", "--locked"],
            Dir::Root,
        ),
    ]
}

/// The engine crates build for every portability target.
fn portability_steps() -> Vec<Step> {
    PORTABILITY_TARGETS
        .iter()
        .map(|&target| {
            run(
                &format!("engine portability ({target})"),
                "cargo",
                &[
                    "build",
                    "--package",
                    "engine-core",
                    "--package",
                    "engine-objects",
                    "--package",
                    "engine-patch",
                    "--package",
                    "engine-hash",
                    "--locked",
                    "--target",
                    target,
                ],
                Dir::Root,
            )
        })
        .collect()
}

/// Coverage and mutation testing, with their gates.
fn proof_steps() -> Vec<Step> {
    vec![
        run(
            "Rust coverage report",
            "cargo",
            &[
                NIGHTLY_TOOLCHAIN,
                "llvm-cov",
                "--workspace",
                "--locked",
                "--branch",
                "--json",
                "--summary-only",
                "--output-path",
                COVERAGE_REPORT,
            ],
            Dir::Root,
        ),
        Step {
            name: "Rust coverage: 100% lines, functions and branches".to_owned(),
            action: Action::CoverageGate(COVERAGE_REPORT),
        },
        // Lints are capped while building mutants: otherwise a mutant that merely leaves a
        // variable unused fails to compile and is silently skipped instead of tested.
        run(
            "Rust mutation testing",
            "cargo",
            &["mutants", "--workspace", "--cap-lints=true"],
            Dir::Root,
        ),
        Step {
            name: "Rust unviable mutants reviewed".to_owned(),
            action: Action::UnviableGate,
        },
        // The only `unsafe` code of the engine: Miri checks its tests for undefined behavior.
        run(
            "Rust FFI under Miri",
            "cargo",
            &[
                NIGHTLY_TOOLCHAIN,
                "miri",
                "test",
                "--package",
                "engine-wasm",
                "--locked",
            ],
            Dir::Root,
        ),
    ]
}

/// Conformance, supply chain, versions and the TypeScript packages (the engine in the
/// browser, then the roadmap).
fn release_steps() -> Vec<Step> {
    vec![
        run(
            "Conformance (native)",
            "cargo",
            &[
                "run",
                "--package",
                "engine-cli",
                "--locked",
                "--quiet",
                "--",
                "check",
                "conformance",
            ],
            Dir::Root,
        ),
        run(
            "Rust supply chain",
            "cargo",
            &["deny", "--locked", "check"],
            Dir::Root,
        ),
        Step {
            name: "Versions and changelog".to_owned(),
            action: Action::ReleaseGate,
        },
        run(
            "JavaScript dependencies",
            "pnpm",
            &["install", "--frozen-lockfile"],
            Dir::Root,
        ),
        run(
            "Engine WASM checks",
            "pnpm",
            &["run", "check"],
            Dir::EngineWasm,
        ),
        run("Roadmap checks", "pnpm", &["run", "check"], Dir::Roadmap),
    ]
}

/// Every check of the repository, in the order they run.
#[must_use]
pub fn check_steps() -> Vec<Step> {
    [
        static_steps(),
        portability_steps(),
        proof_steps(),
        release_steps(),
    ]
    .concat()
}

/// Reads one `{ count, covered }` total of an llvm-cov JSON summary.
fn total(totals: Option<&serde_json::Value>, kind: &str) -> Result<(u64, u64), String> {
    let field = |name: &str| {
        totals
            .and_then(|t| t.get(kind))
            .and_then(|entry| entry.get(name))
            .and_then(serde_json::Value::as_u64)
    };
    match (field("count"), field("covered")) {
        (Some(count), Some(covered)) => Ok((count, covered)),
        _ => Err(format!("coverage report has no {kind} total")),
    }
}

/// Checks that an llvm-cov JSON summary reports 100% line, function and branch coverage.
///
/// # Errors
///
/// Returns a message naming every kind that is not fully covered, or why the report
/// cannot be read.
pub fn coverage_gate(report: &str) -> Result<(), String> {
    let json: serde_json::Value = serde_json::from_str(report)
        .map_err(|error| format!("invalid coverage report: {error}"))?;
    let totals = json
        .get("data")
        .and_then(|data| data.get(0))
        .and_then(|run| run.get("totals"));
    let mut missing = Vec::new();
    for kind in ["lines", "functions", "branches"] {
        let (count, covered) = total(totals, kind)?;
        if covered != count {
            missing.push(format!("{kind} {covered}/{count}"));
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("coverage is not 100%: {}", missing.join(", ")))
    }
}

/// Identifies a mutant independently of line and column numbers, so that unrelated
/// edits do not invalidate a review: `crates/a/src/b.rs: replace f -> T with x`.
fn mutant_key(line: &str) -> Option<String> {
    let (file, rest) = line.split_once(".rs:")?;
    let mut parts = rest.splitn(3, ':');
    let (Some(_line), Some(_column), Some(description)) =
        (parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    Some(format!("{file}.rs: {}", description.trim()))
}

/// Checks that the unviable mutants of the last run are exactly the reviewed ones.
///
/// `reviewed` holds one `<file>.rs: <mutant> -- <reason>` line per mutant; blank lines
/// and lines starting with `#` are ignored.
///
/// # Errors
///
/// Returns a message listing unreviewed mutants, stale reviews and malformed lines.
pub fn unviable_gate(report: &str, reviewed: &str) -> Result<(), String> {
    let mut problems = Vec::new();
    let mut found = BTreeSet::new();
    for line in report.lines().filter(|l| !l.trim().is_empty()) {
        match mutant_key(line) {
            Some(key) => {
                let _: bool = found.insert(key);
            }
            None => problems.push(format!("unrecognized mutant: {line}")),
        }
    }
    let mut known = BTreeSet::new();
    for line in reviewed
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        match line.split_once(" -- ") {
            // The line is trimmed, so text always follows the separator: the reason is never empty.
            Some((key, _reason)) => {
                let _: bool = known.insert(key.trim().to_owned());
            }
            None => problems.push(format!("review without a reason: {line}")),
        }
    }
    problems.extend(
        found
            .difference(&known)
            .map(|key| format!("not reviewed: {key}")),
    );
    problems.extend(
        known
            .difference(&found)
            .map(|key| format!("stale review: {key}")),
    );
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "unviable mutants must be reviewed in {UNVIABLE_REVIEWED}:\n{}",
            problems.join("\n")
        ))
    }
}

/// The project manifest holding the workspace version.
pub const CARGO_TOML: &str = "Cargo.toml";

/// Every `package.json` of the pnpm workspace: all carry the project version.
pub const PACKAGE_JSONS: [&str; 3] = [
    "package.json",
    "apps/roadmap/package.json",
    "bindings/wasm/package.json",
];

/// The changelog.
pub const CHANGELOG: &str = "CHANGELOG.md";

/// A semantic version `MAJOR.MINOR.PATCH` (no pre-release or build metadata).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    /// Incremented for incompatible changes.
    pub major: u64,
    /// Incremented for compatible additions.
    pub minor: u64,
    /// Incremented for compatible fixes.
    pub patch: u64,
}

impl Version {
    /// Parses `MAJOR.MINOR.PATCH`: digits only, no leading zero, as semantic versioning requires.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let number = |part: &str| {
            let digits = !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
            let leading_zero = part.len() > 1 && part.starts_with('0');
            if digits && !leading_zero {
                part.parse::<u64>().ok()
            } else {
                None
            }
        };
        let mut parts = text.split('.');
        let (Some(major), Some(minor), Some(patch), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return None;
        };
        Some(Self {
            major: number(major)?,
            minor: number(minor)?,
            patch: number(patch)?,
        })
    }
}

impl core::fmt::Display for Version {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Reads `version = "…"` in the `[workspace.package]` table of a `Cargo.toml`.
fn workspace_version(cargo_toml: &str) -> Option<&str> {
    cargo_toml
        .lines()
        .skip_while(|line| line.trim() != "[workspace.package]")
        .skip(1)
        .take_while(|line| !line.trim_start().starts_with('['))
        .find_map(|line| {
            let (key, value) = line.split_once('=')?;
            (key.trim() == "version").then(|| value.trim().trim_matches('"'))
        })
}

/// Parses a released changelog heading: `## [X.Y.Z] - YYYY-MM-DD`.
fn released_heading(heading: &str) -> Option<Version> {
    let rest = heading.strip_prefix("## [")?;
    let (version, date) = rest.split_once("] - ")?;
    let mut parts = date.split('-');
    let well_formed = [4, 2, 2].iter().all(|&len| {
        parts
            .next()
            .is_some_and(|p| p.len() == len && p.bytes().all(|b| b.is_ascii_digit()))
    }) && parts.next().is_none();
    if well_formed {
        Version::parse(version)
    } else {
        None
    }
}

/// Checks that the project has one version everywhere and a well-formed changelog.
///
/// The workspace version and the version of every package manifest must be equal, valid semantic
/// versions, and not older than the latest release in the changelog. The changelog starts with
/// `# Changelog`, has `## [Unreleased]` first, then releases `## [X.Y.Z] - YYYY-MM-DD`
/// in strictly decreasing order.
///
/// # Errors
///
/// Returns a message listing every problem found.
pub fn release_gate(
    cargo_toml: &str,
    packages: &[(&str, String)],
    changelog: &str,
) -> Result<(), String> {
    let mut problems = Vec::new();
    let workspace = workspace_version(cargo_toml);
    let version = workspace.and_then(Version::parse);
    if version.is_none() {
        problems.push(format!(
            "{CARGO_TOML}: [workspace.package] version must be X.Y.Z"
        ));
    }
    for (path, manifest) in packages {
        let package = serde_json::from_str::<serde_json::Value>(manifest)
            .ok()
            .and_then(|json| {
                json.get("version")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            });
        if package.as_deref() != workspace {
            problems.push(format!(
                "{path}: version {} differs from the workspace version {}",
                package.as_deref().unwrap_or("(missing)"),
                workspace.unwrap_or("(missing)")
            ));
        }
    }
    if changelog.lines().next() != Some("# Changelog") {
        problems.push(format!("{CHANGELOG}: must start with \"# Changelog\""));
    }
    let headings: Vec<&str> = changelog.lines().filter(|l| l.starts_with("## ")).collect();
    if headings.first() != Some(&"## [Unreleased]") {
        problems.push(format!(
            "{CHANGELOG}: the first section must be \"## [Unreleased]\""
        ));
    }
    let mut releases = Vec::new();
    for heading in headings.iter().skip(1) {
        match released_heading(heading) {
            Some(release) => releases.push(release),
            None => problems.push(format!("{CHANGELOG}: malformed heading \"{heading}\"")),
        }
    }
    if releases.windows(2).any(|pair| pair.first() <= pair.get(1)) {
        problems.push(format!(
            "{CHANGELOG}: releases must be in strictly decreasing order"
        ));
    }
    if let (Some(current), Some(latest)) = (version, releases.first())
        && current < *latest
    {
        problems.push(format!(
            "version {current} is older than the latest release {latest}"
        ));
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("\n"))
    }
}

/// Commit types allowed by the Conventional Commits rule of the project.
pub const COMMIT_TYPES: [&str; 11] = [
    "feat", "fix", "docs", "test", "refactor", "perf", "build", "ci", "chore", "style", "revert",
];

/// Longest allowed commit header, in characters.
pub const COMMIT_HEADER_MAX: usize = 72;

/// Checks a commit message against Conventional Commits.
///
/// Expected header: `type(scope)!: summary`, scope and `!` optional, at most
/// [`COMMIT_HEADER_MAX`] characters, no trailing period, blank line before any body.
/// Lines starting with `#` are git comments and are ignored. Messages generated by git
/// for merges and reverts are accepted as they are.
///
/// # Errors
///
/// Returns a message explaining the expected format.
pub fn commit_msg_gate(message: &str) -> Result<(), String> {
    let mut lines = message.lines().filter(|line| !line.starts_with('#'));
    let header = lines.next().unwrap_or("");
    if header.starts_with("Merge ") || header.starts_with("Revert \"") {
        return Ok(());
    }
    let fail = |reason: &str| {
        Err(format!(
            "invalid commit message: {reason}\nexpected: <type>(<scope>)!: <summary>, type one of {}",
            COMMIT_TYPES.join(", ")
        ))
    };
    let Some((prefix, summary)) = header.split_once(": ") else {
        return fail("missing \"<type>: \" prefix");
    };
    let prefix = prefix.strip_suffix('!').unwrap_or(prefix);
    let (kind, scope) = match prefix.split_once('(') {
        Some((kind, rest)) => (kind, rest.strip_suffix(')')),
        None => (prefix, Some("x")),
    };
    if !COMMIT_TYPES.contains(&kind) {
        return fail(&format!("unknown type \"{kind}\""));
    }
    let scope_ok = scope.is_some_and(|s| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    });
    if !scope_ok {
        return fail("the scope must be lowercase letters, digits or hyphens");
    }
    if summary.trim().is_empty() || summary.ends_with('.') {
        return fail("the summary must be non-empty and must not end with a period");
    }
    if header.chars().count() > COMMIT_HEADER_MAX {
        return fail(&format!(
            "the header is longer than {COMMIT_HEADER_MAX} characters"
        ));
    }
    if lines.next().is_some_and(|line| !line.trim().is_empty()) {
        return fail("a blank line must separate the header from the body");
    }
    Ok(())
}

/// Executes commands and reads files on behalf of [`run_steps`].
pub trait Runner {
    /// Runs a command and tells whether it succeeded.
    ///
    /// # Errors
    ///
    /// Returns an error when the command cannot be started.
    fn run(&mut self, cmd: &Cmd) -> io::Result<bool>;

    /// Reads a text file below the repository root.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read.
    fn read(&mut self, path: &str) -> io::Result<String>;
}

/// Runs real commands below a repository root.
#[derive(Debug, Clone)]
pub struct SystemRunner {
    /// The repository root.
    pub root: PathBuf,
}

impl Runner for SystemRunner {
    fn run(&mut self, cmd: &Cmd) -> io::Result<bool> {
        Command::new(cmd.program)
            .args(&cmd.args)
            .current_dir(cmd.dir.under(&self.root))
            .status()
            .map(|status| status.success())
    }

    fn read(&mut self, path: &str) -> io::Result<String> {
        std::fs::read_to_string(self.root.join(path))
    }
}

/// Reads a file through `runner`, naming it when it cannot be read.
fn read_file(runner: &mut dyn Runner, path: &str) -> Result<String, String> {
    runner
        .read(path)
        .map_err(|error| format!("cannot read {path}: {error}"))
}

/// Runs `steps` in order and stops at the first failure.
///
/// # Errors
///
/// Returns a message naming the failed step and why.
pub fn run_steps(
    steps: &[Step],
    runner: &mut dyn Runner,
    out: &mut dyn Write,
) -> Result<(), String> {
    let total_steps = steps.len();
    for (index, step) in steps.iter().enumerate() {
        let number = index.saturating_add(1);
        writeln!(out, "▶ [{number}/{total_steps}] {}", step.name)
            .map_err(|error| error.to_string())?;
        let outcome = match &step.action {
            Action::Run(cmd) => match runner.run(cmd) {
                Ok(true) => Ok(()),
                Ok(false) => Err("command failed".to_owned()),
                Err(error) => Err(format!("cannot run {}: {error}", cmd.program)),
            },
            Action::CoverageGate(path) => {
                read_file(runner, path).and_then(|report| coverage_gate(&report))
            }
            Action::ReleaseGate => read_file(runner, CARGO_TOML).and_then(|cargo| {
                PACKAGE_JSONS
                    .iter()
                    .map(|&path| read_file(runner, path).map(|text| (path, text)))
                    .collect::<Result<Vec<_>, _>>()
                    .and_then(|packages| {
                        read_file(runner, CHANGELOG)
                            .and_then(|changelog| release_gate(&cargo, &packages, &changelog))
                    })
            }),
            Action::UnviableGate => read_file(runner, UNVIABLE_REPORT).and_then(|report| {
                read_file(runner, UNVIABLE_REVIEWED)
                    .and_then(|reviewed| unviable_gate(&report, &reviewed))
            }),
        };
        outcome.map_err(|reason| format!("✘ {}: {reason}", step.name))?;
    }
    writeln!(out, "✔ all {total_steps} checks passed").map_err(|error| error.to_string())
}

/// Usage of the `xtask` command.
pub const USAGE: &str = "usage: cargo xtask <check|bless|commit-msg <file>|help>\n  check              run every quality check, as CI and the pre-push hook do\n  bless              rewrite conformance/references.json from the current renders\n  commit-msg <file>  check a commit message (used by the commit-msg hook)\n  help               show this help";

/// The only way to change the reference hashes: renders every conformance case and
/// rewrites `conformance/references.json`. The commit must explain why they changed.
#[must_use]
pub fn bless_step() -> Step {
    run(
        "Bless conformance references",
        "cargo",
        &[
            "run",
            "--package",
            "engine-cli",
            "--locked",
            "--quiet",
            "--",
            "bless",
            "conformance",
        ],
        Dir::Root,
    )
}

/// Entry point: runs the command named by `args` and returns whether it succeeded.
pub fn cli(
    args: &[String],
    runner: &mut dyn Runner,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> bool {
    let result = match args {
        [command] if command == "check" => run_steps(&check_steps(), runner, out),
        [command] if command == "bless" => run_steps(&[bless_step()], runner, out),
        [command, path] if command == "commit-msg" => {
            read_file(runner, path).and_then(|message| commit_msg_gate(&message))
        }
        [command] if command == "help" => {
            writeln!(out, "{USAGE}").map_err(|error| error.to_string())
        }
        _ => Err(USAGE.to_owned()),
    };
    match result {
        Ok(()) => true,
        Err(message) => {
            // Nothing more can be done if stderr itself is broken: the exit code still fails.
            drop(writeln!(err, "{message}"));
            false
        }
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
