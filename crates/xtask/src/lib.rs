//! Project automation for Poyetik.
//!
//! `cargo xtask check` runs every quality check of the repository, in order, and stops
//! at the first failure, mutation testing only the changes ([`Mode::Changes`]); the
//! pre-push hook runs it. `cargo xtask check --full` runs every mutant ([`Mode::Full`]);
//! CI runs it. A green default check proves less than a green full check: see
//! [`Mode`].

#![forbid(unsafe_code)]

extern crate alloc;

use alloc::collections::{BTreeMap, BTreeSet};
use core::num::NonZeroUsize;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Pinned nightly toolchain, used only to measure branch coverage and to run Miri.
pub const NIGHTLY_TOOLCHAIN: &str = "+nightly-2026-11-01";

/// Where the coverage reports of the crates are written, one per measurement.
pub const COVERAGE_DIR: &str = "target/coverage";

/// Where the fixtures of xtask's own checks live: workspaces of their own, which are not
/// parts of the project.
pub const FIXTURES: &str = "crates/xtask/tests/fixtures/";

/// A crate of the Cargo workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RustCrate {
    /// Its package name.
    pub name: &'static str,
    /// Its directory, relative to the root: `<folder of parts>/<name>`.
    pub dir: &'static str,
    /// Whether it has a binary, `src/main.rs`, which its unit tests cannot reach.
    pub binary: bool,
}

/// The crates of the Cargo workspace (checked by [`membership_gate`]): each is measured by
/// its own unit tests, and each binary by the crate's tests.
pub const RUST_CRATES: [RustCrate; 9] = [
    RustCrate {
        name: "engine-core",
        dir: "crates/engine-core",
        binary: false,
    },
    RustCrate {
        name: "engine-objects",
        dir: "crates/engine-objects",
        binary: false,
    },
    RustCrate {
        name: "engine-patch",
        dir: "crates/engine-patch",
        binary: false,
    },
    RustCrate {
        name: "engine-hash",
        dir: "crates/engine-hash",
        binary: false,
    },
    RustCrate {
        name: "engine-alloc-counter",
        dir: "crates/engine-alloc-counter",
        binary: false,
    },
    RustCrate {
        name: "engine-testkit",
        dir: "crates/engine-testkit",
        binary: false,
    },
    RustCrate {
        name: "engine-cli",
        dir: "crates/engine-cli",
        binary: true,
    },
    RustCrate {
        name: "engine-wasm",
        dir: "bindings/wasm",
        binary: false,
    },
    RustCrate {
        name: "xtask",
        dir: "crates/xtask",
        binary: true,
    },
];

/// A crate whose unit tests reach one function and whose integration test reaches both:
/// the coverage gate must refuse it, proving that only a crate's own unit tests count.
pub const COVERAGE_LOCALITY: RustCrate = RustCrate {
    name: "coverage-locality",
    dir: "crates/xtask/tests/fixtures/coverage-locality",
    binary: false,
};

/// What a coverage measurement covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Measured {
    /// The library, by its own unit tests only (`--lib`): a path reached only from another
    /// crate or an integration test is not covered.
    Library,
    /// The binary's `src/main.rs`, by every test of the crate: unit tests cannot reach it.
    Binary,
}

/// One coverage measurement: a crate, and what of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Coverage {
    /// The crate measured.
    pub krate: RustCrate,
    /// What of it.
    pub measured: Measured,
}

impl Coverage {
    /// Where its report is written.
    #[must_use]
    pub fn report(self) -> String {
        let suffix = match self.measured {
            Measured::Library => "",
            Measured::Binary => "-main",
        };
        format!("{COVERAGE_DIR}/{}{suffix}.json", self.krate.name)
    }

    /// The command emptying the coverage build first. The measured crates share one build
    /// directory: an object left there by an earlier measurement (the same crate compiled
    /// as a dependency of another) would enter the report as a second copy of its code,
    /// never run, and halve its coverage; each measurement starts from nothing.
    #[must_use]
    pub fn clean_command() -> Cmd {
        Cmd::new(
            "cargo",
            &[NIGHTLY_TOOLCHAIN, "llvm-cov", "clean", "--workspace"],
            Dir::Root,
        )
    }

    /// The command measuring it, with branch coverage (which needs the pinned nightly).
    #[must_use]
    pub fn command(self) -> Cmd {
        let manifest = format!("{}/Cargo.toml", self.krate.dir);
        let report = self.report();
        let mut args = vec![NIGHTLY_TOOLCHAIN, "llvm-cov", "--manifest-path", &manifest];
        if self.measured == Measured::Library {
            args.push("--lib");
        }
        args.extend([
            "--locked",
            "--branch",
            "--json",
            "--summary-only",
            "--output-path",
            &report,
        ]);
        Cmd::new("cargo", &args, Dir::Root)
    }
}

/// Where the native engine exports the WAV file of every conformance case, emptied
/// before each export so that no file from an earlier run is judged.
pub const CONFORMANCE_EXPORT: &str = "target/conformance";

/// Where cargo-mutants writes the report of its run, emptied before each run so that the
/// gates read only what this run found.
pub const MUTANTS_OUT: &str = "mutants.out";

/// Mutants that cargo-mutants could not build, as reported by the last run (in
/// [`MUTANTS_OUT`]).
pub const UNVIABLE_REPORT: &str = "mutants.out/unviable.txt";

/// Where the default check writes the changes whose Rust mutants it tests (`git diff`
/// against the base of [`MUTATION_BASES`], new Rust files included), for
/// `cargo mutants --in-diff`.
pub const CHANGES_DIFF: &str = "target/xtask/changes.diff";

/// What the changes of the default check are measured against, in order.
///
/// The merge base of `HEAD` with the first of these that git resolves: the branch's
/// upstream first, so that
/// before a push the changes are what the push adds; then `origin/main`, then `main`, for a
/// branch without an upstream. With none of them, the check fails and says to run
/// `--full`.
pub const MUTATION_BASES: [&str; 3] = ["@{upstream}", "origin/main", "main"];

/// Stryker's incremental file, relative to its package.
///
/// Kept between runs (in the package's ignored `reports/`, outside the report directories
/// emptied before each run) so that the default check reuses the results of the mutants
/// nothing changed.
pub const STRYKER_INCREMENTAL: &str = "reports/stryker-incremental.json";

/// How much mutation testing `cargo xtask check` does; every other check is the same in
/// both modes.
///
/// [`Mode::Changes`] proves that every mutant of the changed code is killed, by the tests
/// as they are now. It does not prove what only [`Mode::Full`] sees: a Rust mutant outside
/// the changed lines that a change elsewhere (in the tests, in a callee) lets survive, and
/// a TypeScript result Stryker reuses although something it does not track changed (a test
/// helper, a fixture, a dependency, the WebAssembly module). So a green default check does
/// not imply a green full check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Rust: the mutants of the lines changed since the merge base ([`MUTATION_BASES`]),
    /// uncommitted changes and new Rust files included. TypeScript: Stryker's incremental
    /// mode ([`STRYKER_INCREMENTAL`]), which reruns the mutants whose code or covering
    /// tests changed and reuses the earlier results of the others; its report holds every
    /// mutant of the package. Rust in `jobs` parallel jobs, as [`Mode::Full`].
    Changes {
        /// The number of cargo-mutants jobs.
        jobs: NonZeroUsize,
    },
    /// Every mutant: Rust in `jobs` parallel jobs ([`mutation_jobs`]), every TypeScript
    /// mutant run again (Stryker's `--force`), which also rewrites the incremental file.
    Full {
        /// The number of cargo-mutants jobs.
        jobs: NonZeroUsize,
    },
}

/// The number of parallel cargo-mutants jobs on a machine of `parallelism` threads: half of
/// them, at least one.
///
/// Each job builds and tests its own copy of the tree; cargo-mutants shares the processors
/// among the builds and tests of every job (its jobserver), so more jobs than processors
/// gain nothing, while each job adds a copy of the build on disk and its test threads.
/// Half lets the serial parts of one job (linking, starting the tests) overlap the work of
/// another without doubling the load.
#[must_use]
pub fn mutation_jobs(parallelism: NonZeroUsize) -> NonZeroUsize {
    NonZeroUsize::new(parallelism.get() / 2).unwrap_or(NonZeroUsize::MIN)
}

/// Whether a mutation run tested every mutant there is, so that a gate on its report may
/// require what only a complete run shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Extent {
    /// Every mutant of what it covers: a review of a mutant it did not find is stale, and a
    /// report without any mutant is suspect.
    Complete,
    /// The mutants of a part only (the changes, one target): a review of a mutant it did
    /// not generate may be current elsewhere, and finding no mutant is possible.
    Partial,
}

/// Reviewed unviable mutants, each with the reason it is legitimately unbuildable.
pub const UNVIABLE_REVIEWED: &str = "mutants-unviable.txt";

/// Reviewed equivalent mutants of the TypeScript code: every `// Stryker disable
/// next-line` comment, each with the reason no test can tell the mutant apart.
pub const EQUIVALENT_REVIEWED: &str = "mutants-equivalent.txt";

/// A TypeScript package of the pnpm workspace.
///
/// Everything xtask does with a package derives from its entry in [`PACKAGES`]: its check
/// step, its report directories (emptied before it runs, then read by the reports gate),
/// its `package.json` (version and membership), its README, and whether its production
/// dependencies are held to the shipped licenses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Package {
    /// Its directory, relative to the root: `<folder of parts>/<name>`.
    pub dir: &'static str,
    /// The name of its check step (`pnpm run check` in its directory).
    pub step: &'static str,
    /// Whether what it ships reaches users, so that its production dependencies must be
    /// under [`SHIPPED_LICENSES`].
    pub shipped: bool,
    /// Its build output directories that its build does not empty itself (`tsc` never
    /// removes an output whose source is gone), relative to its directory: emptied before
    /// the checks, so that no stale output of an earlier run is used by a later step.
    pub outputs: &'static [&'static str],
}

impl Package {
    /// Its report directories: the coverage summary, then the mutation report.
    #[must_use]
    pub fn reports(self) -> [String; 2] {
        [
            format!("{}/coverage", self.dir),
            format!("{}/reports/mutation", self.dir),
        ]
    }
}

/// The TypeScript packages, in the order their checks run.
///
/// The shared policy, the oracle, the engine in the browser, the playground built on it,
/// then the roadmap. Each writes its coverage summary and its mutation report, both checked
/// by xtask rather than trusted to their own thresholds.
pub const PACKAGES: [Package; 5] = [
    Package {
        dir: "tooling/config",
        step: "Shared TypeScript policy (each rule proven to reject what it forbids)",
        shipped: false,
        outputs: &[],
    },
    Package {
        dir: "tooling/oracle",
        step: "Conformance oracle (independent, TypeScript)",
        shipped: false,
        outputs: &[],
    },
    Package {
        dir: "bindings/wasm",
        step: "Engine WASM checks",
        shipped: true,
        outputs: &["dist"],
    },
    Package {
        dir: "apps/playground",
        step: "Playground checks",
        shipped: true,
        outputs: &[],
    },
    Package {
        dir: "apps/roadmap",
        step: "Roadmap checks",
        shipped: true,
        outputs: &[],
    },
];

/// The report directories of every package of [`PACKAGES`], emptied before they run so
/// that every report read by the gate comes from this run.
#[must_use]
pub fn typescript_reports() -> Vec<String> {
    PACKAGES
        .iter()
        .flat_map(|package| package.reports())
        .collect()
}

/// The build output directories of every package of [`PACKAGES`] that its build does not
/// empty, emptied before the checks.
#[must_use]
pub fn build_outputs() -> Vec<String> {
    PACKAGES
        .iter()
        .flat_map(|package| {
            package
                .outputs
                .iter()
                .map(move |output| format!("{}/{output}", package.dir))
        })
        .collect()
}

/// Every `package.json` of the pnpm workspace, all carrying the project version: the root
/// one, then one per package of [`PACKAGES`].
#[must_use]
pub fn package_jsons() -> Vec<String> {
    core::iter::once("package.json".to_owned())
        .chain(
            PACKAGES
                .iter()
                .map(|package| format!("{}/package.json", package.dir)),
        )
        .collect()
}

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
    /// The directory of a package of [`PACKAGES`].
    Package(&'static str),
}

