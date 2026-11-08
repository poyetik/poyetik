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

/// Where the native engine exports the WAV file of every conformance case, emptied
/// before each export so that no file from an earlier run is judged.
pub const CONFORMANCE_EXPORT: &str = "target/conformance";

/// Mutants that cargo-mutants could not build, as reported by the last run.
pub const UNVIABLE_REPORT: &str = "mutants.out/unviable.txt";

/// Reviewed unviable mutants, each with the reason it is legitimately unbuildable.
pub const UNVIABLE_REVIEWED: &str = "mutants-unviable.txt";

/// Reviewed equivalent mutants of the TypeScript code: every `// Stryker disable
/// next-line` comment, each with the reason no test can tell the mutant apart.
pub const EQUIVALENT_REVIEWED: &str = "mutants-equivalent.txt";

/// The TypeScript packages, by directory: each writes its coverage summary and its
/// mutation report, both checked by xtask rather than trusted to their own thresholds.
pub const TYPESCRIPT_PACKAGES: [&str; 4] = [
    "tooling/oracle",
    "bindings/wasm",
    "apps/playground",
    "apps/roadmap",
];

/// The report directories of the TypeScript packages, emptied before they run so that
/// every report read by the gate comes from this run.
pub const TYPESCRIPT_REPORTS: [&str; 8] = [
    "tooling/oracle/coverage",
    "tooling/oracle/reports/mutation",
    "bindings/wasm/coverage",
    "bindings/wasm/reports/mutation",
    "apps/playground/coverage",
    "apps/playground/reports/mutation",
    "apps/roadmap/coverage",
    "apps/roadmap/reports/mutation",
];

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
    /// The `apps/playground/` package.
    Playground,
    /// The `tooling/oracle/` package.
    Oracle,
}

impl Dir {
    /// The directory, below `root`.
    #[must_use]
    pub fn under(self, root: &Path) -> PathBuf {
        match self {
            Self::Root => root.to_path_buf(),
            Self::Roadmap => root.join("apps").join("roadmap"),
            Self::EngineWasm => root.join("bindings").join("wasm"),
            Self::Playground => root.join("apps").join("playground"),
            Self::Oracle => root.join("tooling").join("oracle"),
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

impl Cmd {
    /// The command line, as a shell would show it (in its directory).
    #[must_use]
    pub fn line(&self) -> String {
        let mut line = String::from(self.program);
        for arg in &self.args {
            line.push(' ');
            line.push_str(arg);
        }
        line
    }
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
    /// Require every disabled TypeScript mutant to be reviewed, and every review to be
    /// current.
    EquivalentGate,
    /// Require one project version everywhere, consistent with a well-formed changelog.
    ReleaseGate,
    /// Empty this directory (relative to the root), creating it if needed: what a later
    /// step reads there is then produced by this run only.
    FreshDir(&'static str),
    /// Empty each of these directories, as [`Action::FreshDir`].
    FreshDirs(&'static [&'static str]),
    /// Require 100% coverage and no surviving mutant in the reports of every TypeScript
    /// package.
    TypeScriptReportsGate,
    /// Require the packages and crates known to xtask to be exactly those of the
    /// repository, every crate on the workspace version.
    MembershipGate,
    /// Require every JavaScript dependency under an allowed license: the strict list for
    /// what the apps ship, the wider one for development tooling.
    LicenseGate,
    /// Require the lint against unsafe code to be lifted only in the audited files.
    UnsafeGate,
    /// Require every tool the checks use, at the versions they need, on a supported
    /// platform: checked first, so that a missing tool is named before anything runs.
    Preflight,
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
        Step {
            name: "Tools and platform".to_owned(),
            action: Action::Preflight,
        },
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
            &[
                "mutants",
                "--workspace",
                "--cap-lints=true",
                "--gitignore=true",
            ],
            Dir::Root,
        ),
        Step {
            name: "Rust unviable mutants reviewed".to_owned(),
            action: Action::UnviableGate,
        },
        Step {
            name: "TypeScript equivalent mutants reviewed".to_owned(),
            action: Action::EquivalentGate,
        },
        Step {
            name: "Unsafe code confined to the audited files".to_owned(),
            action: Action::UnsafeGate,
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
                "--package",
                "engine-alloc-counter",
                "--locked",
            ],
            Dir::Root,
        ),
    ]
}

