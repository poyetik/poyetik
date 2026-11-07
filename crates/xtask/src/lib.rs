//! Project automation for poyetik.
//!
//! `cargo xtask check` runs every quality check of the repository, in order, and stops
//! at the first failure. CI and the pre-push hook run exactly this command, so a green
//! local check means a green CI.

#![forbid(unsafe_code)]

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Pinned nightly toolchain, used only to measure branch coverage.
pub const COVERAGE_TOOLCHAIN: &str = "+nightly-2026-11-01";

/// Where the coverage report is written.
pub const COVERAGE_REPORT: &str = "target/coverage.json";

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
    /// The `roadmap/` package.
    Roadmap,
}

impl Dir {
    /// The directory, below `root`.
    #[must_use]
    pub fn under(self, root: &Path) -> PathBuf {
        match self {
            Self::Root => root.to_path_buf(),
            Self::Roadmap => root.join("roadmap"),
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

/// Every check of the repository, in the order they run.
#[must_use]
pub fn check_steps() -> Vec<Step> {
    let mut steps = vec![
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
            "Rust tests",
            "cargo",
            &["test", "--workspace", "--locked"],
            Dir::Root,
        ),
    ];
    steps.extend(PORTABILITY_TARGETS.iter().map(|&target| {
        run(
            &format!("engine-core portability ({target})"),
            "cargo",
            &[
                "build",
                "--package",
                "engine-core",
                "--locked",
                "--target",
                target,
            ],
            Dir::Root,
        )
    }));
    steps.extend([
        run(
            "Rust coverage report",
            "cargo",
            &[
                COVERAGE_TOOLCHAIN,
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
        run(
            "Rust supply chain",
            "cargo",
            &["deny", "--locked", "check"],
            Dir::Root,
        ),
        run(
            "Roadmap dependencies",
            "pnpm",
            &["install", "--frozen-lockfile"],
            Dir::Roadmap,
        ),
        run("Roadmap checks", "pnpm", &["run", "check"], Dir::Roadmap),
    ]);
    steps
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
            Action::CoverageGate(path) => runner
                .read(path)
                .map_err(|error| format!("cannot read {path}: {error}"))
                .and_then(|report| coverage_gate(&report)),
        };
        outcome.map_err(|reason| format!("✘ {}: {reason}", step.name))?;
    }
    writeln!(out, "✔ all {total_steps} checks passed").map_err(|error| error.to_string())
}

/// Usage of the `xtask` command.
pub const USAGE: &str = "usage: cargo xtask <check|help>\n  check  run every quality check, as CI and the pre-push hook do\n  help   show this help";

/// Entry point: runs the command named by `args` and returns whether it succeeded.
pub fn cli(
    args: &[String],
    runner: &mut dyn Runner,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> bool {
    let result = match args {
        [command] if command == "check" => run_steps(&check_steps(), runner, out),
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