impl Dir {
    /// The directory, below `root`.
    #[must_use]
    pub fn under(self, root: &Path) -> PathBuf {
        match self {
            Self::Root => root.to_path_buf(),
            Self::Package(dir) => root.join(dir),
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
    /// `program` with `args`, run in `dir`.
    #[must_use]
    pub fn new(program: &'static str, args: &[&str], dir: Dir) -> Self {
        Self {
            program,
            args: args.iter().map(|&arg| (*arg).to_owned()).collect(),
            dir,
        }
    }

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
    /// Measure a crate and require 100% line, function and branch coverage of it.
    Coverage(Coverage),
    /// Measure [`COVERAGE_LOCALITY`] and require the coverage gate to refuse it.
    CoverageLocality,
    /// Run cargo-mutants on the changes of [`Mode::Changes`]: the diff since the merge base
    /// of [`MUTATION_BASES`], written to [`CHANGES_DIFF`]; nothing when there is no change.
    RustMutantsOfChanges {
        /// The number of cargo-mutants jobs.
        jobs: NonZeroUsize,
    },
    /// Require every unviable mutant of the last run to be reviewed; after a
    /// [`Extent::Complete`] run, also every review to be current.
    UnviableGate(Extent),
    /// Require every mutant of the mutation report of `package` to be killed
    /// ([`ts_mutation_gate`]), after a run of `extent`.
    TypeScriptMutationGate {
        /// The package mutated.
        package: Package,
        /// How much of the package the run mutated.
        extent: Extent,
    },
    /// Require every disabled TypeScript mutant to be reviewed, and every review to be
    /// current.
    EquivalentGate,
    /// Require one project version everywhere, consistent with a well-formed changelog
    /// (its format; the changelog entry of each change is the [`Action::HistoryGate`]'s).
    VersionGate,
    /// Empty each of these directories (relative to the root), creating them if needed:
    /// what a later step reads there is then produced by this run only.
    FreshDirs(Vec<String>),
    /// Require 100% coverage and every mutant killed in the reports of every TypeScript
    /// package.
    TypeScriptReportsGate,
    /// Require every Stryker configuration to hide no mutant.
    StrykerConfigGate,
    /// Require every TypeScript source of every package to be measured by its coverage and
    /// mutated by Stryker, or reviewed as an exception.
    TypeScriptScopeGate,
    /// Require every bypass of coverage or mutation testing to be reviewed.
    BypassGate,
    /// Require the packages and crates known to xtask to be exactly those of the
    /// repository, every crate on the workspace version.
    MembershipGate,
    /// Require every part to be described in the README of its folder, and every folder
    /// in the root README.
    ReadmeGate,
    /// Require the prose of the repository to use the words of the glossary, not the
    /// synonyms [`VOCABULARY`] lists.
    VocabularyGate,
    /// Require every JavaScript dependency under an allowed license: the strict list for
    /// what the apps ship, the wider one for development tooling.
    LicenseGate,
    /// Require the lint against unsafe code to be lifted only in the audited files, and
    /// forbidden in every crate root.
    UnsafeGate,
    /// Require the Cargo configuration to be exactly [`CARGO_CONFIG`].
    CargoConfigGate,
    /// Require every direct dependency to be reviewed, and every review current.
    DependencyGate,
    /// Require the supply-chain settings of [`SUPPLY_CHAIN_SETTINGS`].
    SupplyChainGate,
    /// Require the CI workflow to pin every action, tool and runtime.
    WorkflowGate,
    /// Record the state of the working tree ([`TreeState`]), for [`Action::TreeUnchanged`].
    TreeRecorded,
    /// Require the working tree to be as [`Action::TreeRecorded`] found it: the checks
    /// change no tracked file and leave no new untracked one.
    TreeUnchanged,
    /// Require every commit message to follow the convention, and every change of behavior
    /// since [`CHANGELOG_SINCE`] to update the changelog.
    HistoryGate,
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

/// Why a check or a command failed: one line for stderr, and the details, one per line,
/// for stdout (every problem a gate found, how to fix it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// What failed, in one line, naming the file or the command involved.
    pub summary: String,
    /// The details: one problem or hint per line.
    pub details: Vec<String>,
}

impl From<String> for Failure {
    fn from(summary: String) -> Self {
        Self {
            summary,
            details: Vec::new(),
        }
    }
}

/// The verdict of a gate that found `problems` about `context`: none passes; a single one
/// is said in the summary line; several are counted there and listed in the details.
fn verdict(context: &str, problems: Vec<String>) -> Result<(), Failure> {
    match problems.as_slice() {
        [] => Ok(()),
        [only] => Err(Failure::from(format!("{context}: {only}"))),
        _ => Err(Failure {
            summary: format!("{context}: {} problems, listed above", problems.len()),
            details: problems,
        }),
    }
}

/// Builds a command step.
fn run(name: &str, program: &'static str, args: &[&str], dir: Dir) -> Step {
    Step {
        name: name.to_owned(),
        action: Action::Run(Cmd::new(program, args, dir)),
    }
}

/// Formatting, lints, documentation and tests.
fn static_steps() -> Vec<Step> {
    vec![
        Step {
            name: "Tools and platform".to_owned(),
            action: Action::Preflight,
        },
        Step {
            name: "Working tree recorded".to_owned(),
            action: Action::TreeRecorded,
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
                &format!("Engine portability ({target})"),
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

/// The arguments of every cargo-mutants run: lints are capped while building mutants
/// (otherwise a mutant that merely leaves a variable unused fails to compile and is silently
/// skipped instead of tested), and the ignored files are not copied.
const MUTANTS_ARGS: [&str; 3] = ["mutants", "--cap-lints=true", "--gitignore=true"];

/// A cargo-mutants command: [`MUTANTS_ARGS`], then `args`.
fn mutants_command(args: &[&str]) -> Cmd {
    Cmd::new("cargo", &[&MUTANTS_ARGS[..], args].concat(), Dir::Root)
}

/// The cargo-mutants command of [`Mode::Changes`]: the workspace in `jobs` jobs, filtered
/// by [`CHANGES_DIFF`].
#[must_use]
pub fn changes_mutants_command(jobs: NonZeroUsize) -> Cmd {
    mutants_command(&[
        "--workspace",
        "--jobs",
        &jobs.to_string(),
        "--in-diff",
        CHANGES_DIFF,
    ])
}

/// Rust mutation testing in `mode`, after emptying [`MUTANTS_OUT`], then its gate.
fn rust_mutation_steps(mode: Mode) -> Vec<Step> {
    let (name, action, extent) = match mode {
        Mode::Changes { jobs } => (
            "Rust mutation testing of the changes",
            Action::RustMutantsOfChanges { jobs },
            Extent::Partial,
        ),
        Mode::Full { jobs } => (
            "Rust mutation testing",
            Action::Run(mutants_command(&[
                "--workspace",
                "--jobs",
                &jobs.to_string(),
            ])),
            Extent::Complete,
        ),
    };
    vec![
        Step {
            name: "Fresh Rust mutation report".to_owned(),
            action: Action::FreshDirs(vec![MUTANTS_OUT.to_owned()]),
        },
        Step {
            name: name.to_owned(),
            action,
        },
        Step {
            name: "Rust unviable mutants reviewed".to_owned(),
            action: Action::UnviableGate(extent),
        },
    ]
}

/// Coverage and mutation testing in `mode`, with their gates.
fn proof_steps(mode: Mode) -> Vec<Step> {
    let mut steps = vec![
        Step {
            name: "Fresh Rust coverage reports".to_owned(),
            action: Action::FreshDirs(vec![COVERAGE_DIR.to_owned()]),
        },
        Step {
            name: "Rust coverage counts a crate's own unit tests only".to_owned(),
            action: Action::CoverageLocality,
        },
    ];
    steps.extend(RUST_CRATES.iter().map(|&krate| Step {
        name: format!(
            "Rust coverage of {}: 100% by its own unit tests",
            krate.name
        ),
        action: Action::Coverage(Coverage {
            krate,
            measured: Measured::Library,
        }),
    }));
    steps.extend(
        RUST_CRATES
            .iter()
            .filter(|krate| krate.binary)
            .map(|&krate| Step {
                name: format!(
                    "Rust coverage of the {} binary: 100% by its tests",
                    krate.name
                ),
                action: Action::Coverage(Coverage {
                    krate,
                    measured: Measured::Binary,
                }),
            }),
    );
    steps.extend(rust_mutation_steps(mode));
    steps.extend([
        Step {
            name: "TypeScript equivalent mutants reviewed".to_owned(),
            action: Action::EquivalentGate,
        },
        Step {
            name: "Every bypass of coverage or mutation testing reviewed".to_owned(),
            action: Action::BypassGate,
        },
        Step {
            name: "Unsafe code confined to the audited files".to_owned(),
            action: Action::UnsafeGate,
        },
        Step {
            name: "Cargo configuration: no build flag, a locked alias".to_owned(),
            action: Action::CargoConfigGate,
        },
        Step {
            name: "Unsafe code under Miri".to_owned(),
            action: Action::Run(miri_command()),
        },
    ]);
    steps
}

/// The crates holding a file of [`UNSAFE_FILES`], whose tests Miri runs.
#[must_use]
pub fn unsafe_crates() -> Vec<&'static str> {
    RUST_CRATES
        .iter()
        .filter(|krate| {
            UNSAFE_FILES
                .iter()
                .any(|file| file.starts_with(&format!("{}/", krate.dir)))
        })
        .map(|krate| krate.name)
        .collect()
}

/// Miri runs the tests of every crate holding unsafe code ([`unsafe_crates`]), checking
/// them for undefined behavior.
#[must_use]
pub fn miri_command() -> Cmd {
    let mut args = vec![NIGHTLY_TOOLCHAIN, "miri", "test"];
    for krate in unsafe_crates() {
        args.extend(["--package", krate]);
    }
    args.push("--locked");
    Cmd::new("cargo", &args, Dir::Root)
}

/// Conformance, supply chain, then the consistency of the repository: membership, READMEs,
/// versions, history and the CI workflow.
fn consistency_steps() -> Vec<Step> {
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
            action: Action::FreshDirs(vec![CONFORMANCE_EXPORT.to_owned()]),
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
            name: "Every direct dependency reviewed".to_owned(),
            action: Action::DependencyGate,
        },
        Step {
            name: "Supply-chain settings: a week before a release, libm without features"
                .to_owned(),
            action: Action::SupplyChainGate,
        },
        Step {
            name: "Packages and crates known to the checks".to_owned(),
            action: Action::MembershipGate,
        },
        Step {
            name: "Every part described in a README".to_owned(),
            action: Action::ReadmeGate,
        },
        Step {
            name: "The words of the glossary, not their synonyms".to_owned(),
            action: Action::VocabularyGate,
        },
        Step {
            name: "One version everywhere, changelog well formed".to_owned(),
            action: Action::VersionGate,
        },
        Step {
            name: "Commit messages, and a changelog entry for each change of behavior".to_owned(),
            action: Action::HistoryGate,
        },
        Step {
            name: "CI workflow pinned".to_owned(),
            action: Action::WorkflowGate,
        },
    ]
}

/// The Stryker command of the package in `dir`, in `mode`.
///
/// Always incremental, so that the default check reuses what the last run found;
/// [`Mode::Full`] adds `--force`, which runs every mutant again and rewrites the incremental
/// file from that complete run.
#[must_use]
pub fn stryker_command(dir: &'static str, mode: Mode) -> Cmd {
    let mut args = vec![
        "exec",
        "stryker",
        "run",
        "--incremental",
        "--incrementalFile",
        STRYKER_INCREMENTAL,
    ];
    if matches!(mode, Mode::Full { .. }) {
        args.push("--force");
    }
    Cmd::new("pnpm", &args, Dir::Package(dir))
}

/// The TypeScript packages: dependencies, their audit and licenses, then the checks of
/// each package of [`PACKAGES`] and its mutation testing in `mode`, in order, and the gates
/// on their reports.
fn typescript_steps(mode: Mode) -> Vec<Step> {
    let mut steps = vec![
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
            name: "Stryker configurations hide no mutant".to_owned(),
            action: Action::StrykerConfigGate,
        },
        Step {
            name: "Fresh TypeScript report directories".to_owned(),
            action: Action::FreshDirs(typescript_reports()),
        },
        Step {
            name: "Fresh TypeScript build outputs".to_owned(),
            action: Action::FreshDirs(build_outputs()),
        },
    ];
    for package in PACKAGES {
        steps.push(run(
            package.step,
            "pnpm",
            &["run", "check"],
            Dir::Package(package.dir),
        ));
        let what = match mode {
            Mode::Changes { .. } => "what changed since the last run",
            Mode::Full { .. } => "every mutant",
        };
        steps.push(Step {
            name: format!("Mutation testing of {} ({what})", package.dir),
            action: Action::Run(stryker_command(package.dir, mode)),
        });
    }
    steps.push(Step {
        name: "TypeScript coverage and mutation reports".to_owned(),
        action: Action::TypeScriptReportsGate,
    });
    steps.push(Step {
        name: "Every TypeScript source measured and mutated, or reviewed".to_owned(),
        action: Action::TypeScriptScopeGate,
    });
    steps
}

/// Every check of the repository in `mode`, in the order they run.
#[must_use]
pub fn check_steps(mode: Mode) -> Vec<Step> {
    [
        static_steps(),
        portability_steps(),
        proof_steps(mode),
        consistency_steps(),
        typescript_steps(mode),
        vec![Step {
            name: "Working tree unchanged by the checks".to_owned(),
            action: Action::TreeUnchanged,
        }],
    ]
    .concat()
}

/// What the working tree holds that is not committed: the files git reports as changed or
/// untracked (ignored files apart), and the changes of the tracked ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeState {
    /// `git status --porcelain --untracked-files=all`.
    pub status: String,
    /// `git diff HEAD`: the content of the changes.
    pub diff: String,
}

/// Checks that the checks left the working tree as they found it.
///
/// A tree may be dirty (a local check of uncommitted work), but the checks must not change
/// it: a command that rewrites a tracked file (a stale `Cargo.lock` an unlocked `cargo`
/// would update, a generated file) or leaves a file git does not ignore makes CI differ from
/// what was checked.
///
/// # Errors
///
/// Returns every file whose state changed, or that the tree was not recorded.
pub fn tree_gate(before: Option<&TreeState>, after: &TreeState) -> Result<(), Failure> {
    let Some(before) = before else {
        return Err(Failure::from(
            "the working tree was not recorded before the checks".to_owned(),
        ));
    };
    let lines = |status: &str| -> BTreeSet<String> { status.lines().map(str::to_owned).collect() };
    let (old, new) = (lines(&before.status), lines(&after.status));
    let mut problems: Vec<String> = new
        .difference(&old)
        .map(|line| format!("`{line}` appeared"))
        .chain(
            old.difference(&new)
                .map(|line| format!("`{line}` went away")),
        )
        .collect();
    if problems.is_empty() && before.diff != after.diff {
        problems.push("the uncommitted changes of a tracked file changed".to_owned());
    }
    verdict("the checks changed the working tree", problems)
}

/// Reads the state of the working tree.
fn tree_state(runner: &mut dyn Runner) -> Result<TreeState, Failure> {
    Ok(TreeState {
        status: git_output(
            runner,
            &["status", "--porcelain", "--untracked-files=all"],
            "read the work tree",
        )?,
        diff: git_output(runner, &["diff", "HEAD"], "read the work tree")?,
    })
}

/// Reads one `{ count, covered }` total of an llvm-cov JSON summary.
fn total(totals: Option<&serde_json::Value>, kind: &str) -> Result<(u64, u64), String> {
    let field = |name: &str| {
        totals
            .and_then(|value| value.get(kind))
            .and_then(|entry| entry.get(name))
            .and_then(serde_json::Value::as_u64)
    };
    match (field("count"), field("covered")) {
        (Some(count), Some(covered)) => Ok((count, covered)),
        _ => Err(format!("coverage report has no {kind} total")),
    }
}

/// Checks that an llvm-cov JSON summary of `coverage` reports 100% line, function and
/// branch coverage of every file it measures.
///
/// For a library, every file of the report belongs to the crate (`<dir>/src/`) and at least
/// one is measured; for a binary, only its `src/main.rs` is read, and it must be there.
///
/// # Errors
///
/// Returns every file not fully covered with its counts and the command showing the lines,
/// every file of another crate, or why the report cannot be read.
pub fn coverage_gate(report: &str, coverage: Coverage) -> Result<(), Failure> {
    let krate = coverage.krate;
    let json: serde_json::Value = serde_json::from_str(report)
        .map_err(|error| Failure::from(format!("invalid coverage report: {error}")))?;
    let files = json
        .pointer("/data/0/files")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| Failure::from("coverage report has no files".to_owned()))?;
    let own = format!("/{}/src/", krate.dir);
    let mut problems = Vec::new();
    let mut measured = 0_usize;
    let mut incomplete = false;
    for file in files {
        let name = file
            .get("filename")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let inside = match (name.split_once(&own), coverage.measured) {
            (Some((_, inside)), Measured::Library) => inside,
            (Some((_, "main.rs")), Measured::Binary) => "main.rs",
            (None, Measured::Library) => {
                problems.push(format!("{name} is not a file of {}", krate.name));
                continue;
            }
            (_, Measured::Binary) => continue,
        };
        measured = measured.saturating_add(1);
        let mut missing = Vec::new();
        for kind in ["lines", "functions", "branches"] {
            let (count, covered) = total(file.get("summary"), kind)?;
            if covered != count {
                missing.push(format!("{kind} {covered}/{count}"));
            }
        }
        if !missing.is_empty() {
            incomplete = true;
            problems.push(format!(
                "{}/src/{inside}: {}",
                krate.dir,
                missing.join(", ")
            ));
        }
    }
    if measured == 0 {
        problems.push(format!("no file of {} measured", krate.name));
    }
    let context = match coverage.measured {
        Measured::Library => format!("{} is not covered 100% by its own unit tests", krate.name),
        Measured::Binary => format!("the {} binary is not covered 100% by its tests", krate.name),
    };
    verdict(&context, problems).map_err(|mut failure| {
        if incomplete {
            let lib = if coverage.measured == Measured::Library {
                " --lib"
            } else {
                ""
            };
            failure.details.push(format!(
                "the lines: cargo {NIGHTLY_TOOLCHAIN} llvm-cov --manifest-path {}/Cargo.toml{lib} --branch --show-missing-lines",
                krate.dir
            ));
        }
        failure
    })
}

/// Measures [`COVERAGE_LOCALITY`], whose integration test reaches a function its unit tests
/// do not, and requires the coverage gate to refuse its `src/lib.rs`.
fn coverage_locality(runner: &mut dyn Runner) -> Result<(), Failure> {
    let coverage = Coverage {
        krate: COVERAGE_LOCALITY,
        measured: Measured::Library,
    };
    let report = measure(runner, coverage)?;
    let refused = format!("{}/src/lib.rs: ", COVERAGE_LOCALITY.dir);
    match coverage_gate(&report, coverage) {
        Err(failure) if failure.summary.contains(&refused) => Ok(()),
        Err(failure) => Err(Failure::from(format!(
            "the coverage gate refused {} for another reason: {}",
            COVERAGE_LOCALITY.dir, failure.summary
        ))),
        Ok(()) => Err(Failure::from(format!(
            "the coverage gate accepted {}: a path reached only by an integration test counts as covered",
            COVERAGE_LOCALITY.dir
        ))),
    }
}

/// Runs a coverage measurement and reads its report.
fn measure(runner: &mut dyn Runner, coverage: Coverage) -> Result<String, Failure> {
    runner
        .run(&Coverage::clean_command())
        .map_err(|failure| Failure::from(failure.to_string()))?;
    runner
        .run(&coverage.command())
        .map_err(|failure| Failure::from(failure.to_string()))?;
    read_file(runner, &coverage.report())
}

/// Checks the coverage summary of a TypeScript package (Vitest `json-summary`).
///
/// 100% of lines, statements, functions and branches. Lines, statements and functions must
/// be measured (a report of nothing fails); branches may number zero, in code without any.
///
/// # Errors
///
/// Returns a message naming the package: the summary is not a valid coverage summary,
/// or it lists every kind not fully covered.
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
            // Code without a branch has none to cover; lines, statements and functions
            // must still be measured, so an empty report still fails.
            (Some(0), _) if kind == "branches" => {}
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

/// A mutant disabled in a TypeScript source by a well-formed, reviewed comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disable {
    /// The source, relative to the root.
    pub path: String,
    /// The line the comment disables, numbered from 1.
    pub line: usize,
    /// The mutator it disables.
    pub mutator: String,
}

/// Checks the mutation report of a TypeScript package (Stryker `json`).
///
/// At least one mutant after a [`Extent::Complete`] run (a partial one may find none);
/// every mutant `Killed`, or `Timeout` (a mutant that makes a test
/// hang is detected, as Stryker counts it), or `Ignored` exactly where a disable comment
/// of `disabled` says so (same file, line and mutator; [`equivalent_gate`] requires every
/// such comment to be reviewed). Any other status (`Survived`, `NoCoverage`,
/// `RuntimeError`, `CompileError`, one a future Stryker adds) fails.
///
/// # Errors
///
/// Returns every mutant that was not killed, named `file:line Mutator status`, or why the
/// report cannot be read.
pub fn ts_mutation_gate(
    package: &str,
    report: &str,
    disabled: &[Disable],
    extent: Extent,
) -> Result<(), Failure> {
    let json: serde_json::Value = serde_json::from_str(report)
        .map_err(|error| Failure::from(format!("{package}: invalid mutation report: {error}")))?;
    let mut mutants = 0_usize;
    let mut problems = Vec::new();
    let files = json
        .get("files")
        .and_then(serde_json::Value::as_object)
        .into_iter()
        .flatten();
    for (file, entry) in files {
        let path = format!("{package}/{file}");
        let listed = entry
            .get("mutants")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten();
        for mutant in listed {
            mutants = mutants.saturating_add(1);
            let text = |key: &str| {
                mutant
                    .get(key)
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
            };
            let line = mutant
                .pointer("/location/start/line")
                .and_then(serde_json::Value::as_u64)
                .and_then(|line| usize::try_from(line).ok())
                .unwrap_or_default();
            let (mutator, status) = (text("mutatorName"), text("status"));
            let reviewed = disabled.iter().any(|disable| {
                disable.path == path && disable.line == line && disable.mutator == mutator
            });
            let detected = matches!(status, "Killed" | "Timeout");
            let accepted = detected || (status == "Ignored" && reviewed);
            if !accepted {
                problems.push(format!("{path}:{line} {mutator} {status}"));
            }
        }
    }
    if mutants == 0 && extent == Extent::Complete {
        problems.push("no mutant in the mutation report".to_owned());
    }
    verdict(&format!("{package}: mutants not killed"), problems)
}

/// The keys a Stryker configuration may use: none of them hides a mutant. Any other key
/// (`ignoreStatic`, `mutator.excludedMutations`, `ignorers` …) could, and is refused.
pub const STRYKER_KEYS: [&str; 11] = [
    "$schema",
    "coverageAnalysis",
    "disableTypeChecks",
    "ignorePatterns",
    "mutate",
    "plugins",
    "reporters",
    "tempDirName",
    "testRunner",
    "thresholds",
    "vitest",
];

/// Checks the Stryker configuration of a TypeScript package.
///
/// Only [`STRYKER_KEYS`], no negated (`!`) glob in `mutate`, which would exclude files from
/// mutation, and the `json` reporter, whose report [`ts_mutation_gate`] reads.
///
/// # Errors
///
/// Returns every key, glob or missing reporter at fault, or why the file cannot be read.
pub fn stryker_config_gate(package: &str, config: &str) -> Result<(), Failure> {
    let json: serde_json::Value = serde_json::from_str(config).map_err(|error| {
        Failure::from(format!("{package}: invalid Stryker configuration: {error}"))
    })?;
    let keys = json.as_object().ok_or_else(|| {
        Failure::from(format!(
            "{package}: the Stryker configuration is not an object"
        ))
    })?;
    let strings = |key: &str| -> Vec<&str> {
        keys.get(key)
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .collect()
    };
    let mut problems: Vec<String> = keys
        .keys()
        .filter(|key| !STRYKER_KEYS.contains(&key.as_str()))
        .map(|key| format!("`{key}` is not allowed: it could hide mutants"))
        .collect();
    problems.extend(
        strings("mutate")
            .into_iter()
            .filter(|glob| glob.starts_with('!'))
            .map(|glob| format!("`{glob}` in mutate excludes files from mutation")),
    );
    if !strings("reporters").contains(&"json") {
        problems
            .push("the `json` reporter is missing: the mutation report is not written".to_owned());
    }
    verdict(
        &format!("{package}/stryker.config.json hides mutants"),
        problems,
    )
}