/// Conformance, supply chain, versions and the TypeScript packages (the engine in the
/// browser, the playground built on it, then the roadmap).
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
        // The independent oracle (tooling/oracle) checks these files after the JavaScript
        // dependencies are installed.
        Step {
            name: "Fresh conformance export directory".to_owned(),
            action: Action::FreshDir(CONFORMANCE_EXPORT),
        },
        run(
            "Conformance export (native WAV files)",
            "cargo",
            &[
                "run",
                "--package",
                "engine-cli",
                "--locked",
                "--quiet",
                "--",
                "export",
                "conformance",
                CONFORMANCE_EXPORT,
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
            name: "Packages and crates known to the checks".to_owned(),
            action: Action::MembershipGate,
        },
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
        run("JavaScript vulnerabilities", "pnpm", &["audit"], Dir::Root),
        Step {
            name: "JavaScript licenses".to_owned(),
            action: Action::LicenseGate,
        },
        Step {
            name: "Fresh TypeScript report directories".to_owned(),
            action: Action::FreshDirs(&TYPESCRIPT_REPORTS),
        },
        run(
            "Conformance oracle (independent, TypeScript)",
            "pnpm",
            &["run", "check"],
            Dir::Oracle,
        ),
        run(
            "Engine WASM checks",
            "pnpm",
            &["run", "check"],
            Dir::EngineWasm,
        ),
        run(
            "Playground checks",
            "pnpm",
            &["run", "check"],
            Dir::Playground,
        ),
        run("Roadmap checks", "pnpm", &["run", "check"], Dir::Roadmap),
        Step {
            name: "TypeScript coverage and mutation reports".to_owned(),
            action: Action::TypeScriptReportsGate,
        },
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
        if count == 0 {
            missing.push(format!("{kind} not measured"));
        } else if covered != count {
            missing.push(format!("{kind} {covered}/{count}"));
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("coverage is not 100%: {}", missing.join(", ")))
    }
}

/// Checks the coverage summary of a TypeScript package (Vitest `json-summary`): 100% of
/// lines, statements, functions and branches, each measured.
///
/// # Errors
///
/// Returns a message naming the package and every kind not fully covered.
pub fn ts_coverage_gate(package: &str, summary: &str) -> Result<(), String> {
    let json: serde_json::Value = serde_json::from_str(summary)
        .map_err(|error| format!("{package}: invalid coverage summary: {error}"))?;
    let mut missing = Vec::new();
    for kind in ["lines", "statements", "functions", "branches"] {
        let field = |name: &str| {
            json.get("total")
                .and_then(|total| total.get(kind))
                .and_then(|entry| entry.get(name))
                .and_then(serde_json::Value::as_u64)
        };
        match (field("total"), field("covered")) {
            (Some(0), _) => missing.push(format!("{kind} not measured")),
            (Some(count), Some(covered)) if covered == count => {}
            (Some(count), Some(covered)) => missing.push(format!("{kind} {covered}/{count}")),
            _ => missing.push(format!("no {kind} total")),
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{package}: coverage is not 100%: {}",
            missing.join(", ")
        ))
    }
}