/// What may remove code from a proof without a test failing: a coverage exclusion, a
/// skipped mutant, an ignored or focused test. Built at run time, so that this file does
/// not hold them; each with the extensions of the files where it acts.
fn bypass_tokens() -> Vec<(String, &'static [&'static str])> {
    let rust: &[&str] = &["rs"];
    let typescript: &[&str] = &["ts", "tsx", "mts", "astro", "js", "mjs"];
    vec![
        (format!("{}::{}", "mutants", "skip"), rust),
        (format!("#[{}", "ignore"), rust),
        (format!("{}(off)", "coverage"), rust),
        (format!("cfg({}", "coverage"), rust),
        (format!("{}_{}", "coverage", "nightly"), rust),
        (format!("{} {}", "v8", "ignore"), typescript),
        (format!("{} {}", "istanbul", "ignore"), typescript),
        (format!("{} {}", "c8", "ignore"), typescript),
        (format!(".{}(", "only"), typescript),
        (format!(".{}(", "skip"), typescript),
        (format!(".{}(", "todo"), typescript),
        (format!(".{}(", "fixme"), typescript),
        (format!("{}If(", "skip"), typescript),
        (format!("{}If(", "run"), typescript),
    ]
}

/// The fixtures of the shared TypeScript policy: each holds on purpose what a rule forbids,
/// to prove the rule rejects it, and is never run as a test.
pub const POLICY_FIXTURES: [&str; 2] = [
    "tooling/config/test/policy/",
    "tooling/config/test/compiler/",
];

/// The tracked files the bypass gate reads (git pathspecs).
pub const BYPASS_SOURCES: [&str; 8] = [
    "*mutants.toml",
    "*.rs",
    "*.ts",
    "*.tsx",
    "*.mts",
    "*.astro",
    "*.js",
    "*.mjs",
];

/// Reviewed bypasses of coverage or mutation testing: every occurrence of a bypass token
/// ([`bypass_gate`]), each with the reason it is legitimate. Empty while there is none.
pub const BYPASSES_REVIEWED: &str = "proof-bypasses.txt";

/// Checks that nothing removes code from coverage or mutation testing unseen.
///
/// `sources` are the tracked source files (path and text): every line holding a bypass
/// token (a skipped mutant, an ignored test, a coverage exclusion, an
/// ignore comment of a coverage tool, a focused or skipped test, each in the languages
/// where it acts) outside the [`POLICY_FIXTURES`] is an occurrence, and so is any tracked
/// `mutants.toml` (a cargo-mutants configuration can exclude code); every occurrence must
/// be reviewed in `reviewed` ([`BYPASSES_REVIEWED`]: one `<file>: <token> at <line> --
/// <reason>` line per
/// occurrence), and every review must be current.
///
/// # Errors
///
/// Returns every unreviewed occurrence, stale review and review without a reason.
pub fn bypass_gate(sources: &[(String, String)], reviewed: &str) -> Result<(), Failure> {
    let tokens = bypass_tokens();
    let mut keys = Vec::new();
    for (path, text) in sources {
        if POLICY_FIXTURES
            .iter()
            .any(|fixtures| path.starts_with(fixtures))
        {
            continue;
        }
        if path.rsplit('/').next() == Some("mutants.toml") {
            keys.push(format!("{path}: a cargo-mutants configuration"));
            continue;
        }
        let extension = path.rsplit_once('.').map_or("", |(_, extension)| extension);
        for line in text.lines() {
            for (token, extensions) in &tokens {
                if extensions.contains(&extension) && line.contains(token.as_str()) {
                    keys.push(format!("{path}: {token} at {}", line.trim()));
                }
            }
        }
    }
    let mut problems = Vec::new();
    compare_reviews(
        &count(keys),
        Extent::Complete,
        &reviews_of(reviewed, false),
        &mut problems,
    );
    verdict(
        &format!(
            "bypasses of coverage or mutation testing must be reviewed in {BYPASSES_REVIEWED}"
        ),
        problems,
    )
}

/// The suffixes of the reviews of [`scope_gate`] in [`BYPASSES_REVIEWED`].
///
/// A source the coverage of its package does not measure, and one it measures but Stryker
/// does not mutate. Every other review of that file belongs to [`bypass_gate`].
pub const SCOPE_REVIEWS: [&str; 2] = [": not measured", ": measured, not mutated"];

/// The lines of `reviewed` that are reviews of [`scope_gate`] (`scope`), or the others.
fn reviews_of(reviewed: &str, scope: bool) -> String {
    reviewed
        .lines()
        .filter(|line| {
            let key = line.split_once(" -- ").map_or(*line, |(key, _)| key).trim();
            SCOPE_REVIEWS.iter().any(|suffix| key.ends_with(suffix)) == scope
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The tracked files the scope gate reads (git pathspecs): every language a package's
/// coverage and mutation testing could reach.
pub const TS_SOURCES: [&str; 9] = [
    "*.ts", "*.tsx", "*.mts", "*.cts", "*.js", "*.jsx", "*.mjs", "*.cjs", "*.astro",
];

/// What [`scope_gate`] reads of one TypeScript package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    /// Its directory, as in [`Package::dir`].
    pub dir: &'static str,
    /// The text of its `stryker.config.json`.
    pub stryker: String,
    /// The text of its coverage summary (Vitest `json-summary`), whose keys are the
    /// absolute paths of the files measured.
    pub summary: String,
}

/// Whether `rel`, a path relative to its package, is test code: under `test/`, or a test or
/// end-to-end specification file. Tests are what measures, not what is measured.
fn is_test(rel: &str) -> bool {
    rel.starts_with("test/")
        || [".test.ts", ".test.tsx", ".spec.ts"]
            .iter()
            .any(|suffix| rel.ends_with(suffix))
}

/// Whether `name` (one path segment) matches `pattern`, where `*` matches any run of
/// characters.
fn segment_matches(pattern: &str, name: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or_default();
    let Some(mut rest) = name.strip_prefix(first) else {
        return false;
    };
    let middle: Vec<&str> = parts.collect();
    let Some((last, between)) = middle.split_last() else {
        return rest.is_empty();
    };
    for part in between {
        match rest.find(part) {
            Some(at) => {
                rest = rest
                    .get(at.saturating_add(part.len())..)
                    .unwrap_or_default();
            }
            None => return false,
        }
    }
    rest.ends_with(last)
}

/// Whether the path `segments` match the glob `pattern` (segments too), where `**` matches
/// any number of whole segments, none included.
fn glob_matches(pattern: &[&str], segments: &[&str]) -> bool {
    match (pattern.split_first(), segments.split_first()) {
        (None, None) => true,
        (Some((&"**", rest)), _) => {
            glob_matches(rest, segments)
                || segments
                    .split_first()
                    .is_some_and(|(_, tail)| glob_matches(pattern, tail))
        }
        (Some((glob, rest)), Some((name, tail))) => {
            segment_matches(glob, name) && glob_matches(rest, tail)
        }
        (Some(_), None) | (None, Some(_)) => false,
    }
}

/// The characters of a glob syntax [`glob_matches`] does not understand (braces, classes,
/// single characters, negations, extended globs): a `mutate` glob holding one is refused
/// rather than read wrongly.
const GLOB_UNSUPPORTED: [char; 9] = ['{', '}', '[', ']', '?', '!', '(', ')', '@'];

/// Checks that every TypeScript source is both measured and mutated, or reviewed.
///
/// `tracked` are the files tracked with an extension of [`TS_SOURCES`]; `scopes` what each
/// package of [`PACKAGES`] measures (its coverage summary) and mutates (the `mutate` globs of
/// its Stryker configuration, of which only `*` and `**` are understood). The sources of a
/// package are its tracked files but its tests (`test/`, `*.test.ts`, `*.spec.ts`). Each
/// source must be measured and mutated; one measured but not mutated is a
/// `<source>: measured, not mutated` occurrence, one neither measured nor mutated a
/// `<source>: not measured` occurrence, each to be reviewed in `reviewed`
/// ([`BYPASSES_REVIEWED`], whose other reviews are [`bypass_gate`]'s), every review
/// current. So the coverage `include` and the Stryker `mutate` of a package select the same
/// sources, but for reviewed differences, and no source escapes both unseen.
///
/// # Errors
///
/// Returns every source mutated but not measured, every file measured that is not a source
/// of the package, every `mutate` glob that is not understood or matches no source, every
/// unreadable configuration or summary, and every unreviewed occurrence or stale review.
pub fn scope_gate(tracked: &[String], scopes: &[Scope], reviewed: &str) -> Result<(), Failure> {
    let mut problems = Vec::new();
    let mut keys = Vec::new();
    for scope in scopes {
        let dir = scope.dir;
        let prefix = format!("{dir}/");
        let sources: Vec<&str> = tracked
            .iter()
            .filter_map(|path| path.strip_prefix(&prefix))
            .filter(|rel| !is_test(rel))
            .collect();
        let stryker: serde_json::Value = match serde_json::from_str(&scope.stryker) {
            Ok(json) => json,
            Err(error) => {
                problems.push(format!("{dir}: invalid Stryker configuration: {error}"));
                continue;
            }
        };
        let globs: Vec<&str> = stryker
            .get("mutate")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .collect();
        let summary: serde_json::Value = match serde_json::from_str(&scope.summary) {
            Ok(json) => json,
            Err(error) => {
                problems.push(format!("{dir}: invalid coverage summary: {error}"));
                continue;
            }
        };
        let mut measured = BTreeSet::new();
        for file in summary
            .as_object()
            .into_iter()
            .flat_map(serde_json::Map::keys)
            .filter(|key| *key != "total")
        {
            match sources
                .iter()
                .find(|rel| file.ends_with(&format!("/{prefix}{rel}")))
            {
                Some(rel) => {
                    let _new = measured.insert(*rel);
                }
                None => problems.push(format!(
                    "{dir}: the coverage measures {file}, which is not a tracked source of the package"
                )),
            }
        }
        let mut used = BTreeSet::new();
        for glob in &globs {
            if glob.contains(GLOB_UNSUPPORTED) {
                problems.push(format!(
                    "{dir}/stryker.config.json: `{glob}` in mutate: only `*` and `**` are understood"
                ));
                let _new = used.insert(*glob);
            }
        }
        for rel in &sources {
            let segments: Vec<&str> = rel.split('/').collect();
            let matching: Vec<&str> = globs
                .iter()
                .copied()
                .filter(|glob| glob_matches(&glob.split('/').collect::<Vec<_>>(), &segments))
                .collect();
            used.extend(matching.iter().copied());
            match (measured.contains(rel), !matching.is_empty()) {
                (true, true) => {}
                (false, true) => problems.push(format!(
                    "{prefix}{rel} is mutated but not measured: the coverage `include` misses it"
                )),
                (true, false) => keys.push(format!("{prefix}{rel}: measured, not mutated")),
                (false, false) => keys.push(format!("{prefix}{rel}: not measured")),
            }
        }
        problems.extend(
            globs
                .iter()
                .filter(|glob| !used.contains(*glob))
                .map(|glob| {
                    format!("{dir}/stryker.config.json: `{glob}` in mutate matches no source")
                }),
        );
    }
    compare_reviews(
        &count(keys),
        Extent::Complete,
        &reviews_of(reviewed, true),
        &mut problems,
    );
    verdict(
        &format!(
            "TypeScript sources outside coverage or mutation testing must be reviewed in {BYPASSES_REVIEWED}"
        ),
        problems,
    )
}

/// The disable comments of `sources` (path and text of each TypeScript file), and a problem
/// for every malformed one.
///
/// A disable comment must be `// Stryker disable next-line <Mutator>: <reason>`, one
/// mutator, never `all`, with a reason, above a line that is not empty; any other use of
/// `Stryker disable` is refused.
fn disables(sources: &[(String, String)]) -> (Vec<(Disable, String)>, Vec<String>) {
    let mut found = Vec::new();
    let mut problems = Vec::new();
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
            let next = lines
                .get(index.saturating_add(1))
                .map_or("", |following| following.trim());
            match rule {
                Some((mutator, _reason)) if !next.is_empty() => found.push((
                    Disable {
                        path: path.clone(),
                        line: index.saturating_add(2),
                        mutator: mutator.to_owned(),
                    },
                    next.to_owned(),
                )),
                _ => problems.push(format!(
                    "{path}:{}: write `{STRYKER_DISABLE}<Mutator>: <reason>` above the line",
                    index.saturating_add(1)
                )),
            }
        }
    }
    (found, problems)
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

/// Counts each key of `keys`.
fn count<I: IntoIterator<Item = String>>(keys: I) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for key in keys {
        let entry: &mut usize = counts.entry(key).or_default();
        *entry = entry.saturating_add(1);
    }
    counts
}

/// Compares what a gate `found`, each key with how many times, with the keys of a review
/// file, adding to `problems` every line without a reason, every key found more often than
/// reviewed and, when `found` is [`Extent::Complete`], every review that is stale (after a
/// partial run, a key reviewed more often than found may be found elsewhere).
///
/// `reviewed` holds one `<key> -- <reason>` line per occurrence (a key found twice is
/// reviewed twice, each with its reason); blank lines and lines starting with `#` are
/// ignored.
fn compare_reviews(
    found: &BTreeMap<String, usize>,
    extent: Extent,
    reviewed: &str,
    problems: &mut Vec<String>,
) {
    let mut keys = Vec::new();
    for line in reviewed
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        match line.split_once(" -- ") {
            // The line is trimmed, so text always follows the separator: the reason is never empty.
            Some((key, _reason)) => keys.push(key.trim().to_owned()),
            None => problems.push(format!("review without a reason: {line}")),
        }
    }
    let known = count(keys);
    let counted = |key: &str, seen: usize, listed: usize| match (seen, listed) {
        (_, 0) | (0, _) => key.to_owned(),
        _ => format!("{key} ({seen} found, {listed} reviewed)"),
    };
    for (key, &seen) in found {
        let listed = known.get(key).copied().unwrap_or_default();
        if seen > listed {
            problems.push(format!("not reviewed: {}", counted(key, seen, listed)));
        }
    }
    if extent == Extent::Partial {
        return;
    }
    for (key, &listed) in &known {
        let seen = found.get(key).copied().unwrap_or_default();
        if listed > seen {
            problems.push(format!("stale review: {}", counted(key, seen, listed)));
        }
    }
}

/// Checks that the unviable mutants of the last run are reviewed and, after a
/// [`Extent::Complete`] run, exactly the reviewed ones.
///
/// A mutant is known by its file and description, not its line, so that
/// unrelated edits do not invalidate a review; two mutants with the same key (the same
/// operator twice in one function) need two reviews. `reviewed` holds one
/// `<file>.rs: <mutant> -- <reason>` line per mutant; blank lines and lines starting with
/// `#` are ignored.
///
/// # Errors
///
/// Returns a message listing unreviewed mutants, stale reviews (after a complete run) and
/// malformed lines.
pub fn unviable_gate(report: &str, extent: Extent, reviewed: &str) -> Result<(), Failure> {
    let mut problems = Vec::new();
    let mut keys = Vec::new();
    for line in report.lines().filter(|line| !line.trim().is_empty()) {
        match mutant_key(line) {
            Some(key) => keys.push(key),
            None => problems.push(format!("unrecognized mutant: {line}")),
        }
    }
    let found = count(keys);
    compare_reviews(&found, extent, reviewed, &mut problems);
    verdict(
        &format!("unviable mutants must be reviewed in {UNVIABLE_REVIEWED}"),
        problems,
    )
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
pub fn equivalent_gate(sources: &[(String, String)], reviewed: &str) -> Result<(), Failure> {
    let (found, mut problems) = disables(sources);
    let keys = found
        .into_iter()
        .map(|(disable, next)| format!("{}: {} at {next}", disable.path, disable.mutator));
    compare_reviews(&count(keys), Extent::Complete, reviewed, &mut problems);
    verdict(
        &format!("disabled mutants must be reviewed in {EQUIVALENT_REVIEWED}"),
        problems,
    )
}

/// Checks that the manifests tracked by git are exactly those xtask knows.
///
/// The tracked `package.json` files are exactly [`package_jsons`]: the root one and one per
/// package of [`PACKAGES`], whose checks and reports therefore run. Every crate manifest
/// (`Cargo.toml` below the root) takes the workspace version and repository, and inherits
/// the workspace lints. The only `.gitignore` is the
/// root one: build outputs go to `dist/` or `target/`, never hidden inside a source folder.
///
/// # Errors
///
/// Returns every manifest unknown to xtask or missing from the repository, and every crate
/// with a version of its own.
pub fn membership_gate(
    packages: &[String],
    crates: &[(String, String)],
    ignores: &[String],
) -> Result<(), Failure> {
    let tracked: BTreeSet<&str> = packages.iter().map(String::as_str).collect();
    let expected = package_jsons();
    let known: BTreeSet<&str> = expected.iter().map(String::as_str).collect();
    let mut problems: Vec<String> = tracked
        .difference(&known)
        .map(|path| {
            format!("{path} is not a package of PACKAGES: its checks and reports would not run")
        })
        .chain(
            known
                .difference(&tracked)
                .map(|path| format!("{path} belongs to PACKAGES but is not tracked")),
        )
        .collect();
    let manifests: BTreeSet<&str> = crates
        .iter()
        .map(|(path, _)| path.as_str())
        .filter(|path| *path != CARGO_TOML && !path.starts_with(FIXTURES))
        .collect();
    let members: Vec<String> = RUST_CRATES
        .iter()
        .map(|krate| format!("{}/Cargo.toml", krate.dir))
        .collect();
    let members: BTreeSet<&str> = members.iter().map(String::as_str).collect();
    problems.extend(manifests.difference(&members).map(|path| {
        format!("{path} is not a crate of RUST_CRATES: its coverage would not be measured")
    }));
    problems.extend(
        members
            .difference(&manifests)
            .map(|path| format!("{path} belongs to RUST_CRATES but is not tracked")),
    );
    for (key, what) in [("version", "version"), ("repository", "repository")] {
        let inherited = format!("{key}.workspace = true");
        problems.extend(
            crates
                .iter()
                .filter(|(path, _)| path != CARGO_TOML && !path.starts_with(FIXTURES))
                .filter(|(_, text)| !text.lines().any(|line| line.trim() == inherited))
                .map(|(path, _)| format!("{path} does not take the workspace {what}")),
        );
    }
    // Cargo refuses any `[lints.*]` table next to `workspace = true`: this pins the whole
    // policy, which a crate without it would silently lose.
    problems.extend(
        crates
            .iter()
            .filter(|(path, _)| path != CARGO_TOML && !path.starts_with(FIXTURES))
            .filter(|(_, text)| {
                let lines: Vec<&str> = text.lines().map(str::trim).collect();
                !lines
                    .windows(2)
                    .any(|pair| pair == ["[lints]", "workspace = true"])
            })
            .map(|(path, _)| {
                format!("{path} does not inherit the workspace lints (`[lints]` then `workspace = true`)")
            }),
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
    verdict("the manifests differ from what xtask knows", problems)
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

/// Licenses allowed for development tooling only (never shipped), besides the
/// [`SHIPPED_LICENSES`], which tooling may always use: those of data and tools used at build
/// and test time.
pub const TOOLING_EXTRA_LICENSES: [&str; 5] = [
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
    let pnpm = |args: &[&str]| Cmd::new("pnpm", args, Dir::Root);
    let filters: Vec<String> = PACKAGES
        .iter()
        .filter(|package| package.shipped)
        .flat_map(|package| ["--filter".to_owned(), format!("./{}", package.dir)])
        .collect();
    let mut shipped = vec!["licenses", "list", "--json", "--prod"];
    shipped.extend(filters.iter().map(String::as_str));
    [
        pnpm(&shipped),
        pnpm(&["licenses", "list", "--json", "--recursive"]),
    ]
}

/// The packages of a `pnpm licenses list --json` report, with their license.
///
/// The report is read strictly, at the boundary: an object whose every value is an array of
/// objects with a string `name`, holding at least one package, each of `expected`. A report
/// of another shape (a new pnpm, an empty listing) is refused instead of passing as if it
/// listed nothing.
fn licensed(report: &str, what: &str, expected: &[&str]) -> Result<Vec<(String, String)>, String> {
    let invalid = |why: &str| format!("invalid {what} license report: {why}");
    let json: serde_json::Value =
        serde_json::from_str(report).map_err(|error| invalid(&error.to_string()))?;
    let groups = json.as_object().ok_or_else(|| invalid("not an object"))?;
    let mut packages = Vec::new();
    for (license, group) in groups {
        let entries = group
            .as_array()
            .ok_or_else(|| invalid(&format!("{license} does not list packages")))?;
        for entry in entries {
            let name = entry
                .get("name")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| invalid(&format!("a package under {license} has no name")))?;
            packages.push((name.to_owned(), license.clone()));
        }
    }
    let missing: Vec<&str> = expected
        .iter()
        .filter(|name| !packages.iter().any(|(listed, _)| listed == *name))
        .copied()
        .collect();
    if packages.is_empty() {
        Err(invalid("no package"))
    } else if missing.is_empty() {
        Ok(packages)
    } else {
        Err(invalid(&format!("{} missing", missing.join(", "))))
    }
}

/// Packages each license report must list, proving it lists what it should: React for what
/// the apps ship, TypeScript for everything.
pub const LICENSE_CONTROLS: [&[&str]; 2] = [&["react", "react-dom"], &["typescript"]];

/// Checks the license reports.
///
/// Every shipped package under [`SHIPPED_LICENSES`], every other one under those or
/// [`TOOLING_EXTRA_LICENSES`], or a [`TOOLING_EXCEPTIONS`] entry; each report of the
/// expected shape, listing its [`LICENSE_CONTROLS`].
///
/// # Errors
///
/// Returns a message naming every package under a license it may not have.
pub fn license_gate(shipped: &str, all: &str) -> Result<(), Failure> {
    let [shipped_controls, all_controls] = LICENSE_CONTROLS;
    let mut problems: Vec<String> = licensed(shipped, "shipped", shipped_controls)?
        .into_iter()
        .filter(|(_, license)| !SHIPPED_LICENSES.contains(&license.as_str()))
        .map(|(name, license)| format!("{name} is shipped under {license}"))
        .collect();
    problems.extend(
        licensed(all, "tooling", all_controls)?
            .into_iter()
            .filter(|(name, license)| {
                !SHIPPED_LICENSES.contains(&license.as_str())
                    && !TOOLING_EXTRA_LICENSES.contains(&license.as_str())
                    && !TOOLING_EXCEPTIONS
                        .iter()
                        .any(|(prefix, allowed, _)| name.starts_with(prefix) && license == allowed)
            })
            .map(|(name, license)| format!("{name} is under {license}")),
    );
    verdict("licenses not allowed", problems)
}

/// The only files allowed to lift the workspace lint against unsafe code, each audited:
/// the C interface of the WebAssembly module, and the counting allocator of the tests.
pub const UNSAFE_FILES: [&str; 2] = [
    "bindings/wasm/src/ffi.rs",
    "crates/engine-alloc-counter/src/lib.rs",
];

/// Whether `relative` (the path of a Rust file inside a crate's directory) is the root of a
/// crate target: the library, a binary, a test, a benchmark, an example or a build script.
fn crate_root(relative: &str) -> bool {
    let single = |folder: &str| {
        relative
            .strip_prefix(folder)
            .is_some_and(|file| !file.contains('/'))
    };
    ["src/lib.rs", "src/main.rs", "build.rs"].contains(&relative)
        || ["tests/", "benches/", "examples/", "src/bin/"]
            .into_iter()
            .any(single)
}

/// Checks that unsafe code is impossible outside [`UNSAFE_FILES`], by the compiler.
///
/// Only those files lift the lint against unsafe code (an `allow` or `expect` of it), and
/// each still does; a line naming the lint is exempt only if it is exactly the attribute
/// forbidding or denying it. Every crate root of [`RUST_CRATES`] (library, binary, test,
/// benchmark, example, build script) holds the exact inner attribute forbidding the lint, so
/// that the compiler refuses any later lifting (E0453); in a crate holding an unsafe file,
/// whose library root cannot forbid it, every other module and every root outside `src/`
/// does. The lint name is built at run time, so that this file does not name it.
///
/// # Errors
///
/// Returns every other file that lifts the lint, every listed file that no longer does,
/// and every file that should forbid it and does not.
pub fn unsafe_gate(sources: &[(String, String)]) -> Result<(), Failure> {
    let lint = format!("{}_{}", "unsafe", "code");
    let forbid = format!("#![forbid({lint})]");
    let exempt = [
        forbid.clone(),
        format!("#![deny({lint})]"),
        format!("#[forbid({lint})]"),
        format!("#[deny({lint})]"),
    ];
    let lifting: BTreeSet<&str> = sources
        .iter()
        .filter(|(_, text)| {
            text.lines().any(|line| {
                line.contains(&lint) && !exempt.iter().any(|attribute| line.trim() == attribute)
            })
        })
        .map(|(path, _)| path.as_str())
        .collect();
    let allowed: BTreeSet<&str> = UNSAFE_FILES.into_iter().collect();
    let mut problems: Vec<String> =
        lifting
            .difference(&allowed)
            .map(|path| format!("{path} lifts the lint against unsafe code"))
            .chain(allowed.difference(&lifting).map(|path| {
                format!("{path} is listed in UNSAFE_FILES but no longer lifts the lint")
            }))
            .collect();
    for (path, text) in sources {
        let Some((krate, relative)) = RUST_CRATES.iter().find_map(|krate| {
            path.strip_prefix(&format!("{}/", krate.dir))
                .map(|relative| (krate, relative))
        }) else {
            continue;
        };
        let holds_unsafe = UNSAFE_FILES
            .iter()
            .any(|file| file.starts_with(&format!("{}/", krate.dir)));
        let required = if holds_unsafe {
            !UNSAFE_FILES.contains(&path.as_str())
                && relative != "src/lib.rs"
                && (relative.starts_with("src/") || crate_root(relative))
        } else {
            crate_root(relative)
        };
        if required && !text.lines().any(|line| line.trim() == forbid) {
            problems.push(format!("{path} does not hold `{forbid}`"));
        }
    }
    verdict("the lint against unsafe code", problems)
}

/// The exact Cargo configuration of the repository: an alias, nothing else.
///
/// Build flags (`rustflags`) could lift any lint of the workspace in every crate, and an
/// unlocked alias would rewrite a stale `Cargo.lock` before the checks.
pub const CARGO_CONFIG: &str = "[alias]
# The single entry point for project automation: `cargo xtask check` runs every check.
xtask = \"run --package xtask --locked --quiet --\"
";

/// The reviewed direct dependencies of every part, each with its reason.
pub const DEPENDENCIES_REVIEWED: &str = "dependencies.txt";

/// The fields of a `package.json` that declare dependencies.
pub const NPM_FIELDS: [&str; 4] = [
    "dependencies",
    "devDependencies",
    "peerDependencies",
    "optionalDependencies",
];

/// The command listing the crates of the workspace and their direct dependencies.
#[must_use]
pub fn metadata_command() -> Cmd {
    Cmd::new(
        "cargo",
        &["metadata", "--format-version", "1", "--no-deps", "--locked"],
        Dir::Root,
    )
}

/// Checks that every direct third-party dependency is reviewed, and every review current.
///
/// `metadata` is the output of [`metadata_command`]: each crate's dependencies, those on
/// another crate of the workspace (a `path`) apart; `packages` the `package.json` files
/// (path and text), each declaring dependencies in its [`NPM_FIELDS`], those on another
/// package of the workspace (`workspace:`) apart. `reviewed` ([`DEPENDENCIES_REVIEWED`])
/// holds one `<crate or package directory>: <dependency> (<kind>) -- <reason>` line per
/// dependency.
///
/// # Errors
///
/// Returns every unreviewed dependency, stale review and review without a reason, or why
/// the metadata or a manifest cannot be read.
pub fn dependency_gate(
    metadata: &str,
    packages: &[(String, String)],
    reviewed: &str,
) -> Result<(), Failure> {
    let json: serde_json::Value = serde_json::from_str(metadata)
        .map_err(|error| Failure::from(format!("invalid cargo metadata: {error}")))?;
    let crates = json
        .get("packages")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| Failure::from("cargo metadata lists no packages".to_owned()))?;
    let mut keys = Vec::new();
    for krate in crates {
        let name = krate
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let dependencies = krate
            .get("dependencies")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter(|dependency| dependency.get("path").is_none());
        for dependency in dependencies {
            let text = |key: &str| dependency.get(key).and_then(serde_json::Value::as_str);
            let kind = text("kind").unwrap_or("normal");
            keys.push(format!(
                "{name}: {} ({kind})",
                text("name").unwrap_or_default()
            ));
        }
    }
    for (path, manifest) in packages {
        let fields: serde_json::Value = serde_json::from_str(manifest)
            .map_err(|error| Failure::from(format!("{path}: invalid package.json: {error}")))?;
        let part = path.strip_suffix("/package.json").unwrap_or(".");
        for field in NPM_FIELDS {
            let declared = fields
                .get(field)
                .and_then(serde_json::Value::as_object)
                .into_iter()
                .flatten()
                .filter(|(_, version)| {
                    !version
                        .as_str()
                        .is_some_and(|version| version.starts_with("workspace:"))
                });
            for (dependency, _) in declared {
                keys.push(format!("{part}: {dependency} ({field})"));
            }
        }
    }
    let mut problems = Vec::new();
    compare_reviews(&count(keys), Extent::Complete, reviewed, &mut problems);
    verdict(
        &format!("direct dependencies must be reviewed in {DEPENDENCIES_REVIEWED}"),
        problems,
    )
}

/// The Dependabot configuration.
pub const DEPENDABOT: &str = ".github/dependabot.yml";

/// The pnpm workspace configuration.
pub const PNPM_WORKSPACE: &str = "pnpm-workspace.yaml";

/// The cargo-deny configuration.
pub const DENY_TOML: &str = "deny.toml";

/// The supply-chain settings xtask requires, by file: a week before a release is proposed
/// (each Dependabot ecosystem) or installable (pnpm, in minutes), and `libm` with no
/// feature, as the engine uses it.
pub const SUPPLY_CHAIN_SETTINGS: [(&str, &[&str]); 3] = [
    (DEPENDABOT, &["cooldown:", "default-days: 7"]),
    (PNPM_WORKSPACE, &["minimumReleaseAge: 10080"]),
    (
        DENY_TOML,
        &[
            "[[bans.features]]",
            "crate = \"libm\"",
            "exact = true",
            "allow = []",
        ],
    ),
];

/// Checks the supply-chain settings: each file of [`SUPPLY_CHAIN_SETTINGS`] holds its lines
/// in a row, Dependabot's once for each of its ecosystems.
///
/// `files` holds the text of each file, in the order of [`SUPPLY_CHAIN_SETTINGS`].
///
/// # Errors
///
/// Returns every file, or Dependabot ecosystem, without its settings.
pub fn supply_chain_gate(files: &[String]) -> Result<(), Failure> {
    let mut problems = Vec::new();
    for ((path, settings), text) in SUPPLY_CHAIN_SETTINGS.iter().zip(files) {
        let trimmed: String = text.lines().map(str::trim).collect::<Vec<_>>().join("\n");
        let blocks: Vec<&str> = if *path == DEPENDABOT {
            trimmed.split("- package-ecosystem:").skip(1).collect()
        } else {
            vec![trimmed.as_str()]
        };
        if blocks.is_empty() {
            problems.push(format!("{path} configures nothing"));
        }
        for block in blocks {
            if !holds_block(block, settings) {
                let name = block.lines().next().unwrap_or_default().trim();
                let place = if *path == DEPENDABOT {
                    format!("{path} ({name})")
                } else {
                    (*path).to_owned()
                };
                problems.push(format!("{place} lacks `{}`", settings.join(" ")));
            }
        }
    }
    verdict("the supply-chain settings", problems)
}

/// The Cargo configuration file.
pub const CARGO_CONFIG_FILE: &str = ".cargo/config.toml";

/// Checks that the Cargo configuration is exactly [`CARGO_CONFIG`].
///
/// # Errors
///
/// Returns that it differs, with both texts in the details.
pub fn cargo_config_gate(config: &str) -> Result<(), Failure> {
    if config == CARGO_CONFIG {
        Ok(())
    } else {
        Err(Failure {
            summary: format!(
                "{CARGO_CONFIG_FILE} differs from the configuration xtask expects (no build flag, a locked alias)"
            ),
            details: ["expected:", CARGO_CONFIG, "found:", config]
                .map(str::to_owned)
                .to_vec(),
        })
    }
}

/// The Cargo tools the checks run, at the exact version they need (as `cargo install`
/// names them): the preflight refuses another version, and the CI workflow installs these.
pub const PINNED_TOOLS: [(&str, &str); 3] = [
    ("cargo-llvm-cov", "0.9.1"),
    ("cargo-mutants", "27.1.0"),
    ("cargo-deny", "0.20.2"),
];

/// Which version of a tool the checks need.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// Any version: the tool only has to answer.
    Present,
    /// Exactly the version of this pin of [`PINNED_TOOLS`] (name, version): the second
    /// word of its answer, as in `cargo-mutants 27.1.0`.
    Pinned((&'static str, &'static str)),
    /// Node.js, exactly the release of [`NODE_VERSION_FILE`], which CI installs.
    NodeRelease,
}

/// How to install a tool, rendered from the constants it depends on so that a hint never
/// names another version than the one the checks need.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Install {
    /// `cargo install --locked <name>@<version>`, from a pin of [`PINNED_TOOLS`].
    Cargo((&'static str, &'static str)),
    /// Install the [`NIGHTLY_TOOLCHAIN`] with these components.
    Nightly(&'static [&'static str]),
    /// Add these components to the [`NIGHTLY_TOOLCHAIN`].
    NightlyComponents(&'static [&'static str]),
    /// Install the Node.js release of [`NODE_VERSION_FILE`].
    Node,
    /// What to do, in words.
    Text(&'static str),
}

impl core::fmt::Display for Install {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let nightly = NIGHTLY_TOOLCHAIN.trim_start_matches('+');
        match self {
            Self::Cargo((name, version)) => write!(f, "cargo install --locked {name}@{version}"),
            Self::Nightly(components) => {
                write!(f, "rustup toolchain install {nightly}")?;
                components
                    .iter()
                    .try_for_each(|component| write!(f, " --component {component}"))
            }
            Self::NightlyComponents(components) => write!(
                f,
                "rustup component add --toolchain {nightly} {}",
                components.join(" ")
            ),
            Self::Node => write!(
                f,
                "install the Node.js release of {NODE_VERSION_FILE} (fnm, nvm and volta read it)"
            ),
            Self::Text(text) => f.write_str(text),
        }
    }
}

/// A tool the checks run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tool {
    /// The program.
    pub program: &'static str,
    /// The arguments that make it print its version, proving it is installed.
    pub args: &'static [&'static str],
    /// Which version the checks need.
    pub need: Need,
    /// How to install it.
    pub install: Install,
}

/// The tools the checks run.
pub const TOOLS: [Tool; 9] = [
    Tool {
        program: "git",
        args: &["--version"],
        need: Need::Present,
        install: Install::Text("install git"),
    },
    Tool {
        program: "cargo",
        args: &["--version"],
        need: Need::Present,
        install: Install::Text(
            "install rustup (https://rustup.rs): the pinned toolchain installs itself",
        ),
    },
    Tool {
        program: "cargo",
        args: &[NIGHTLY_TOOLCHAIN, "--version"],
        need: Need::Present,
        install: Install::Nightly(&["llvm-tools-preview", "miri", "rust-src"]),
    },
    Tool {
        program: "cargo",
        args: &[NIGHTLY_TOOLCHAIN, "miri", "--version"],
        need: Need::Present,
        install: Install::NightlyComponents(&["miri", "rust-src"]),
    },
    Tool {
        program: "cargo",
        args: &["llvm-cov", "--version"],
        need: Need::Pinned(PINNED_TOOLS[0]),
        install: Install::Cargo(PINNED_TOOLS[0]),
    },
    Tool {
        program: "cargo",
        args: &["mutants", "--version"],
        need: Need::Pinned(PINNED_TOOLS[1]),
        install: Install::Cargo(PINNED_TOOLS[1]),
    },
    Tool {
        program: "cargo",
        args: &["deny", "--version"],
        need: Need::Pinned(PINNED_TOOLS[2]),
        install: Install::Cargo(PINNED_TOOLS[2]),
    },
    Tool {
        program: "node",
        args: &["--version"],
        need: Need::NodeRelease,
        install: Install::Node,
    },
    Tool {
        program: "pnpm",
        args: &["--version"],
        need: Need::Present,
        install: Install::Text("corepack enable (pnpm 10, as packageManager in package.json)"),
    },
];