/// Checks the mutation report of a TypeScript package (Stryker `json`): at least one
/// mutant, none survived or left without coverage.
///
/// # Errors
///
/// Returns a message naming the package and the mutants that were not killed.
pub fn ts_mutation_gate(package: &str, report: &str) -> Result<(), String> {
    let json: serde_json::Value = serde_json::from_str(report)
        .map_err(|error| format!("{package}: invalid mutation report: {error}"))?;
    let statuses: Vec<&str> = json
        .get("files")
        .and_then(serde_json::Value::as_object)
        .into_iter()
        .flat_map(|files| files.values())
        .filter_map(|file| file.get("mutants").and_then(serde_json::Value::as_array))
        .flatten()
        .filter_map(|mutant| mutant.get("status").and_then(serde_json::Value::as_str))
        .collect();
    if statuses.is_empty() {
        return Err(format!("{package}: no mutant in the mutation report"));
    }
    let left = |status: &str| statuses.iter().filter(|s| **s == status).count();
    let (survived, uncovered) = (left("Survived"), left("NoCoverage"));
    if survived == 0 && uncovered == 0 {
        Ok(())
    } else {
        Err(format!(
            "{package}: {survived} mutant(s) survived, {uncovered} without coverage"
        ))
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

/// The comment that disables one mutator on the next line, in TypeScript sources.
const STRYKER_DISABLE: &str = "// Stryker disable next-line ";

/// Checks that the mutants disabled in `sources` (path and text of each TypeScript file)
/// are exactly the reviewed ones.
///
/// A disable comment must be `// Stryker disable next-line <Mutator>: <reason>`, one
/// mutator, never `all`, with a reason; any other use of `Stryker disable` is refused.
/// `reviewed` holds one `<file>: <Mutator> at <next line> -- <reason>` line per comment;
/// blank lines and lines starting with `#` are ignored.
///
/// # Errors
///
/// Returns a message listing malformed comments, unreviewed and stale entries.
pub fn equivalent_gate(sources: &[(String, String)], reviewed: &str) -> Result<(), String> {
    let mut problems = Vec::new();
    let mut found = BTreeSet::new();
    for (path, text) in sources {
        let lines: Vec<&str> = text.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            if !line.contains("Stryker disable") {
                continue;
            }
            let comment = line.trim();
            let rule = comment
                .strip_prefix(STRYKER_DISABLE)
                .and_then(|rest| rest.split_once(':'))
                .filter(|(mutator, reason)| {
                    !mutator.contains([' ', ',']) && *mutator != "all" && !reason.trim().is_empty()
                });
            let next = lines.get(index.saturating_add(1)).map_or("", |l| l.trim());
            match rule {
                Some((mutator, _reason)) if !next.is_empty() => {
                    let _: bool = found.insert(format!("{path}: {mutator} at {next}"));
                }
                _ => problems.push(format!(
                    "{path}:{}: write `{STRYKER_DISABLE}<Mutator>: <reason>` above the line",
                    index.saturating_add(1)
                )),
            }
        }
    }
    let mut known = BTreeSet::new();
    for line in reviewed
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        match line.split_once(" -- ") {
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
            "disabled mutants must be reviewed in {EQUIVALENT_REVIEWED}:\n{}",
            problems.join("\n")
        ))
    }
}

/// The package that holds shared configuration only: a workspace member without checks or
/// reports of its own.
pub const CONFIGURATION_PACKAGE: &str = "tooling/config/package.json";