/// Reads a Node.js release as `node --version` and `.node-version` write it: `X.Y.Z`, with
/// or without a leading `v`, nothing else (no pre-release, no partial version).
#[must_use]
pub fn node_release(text: &str) -> Option<Version> {
    let text = text.trim();
    Version::parse(text.strip_prefix('v').unwrap_or(text))
}

/// Whether a Node.js release is at [`NODE_FLOOR`] or later.
fn supported(release: Version) -> bool {
    (release.major, release.minor) >= NODE_FLOOR
}

/// The oldest Node.js release the packages support, major and minor: the `engines` of
/// every `package.json` is exactly `>=` this release (checked by [`release_gate`]).
pub const NODE_FLOOR: (u64, u64) = (22, 18);

/// Checks the platform and the tools the checks need.
///
/// `answers` holds what each of [`TOOLS`] answered: its version line, or why asking failed
/// (a missing program, a missing cargo subcommand or toolchain, a broken install). Every
/// tool must answer, the Cargo tools at their pinned version and Node.js at exactly the
/// release of `node_version` (the text of [`NODE_VERSION_FILE`], which CI installs: a
/// green local check means a green CI), on a platform the checks support (macOS or Linux;
/// Windows through WSL2).
///
/// # Errors
///
/// Returns one failure listing every problem found, each with how to fix it: an
/// unsupported platform, a tool that did not answer (with its own reason), or a tool at a
/// version the checks do not accept (Node.js included, which must be `node_version`).
pub fn preflight_gate(
    os: &str,
    answers: &[Result<String, CommandFailure>],
    node_version: &str,
) -> Result<(), Failure> {
    let mut problems = Vec::new();
    if os == "windows" {
        problems.push("the checks run on macOS and Linux: on Windows, run them in WSL2".to_owned());
    }
    for (tool, answer) in TOOLS.iter().zip(answers) {
        let install = tool.install;
        let version = match answer {
            Err(failure) => {
                problems.push(format!("{failure}; to install: {install}"));
                continue;
            }
            Ok(version) => version.trim(),
        };
        match tool.need {
            Need::Present => {}
            Need::NodeRelease => {
                let expected = node_release(node_version);
                if expected.is_none() || node_release(version) != expected {
                    problems.push(format!(
                        "Node.js {version} is installed, the checks need {}: {install}",
                        node_version.trim()
                    ));
                }
            }
            Need::Pinned((name, needed)) => {
                if version.split_whitespace().nth(1) != Some(needed) {
                    problems.push(format!(
                        "{version} is installed, the checks need {name} {needed}: {install}"
                    ));
                }
            }
        }
    }
    verdict("the checks cannot run here", problems)
}

/// The CI workflow.
pub const WORKFLOW: &str = ".github/workflows/quality.yml";

/// The Node.js release CI uses, pinned.
pub const NODE_VERSION_FILE: &str = ".node-version";

/// The WebAssembly module the engine package builds: uploaded by a failed CI run, so that
/// its bytes can be compared with a local build.
pub const WASM_MODULE: &str = "bindings/wasm/dist/engine.wasm";

/// What a failed CI run uploads.
///
/// Every report a gate of xtask reads (the mutants, the Rust coverage, the coverage and
/// mutation reports of every TypeScript package, as one glob per folder of parts) and the
/// WebAssembly module.
#[must_use]
pub fn failure_uploads() -> Vec<String> {
    let mut uploads: Vec<String> = [
        UNVIABLE_REPORT.split('/').next().unwrap_or(UNVIABLE_REPORT),
        COVERAGE_DIR,
        WASM_MODULE,
    ]
    .map(str::to_owned)
    .to_vec();
    for report in typescript_reports() {
        // `<folder>/<package>/<kind>[/…]` becomes `<folder>/*/<kind>`.
        let mut parts = report.split('/');
        let folder = parts.next().unwrap_or_default();
        let glob = format!("{folder}/*/{}", parts.nth(1).unwrap_or_default());
        if !uploads.contains(&glob) {
            uploads.push(glob);
        }
    }
    uploads
}

/// Workflow settings that would let CI pass without checking, or expose its token.
///
/// A step allowed to fail, triggers that run with write access on code from a fork,
/// secrets, and build flags that could lift lints.
pub const WORKFLOW_REFUSED: [&str; 5] = [
    "continue-on-error",
    "pull_request_target",
    "workflow_run",
    "secrets.",
    "RUSTFLAGS",
];

/// The exact triggers of the workflow: every push, every pull request, and by hand.
pub const WORKFLOW_TRIGGERS: [&str; 4] =
    ["on:", "  push:", "  pull_request:", "  workflow_dispatch:"];

/// The exact permissions of the workflow's token: reading the repository.
pub const WORKFLOW_PERMISSIONS: [&str; 2] = ["permissions:", "  contents: read"];

/// Whether the lines of `workflow` hold `block` as consecutive lines.
fn holds_block(workflow: &str, block: &[&str]) -> bool {
    let lines: Vec<&str> = workflow.lines().map(str::trim_end).collect();
    lines.windows(block.len()).any(|window| window == block)
}

/// The only conditions the workflow may hold, exactly.
///
/// The upload of the reports after a failure, and the maintainer's decision that a push runs
/// the check only when its last commit asks for it with `[ci]` (pull requests and manual
/// runs always run it). Any other condition, or a variant of these, could skip the check
/// without anyone deciding it.
pub const WORKFLOW_CONDITIONS: [&str; 2] = [
    "if: failure()",
    "if: github.event_name != 'push' || contains(github.event.head_commit.message, '[ci]')",
];

/// Checks that CI cannot change under the project's feet, nor pass without checking.
///
/// `workflows` are the workflow files tracked by git: only [`WORKFLOW`]. In it, every line
/// using an action pins it to a full commit hash, whatever its form (`- uses:`, a flow
/// mapping); nothing of [`WORKFLOW_REFUSED`] appears; the only conditions are those of
/// [`WORKFLOW_CONDITIONS`]; the triggers and the token's permissions are exactly
/// [`WORKFLOW_TRIGGERS`] and [`WORKFLOW_PERMISSIONS`]; every nightly toolchain named is
/// [`NIGHTLY_TOOLCHAIN`]; the checkout keeps no credential; the Cargo tools are
/// [`PINNED_TOOLS`]; Node.js is the release in `.node-version` (at [`NODE_FLOOR`] or later);
/// the runner is an OS release (never `-latest`); the whole history is fetched for the
/// history gate; the workflow runs `cargo xtask check --full`; its Rust cache is saved even when
/// the run fails; and a failed run uploads every [`failure_uploads`] path.
///
/// # Errors
///
/// Returns every unpinned action, refused setting and missing pin.
pub fn workflow_gate(
    workflows: &[String],
    workflow: &str,
    node_version: &str,
) -> Result<(), Failure> {
    let mut problems: Vec<String> = workflows
        .iter()
        .filter(|path| *path != WORKFLOW)
        .map(|path| format!("{path}: the only workflow is {WORKFLOW}, which xtask checks"))
        .collect();
    for line in workflow.lines().map(str::trim) {
        if let Some((_, used)) = line.split_once("uses:") {
            let action = used.trim().trim_end_matches(['}', ' ']);
            let reference = action
                .split_once('@')
                .map_or("", |(_, reference)| reference);
            let commit = reference
                .split([' ', '}', ',', '"', '\''])
                .next()
                .unwrap_or_default();
            if commit.len() != 40 || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                problems.push(format!("{action} is not pinned to a full commit hash"));
            }
        }
        problems.extend(
            WORKFLOW_REFUSED
                .iter()
                .filter(|refused| line.contains(*refused))
                .map(|refused| format!("`{line}`: `{refused}` is not allowed")),
        );
        if line.contains("if:") && !WORKFLOW_CONDITIONS.contains(&line) {
            problems.push(format!(
                "`{line}`: the only conditions are `{}`",
                WORKFLOW_CONDITIONS.join("` and `")
            ));
        }
        if line.starts_with("runs-on:") && line.ends_with("-latest") {
            problems.push(format!(
                "`{line}` moves to a new OS release without a commit: name the release"
            ));
        }
        let nightly = NIGHTLY_TOOLCHAIN.trim_start_matches('+');
        problems.extend(
            line.split(|character: char| character.is_whitespace() || character == ',')
                .map(|word| word.trim_start_matches('+'))
                .filter(|word| word.starts_with("nightly-") && *word != nightly)
                .map(|word| format!("`{word}` is not the pinned nightly, {nightly}")),
        );
    }
    for (block, what) in [
        (&WORKFLOW_TRIGGERS[..], "triggers"),
        (&WORKFLOW_PERMISSIONS[..], "permissions"),
    ] {
        if !holds_block(workflow, block) {
            problems.push(format!("{WORKFLOW} lacks the {what} `{}`", block.join(" ")));
        }
    }
    let declared = |key: &str| {
        workflow
            .lines()
            .filter(|line| line.trim_start().starts_with(key))
            .count()
    };
    if declared("on:") != 1 || declared("permissions:") != 1 {
        problems.push(format!(
            "{WORKFLOW} declares triggers and permissions once, for the whole workflow"
        ));
    }
    let tools = PINNED_TOOLS
        .iter()
        .map(|(name, version)| format!("{name}@{version}"))
        .collect::<Vec<_>>()
        .join(",");
    for expected in [
        format!("tool: {tools}"),
        "node-version-file: .node-version".to_owned(),
        "fetch-depth: 0".to_owned(),
        "persist-credentials: false".to_owned(),
        "run: cargo xtask check --full".to_owned(),
        "cache-on-failure: true".to_owned(),
    ]
    .into_iter()
    .chain(failure_uploads())
    {
        if !workflow.lines().any(|line| line.trim() == expected) {
            problems.push(format!("{WORKFLOW} lacks `{expected}`"));
        }
    }
    if !node_release(node_version).is_some_and(supported) {
        problems.push(format!(
            "{NODE_VERSION_FILE} must pin one Node.js release, {}.{} or later (X.Y.Z): found {:?}",
            NODE_FLOOR.0,
            NODE_FLOOR.1,
            node_version.trim()
        ));
    }
    verdict(
        &format!("{WORKFLOW} can change CI without a commit"),
        problems,
    )
}

/// The project manifest holding the workspace version.
pub const CARGO_TOML: &str = "Cargo.toml";

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
            let digits = !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
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
            .is_some_and(|part| part.len() == len && part.bytes().all(|byte| byte.is_ascii_digit()))
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
/// versions, and not older than the latest release in the changelog; every package manifest
/// supports Node.js from [`NODE_FLOOR`] on (`"engines": { "node": ">=22.18" }`). The changelog starts with
/// `# Changelog`, has `## [Unreleased]` first, then releases `## [X.Y.Z] - YYYY-MM-DD`
/// in strictly decreasing order.
///
/// # Errors
///
/// Returns a message listing every problem found.
pub fn release_gate(
    cargo_toml: &str,
    packages: &[(String, String)],
    changelog: &str,
) -> Result<(), Failure> {
    let mut problems = Vec::new();
    let workspace = workspace_version(cargo_toml);
    let version = workspace.and_then(Version::parse);
    if version.is_none() {
        problems.push(format!(
            "{CARGO_TOML}: [workspace.package] version must be X.Y.Z"
        ));
    }
    let floor = format!(">={}.{}", NODE_FLOOR.0, NODE_FLOOR.1);
    for (path, manifest) in packages {
        let json = serde_json::from_str::<serde_json::Value>(manifest).ok();
        let field = |pointer: &str| {
            json.as_ref()
                .and_then(|value| value.pointer(pointer))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        };
        let engine = field("/engines/node");
        if engine.as_deref() != Some(floor.as_str()) {
            problems.push(format!(
                "{path}: engines.node is {}, the packages support Node.js {floor} (NODE_FLOOR)",
                engine.as_deref().unwrap_or("(missing)")
            ));
        }
        let package = field("/version");
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
    let headings: Vec<&str> = changelog
        .lines()
        .filter(|line| line.starts_with("## "))
        .collect();
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
    verdict("the versions and the changelog", problems)
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
/// `message` is the file git hands the commit-msg hook: its lines starting with `#` are
/// git's comments, removed before the commit is recorded, and are ignored. Every message
/// follows the convention, those git writes for merges and reverts included (`Merge …`,
/// `Revert "…"`): write `revert: …`.
///
/// # Errors
///
/// Returns a message explaining the expected format.
pub fn commit_msg_gate(message: &str) -> Result<(), Failure> {
    let text: Vec<&str> = message
        .lines()
        .filter(|line| !line.starts_with('#'))
        .collect();
    let text = text.join("\n");
    conventional(&text)?;
    ci_marker(&text).map_or(Ok(()), |problem| Err(Failure::from(problem)))
}

/// The marker asking CI to run on a push: the workflow runs the check when the last
/// commit's message contains it (see [`WORKFLOW_CONDITIONS`]).
pub const CI_MARKER: &str = "[ci]";

/// Commits published before the marker was confined to a line of its own, whose message
/// names it in a sentence: their history is not rewritten.
pub const CI_MARKER_EXEMPT: [&str; 1] = ["baa21644eeb1ec9354743fe07ef69f21a173f685"];

/// Why `message` names [`CI_MARKER`] other than alone on a line, if it does.
///
/// GitHub only offers `contains` to read a commit message, so the marker in a sentence
/// (a commit describing the rule, say) would start CI unasked: it is refused anywhere but
/// on a line of its own, the one way to ask for CI.
#[must_use]
pub fn ci_marker(message: &str) -> Option<String> {
    message
        .lines()
        .any(|line| line.contains(CI_MARKER) && line.trim() != CI_MARKER)
        .then(|| {
            format!(
                "invalid commit message: {CI_MARKER} asks CI to run; write it alone on a line, or not at all"
            )
        })
}

/// Checks a recorded commit message, as it is, against the convention of
/// [`commit_msg_gate`].
fn conventional(message: &str) -> Result<(), Failure> {
    let mut lines = message.lines();
    let header = lines.next().unwrap_or("");
    let fail = |reason: &str| {
        Err(Failure {
            summary: format!("invalid commit message: {reason}"),
            details: vec![format!(
                "expected: <type>(<scope>)!: <summary>, type one of {}",
                COMMIT_TYPES.join(", ")
            )],
        })
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
    let scope_ok = scope.is_some_and(|name| {
        !name.is_empty()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
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

/// The last commit the history gate does not hold to the changelog rule: every change of
/// behavior after it must update `CHANGELOG.md` in the same commit.
///
/// The rule itself is older (9a32301), but it was enforced only when the gate came, after
/// this commit: this commit and 81509cf, before it, added their entries in later commits,
/// and the history is not rewritten to move them.
pub const CHANGELOG_SINCE: &str = "8182fda87a1c631295548ff52eeb4ea08335fa36";

/// The two history listings the history gate reads.
///
/// Every commit with its message, and the files each commit after `since` touched (`since`
/// itself excluded; the check passes [`CHANGELOG_SINCE`]). Records are separated by `0x1e`.
#[must_use]
pub fn history_commands(since: &str) -> [Cmd; 2] {
    let git = |args: &[&str]| Cmd::new("git", args, Dir::Root);
    [
        git(&["log", "--format=%x1e%H%x1f%B", "HEAD"]),
        git(&[
            "log",
            "--format=%x1e%H",
            "--name-only",
            &format!("{since}..HEAD"),
        ]),
    ]
}

/// Whether a commit message announces a change of behavior: a `feat`, `fix` or `perf`
/// commit, or any breaking one (`!` or a `BREAKING CHANGE:` footer).
#[must_use]
pub fn changes_behavior(message: &str) -> bool {
    let header = message.lines().next().unwrap_or_default();
    let prefix = header.split_once(": ").map_or(header, |(prefix, _)| prefix);
    let kind = prefix.split(['(', '!']).next().unwrap_or_default();
    ["feat", "fix", "perf"].contains(&kind)
        || prefix.ends_with('!')
        || message
            .lines()
            .any(|line| line.starts_with("BREAKING CHANGE:"))
}

/// Checks the history of commits.
///
/// Every commit message, as recorded, follows the convention ([`commit_msg_gate`]), and every commit
/// since [`CHANGELOG_SINCE`] that changes behavior touches `CHANGELOG.md`.
///
/// `messages` and `files` are the outputs of [`history_commands`].
///
/// # Errors
///
/// Returns one line per commit at fault, naming it by its short hash.
pub fn history_gate(messages: &str, files: &str) -> Result<(), Failure> {
    let short = |hash: &str| hash.chars().take(7).collect::<String>();
    let touched: BTreeMap<&str, Vec<&str>> = files
        .split('\u{1e}')
        .filter_map(|record| {
            let mut lines = record.lines();
            lines
                .next()
                .filter(|hash| !hash.is_empty())
                .map(|hash| (hash, lines.filter(|line| !line.is_empty()).collect()))
        })
        .collect();
    let mut problems = Vec::new();
    for record in messages.split('\u{1e}').filter(|record| !record.is_empty()) {
        let (hash, message) = record.split_once('\u{1f}').unwrap_or((record, ""));
        if let Err(failure) = conventional(message) {
            problems.push(format!("commit {}: {}", short(hash), failure.summary));
        }
        if let Some(problem) = ci_marker(message).filter(|_| !CI_MARKER_EXEMPT.contains(&hash)) {
            problems.push(format!("commit {}: {problem}", short(hash)));
        }
        let unrecorded = touched
            .get(hash)
            .is_some_and(|paths| !paths.contains(&CHANGELOG));
        if unrecorded && changes_behavior(message) {
            problems.push(format!(
                "commit {} changes behavior but not {CHANGELOG}",
                short(hash)
            ));
        }
    }
    verdict("the history breaks the commit rules", problems)
}

/// Checks that the pre-push hook validates exactly what is pushed.
///
/// The working tree has nothing uncommitted (`status`, from `git status --porcelain`), and
/// every commit pushed (`pushed`, each object the hook received peeled to its commit) is
/// `head`, the commit checked out. A deleted branch (all zeros) pushes nothing.
///
/// # Errors
///
/// Returns what to commit or check out first.
pub fn pre_push_gate(head: &str, status: &str, pushed: &[String]) -> Result<(), Failure> {
    let mut problems = Vec::new();
    if !status.trim().is_empty() {
        problems.push(format!(
            "the working tree has changes not committed, which the check would include: {}",
            status.lines().map(str::trim).collect::<Vec<_>>().join(", ")
        ));
    }
    for commit in pushed {
        if !is_deletion(commit) && commit != head.trim() {
            problems.push(format!(
                "pushing {commit}, but {} is checked out: check out the pushed commit, then push again",
                head.trim()
            ));
        }
    }
    verdict("the push cannot be checked", problems)
}

/// How a command that ran ended without success.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// It exited with this code (never 0).
    Code(i32),
    /// A signal stopped it: its number.
    Signal(i32),
}

/// Why a command did not succeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failed {
    /// The program could not be started (missing, not executable): the system's reason.
    NotStarted(String),
    /// It ran and ended unsuccessfully, with the line of its error output that says why
    /// when that output was captured ([`error_reason`]).
    Ended {
        /// How it ended.
        exit: Exit,
        /// What it said about it.
        reason: Option<String>,
    },
    /// It succeeded but wrote something other than UTF-8 text.
    NotText,
}

/// A command that did not succeed: its line and why. Displayed as one line naming the
/// command once, with its exit code or signal and its own reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandFailure {
    /// The command line ([`Cmd::line`]).
    pub line: String,
    /// Why it failed.
    pub failed: Failed,
}

impl CommandFailure {
    /// A failure of `cmd`.
    #[must_use]
    pub fn of(cmd: &Cmd, failed: Failed) -> Self {
        Self {
            line: cmd.line(),
            failed,
        }
    }
}

/// The usual names of the signals whose numbers POSIX fixes, the same on Linux and macOS.
const SIGNAL_NAMES: [(i32, &str); 8] = [
    (1, "SIGHUP"),
    (2, "SIGINT"),
    (3, "SIGQUIT"),
    (6, "SIGABRT"),
    (9, "SIGKILL"),
    (11, "SIGSEGV"),
    (13, "SIGPIPE"),
    (15, "SIGTERM"),
];

impl core::fmt::Display for CommandFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let line = &self.line;
        match &self.failed {
            Failed::NotStarted(error) => write!(f, "cannot run `{line}`: {error}"),
            Failed::NotText => write!(f, "`{line}` wrote output that is not UTF-8 text"),
            Failed::Ended { exit, reason } => {
                match exit {
                    Exit::Code(code) => write!(f, "`{line}` exited with code {code}")?,
                    Exit::Signal(signal) => {
                        write!(f, "`{line}` was stopped by signal {signal}")?;
                        if let Some((_, name)) =
                            SIGNAL_NAMES.iter().find(|(number, _)| number == signal)
                        {
                            write!(f, " ({name})")?;
                        }
                    }
                }
                reason
                    .as_ref()
                    .map_or(Ok(()), |reason| write!(f, ": {reason}"))
            }
        }
    }
}

/// The line of a command's error output that says why it failed.
///
/// The first line starting with `error` or `fatal` (cargo writes `help:` lines after its
/// error, rustup `info:` lines before it, git `fatal:`), otherwise the first non-empty
/// line; `None` when the command wrote nothing.
#[must_use]
pub fn error_reason(stderr: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(stderr);
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let first = lines.clone().next();
    lines
        .find(|line| line.starts_with("error") || line.starts_with("fatal"))
        .or(first)
        .map(str::to_owned)
}

/// How a process that did not succeed ended.
fn exit_of(status: std::process::ExitStatus) -> Exit {
    #[cfg(unix)]
    let signal = {
        use std::os::unix::process::ExitStatusExt as _;
        status.signal()
    };
    #[cfg(not(unix))]
    let signal = None;
    // On Unix a process that has no exit code was stopped by a signal.
    status
        .code()
        .map_or_else(|| Exit::Signal(signal.unwrap_or_default()), Exit::Code)
}

/// Executes commands and reads files on behalf of [`run_steps`].
pub trait Runner {
    /// Runs a command, its output going to the terminal.
    ///
    /// # Errors
    ///
    /// Returns why the command could not be started or did not succeed.
    fn run(&mut self, cmd: &Cmd) -> Result<(), CommandFailure>;

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
    /// Returns why git could not list them.
    fn tracked(&mut self, patterns: &[&str]) -> Result<Vec<String>, CommandFailure>;

    /// Runs a command and returns what it wrote on stdout.
    ///
    /// # Errors
    ///
    /// Returns why the command could not be started or did not succeed, with the reason
    /// it wrote on stderr.
    fn capture(&mut self, cmd: &Cmd) -> Result<String, CommandFailure>;

    /// Writes `text` to a file below the repository root, creating its directory if needed.
    ///
    /// # Errors
    ///
    /// Returns an error when the directory or the file cannot be written.
    fn write(&mut self, path: &str, text: &str) -> io::Result<()>;

    /// How many threads the machine can run at once (at least one).
    fn parallelism(&mut self) -> NonZeroUsize;
}

/// Runs real commands below a repository root.
#[derive(Debug, Clone)]
pub struct SystemRunner {
    /// The repository root.
    pub root: PathBuf,
}

/// The variables git sets for a hook that name one repository.
///
/// They are listed by `git rev-parse --local-env-vars`. A command inheriting them acts on
/// that repository whatever its working directory, so every command xtask runs is started
/// without them: its working directory alone says which repository it works on.
pub const LOCAL_GIT_VARIABLES: [&str; 15] = [
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_OBJECT_DIRECTORY",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_GRAFT_FILE",
    "GIT_INDEX_FILE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_REPLACE_REF_BASE",
    "GIT_PREFIX",
    "GIT_SHALLOW_FILE",
    "GIT_COMMON_DIR",
];

/// A command for `program` in `dir`, without the variables of [`LOCAL_GIT_VARIABLES`].
fn command(program: &str, dir: &Path) -> Command {
    let mut command = Command::new(program);
    let _in_dir = command.current_dir(dir);
    for variable in LOCAL_GIT_VARIABLES {
        let _without = command.env_remove(variable);
    }
    command
}

impl Runner for SystemRunner {
    fn run(&mut self, cmd: &Cmd) -> Result<(), CommandFailure> {
        let status = command(cmd.program, &cmd.dir.under(&self.root))
            .args(&cmd.args)
            .status()
            .map_err(|error| CommandFailure::of(cmd, Failed::NotStarted(error.to_string())))?;
        if status.success() {
            Ok(())
        } else {
            Err(CommandFailure::of(
                cmd,
                Failed::Ended {
                    exit: exit_of(status),
                    reason: None,
                },
            ))
        }
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

    fn capture(&mut self, cmd: &Cmd) -> Result<String, CommandFailure> {
        let output = command(cmd.program, &cmd.dir.under(&self.root))
            .args(&cmd.args)
            .output()
            .map_err(|error| CommandFailure::of(cmd, Failed::NotStarted(error.to_string())))?;
        if output.status.success() {
            String::from_utf8(output.stdout).map_err(|_| CommandFailure::of(cmd, Failed::NotText))
        } else {
            Err(CommandFailure::of(
                cmd,
                Failed::Ended {
                    exit: exit_of(output.status),
                    reason: error_reason(&output.stderr),
                },
            ))
        }
    }

    fn tracked(&mut self, patterns: &[&str]) -> Result<Vec<String>, CommandFailure> {
        let mut args = vec!["ls-files", "--"];
        args.extend(patterns);
        self.capture(&Cmd::new("git", &args, Dir::Root))
            .map(|text| text.lines().map(str::to_owned).collect())
    }

    fn write(&mut self, path: &str, text: &str) -> io::Result<()> {
        let file = self.root.join(path);
        file.parent().map_or(Ok(()), std::fs::create_dir_all)?;
        std::fs::write(file, text)
    }

    fn parallelism(&mut self) -> NonZeroUsize {
        std::thread::available_parallelism().unwrap_or(NonZeroUsize::MIN)
    }
}

/// Reads a file through `runner`, naming it when it cannot be read.
fn read_file(runner: &mut dyn Runner, path: &str) -> Result<String, Failure> {
    runner
        .read(path)
        .map_err(|error| Failure::from(format!("cannot read {path}: {error}")))
}

/// Captures what a git command run at the root writes, saying what could not be done
/// (`doing`: "read the work tree") when it fails.
fn git_output(runner: &mut dyn Runner, args: &[&str], doing: &str) -> Result<String, Failure> {
    runner
        .capture(&Cmd::new("git", args, Dir::Root))
        .map_err(|failure| Failure::from(format!("cannot {doing}: {failure}")))
}

/// Reads the commit checked out and the state of the working tree, then checks a push.
fn pre_push(runner: &mut dyn Runner, pushed: &[String]) -> Result<(), Failure> {
    let head = git_output(runner, &["rev-parse", "HEAD"], "read the work tree")?;
    let status = git_output(runner, &["status", "--porcelain"], "read the work tree")?;
    // Git gives the hook the object pushed: for an annotated tag, the tag object, which is
    // never the commit checked out. The gate compares the commit each object names.
    let commits = pushed
        .iter()
        .map(|object| {
            if is_deletion(object) {
                return Ok(object.clone());
            }
            git_output(
                runner,
                &["rev-parse", "--verify", &format!("{object}^{{commit}}")],
                &format!("resolve the pushed {object}"),
            )
            .map(|commit| commit.trim().to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    pre_push_gate(&head, &status, &commits)
}

/// Whether git pushes `object` to delete a ref: the hook then receives all zeros.
fn is_deletion(object: &str) -> bool {
    object.bytes().all(|byte| byte == b'0')
}

/// Reads the history, then checks it.
fn history(runner: &mut dyn Runner) -> Result<(), Failure> {
    let [messages, files] = history_commands(CHANGELOG_SINCE).map(|cmd| {
        runner
            .capture(&cmd)
            .map_err(|failure| Failure::from(format!("cannot read the history: {failure}")))
    });
    history_gate(&messages?, &files?)
}

/// Asks every tool of [`TOOLS`] for its version, then checks them.
fn preflight(runner: &mut dyn Runner) -> Result<(), Failure> {
    let answers: Vec<Result<String, CommandFailure>> = TOOLS
        .iter()
        .map(|tool| runner.capture(&Cmd::new(tool.program, tool.args, Dir::Root)))
        .collect();
    preflight_gate(
        std::env::consts::OS,
        &answers,
        &read_file(runner, NODE_VERSION_FILE)?,
    )
}

/// The files tracked by git matching `patterns`, described as `what` if they cannot be
/// listed.
fn list_tracked(
    runner: &mut dyn Runner,
    patterns: &[&str],
    what: &str,
) -> Result<Vec<String>, Failure> {
    runner
        .tracked(patterns)
        .map_err(|failure| Failure::from(format!("cannot list {what}: {failure}")))
}

/// Reads each of `paths`, with its path.
fn read_all(runner: &mut dyn Runner, paths: Vec<String>) -> Result<Vec<(String, String)>, Failure> {
    paths
        .into_iter()
        .map(|path| read_file(runner, &path).map(|text| (path, text)))
        .collect()
}

/// Lists the manifests tracked by git, then checks them.
fn membership(runner: &mut dyn Runner) -> Result<(), Failure> {
    let packages = list_tracked(runner, &["*package.json"], "the *package.json files")?;
    let ignores = list_tracked(runner, &["*.gitignore"], "the *.gitignore files")?;
    let manifests = list_tracked(runner, &["*Cargo.toml"], "the *Cargo.toml files")?;
    membership_gate(&packages, &read_all(runner, manifests)?, &ignores)
}

/// Lists the parts (the packages and the crates) and the READMEs tracked by git, then checks
/// that every part is described.
fn readmes(runner: &mut dyn Runner) -> Result<(), Failure> {
    let readmes = list_tracked(runner, &["*README.md"], "the *README.md files")?;
    let readmes = read_all(runner, readmes)?;
    let parts: Vec<String> = PACKAGES
        .iter()
        .map(|package| package.dir.to_owned())
        .chain(RUST_CRATES.iter().map(|krate| krate.dir.to_owned()))
        .collect();
    readme_gate(&parts, &readmes)
}

/// Checks that every part is described where a reader looks for it.
///
/// `parts` are the directories of the TypeScript packages and of the crates (`apps/roadmap`,
/// `crates/engine-core`); `readmes` the tracked `README.md` files with their text. Each
/// TypeScript package has its own README; each part is named, in backquotes, in the README
/// of the folder that holds it (the parts table of `crates/README.md`, `apps/README.md` …),
/// and each such folder is named in the root README. Crates are described by their crate
/// documentation, which rustdoc requires.
///
/// # Errors
///
/// Returns every missing README and every part a README does not name.
pub fn readme_gate(parts: &[String], readmes: &[(String, String)]) -> Result<(), Failure> {
    let text = |path: &str| {
        readmes
            .iter()
            .find(|(readme, _)| readme == path)
            .map(|(_, content)| content.as_str())
    };
    let mut problems = Vec::new();
    for package in PACKAGES {
        if text(&format!("{}/README.md", package.dir)).is_none() {
            problems.push(format!("{} has no README.md", package.dir));
        }
    }
    let mut folders = BTreeSet::new();
    for part in parts {
        let Some((folder, name)) = part.rsplit_once('/') else {
            problems.push(format!(
                "{part} is not inside a folder of parts (apps/, crates/ …)"
            ));
            continue;
        };
        let first = folders.insert(folder);
        match text(&format!("{folder}/README.md")) {
            // Said once per folder, not once per part it holds.
            None if first => problems.push(format!("{folder} has no README.md")),
            Some(content) if !content.contains(&format!("`{name}")) => {
                problems.push(format!("{folder}/README.md does not name {name}"));
            }
            None | Some(_) => {}
        }
    }
    let root = text("README.md").unwrap_or_default();
    problems.extend(
        folders
            .into_iter()
            .filter(|folder| !root.contains(&format!("`{folder}/`")))
            .map(|folder| format!("README.md does not name {folder}/")),
    );
    verdict("parts not described", problems)
}

/// Reads what each TypeScript package measures and mutates, then checks that every source
/// is in both, or reviewed.
fn scope(runner: &mut dyn Runner) -> Result<(), Failure> {
    let tracked = list_tracked(runner, &TS_SOURCES, "the TypeScript sources")?;
    let mut scopes = Vec::new();
    for package in PACKAGES {
        let [coverage, _] = package.reports();
        scopes.push(Scope {
            dir: package.dir,
            stryker: read_file(runner, &format!("{}/stryker.config.json", package.dir))?,
            summary: read_file(runner, &format!("{coverage}/coverage-summary.json"))?,
        });
    }
    scope_gate(&tracked, &scopes, &read_file(runner, BYPASSES_REVIEWED)?)
}

/// The reviewed vocabulary: the synonyms `GLOSSARY.md` replaces, each with the word to use
/// instead, where it is refused and the contexts where it is allowed ([`vocabulary_gate`]).
pub const VOCABULARY: &str = "vocabulary.txt";

/// The tracked files whose prose [`vocabulary_gate`] reads (git pathspecs): Markdown, and
/// the comments and string literals of Rust and TypeScript.
pub const VOCABULARY_SOURCES: [&str; 7] =
    ["*.md", "*.rs", "*.ts", "*.tsx", "*.mts", "*.js", "*.mjs"];

/// The language of a file, which decides what of it is prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Language {
    /// Markdown: everything but code blocks, code spans and link targets.
    Markdown,
    /// Rust: comments and string literals, format placeholders apart.
    Rust,
    /// TypeScript or JavaScript: comments and string literals, template substitutions
    /// apart.
    Script,
}

/// The language of `path`, by its extension; `None` for a file without prose to read.
fn language(path: &str) -> Option<Language> {
    match path.rsplit_once('.').map(|(_, extension)| extension) {
        Some("md") => Some(Language::Markdown),
        Some("rs") => Some(Language::Rust),
        Some("ts" | "tsx" | "mts" | "js" | "mjs") => Some(Language::Script),
        _ => None,
    }
}

/// Collects prose while lexing a source: each line of it with its line number.
#[derive(Debug, Default)]
struct Prose {
    /// The lines of prose found so far: (line number from 1, text).
    lines: Vec<(usize, String)>,
    /// The line being lexed, from 1.
    line: usize,
    /// The prose of the current line, not pushed yet.
    current: String,
}

impl Prose {
    /// Ends the current piece of prose: pushes it unless it is blank.
    fn flush(&mut self) {
        let text = core::mem::take(&mut self.current);
        if !text.trim().is_empty() {
            self.lines.push((self.line, text));
        }
    }

    /// Adds a character of prose; a line break ends the line.
    fn push(&mut self, character: char) {
        if character == '\n' {
            self.flush();
            self.line = self.line.saturating_add(1);
        } else {
            self.current.push(character);
        }
    }

    /// Passes over a character of code: only its line break counts.
    fn skip(&mut self, character: char) {
        if character == '\n' {
            self.flush();
            self.line = self.line.saturating_add(1);
        }
    }

    /// Adds a string literal: one whose whole text is a single word is a name (a program,
    /// a key, an identifier), not prose; so are Rust's format placeholders (`{name}`).
    fn literal(&mut self, text: &str, rust: bool) {
        let word = !text.is_empty()
            && text
                .chars()
                .all(|character| character.is_alphanumeric() || "_-~.:/@".contains(character));
        if word {
            for character in text.chars() {
                self.skip(character);
            }
        } else {
            self.flush();
            let mut depth = 0_usize;
            for character in text.chars() {
                match character {
                    '{' if rust => depth = depth.saturating_add(1),
                    '}' if rust && depth > 0 => depth = depth.saturating_sub(1),
                    _ if depth > 0 && !character.is_whitespace() => {}
                    _ => self.push(character),
                }
            }
            self.flush();
        }
    }
}

/// Reads the prose of a Rust or TypeScript source, one character at a time.
#[derive(Debug)]
struct Lexer {
    /// The source.
    chars: Vec<char>,
    /// The next character to read.
    index: usize,
    /// Whether the source is Rust (else TypeScript or JavaScript).
    rust: bool,
    /// The prose read so far.
    prose: Prose,
}

impl Lexer {
    /// The character `offset` after the next one to read.
    fn at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.index.saturating_add(offset)).copied()
    }

    /// The characters from `start` (an index) to `end`, or to the end of the source.
    fn span(&self, start: usize, end: usize) -> &[char] {
        let end = end.min(self.chars.len());
        self.chars.get(start.min(end)..end).unwrap_or_default()
    }

    /// The index of the first occurrence of `pattern` from `start` on, or the end of the
    /// source when there is none.
    fn find(&self, start: usize, pattern: &[char]) -> usize {
        self.span(start, usize::MAX)
            .windows(pattern.len())
            .position(|window| window == pattern)
            .map_or(self.chars.len(), |found| start.saturating_add(found))
    }

    /// Reads the characters up to `end` (an index) as code: only their line breaks count.
    fn skip_to(&mut self, end: usize) {
        let end = end.min(self.chars.len());
        for character in self.span(self.index, end).to_vec() {
            self.prose.skip(character);
        }
        self.index = end;
    }

    /// Reads the characters from `start` to `end` (indices) as prose, then goes past
    /// `after` more characters.
    fn read_prose(&mut self, start: usize, end: usize, after: usize) {
        for character in self.span(start, end).to_vec() {
            self.prose.push(character);
        }
        self.prose.flush();
        self.index = end.saturating_add(after).min(self.chars.len());
    }

    /// Reads a line comment, `//` included, as prose.
    fn line_comment(&mut self) {
        let end = self.find(self.index, &['\n']);
        self.read_prose(self.index, end, 0);
    }

    /// Reads a block comment, `/*` and `*/` apart, as prose.
    fn block_comment(&mut self) {
        let start = self.index.saturating_add(2);
        let end = self.find(start, &['*', '/']);
        self.read_prose(start, end, 2);
    }

    /// Reads a Rust raw string (`r"…"`, `r#"…"#`) if one starts here.
    fn raw_string(&mut self) -> bool {
        let hashes = self
            .span(self.index.saturating_add(1), usize::MAX)
            .iter()
            .take_while(|&&hash| hash == '#')
            .count();
        let start = self.index.saturating_add(hashes).saturating_add(2);
        if self.chars.get(start.saturating_sub(1)) != Some(&'"') {
            return false;
        }
        let closing: Vec<char> = core::iter::once('"')
            .chain(core::iter::repeat_n('#', hashes))
            .collect();
        let end = self.find(start, &closing);
        let literal: String = self.span(start, end).iter().collect();
        self.prose.literal(&literal, true);
        self.index = end.saturating_add(closing.len()).min(self.chars.len());
        true
    }

    /// Reads a Rust character literal (`'a'`, `'\n'`, `'\u{22}'`, `'"'`) or a lifetime
    /// (`'a`): code.
    fn quote_mark(&mut self) {
        let end = match (self.at(1), self.at(2)) {
            // An escape: up to the quote after the escaped character.
            (Some('\\'), _) => self
                .find(self.index.saturating_add(3), &['\''])
                .saturating_add(1),
            (Some(_), Some('\'')) => self.index.saturating_add(3),
            _ => self.index.saturating_add(1),
        };
        self.skip_to(end);
    }

    /// Reads a template substitution (`${…}`), from its `$`, up to its matching brace:
    /// code, of which it returns the line breaks.
    fn substitution(&mut self) -> String {
        let mut depth = 0_usize;
        let mut breaks = String::new();
        let rest = self.span(self.index, usize::MAX).to_vec();
        let mut used = rest.len();
        for (offset, &character) in rest.iter().enumerate() {
            match character {
                '\n' => breaks.push('\n'),
                '{' => depth = depth.saturating_add(1),
                '}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        used = offset.saturating_add(1);
                        break;
                    }
                }
                _ => {}
            }
        }
        self.index = self.index.saturating_add(used);
        breaks
    }

    /// Reads a string literal delimited by `quote`, as prose.
    fn string(&mut self, quote: char) {
        let mut literal = String::new();
        self.index = self.index.saturating_add(1);
        while let Some(&character) = self.chars.get(self.index) {
            self.index = self.index.saturating_add(1);
            if character == quote {
                break;
            }
            if character == '\\' {
                // An escaped quote stays a quote, an escaped line break a line break; any
                // other escape is no prose.
                let escaped = self.chars.get(self.index).copied().unwrap_or(' ');
                literal.push(if matches!(escaped, '"' | '\'' | '\n') {
                    escaped
                } else {
                    ' '
                });
                self.index = self.index.saturating_add(1);
            } else if quote == '`' && character == '$' && self.chars.get(self.index) == Some(&'{') {
                literal.push(' ');
                literal.push_str(&self.substitution());
            } else {
                literal.push(character);
            }
        }
        self.prose.literal(&literal, self.rust);
    }
}