/// Checks that the manifests tracked by git are exactly those xtask knows.
///
/// Every `package.json` is in [`PACKAGE_JSONS`], every TypeScript package but the root
/// and the configuration one is in [`TYPESCRIPT_PACKAGES`], and every crate manifest
/// (`Cargo.toml` below the root) takes the workspace version. The only `.gitignore` is the
/// root one: build outputs go to `dist/` or `target/`, never hidden inside a source folder.
///
/// # Errors
///
/// Returns a message listing every manifest unknown to xtask or missing from the
/// repository, and every crate with a version of its own.
pub fn membership_gate(
    packages: &[String],
    crates: &[(String, String)],
    ignores: &[String],
) -> Result<(), String> {
    let tracked: BTreeSet<&str> = packages.iter().map(String::as_str).collect();
    let known: BTreeSet<&str> = PACKAGE_JSONS.into_iter().collect();
    let mut problems: Vec<String> = tracked
        .difference(&known)
        .map(|path| format!("{path} is not in PACKAGE_JSONS"))
        .chain(
            known
                .difference(&tracked)
                .map(|path| format!("{path} is in PACKAGE_JSONS but not tracked")),
        )
        .collect();
    let checked: BTreeSet<String> = tracked
        .iter()
        .filter(|path| **path != "package.json" && **path != CONFIGURATION_PACKAGE)
        .filter_map(|path| path.strip_suffix("/package.json"))
        .map(str::to_owned)
        .collect();
    let gated: BTreeSet<String> = TYPESCRIPT_PACKAGES.into_iter().map(str::to_owned).collect();
    problems.extend(checked.difference(&gated).map(|dir| {
        format!("{dir} is not in TYPESCRIPT_PACKAGES: its checks and reports are not run")
    }));
    problems.extend(
        gated
            .difference(&checked)
            .map(|dir| format!("{dir} is in TYPESCRIPT_PACKAGES but has no tracked package.json")),
    );
    problems.extend(
        crates
            .iter()
            .filter(|(path, _)| path != CARGO_TOML)
            .filter(|(_, text)| {
                !text
                    .lines()
                    .any(|line| line.trim() == "version.workspace = true")
            })
            .map(|(path, _)| format!("{path} does not take the workspace version")),
    );
    problems.extend(
        ignores
            .iter()
            .filter(|path| path.as_str() != ".gitignore")
            .map(|path| {
                format!(
                    "{path}: build outputs go to dist/ or target/, not hidden in a source folder"
                )
            }),
    );
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

/// Licenses allowed for what the apps ship (their production dependencies): permissive,
/// with nothing to publish but a notice.
pub const SHIPPED_LICENSES: [&str; 6] = [
    "MIT",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "0BSD",
];

/// Licenses allowed for development tooling only (never shipped): the shipped ones, plus
/// those of data and tools used at build and test time.
pub const TOOLING_LICENSES: [&str; 11] = [
    "MIT",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "0BSD",
    "BlueOak-1.0.0",
    "MPL-2.0",
    "CC-BY-3.0",
    "CC-BY-4.0",
    "CC0-1.0",
];

/// Tooling packages allowed under another license, by name prefix, with the reason.
pub const TOOLING_EXCEPTIONS: [(&str, &str, &str); 1] = [(
    "@img/sharp-libvips-",
    "LGPL-3.0-or-later",
    "the image library of sharp, loaded by the Astro build of the roadmap at build time, never shipped",
)];

/// The pnpm commands listing licenses: of what the apps ship, then of everything.
fn license_commands() -> [Cmd; 2] {
    let pnpm = |args: &[&str]| Cmd {
        program: "pnpm",
        args: args.iter().map(|&arg| arg.to_owned()).collect(),
        dir: Dir::Root,
    };
    [
        pnpm(&[
            "licenses",
            "list",
            "--json",
            "--prod",
            "--filter",
            "./apps/playground",
            "--filter",
            "./apps/roadmap",
            "--filter",
            "./bindings/wasm",
        ]),
        pnpm(&["licenses", "list", "--json", "--recursive"]),
    ]
}

/// The packages of a `pnpm licenses list --json` report, with their license.
fn licensed(report: &str, what: &str) -> Result<Vec<(String, String)>, String> {
    let json: serde_json::Value = serde_json::from_str(report)
        .map_err(|error| format!("invalid {what} license report: {error}"))?;
    let groups = json
        .as_object()
        .ok_or_else(|| format!("invalid {what} license report: not an object"))?;
    Ok(groups
        .iter()
        .flat_map(|(license, packages)| {
            packages
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|package| package.get("name").and_then(serde_json::Value::as_str))
                .map(move |name| (name.to_owned(), license.clone()))
        })
        .collect())
}