/// The prose of a Rust or TypeScript source: its comments and string literals, each line
/// with its number.
fn code_prose(text: &str, language: Language) -> Vec<(usize, String)> {
    let rust = language == Language::Rust;
    let mut lexer = Lexer {
        chars: text.chars().collect(),
        index: 0,
        rust,
        prose: Prose {
            line: 1,
            ..Prose::default()
        },
    };
    while let Some(&character) = lexer.chars.get(lexer.index) {
        let start = lexer.index;
        // Inside a name, `r` starts no raw string; after a lone `b`, a raw byte string.
        let before = |back: usize| {
            start
                .checked_sub(back)
                .and_then(|at| lexer.chars.get(at))
                .is_some_and(|&named| named.is_alphanumeric() || named == '_')
        };
        let byte = start.checked_sub(1).and_then(|at| lexer.chars.get(at)) == Some(&'b');
        let identifier = before(1) && (!byte || before(2));
        match (character, lexer.at(1)) {
            ('/', Some('/')) => lexer.line_comment(),
            ('/', Some('*')) => lexer.block_comment(),
            ('r', Some(after))
                if matches!(after, '"' | '#') && rust && !identifier && lexer.raw_string() => {}
            ('"', _) => lexer.string('"'),
            ('\'' | '`', _) if !rust => lexer.string(character),
            // Only Rust gets here with a quote mark.
            ('\'', _) => lexer.quote_mark(),
            _ => lexer.skip_to(start.saturating_add(1)),
        }
        // Every step reads at least one character, so that lexing always ends.
        lexer.index = lexer.index.max(start.saturating_add(1));
    }
    lexer.prose.flush();
    lexer.prose.lines
}

/// The prose of a Markdown file: every line outside fenced code blocks.
fn markdown_prose(text: &str) -> Vec<(usize, String)> {
    let mut fenced = false;
    let mut lines = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        } else if !fenced {
            lines.push((index.saturating_add(1), line.to_owned()));
        }
    }
    lines
}

/// `text` without its code spans (between backquotes) and link targets (`](…)`): code and
/// addresses, not prose.
fn without_code(text: &str) -> String {
    let mut kept = String::new();
    let mut code = false;
    let mut target = false;
    let mut previous = ' ';
    for character in text.chars() {
        match character {
            '`' if !target => {
                code = !code;
                kept.push(' ');
            }
            '(' if !code && previous == ']' => {
                target = true;
                kept.push(' ');
            }
            ')' if target => {
                target = false;
                kept.push(' ');
            }
            _ if code || target => kept.push(' '),
            _ => kept.push(character),
        }
        previous = character;
    }
    kept
}

/// Whether `character` continues a word or a name around an occurrence: then the
/// occurrence is part of a name (`engine-objects`, `.node-version`, `nodes_by_id`), not a
/// word of prose.
fn joins(character: char) -> bool {
    character.is_alphanumeric() || "_-/~@#$\\".contains(character)
}

/// The byte offsets in `text` of every occurrence of `term` as a word of prose: not inside
/// a longer word or name, not quoted alone (`"nodes"`, a key), not followed by `.` or `:`
/// and a letter (`Node.js`, `node:fs`).
fn occurrences(text: &str, term: &str) -> Vec<usize> {
    text.match_indices(term)
        .map(|(start, _)| start)
        .filter(|&start| {
            let before = text.get(..start).and_then(|head| head.chars().next_back());
            let mut after = text
                .get(start.saturating_add(term.len())..)
                .unwrap_or_default()
                .chars();
            let (next, then) = (after.next(), after.next());
            let quoted = matches!(
                (before, next),
                (Some('"'), Some('"')) | (Some('\''), Some('\''))
            );
            let dotted = matches!(next, Some('.' | ':')) && then.is_some_and(char::is_alphanumeric);
            !before.is_some_and(|character| joins(character) || character == '.')
                && !next.is_some_and(joins)
                && !quoted
                && !dotted
        })
        .collect()
}

/// One rule of [`VOCABULARY`].
#[derive(Debug, Clone, PartialEq, Eq)]
enum Wording {
    /// `forbid <term> -> <instead>[ in <path>…] -- <reason>`: `term` is refused (in the
    /// files under `paths` only, when given), `instead` said in its place.
    Forbid {
        /// The word or phrase refused, matched case-sensitively as a word of prose.
        term: String,
        /// What to say instead.
        instead: String,
        /// Where the rule applies (a file, or a directory ending with `/`); everywhere
        /// when empty.
        paths: Vec<String>,
        /// Why.
        reason: String,
    },
    /// `allow <term> in <path> -- <reason>`: `term` is allowed in that file or directory.
    InPath {
        /// The refused word or phrase allowed here.
        term: String,
        /// A file, or a directory ending with `/`.
        path: String,
    },
    /// `allow <term> as "<phrase>" -- <reason>`: `term` is allowed inside `phrase`.
    InPhrase {
        /// The refused word or phrase allowed here.
        term: String,
        /// A phrase holding `term`.
        phrase: String,
    },
}

/// Whether `path` is `scope`, or under it when `scope` is a directory (ending with `/`).
fn within(path: &str, scope: &str) -> bool {
    if scope.ends_with('/') {
        path.starts_with(scope)
    } else {
        path == scope
    }
}

/// Reads the rules of [`VOCABULARY`], adding a problem for every line it cannot read.
fn wordings(vocabulary: &str, problems: &mut Vec<String>) -> Vec<Wording> {
    let mut rules = Vec::new();
    for (index, line) in vocabulary.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let number = index.saturating_add(1);
        let rule = line.split_once(" -- ").and_then(|(rule, reason)| {
            let reason = reason.trim().to_owned();
            if let Some(forbid) = rule.strip_prefix("forbid ") {
                let (term, rest) = forbid.split_once(" -> ")?;
                let (instead, paths) = rest.split_once(" in ").unwrap_or((rest, ""));
                Some(Wording::Forbid {
                    term: term.trim().to_owned(),
                    instead: instead.trim().to_owned(),
                    paths: paths.split_whitespace().map(str::to_owned).collect(),
                    reason,
                })
            } else {
                let allow = rule.strip_prefix("allow ")?;
                if let Some((term, phrase)) = allow.split_once(" as ") {
                    let phrase = phrase.trim().strip_prefix('"')?.strip_suffix('"')?;
                    Some(Wording::InPhrase {
                        term: term.trim().to_owned(),
                        phrase: phrase.to_owned(),
                    })
                } else {
                    let (term, path) = allow.split_once(" in ")?;
                    Some(Wording::InPath {
                        term: term.trim().to_owned(),
                        path: path.trim().to_owned(),
                    })
                }
            }
        });
        match rule {
            Some(rule) => rules.push(rule),
            None => problems.push(format!(
                "{VOCABULARY}:{number}: write `forbid <term> -> <instead>[ in <path>…] -- <reason>`, \
                 `allow <term> in <path> -- <reason>` or `allow <term> as \"<phrase>\" -- <reason>`"
            )),
        }
    }
    rules
}

/// Checks that the prose of the repository uses the words of the glossary.
///
/// `sources` are the tracked files of [`VOCABULARY_SOURCES`] (path and text); their prose
/// is the text of Markdown outside code blocks, and the comments and string literals of
/// Rust and TypeScript, code spans (between backquotes), link targets, Rust format
/// placeholders, template substitutions and literals of a single word (a key, a program
/// name) apart. `vocabulary` ([`VOCABULARY`]) lists the refused synonyms, each with what to
/// say instead and where it applies, and the contexts where one is allowed: a file or a
/// directory, or a phrase holding it. A term is matched case-sensitively, as a whole word,
/// never inside a name (`engine-objects`, `.node-version`, `Node.js`). Every allowance must
/// still allow something, so that the list stays current.
///
/// # Errors
///
/// Returns every refused term found, as `<file>:<line>: "<term>": say "<instead>"
/// (<reason>)`, every malformed line, every allowance of a term no rule refuses, and every
/// allowance that allows nothing.
pub fn vocabulary_gate(sources: &[(String, String)], vocabulary: &str) -> Result<(), Failure> {
    let mut problems = Vec::new();
    let rules = wordings(vocabulary, &mut problems);
    let forbidden: Vec<(&str, &str, &[String], &str)> = rules
        .iter()
        .filter_map(|rule| match rule {
            Wording::Forbid {
                term,
                instead,
                paths,
                reason,
            } => Some((
                term.as_str(),
                instead.as_str(),
                paths.as_slice(),
                reason.as_str(),
            )),
            Wording::InPath { .. } | Wording::InPhrase { .. } => None,
        })
        .collect();
    let mut used = vec![false; rules.len()];
    for (path, text) in sources {
        let Some(language) = language(path) else {
            continue;
        };
        let prose = match language {
            Language::Markdown => markdown_prose(text),
            Language::Rust | Language::Script => code_prose(text, language),
        };
        for (number, line) in &prose {
            let line = without_code(line);
            for &(term, instead, paths, reason) in &forbidden {
                if !paths.is_empty() && !paths.iter().any(|scope| within(path, scope)) {
                    continue;
                }
                for start in occurrences(&line, term) {
                    let end = start.saturating_add(term.len());
                    let allowed = rules.iter().position(|rule| match rule {
                        Wording::InPath {
                            term: allowed,
                            path: scope,
                        } => allowed == term && within(path, scope),
                        Wording::InPhrase {
                            term: allowed,
                            phrase,
                        } => {
                            allowed == term
                                && line.match_indices(phrase.as_str()).any(|(from, _)| {
                                    from <= start && end <= from.saturating_add(phrase.len())
                                })
                        }
                        Wording::Forbid { .. } => false,
                    });
                    match allowed.and_then(|index| used.get_mut(index)) {
                        Some(flag) => *flag = true,
                        None => problems.push(format!(
                            "{path}:{number}: \"{term}\": say \"{instead}\" ({reason})"
                        )),
                    }
                }
            }
        }
    }
    for (rule, &allowed) in rules.iter().zip(&used) {
        let (term, context) = match rule {
            Wording::InPath { term, path } => (term, format!("in {path}")),
            Wording::InPhrase { term, phrase } => (term, format!("as \"{phrase}\"")),
            Wording::Forbid { .. } => continue,
        };
        if !forbidden.iter().any(|(refused, ..)| refused == term) {
            problems.push(format!(
                "allow {term} {context}: no rule refuses \"{term}\""
            ));
        } else if !allowed {
            problems.push(format!("stale: allow {term} {context}: it allows nothing"));
        }
    }
    verdict(
        &format!("words the glossary replaces (reviewed in {VOCABULARY})"),
        problems,
    )
}

/// Lists the licenses of what ships and of everything, then checks them.
fn licenses(runner: &mut dyn Runner) -> Result<(), Failure> {
    let [shipped, all] = license_commands().map(|cmd| {
        runner
            .capture(&cmd)
            .map_err(|failure| Failure::from(format!("cannot list licenses: {failure}")))
    });
    license_gate(&shipped?, &all?)
}

/// The merge base of `HEAD` with the first of [`MUTATION_BASES`] git resolves, and that
/// base's name.
///
/// # Errors
///
/// Returns that none could be resolved, with why each could not.
fn mutation_base(runner: &mut dyn Runner) -> Result<(String, &'static str), Failure> {
    let mut tried = Vec::new();
    for against in MUTATION_BASES {
        match runner.capture(&Cmd::new(
            "git",
            &["merge-base", "HEAD", against],
            Dir::Root,
        )) {
            Ok(base) => return Ok((base.trim().to_owned(), against)),
            Err(failure) => tried.push(failure.to_string()),
        }
    }
    Err(Failure {
        summary: format!(
            "no base to measure the changes against: HEAD shares no history with any of {} \
             (run `cargo xtask check --full`)",
            MUTATION_BASES.join(", ")
        ),
        details: tried,
    })
}

/// The diff git would show for `path`, a new file holding `text`, added whole; nothing for
/// an empty file, which holds no mutant.
#[must_use]
pub fn new_file_diff(path: &str, text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let mut diff = format!(
        "diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{} @@\n",
        text.lines().count()
    );
    for line in text.lines() {
        diff.push('+');
        diff.push_str(line);
        diff.push('\n');
    }
    if !text.ends_with('\n') {
        diff.push_str("\\ No newline at end of file\n");
    }
    diff
}

/// Runs cargo-mutants on what changed since the merge base ([`Action::RustMutantsOfChanges`]):
/// the changes of the tracked files, committed or not, and every new Rust file git does not
/// ignore; nothing when nothing changed.
fn rust_mutants_of_changes(
    runner: &mut dyn Runner,
    out: &mut dyn Write,
    jobs: NonZeroUsize,
) -> Result<(), Failure> {
    let (base, against) = mutation_base(runner)?;
    let mut diff = git_output(
        runner,
        &[
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-renames",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            &base,
        ],
        "read the changes",
    )?;
    let new = git_output(
        runner,
        &["ls-files", "--others", "--exclude-standard", "--", "*.rs"],
        "list the new Rust files",
    )?;
    for path in new.lines() {
        diff.push_str(&new_file_diff(path, &read_file(runner, path)?));
    }
    if diff.is_empty() {
        return say(
            out,
            &format!("  no change since {base} (merge base with {against}): no mutant to test"),
        );
    }
    say(
        out,
        &format!("  the changes since {base} (merge base with {against})"),
    )?;
    runner
        .write(CHANGES_DIFF, &diff)
        .map_err(|error| Failure::from(format!("cannot write {CHANGES_DIFF}: {error}")))?;
    runner
        .run(&changes_mutants_command(jobs))
        .map_err(|failure| Failure::from(failure.to_string()))
}