/// Checks the license reports: every shipped package under [`SHIPPED_LICENSES`], every
/// other one under [`TOOLING_LICENSES`] or a [`TOOLING_EXCEPTIONS`] entry.
///
/// # Errors
///
/// Returns a message naming every package under a license it may not have.
pub fn license_gate(shipped: &str, all: &str) -> Result<(), String> {
    let mut problems: Vec<String> = licensed(shipped, "shipped")?
        .into_iter()
        .filter(|(_, license)| !SHIPPED_LICENSES.contains(&license.as_str()))
        .map(|(name, license)| format!("{name} is shipped under {license}"))
        .collect();
    problems.extend(
        licensed(all, "tooling")?
            .into_iter()
            .filter(|(name, license)| {
                !TOOLING_LICENSES.contains(&license.as_str())
                    && !TOOLING_EXCEPTIONS
                        .iter()
                        .any(|(prefix, allowed, _)| name.starts_with(prefix) && license == allowed)
            })
            .map(|(name, license)| format!("{name} is under {license}")),
    );
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!("licenses not allowed: {}", problems.join(", ")))
    }
}

/// The only files allowed to lift the workspace lint against unsafe code, each audited:
/// the C interface of the WebAssembly module, and the counting allocator of the tests.
pub const UNSAFE_FILES: [&str; 2] = [
    "bindings/wasm/src/ffi.rs",
    "crates/engine-alloc-counter/src/lib.rs",
];