/// The mutants disabled by a well-formed comment in the tracked TypeScript sources, which
/// [`ts_mutation_gate`] accepts as ignored.
///
/// # Errors
///
/// Returns why the sources cannot be listed or read.
fn disabled_mutants(runner: &mut dyn Runner) -> Result<Vec<Disable>, Failure> {
    let paths = list_tracked(runner, &["*.ts", "*.tsx"], "the TypeScript files")?;
    let (found, _) = disables(&read_all(runner, paths)?);
    Ok(found.into_iter().map(|(disable, _)| disable).collect())
}

/// The unviable mutants cargo-mutants reported ([`UNVIABLE_REPORT`]) in a run of `extent`.
///
/// cargo-mutants writes no report when the changes hold no mutant, and its report directory
/// was emptied just before it ran: after a partial run, a missing report means none found.
/// A complete run always writes it.
///
/// # Errors
///
/// Returns why the report cannot be read.
fn unviable_report(runner: &mut dyn Runner, extent: Extent) -> Result<String, Failure> {
    match (runner.read(UNVIABLE_REPORT), extent) {
        (Err(error), Extent::Partial) if error.kind() == io::ErrorKind::NotFound => {
            Ok(String::new())
        }
        (read, _) => {
            read.map_err(|error| Failure::from(format!("cannot read {UNVIABLE_REPORT}: {error}")))
        }
    }
}

/// Runs one action of a step, saying on `out` what it found when that matters.
fn run_action(
    action: &Action,
    runner: &mut dyn Runner,
    out: &mut dyn Write,
    tree: &mut Option<TreeState>,
) -> Result<(), Failure> {
    match action {
        Action::TreeRecorded => {
            *tree = Some(tree_state(runner)?);
            Ok(())
        }
        Action::TreeUnchanged => tree_gate(tree.as_ref(), &tree_state(runner)?),
        Action::Run(cmd) => runner
            .run(cmd)
            .map_err(|failure| Failure::from(failure.to_string())),
        Action::Coverage(coverage) => coverage_gate(&measure(runner, *coverage)?, *coverage),
        Action::CoverageLocality => coverage_locality(runner),
        Action::VersionGate => {
            let cargo = read_file(runner, CARGO_TOML)?;
            let packages = read_all(runner, package_jsons())?;
            release_gate(&cargo, &packages, &read_file(runner, CHANGELOG)?)
        }
        Action::FreshDirs(paths) => paths.iter().try_for_each(|path| fresh(runner, path)),
        Action::MembershipGate => membership(runner),
        Action::ReadmeGate => readmes(runner),
        Action::VocabularyGate => {
            let paths = list_tracked(runner, &VOCABULARY_SOURCES, "the prose files")?;
            let sources = read_all(runner, paths)?;
            vocabulary_gate(&sources, &read_file(runner, VOCABULARY)?)
        }
        Action::Preflight => preflight(runner),
        Action::HistoryGate => history(runner),
        Action::WorkflowGate => {
            let workflows: Vec<String> =
                list_tracked(runner, &["*.yml", "*.yaml"], "the YAML files")?
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
        Action::UnsafeGate => {
            let paths = list_tracked(runner, &["*.rs"], "the Rust files")?;
            unsafe_gate(&read_all(runner, paths)?)
        }
        Action::LicenseGate => licenses(runner),
        Action::CargoConfigGate => cargo_config_gate(&read_file(runner, CARGO_CONFIG_FILE)?),
        Action::SupplyChainGate => supply_chain_gate(
            &SUPPLY_CHAIN_SETTINGS
                .iter()
                .map(|(path, _)| read_file(runner, path))
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Action::DependencyGate => {
            let cmd = metadata_command();
            let metadata = runner
                .capture(&cmd)
                .map_err(|failure| Failure::from(format!("cannot list the crates: {failure}")))?;
            let packages = read_all(runner, package_jsons())?;
            dependency_gate(
                &metadata,
                &packages,
                &read_file(runner, DEPENDENCIES_REVIEWED)?,
            )
        }
        Action::TypeScriptReportsGate => {
            let disabled = disabled_mutants(runner)?;
            PACKAGES.iter().try_for_each(|package| {
                let [coverage, mutation] = package.reports();
                let summary = read_file(runner, &format!("{coverage}/coverage-summary.json"))?;
                ts_coverage_gate(package.dir, &summary)?;
                let report = read_file(runner, &format!("{mutation}/mutation.json"))?;
                ts_mutation_gate(package.dir, &report, &disabled, Extent::Complete)
            })
        }
        Action::TypeScriptMutationGate { package, extent } => {
            let disabled = disabled_mutants(runner)?;
            let [_, mutation] = package.reports();
            let report = read_file(runner, &format!("{mutation}/mutation.json"))?;
            ts_mutation_gate(package.dir, &report, &disabled, *extent)
        }
        Action::TypeScriptScopeGate => scope(runner),
        Action::StrykerConfigGate => PACKAGES.iter().try_for_each(|package| {
            let path = format!("{}/stryker.config.json", package.dir);
            stryker_config_gate(package.dir, &read_file(runner, &path)?)
        }),
        Action::BypassGate => {
            let paths = list_tracked(runner, &BYPASS_SOURCES, "the source files")?;
            let sources = read_all(runner, paths)?;
            bypass_gate(&sources, &read_file(runner, BYPASSES_REVIEWED)?)
        }
        Action::EquivalentGate => {
            let paths = list_tracked(runner, &["*.ts", "*.tsx"], "the TypeScript files")?;
            let sources = read_all(runner, paths)?;
            equivalent_gate(&sources, &read_file(runner, EQUIVALENT_REVIEWED)?)
        }
        Action::RustMutantsOfChanges { jobs } => rust_mutants_of_changes(runner, out, *jobs),
        Action::UnviableGate(extent) => {
            let report = unviable_report(runner, *extent)?;
            unviable_gate(&report, *extent, &read_file(runner, UNVIABLE_REVIEWED)?)
        }
    }
}

/// Empties a directory through `runner`, naming it when it cannot.
fn fresh(runner: &mut dyn Runner, path: &str) -> Result<(), Failure> {
    runner
        .fresh_dir(path)
        .map_err(|error| Failure::from(format!("cannot empty {path}: {error}")))
}

/// Writes one line to stdout, naming stdout when it cannot.
fn say(out: &mut dyn Write, line: &str) -> Result<(), Failure> {
    writeln!(out, "{line}")
        .map_err(|error| Failure::from(format!("cannot write to stdout: {error}")))
}

/// Runs `steps` in order and stops at the first failure.
///
/// # Errors
///
/// Returns the failure of the first step that failed, its summary naming the step, or why
/// the progress could not be written.
pub fn run_steps(
    steps: &[Step],
    runner: &mut dyn Runner,
    out: &mut dyn Write,
) -> Result<(), Failure> {
    let total_steps = steps.len();
    let mut tree = None;
    for (index, step) in steps.iter().enumerate() {
        let number = index.saturating_add(1);
        say(out, &format!("▶ [{number}/{total_steps}] {}", step.name))?;
        run_action(&step.action, runner, out, &mut tree).map_err(|failure| Failure {
            summary: format!("{}: {}", step.name, failure.summary),
            details: failure.details,
        })?;
    }
    say(out, &format!("✔ all {total_steps} checks passed"))
}

/// Usage of the `xtask` command, in the order of its first line.
pub const USAGE: &str = "\
usage: cargo xtask <check [--full]|list [--full]|mutants <target>|bless|commit-msg <file>|pre-push [commit...]|help>
  check [--full]        run every quality check; mutation testing covers the changes since
                        the merge base with the upstream, origin/main or main, or with
                        --full every mutant, as CI does
  list [--full]         list the checks of `check` (or `check --full`), in order
  mutants <target>      run every mutant of one target: a crate (its name), a Rust file
                        of a crate, a TypeScript package (its directory) or one of its
                        sources (their paths from the root)
  bless                 rewrite conformance/references.json from the current renders
  commit-msg <file>     check a commit message (used by the commit-msg hook)
  pre-push [commit...]  check that the tree is clean and the pushed commits are checked
                        out, then run `check` (used by the pre-push hook)
  help                  show this help";

/// What `cargo xtask mutants` mutates, every mutant of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A crate of [`RUST_CRATES`], named (`cargo mutants --package`).
    Crate(RustCrate),
    /// A Rust file of a crate (`cargo mutants --package <crate> --file <path>`).
    RustFile {
        /// The crate holding it.
        krate: RustCrate,
        /// Its path, relative to the root.
        path: String,
    },
    /// A package of [`PACKAGES`], by its directory (Stryker in that package).
    Package(Package),
    /// A TypeScript source of a package, its tests apart (Stryker with `--mutate`).
    TypeScriptFile {
        /// The package holding it.
        package: Package,
        /// Its path, relative to the package.
        source: String,
    },
}

impl Target {
    /// The file the target names, relative to the root, if it is a file.
    #[must_use]
    pub fn file(&self) -> Option<String> {
        match self {
            Self::Crate(_) | Self::Package(_) => None,
            Self::RustFile { path, .. } => Some(path.clone()),
            Self::TypeScriptFile { package, source } => Some(format!("{}/{source}", package.dir)),
        }
    }
}

/// The extensions of the TypeScript sources Stryker can mutate: those of [`TS_SOURCES`],
/// Astro templates apart.
fn mutable_source(path: &str) -> bool {
    TS_SOURCES
        .iter()
        .filter_map(|pattern| pattern.strip_prefix('*'))
        .filter(|extension| *extension != ".astro")
        .any(|extension| path.ends_with(extension))
}

/// Reads the target of `cargo xtask mutants`: a crate name, a package directory, or the
/// path of a file below one, relative to the root and written plainly (no `.`, `..` or
/// empty part).
///
/// # Errors
///
/// Returns the usage error, in one line: a path that is not plain, test code, or anything
/// else that is no target.
pub fn mutation_target(text: &str) -> Result<Target, String> {
    if let Some(krate) = RUST_CRATES.iter().find(|krate| krate.name == text) {
        return Ok(Target::Crate(*krate));
    }
    if let Some(package) = PACKAGES.iter().find(|package| package.dir == text) {
        return Ok(Target::Package(*package));
    }
    if text
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(format!(
            "{text:?} is not a plain path from the root (no `.`, `..` or empty part)"
        ));
    }
    let below = |dir: &str| {
        text.strip_prefix(dir)
            .and_then(|rest| rest.strip_prefix('/'))
    };
    if Path::new(text)
        .extension()
        .is_some_and(|extension| extension == "rs")
        && let Some(krate) = RUST_CRATES.iter().find(|krate| below(krate.dir).is_some())
    {
        return Ok(Target::RustFile {
            krate: *krate,
            path: text.to_owned(),
        });
    }
    if let Some((package, source)) = PACKAGES
        .iter()
        .find_map(|package| below(package.dir).map(|source| (*package, source)))
        && mutable_source(source)
    {
        if is_test(source) {
            return Err(format!("{text:?} is test code: name the source it tests"));
        }
        return Ok(Target::TypeScriptFile {
            package,
            source: source.to_owned(),
        });
    }
    Err(format!(
        "{text:?} is no crate, Rust file of a crate, TypeScript package or source of one"
    ))
}

/// The steps of `cargo xtask mutants`: every mutant of `target`, then its gate.
///
/// Rust runs in `jobs` jobs. The report directory is emptied first, and the gate judges a
/// partial run unless the target is a whole TypeScript package.
#[must_use]
pub fn mutants_steps(target: &Target, jobs: NonZeroUsize) -> Vec<Step> {
    let jobs = jobs.to_string();
    let rust = |what: &str, args: &[&str]| {
        let mut all = args.to_vec();
        all.extend(["--jobs", &jobs]);
        vec![
            Step {
                name: "Fresh Rust mutation report".to_owned(),
                action: Action::FreshDirs(vec![MUTANTS_OUT.to_owned()]),
            },
            Step {
                name: format!("Rust mutation testing of {what}"),
                action: Action::Run(mutants_command(&all)),
            },
            Step {
                name: "Rust unviable mutants reviewed".to_owned(),
                action: Action::UnviableGate(Extent::Partial),
            },
        ]
    };
    let typescript = |package: Package, what: &str, cmd: Cmd, extent: Extent| {
        let [_, mutation] = package.reports();
        vec![
            Step {
                name: format!("Fresh mutation report of {}", package.dir),
                action: Action::FreshDirs(vec![mutation]),
            },
            Step {
                name: format!("Mutation testing of {what}"),
                action: Action::Run(cmd),
            },
            Step {
                name: format!("TypeScript mutants of {what} killed"),
                action: Action::TypeScriptMutationGate { package, extent },
            },
        ]
    };
    match target {
        Target::Crate(krate) => rust(krate.name, &["--package", krate.name]),
        Target::RustFile { krate, path } => rust(path, &["--package", krate.name, "--file", path]),
        Target::Package(package) => typescript(
            *package,
            package.dir,
            stryker_command(
                package.dir,
                Mode::Full {
                    jobs: NonZeroUsize::MIN,
                },
            ),
            Extent::Complete,
        ),
        Target::TypeScriptFile { package, source } => typescript(
            *package,
            &format!("{}/{source}", package.dir),
            Cmd::new(
                "pnpm",
                &["exec", "stryker", "run", "--mutate", source],
                Dir::Package(package.dir),
            ),
            Extent::Partial,
        ),
    }
}

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

/// A command of `cargo xtask`, read from its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Task {
    /// Every check: every mutant if `true` ([`Mode::Full`]), else the changes.
    Check(bool),
    /// The list of the checks: those of `check --full` if `true`.
    List(bool),
    /// Every mutant of one target.
    Mutants(Target),
    /// Rewrite the reference hashes.
    Bless,
    /// Check the commit message in this file.
    CommitMsg(String),
    /// Check what is pushed (these objects), then every check.
    PrePush(Vec<String>),
    /// The usage.
    Help,
}

/// Reads the arguments (the program name excluded): every one must be UTF-8 text.
///
/// # Errors
///
/// Returns the usage error, in one line.
fn task(args: &[OsString]) -> Result<Task, String> {
    let args = args
        .iter()
        .map(|arg| {
            arg.to_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("argument is not valid UTF-8: {}", arg.to_string_lossy()))
        })
        .collect::<Result<Vec<String>, String>>()?;
    let alone = ["bless", "help"];
    let moded = ["check", "list"];
    match args.as_slice() {
        [] => Err("no command given".to_owned()),
        [command] if command == "check" => Ok(Task::Check(false)),
        [command, flag] if command == "check" && flag == "--full" => Ok(Task::Check(true)),
        [command] if command == "list" => Ok(Task::List(false)),
        [command, flag] if command == "list" && flag == "--full" => Ok(Task::List(true)),
        [command] if command == "bless" => Ok(Task::Bless),
        [command] if command == "help" => Ok(Task::Help),
        [command, file] if command == "commit-msg" => Ok(Task::CommitMsg(file.clone())),
        [command, target] if command == "mutants" => mutation_target(target).map(Task::Mutants),
        [command, ..] if command == "mutants" => Err("`mutants` takes one target".to_owned()),
        [command, pushed @ ..] if command == "pre-push" => Ok(Task::PrePush(pushed.to_vec())),
        [command, ..] if alone.contains(&command.as_str()) => {
            Err(format!("`{command}` takes no argument"))
        }
        [command, ..] if moded.contains(&command.as_str()) => {
            Err(format!("`{command}` takes no argument but `--full`"))
        }
        [command, ..] if command == "commit-msg" => Err("`commit-msg` takes one file".to_owned()),
        [command, ..] => Err(format!("unknown command \"{command}\"")),
    }
}

/// Entry point: runs the command named by `args` (the program name excluded) and returns
/// the exit code, as every command-line tool of the repository does.
///
/// 0 on success. 1 when a check or a command failed: one `error: …` line on `err`, its
/// details on `out`. 2 for a usage error: one `error: … (see: cargo xtask help)` line on
/// `err`, nothing on `out`. Only `help` prints the usage, on `out`.
pub fn cli(
    args: &[OsString],
    runner: &mut dyn Runner,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    // A file target must exist: cargo-mutants and Stryker would find nothing in a missing
    // one, and pass.
    let checked = task(args).and_then(|task| {
        if let Task::Mutants(target) = &task
            && let Some(path) = target.file()
        {
            let _text = runner
                .read(&path)
                .map_err(|error| format!("cannot read {path}: {error}"))?;
        }
        Ok(task)
    });
    let task = match checked {
        Ok(task) => task,
        Err(usage) => {
            // Nothing more can be done if stderr itself is broken: the exit code still tells.
            drop(writeln!(err, "error: {usage} (see: cargo xtask help)"));
            return 2;
        }
    };
    let jobs = mutation_jobs(runner.parallelism());
    let mode = |full: bool| {
        if full {
            Mode::Full { jobs }
        } else {
            Mode::Changes { jobs }
        }
    };
    let result = match task {
        Task::Check(full) => {
            let steps = check_steps(mode(full));
            run_steps(&steps, runner, out)
        }
        Task::Bless => run_steps(&[bless_step()], runner, out),
        Task::CommitMsg(path) => {
            read_file(runner, &path).and_then(|message| commit_msg_gate(&message))
        }
        Task::PrePush(pushed) => pre_push(runner, &pushed)
            .and_then(|()| run_steps(&check_steps(mode(false)), runner, out)),
        Task::Mutants(target) => {
            let steps = mutants_steps(&target, jobs);
            run_steps(&steps, runner, out)
        }
        Task::Help => say(out, USAGE),
        Task::List(full) => {
            check_steps(mode(full))
                .iter()
                .enumerate()
                .try_for_each(|(index, step)| {
                    say(
                        out,
                        &format!("{:>2}. {}", index.saturating_add(1), step.name),
                    )
                })
        }
    };
    match result {
        Ok(()) => 0,
        Err(failure) => {
            // Nothing more can be done if an output itself is broken: the exit code still
            // fails.
            for detail in &failure.details {
                drop(writeln!(out, "{detail}"));
            }
            drop(writeln!(err, "error: {}", failure.summary));
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