/// Checks that only [`UNSAFE_FILES`] lift the lint against unsafe code (an `allow` or
/// `expect` of it), and that each of them still does. The lint name is built at run time,
/// so that this file does not name it.
///
/// # Errors
///
/// Returns a message naming every other file that lifts it, and every allowed file that
/// no longer does.
pub fn unsafe_gate(sources: &[(String, String)]) -> Result<(), String> {
    let lint = format!("{}_{}", "unsafe", "code");
    let lifting: BTreeSet<&str> = sources
        .iter()
        .filter(|(_, text)| {
            text.lines().any(|line| {
                line.contains(&lint)
                    && !line.contains(&format!("forbid({lint})"))
                    && !line.contains(&format!("deny({lint})"))
            })
        })
        .map(|(path, _)| path.as_str())
        .collect();
    let allowed: BTreeSet<&str> = UNSAFE_FILES.into_iter().collect();
    let problems: Vec<String> =
        lifting
            .difference(&allowed)
            .map(|path| format!("{path} lifts the lint against unsafe code"))
            .chain(allowed.difference(&lifting).map(|path| {
                format!("{path} is listed in UNSAFE_FILES but no longer lifts the lint")
            }))
            .collect();
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

/// The tools the checks run, each with the command proving it is installed and how to
/// install it.
pub const TOOLS: [(&str, &[&str], &str); 9] = [
    ("git", &["--version"], "install git"),
    (
        "cargo",
        &["--version"],
        "install rustup (https://rustup.rs): the pinned toolchain installs itself",
    ),
    (
        "cargo",
        &[NIGHTLY_TOOLCHAIN, "--version"],
        "rustup toolchain install nightly-2026-11-01 --component llvm-tools-preview --component miri --component rust-src",
    ),
    (
        "cargo",
        &[NIGHTLY_TOOLCHAIN, "miri", "--version"],
        "rustup component add --toolchain nightly-2026-11-01 miri rust-src",
    ),
    (
        "cargo",
        &["llvm-cov", "--version"],
        "cargo install --locked cargo-llvm-cov",
    ),
    (
        "cargo",
        &["mutants", "--version"],
        "cargo install --locked cargo-mutants",
    ),
    (
        "cargo",
        &["deny", "--version"],
        "cargo install --locked cargo-deny",
    ),
    ("node", &["--version"], "install Node.js 22.18 or later"),
    (
        "pnpm",
        &["--version"],
        "corepack enable (pnpm 10, as packageManager in package.json)",
    ),
];

/// The oldest Node.js release the packages support (their `engines`), major and minor.
pub const NODE_FLOOR: (u64, u64) = (22, 18);

/// Checks the platform and the tools the checks need.
///
/// `answers` holds what each of [`TOOLS`] answered (its version line, or `None` when it
/// is missing): every tool must be present, Node.js at [`NODE_FLOOR`] or later, on a
/// platform the checks support (macOS or Linux; Windows through WSL2).
///
/// # Errors
///
/// Returns a message naming each missing tool with how to install it.
pub fn preflight_gate(os: &str, answers: &[Option<String>]) -> Result<(), String> {
    let mut problems = Vec::new();
    if os == "windows" {
        problems.push("the checks run on macOS and Linux: on Windows, run them in WSL2".to_owned());
    }
    for ((program, args, install), answer) in TOOLS.iter().zip(answers) {
        match answer {
            None => problems.push(format!(
                "`{program} {}` is missing: {install}",
                args.join(" ")
            )),
            Some(version) if *program == "node" => {
                let numbers: Vec<u64> = version
                    .trim()
                    .trim_start_matches('v')
                    .split('.')
                    .map_while(|part| part.parse().ok())
                    .collect();
                if (numbers.first().copied(), numbers.get(1).copied())
                    < (Some(NODE_FLOOR.0), Some(NODE_FLOOR.1))
                {
                    problems.push(format!("Node.js {} is too old: {install}", version.trim()));
                }
            }
            Some(_) => {}
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

/// The project manifest holding the workspace version.
pub const CARGO_TOML: &str = "Cargo.toml";

/// Every `package.json` of the pnpm workspace: all carry the project version.
pub const PACKAGE_JSONS: [&str; 6] = [
    "package.json",
    "apps/roadmap/package.json",
    "bindings/wasm/package.json",
    "apps/playground/package.json",
    "tooling/config/package.json",
    "tooling/oracle/package.json",
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
    /// Runs a command and returns its exit code; `None` when a signal stopped it.
    ///
    /// # Errors
    ///
    /// Returns an error when the command cannot be started.
    fn run(&mut self, cmd: &Cmd) -> io::Result<Option<i32>>;

    /// Reads a text file below the repository root.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read.
    fn read(&mut self, path: &str) -> io::Result<String>;

    /// Empties a directory below the repository root, creating it if needed.
    ///
    /// # Errors
    ///
    /// Returns an error when the directory cannot be removed or created.
    fn fresh_dir(&mut self, path: &str) -> io::Result<()>;

    /// The files tracked by git matching any of `patterns` (git pathspecs such as
    /// `*.ts`), relative to the root.
    ///
    /// # Errors
    ///
    /// Returns an error when git cannot list them.
    fn tracked(&mut self, patterns: &[&str]) -> io::Result<Vec<String>>;

    /// Runs a command and returns what it wrote on stdout.
    ///
    /// # Errors
    ///
    /// Returns an error when the command cannot be started or does not succeed.
    fn capture(&mut self, cmd: &Cmd) -> io::Result<String>;
}

/// Runs real commands below a repository root.
#[derive(Debug, Clone)]
pub struct SystemRunner {
    /// The repository root.
    pub root: PathBuf,
}

impl Runner for SystemRunner {
    fn run(&mut self, cmd: &Cmd) -> io::Result<Option<i32>> {
        Command::new(cmd.program)
            .args(&cmd.args)
            .current_dir(cmd.dir.under(&self.root))
            .status()
            .map(|status| status.code())
    }

    fn read(&mut self, path: &str) -> io::Result<String> {
        std::fs::read_to_string(self.root.join(path))
    }

    fn fresh_dir(&mut self, path: &str) -> io::Result<()> {
        let dir = self.root.join(path);
        match std::fs::remove_dir_all(&dir) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
        std::fs::create_dir_all(dir)
    }

    fn capture(&mut self, cmd: &Cmd) -> io::Result<String> {
        Command::new(cmd.program)
            .args(&cmd.args)
            .current_dir(cmd.dir.under(&self.root))
            .output()
            .and_then(|output| {
                if output.status.success() {
                    String::from_utf8(output.stdout).map_err(io::Error::other)
                } else {
                    Err(io::Error::other(format!("`{}` failed", cmd.line())))
                }
            })
    }

    fn tracked(&mut self, patterns: &[&str]) -> io::Result<Vec<String>> {
        Command::new("git")
            .args(["ls-files", "--"])
            .args(patterns)
            .current_dir(&self.root)
            .output()
            .and_then(|output| {
                if output.status.success() {
                    String::from_utf8(output.stdout)
                        .map(|text| text.lines().map(str::to_owned).collect())
                        .map_err(io::Error::other)
                } else {
                    Err(io::Error::other("git ls-files failed"))
                }
            })
    }
}

/// Reads a file through `runner`, naming it when it cannot be read.
fn read_file(runner: &mut dyn Runner, path: &str) -> Result<String, String> {
    runner
        .read(path)
        .map_err(|error| format!("cannot read {path}: {error}"))
}

/// Asks every tool of [`TOOLS`] for its version, then checks them.
fn preflight(runner: &mut dyn Runner) -> Result<(), String> {
    let answers: Vec<Option<String>> = TOOLS
        .iter()
        .map(|(program, args, _)| {
            let cmd = Cmd {
                program,
                args: args.iter().map(|&arg| arg.to_owned()).collect(),
                dir: Dir::Root,
            };
            runner.capture(&cmd).ok()
        })
        .collect();
    preflight_gate(std::env::consts::OS, &answers)
}

/// Lists the manifests tracked by git, then checks them.
fn membership(runner: &mut dyn Runner) -> Result<(), String> {
    let list = |source: &mut dyn Runner, pattern: &str| {
        source
            .tracked(&[pattern])
            .map_err(|error| format!("cannot list the {pattern} files: {error}"))
    };
    list(runner, "*package.json").and_then(|packages| {
        list(runner, "*.gitignore").and_then(|ignores| {
            list(runner, "*Cargo.toml").and_then(|manifests| {
                manifests
                    .into_iter()
                    .map(|path| read_file(runner, &path).map(|text| (path, text)))
                    .collect::<Result<Vec<_>, _>>()
                    .and_then(|crates| membership_gate(&packages, &crates, &ignores))
            })
        })
    })
}

/// Lists the licenses of what ships and of everything, then checks them.
fn licenses(runner: &mut dyn Runner) -> Result<(), String> {
    let [shipped, all] = license_commands();
    let report = |source: &mut dyn Runner, cmd: &Cmd| {
        source
            .capture(cmd)
            .map_err(|error| format!("cannot list licenses: {error}"))
    };
    report(runner, &shipped)
        .and_then(|shipped| report(runner, &all).and_then(|all| license_gate(&shipped, &all)))
}

/// Runs one action of a step.
fn run_action(action: &Action, runner: &mut dyn Runner) -> Result<(), String> {
    match action {
        Action::Run(cmd) => match runner.run(cmd) {
            Ok(Some(0_i32)) => Ok(()),
            Ok(Some(code)) => Err(format!("`{}` exited with code {code}", cmd.line())),
            Ok(None) => Err(format!("`{}` was stopped by a signal", cmd.line())),
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
        Action::FreshDir(path) => runner
            .fresh_dir(path)
            .map_err(|error| format!("cannot empty {path}: {error}")),
        Action::FreshDirs(paths) => paths.iter().try_for_each(|path| {
            runner
                .fresh_dir(path)
                .map_err(|error| format!("cannot empty {path}: {error}"))
        }),
        Action::MembershipGate => membership(runner),
        Action::Preflight => preflight(runner),
        Action::UnsafeGate => runner
            .tracked(&["*.rs"])
            .map_err(|error| format!("cannot list the Rust files: {error}"))
            .and_then(|paths| {
                paths
                    .into_iter()
                    .map(|path| read_file(runner, &path).map(|text| (path, text)))
                    .collect::<Result<Vec<_>, _>>()
            })
            .and_then(|sources| unsafe_gate(&sources)),
        Action::LicenseGate => licenses(runner),
        Action::TypeScriptReportsGate => TYPESCRIPT_PACKAGES.iter().try_for_each(|package| {
            read_file(runner, &format!("{package}/coverage/coverage-summary.json"))
                .and_then(|summary| ts_coverage_gate(package, &summary))?;
            read_file(runner, &format!("{package}/reports/mutation/mutation.json"))
                .and_then(|report| ts_mutation_gate(package, &report))
        }),
        Action::EquivalentGate => runner
            .tracked(&["*.ts", "*.tsx"])
            .map_err(|error| format!("cannot list the TypeScript files: {error}"))
            .and_then(|paths| {
                paths
                    .into_iter()
                    .map(|path| read_file(runner, &path).map(|text| (path, text)))
                    .collect::<Result<Vec<_>, _>>()
            })
            .and_then(|sources| {
                read_file(runner, EQUIVALENT_REVIEWED)
                    .and_then(|reviewed| equivalent_gate(&sources, &reviewed))
            }),
        Action::UnviableGate => read_file(runner, UNVIABLE_REPORT).and_then(|report| {
            read_file(runner, UNVIABLE_REVIEWED)
                .and_then(|reviewed| unviable_gate(&report, &reviewed))
        }),
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
        let outcome = run_action(&step.action, runner);
        outcome.map_err(|reason| format!("✘ {}: {reason}", step.name))?;
    }
    writeln!(out, "✔ all {total_steps} checks passed").map_err(|error| error.to_string())
}

/// Usage of the `xtask` command.
pub const USAGE: &str = "usage: cargo xtask <check|list|bless|commit-msg <file>|help>\n  check              run every quality check, as CI and the pre-push hook do\n  list               list the checks, in the order they run\n  bless              rewrite conformance/references.json from the current renders\n  commit-msg <file>  check a commit message (used by the commit-msg hook)\n  help               show this help";

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

/// Entry point: runs the command named by `args` and returns the exit code: 0 on success,
/// 1 when a check or a command failed (the reason on `err`), 2 for a usage error.
pub fn cli(
    args: &[String],
    runner: &mut dyn Runner,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let result = match args {
        [command] if command == "check" => run_steps(&check_steps(), runner, out),
        [command] if command == "bless" => run_steps(&[bless_step()], runner, out),
        [command, path] if command == "commit-msg" => {
            read_file(runner, path).and_then(|message| commit_msg_gate(&message))
        }
        [command] if command == "help" => {
            writeln!(out, "{USAGE}").map_err(|error| error.to_string())
        }
        [command] if command == "list" => check_steps()
            .iter()
            .enumerate()
            .try_for_each(|(index, step)| {
                writeln!(out, "{:>2}. {}", index.saturating_add(1), step.name)
            })
            .map_err(|error| error.to_string()),
        _ => {
            // Nothing more can be done if stderr itself is broken: the exit code still tells.
            drop(writeln!(err, "{USAGE}"));
            return 2;
        }
    };
    match result {
        Ok(()) => 0,
        Err(message) => {
            // Nothing more can be done if stderr itself is broken: the exit code still fails.
            drop(writeln!(err, "{message}"));
            1
        }
    }
}

/// Writes to `W`, treating a closed reader (a broken pipe, as with `| head`) as success:
/// what nobody reads is not an error, and the exit code stays the outcome of the checks.
#[derive(Debug)]
pub struct BestEffort<W>(pub W);

impl<W: Write> Write for BestEffort<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.0.write(buf) {
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(buf.len()),
            other => other,
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.0.flush() {
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
            other => other,
        }
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
