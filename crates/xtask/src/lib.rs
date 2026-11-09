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
use std::process::{Command, Stdio};

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
pub const RUST_CRATES: [RustCrate; 10] = [
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
        name: "engine-fpenv",
        dir: "crates/engine-fpenv",
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
            "--",
            INCLUDE_IGNORED,
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

/// How much mutation testing `cargo xtask check` does, and whether a failure stops it
/// ([`Mode::on_failure`]); every other check is the same in both modes.
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

impl Mode {
    /// What a check in this mode does when a step fails: [`Mode::Changes`], for
    /// development, stops there (fast feedback); [`Mode::Full`], hours long, runs every
    /// other step too, so that one failure never hides the state of the steps after it.
    #[must_use]
    pub const fn on_failure(self) -> OnFailure {
        match self {
            Self::Changes { .. } => OnFailure::Stop,
            Self::Full { .. } => OnFailure::RunTheRest,
        }
    }
}

/// What [`run_steps_then`] does when a step fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnFailure {
    /// Stop there, and return that failure.
    Stop,
    /// Run every following step, each whatever the earlier ones did (a step that needs what
    /// a failed one builds fails in its own words), then return every failure at once.
    RunTheRest,
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
    /// What its checks need besides the JavaScript dependencies, which every package needs:
    /// prepared again in its part of the full check ([`Part::needs`]), since its checks may
    /// run there without the others.
    pub needs: &'static [Setup],
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
        needs: &[],
    },
    Package {
        dir: "tooling/oracle",
        step: "Conformance oracle (independent, TypeScript)",
        shipped: false,
        outputs: &[],
        needs: &[Setup::ConformanceExport],
    },
    Package {
        dir: "bindings/wasm",
        step: "Engine WASM checks",
        shipped: true,
        outputs: &["dist"],
        needs: &[],
    },
    Package {
        dir: "apps/playground",
        step: "Playground checks",
        shipped: true,
        outputs: &[],
        needs: &[Setup::EngineBuild],
    },
    Package {
        dir: "apps/roadmap",
        step: "Roadmap checks",
        shipped: true,
        outputs: &[],
        needs: &[],
    },
];

/// The package of the engine in the browser, whose build (the WebAssembly module and the
/// JavaScript around it) the packages that use it need: the playground.
pub const ENGINE_PACKAGE: Package = PACKAGES[2];

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
    /// Require 100% coverage and every mutant killed in the reports of a TypeScript package.
    TypeScriptReportsGate(Package),
    /// Require every Stryker configuration to hide no mutant.
    StrykerConfigGate,
    /// Require every TypeScript source of a package to be measured by its coverage and
    /// mutated by Stryker, or reviewed as an exception.
    TypeScriptScopeGate(Package),
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
    /// Require every stable float method of the pinned toolchain to be banned in
    /// `clippy.toml` or reviewed as deterministic ([`float_gate`]).
    FloatMethodGate,
    /// Require every direct dependency to be reviewed, and every review current.
    DependencyGate,
    /// Build and test the engine core and the boxes with the math layer's fused multiply-add
    /// option ([`fused_command`]), and require the option to have been in force
    /// ([`fused_gate`]).
    FusedBuild,
    /// Require `libm` to be a dependency of the crates of [`LIBM_CRATES`] only, outside
    /// tests: boxes reach math functions through the engine's math layer alone
    /// ([`math_layer_gate`]).
    MathLayerGate,
    /// Require the supply-chain settings of [`SUPPLY_CHAIN_SETTINGS`].
    SupplyChainGate,
    /// Require the CI workflow to pin every action, tool and runtime.
    WorkflowGate,
    /// Require the parts of the full check to run every step of it once ([`parts_gate`]).
    PartsGate,
    /// Require every unviable mutant of the shards to be reviewed, and every review
    /// current: [`Action::UnviableGate`] after a complete run, on the reports of every shard
    /// (these paths), together.
    ShardsUnviableGate(Vec<String>),
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

/// Longest line [`safe_message`] returns, in characters after escaping: the same bound as
/// `engine_patch::MAX_MESSAGE_CHARS`.
pub const MAX_MESSAGE_CHARS: usize = 1_000;

/// Whether `character` must not reach a terminal as is: control characters (a newline
/// would forge a second line, ESC starts terminal sequences), bidirectional formatting
/// characters (they reorder what is shown) and the Unicode line and paragraph separators.
const fn needs_escape(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{61c}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
                | '\u{2028}'
                | '\u{2029}'
        )
}

/// `message` made safe to show, on one line and bounded.
///
/// Every character that could break the line or drive a terminal is escaped as `\u{..}`;
/// past [`MAX_MESSAGE_CHARS`] characters (escapes counted whole, never split), the text is
/// cut and marked with `…`. The algorithm of `engine_patch::safe_message`, copied: xtask
/// depends on no engine crate. The messages of xtask quote untrusted text (a commit
/// message, a file, an argument), so [`cli`] passes every line it writes through it.
#[must_use]
pub fn safe_message(message: &str) -> String {
    let mut safe = String::new();
    // Where to cut if the message is too long: after the last whole character (or
    // escape) that leaves room for the `…`.
    let mut cut = 0_usize;
    let mut used = 0_usize;
    for character in message.chars() {
        let piece = if needs_escape(character) {
            character.escape_unicode().to_string()
        } else {
            String::from(character)
        };
        used = used.saturating_add(piece.chars().count());
        if used > MAX_MESSAGE_CHARS {
            safe.truncate(cut);
            safe.push('…');
            return safe;
        }
        safe.push_str(&piece);
        if used < MAX_MESSAGE_CHARS {
            cut = safe.len();
        }
    }
    safe
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

/// The tools first, then the state of the working tree: what every part starts with.
fn opening_steps() -> Vec<(Role, Step)> {
    vec![
        (
            Role::Frame,
            Step {
                name: "Tools and platform".to_owned(),
                action: Action::Preflight,
            },
        ),
        (
            Role::Frame,
            Step {
                name: "Working tree recorded".to_owned(),
                action: Action::TreeRecorded,
            },
        ),
    ]
}

/// Tags each of `steps` with `role`.
fn tagged(role: Role, steps: Vec<Step>) -> Vec<(Role, Step)> {
    steps.into_iter().map(|step| (role, step)).collect()
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
        Step {
            name: "Float methods of the pinned toolchain banned or reviewed as deterministic"
                .to_owned(),
            action: Action::FloatMethodGate,
        },
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
            &["test", "--workspace", "--locked", "--", INCLUDE_IGNORED],
            Dir::Root,
        ),
        Step {
            name: "Boxes unchanged under the math layer's fused multiply-add option (a test build)"
                .to_owned(),
            action: Action::FusedBuild,
        },
    ]
}

/// The target directory of the fused test build, apart from the main one, so that neither
/// build replaces the other's artifacts.
pub const FUSED_TARGET: &str = "target/fused";

/// The configuration of the fused test build.
///
/// The one build-time choice of the math layer a build may make, `poyetik_math = "fused"`,
/// given on the command line of this step only (`.cargo/config.toml` holds no build flag),
/// so that no other build sees it.
pub const FUSED_CONFIG: &str = r#"build.rustflags=["--cfg", "poyetik_math=\"fused\""]"#;

/// The line of the test only the fused test build holds.
///
/// Its output must show it passed: the proof that the option took effect.
pub const FUSED_PROOF: &str =
    "test math::tests::is_built_with_the_fused_multiply_add_option ... ok";

/// The fused test build: the engine core and the box crate, compiled and tested with it.
///
/// The fused multiply-add option of the math layer (`spec/engine-design.md`, decision 10):
/// the boxes compile unchanged and pass their tests; the core's tests prove the option
/// changes what `mul_then_add` gives, in a render too.
#[must_use]
pub fn fused_command() -> Cmd {
    Cmd::new(
        "cargo",
        &[
            "test",
            "--locked",
            "--package",
            "engine-core",
            "--package",
            "engine-objects",
            "--target-dir",
            FUSED_TARGET,
            "--config",
            FUSED_CONFIG,
            "--",
            INCLUDE_IGNORED,
        ],
        Dir::Root,
    )
}

/// Checks the output of [`fused_command`]: the test only the fused build holds passed
/// ([`FUSED_PROOF`]), so the option was in force.
///
/// # Errors
///
/// Returns a failure when that line is missing.
pub fn fused_gate(output: &str) -> Result<(), Failure> {
    if output.lines().any(|line| line == FUSED_PROOF) {
        Ok(())
    } else {
        Err(Failure::from(format!(
            "the fused test build ran without its option: `{FUSED_PROOF}` is missing from its \
             output"
        )))
    }
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

/// The test harness argument that runs the ignored tests too.
///
/// Given to every Rust test run that proves something (the tests, coverage, mutation
/// testing, Miri): an ignored test is then inert, never a way to remove a test from a proof.
/// Rustdoc does not compile an `ignore` doc test even so, nor run a `no_run` one, and a
/// conditional attribute or a `cfg` naming Miri can still remove a test from one of these
/// runs: the bypass gate refuses all of them.
pub const INCLUDE_IGNORED: &str = "--include-ignored";

/// The arguments of every cargo-mutants run: lints are capped while building mutants
/// (otherwise a mutant that merely leaves a variable unused fails to compile and is silently
/// skipped instead of tested), the ignored files are not copied, and every cargo command
/// it runs keeps `Cargo.lock` as it is (`--locked`, as every other cargo command of the
/// checks).
const MUTANTS_ARGS: [&str; 4] = [
    "mutants",
    "--cap-lints=true",
    "--gitignore=true",
    "--cargo-arg=--locked",
];

/// A cargo-mutants command: [`MUTANTS_ARGS`], then `args`, then [`INCLUDE_IGNORED`] for the
/// test harness (cargo-mutants passes what follows its `--` to `cargo test`, which passes
/// what follows its own `--` to the tests).
fn mutants_command(args: &[&str]) -> Cmd {
    Cmd::new(
        "cargo",
        &[&MUTANTS_ARGS[..], args, &["--", "--", INCLUDE_IGNORED]].concat(),
        Dir::Root,
    )
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

/// The cargo-mutants command of [`Mode::Full`]: every mutant of the workspace in `jobs` jobs.
///
/// With a `shard`, only its mutants (cargo-mutants' `--shard <index>/<count>`, mutant `i` on
/// shard `i % count`: round-robin, so that each shard takes its share of every file).
#[must_use]
pub fn workspace_mutants_command(jobs: NonZeroUsize, shard: Option<Shard>) -> Cmd {
    let jobs = jobs.to_string();
    let mut args = vec!["--workspace", "--jobs", &jobs];
    let sharded = shard.map(|shard| format!("{}/{MUTATION_SHARDS}", shard.index()));
    if let Some(sharded) = &sharded {
        args.extend(["--shard", sharded, "--sharding=round-robin"]);
    }
    mutants_command(&args)
}

/// Rust mutation testing in `mode`, after emptying [`MUTANTS_OUT`], then its gate.
fn rust_mutation_steps(mode: Mode) -> Vec<(Role, Step)> {
    let (name, action, extent) = match mode {
        Mode::Changes { jobs } => (
            "Rust mutation testing of the changes",
            Action::RustMutantsOfChanges { jobs },
            Extent::Partial,
        ),
        Mode::Full { jobs } => (
            "Rust mutation testing",
            Action::Run(workspace_mutants_command(jobs, None)),
            Extent::Complete,
        ),
    };
    vec![
        (
            Role::EveryShard,
            Step {
                name: "Fresh Rust mutation report".to_owned(),
                action: Action::FreshDirs(vec![MUTANTS_OUT.to_owned()]),
            },
        ),
        (
            Role::Sharded,
            Step {
                name: name.to_owned(),
                action,
            },
        ),
        (
            Role::AcrossShards,
            Step {
                name: "Rust unviable mutants reviewed".to_owned(),
                action: Action::UnviableGate(extent),
            },
        ),
    ]
}

/// Coverage and mutation testing in `mode`, with their gates, and Miri.
fn proof_steps(mode: Mode) -> Vec<(Role, Step)> {
    let mut coverage = vec![
        Step {
            name: "Fresh Rust coverage reports".to_owned(),
            action: Action::FreshDirs(vec![COVERAGE_DIR.to_owned()]),
        },
        Step {
            name: "Rust coverage counts a crate's own unit tests only".to_owned(),
            action: Action::CoverageLocality,
        },
    ];
    coverage.extend(RUST_CRATES.iter().map(|&krate| Step {
        name: format!(
            "Rust coverage of {}: 100% by its own unit tests",
            krate.name
        ),
        action: Action::Coverage(Coverage {
            krate,
            measured: Measured::Library,
        }),
    }));
    coverage.extend(
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
    let mut steps = tagged(Role::Proof(Part::Coverage), coverage);
    steps.extend(rust_mutation_steps(mode));
    let gates = vec![
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
    ];
    steps.extend(tagged(Role::Proof(Part::Gates), gates));
    steps.push((
        Role::Proof(Part::Miri),
        Step {
            name: "Unsafe code under Miri".to_owned(),
            action: Action::Run(miri_command()),
        },
    ));
    steps
}

/// The crates holding a file of [`UNSAFE_FILES`] that is not in [`ASSEMBLY_FILES`], whose
/// tests Miri runs.
#[must_use]
pub fn unsafe_crates() -> Vec<&'static str> {
    RUST_CRATES
        .iter()
        .filter(|krate| {
            UNSAFE_FILES
                .iter()
                .filter(|file| !ASSEMBLY_FILES.contains(file))
                .any(|file| file.starts_with(&format!("{}/", krate.dir)))
        })
        .map(|krate| krate.name)
        .collect()
}

/// Miri runs the tests of every crate holding unsafe code ([`unsafe_crates`]), the ignored
/// ones included ([`INCLUDE_IGNORED`]), checking them for undefined behavior.
#[must_use]
pub fn miri_command() -> Cmd {
    let mut args = vec![NIGHTLY_TOOLCHAIN, "miri", "test"];
    for krate in unsafe_crates() {
        args.extend(["--package", krate]);
    }
    args.extend(["--locked", "--", INCLUDE_IGNORED]);
    Cmd::new("cargo", &args, Dir::Root)
}

/// Conformance, supply chain, then the consistency of the repository: membership, READMEs,
/// versions, history and the CI workflow.
fn consistency_steps() -> Vec<(Role, Step)> {
    let conformance = run(
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
    );
    // The independent oracle (tooling/oracle) checks these files after the JavaScript
    // dependencies are installed.
    let export = vec![
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
    ];
    let gates = vec![
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
            name: "Box arithmetic only through the math layer: libm in engine-core and \
                   engine-testkit only, no raw float in box code"
                .to_owned(),
            action: Action::MathLayerGate,
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
        Step {
            name: "The parts of CI run every check of the full check once".to_owned(),
            action: Action::PartsGate,
        },
    ];
    let mut steps = vec![(Role::Proof(Part::Gates), conformance)];
    steps.extend(tagged(Role::Setup(Setup::ConformanceExport), export));
    steps.extend(tagged(Role::Proof(Part::Gates), gates));
    steps
}

/// The Stryker command of the package in `dir`, in `mode`.
///
/// It names [`STRYKER_CONFIG`], the one configuration the gates read: run without a file,
/// Stryker would take the first of the names it seeks (`stryker.conf.json` before it).
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
        STRYKER_CONFIG,
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
fn typescript_steps(mode: Mode) -> Vec<(Role, Step)> {
    let javascript = Role::Setup(Setup::JavaScript);
    let gates = Role::Proof(Part::Gates);
    let mut steps = vec![
        (
            javascript,
            run(
                "JavaScript dependencies",
                "pnpm",
                &["install", "--frozen-lockfile"],
                Dir::Root,
            ),
        ),
        (
            gates,
            run("JavaScript vulnerabilities", "pnpm", &["audit"], Dir::Root),
        ),
        (
            gates,
            Step {
                name: "JavaScript licenses".to_owned(),
                action: Action::LicenseGate,
            },
        ),
        (
            gates,
            Step {
                name: "Stryker configurations hide no mutant".to_owned(),
                action: Action::StrykerConfigGate,
            },
        ),
        (
            javascript,
            Step {
                name: "Fresh TypeScript report directories".to_owned(),
                action: Action::FreshDirs(typescript_reports()),
            },
        ),
        (
            javascript,
            Step {
                name: "Fresh TypeScript build outputs".to_owned(),
                action: Action::FreshDirs(build_outputs()),
            },
        ),
        // Built before any package, so that a package using it can be checked without the
        // checks of the engine package itself (as a part of the full check does).
        (
            Role::Setup(Setup::EngineBuild),
            run(
                "Engine WASM build, for the packages that use it",
                "pnpm",
                &["run", "build"],
                Dir::Package(ENGINE_PACKAGE.dir),
            ),
        ),
    ];
    for package in PACKAGES {
        let mut owned = Vec::new();
        owned.push(run(
            package.step,
            "pnpm",
            &["run", "check"],
            Dir::Package(package.dir),
        ));
        let what = match mode {
            Mode::Changes { .. } => "what changed since the last run",
            Mode::Full { .. } => "every mutant",
        };
        owned.push(Step {
            name: format!("Mutation testing of {} ({what})", package.dir),
            action: Action::Run(stryker_command(package.dir, mode)),
        });
        owned.push(Step {
            name: format!(
                "TypeScript coverage and mutation reports of {}",
                package.dir
            ),
            action: Action::TypeScriptReportsGate(package),
        });
        owned.push(Step {
            name: format!(
                "Every TypeScript source of {} measured and mutated, or reviewed",
                package.dir
            ),
            action: Action::TypeScriptScopeGate(package),
        });
        steps.extend(tagged(Role::Proof(Part::Package(package)), owned));
    }
    steps
}

/// Every check of the repository in `mode`, in the order they run, each with its role
/// in the parts of the full check.
fn roles(mode: Mode) -> Vec<(Role, Step)> {
    let rust = Role::Proof(Part::Rust);
    [
        opening_steps(),
        tagged(rust, static_steps()),
        tagged(rust, portability_steps()),
        proof_steps(mode),
        consistency_steps(),
        typescript_steps(mode),
        vec![(
            Role::Frame,
            Step {
                name: "Working tree unchanged by the checks".to_owned(),
                action: Action::TreeUnchanged,
            },
        )],
    ]
    .concat()
}

/// Every check of the repository in `mode`, in the order they run.
#[must_use]
pub fn check_steps(mode: Mode) -> Vec<Step> {
    roles(mode).into_iter().map(|(_, step)| step).collect()
}

/// How many shards the Rust mutation testing of the full check is split into in CI, each a
/// part of its own ([`Part::Mutation`]).
pub const MUTATION_SHARDS: usize = 8;

/// One shard of the Rust mutants: cargo-mutants' `--shard <index>/<MUTATION_SHARDS>`.
///
/// Only [`Shard::all`] makes one, so that its index is always below [`MUTATION_SHARDS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shard(usize);

impl Shard {
    /// Every shard, in order.
    pub fn all() -> impl Iterator<Item = Self> {
        (0..MUTATION_SHARDS).map(Self)
    }

    /// Its index, from 0, as cargo-mutants counts.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

/// What a step needs prepared before it, done again in every part that needs it: the
/// preparation is repeated, never the proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setup {
    /// The JavaScript dependencies installed, and the TypeScript reports and build outputs
    /// emptied.
    JavaScript,
    /// The engine package built ([`ENGINE_PACKAGE`]).
    EngineBuild,
    /// The conformance cases exported as WAV files ([`CONFORMANCE_EXPORT`]), for the oracle.
    ConformanceExport,
}

/// A part of the full check: what one CI job runs (`cargo xtask check --full --part
/// <name>`), in parallel with the others.
///
/// Together the parts run every step of the full check once ([`parts_gate`]); the
/// preparation steps ([`Setup`]) are repeated in the parts that need them, and every part
/// starts with the tools and the state of the working tree and ends checking it unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// Formatting, lints, documentation, tests and portability builds.
    Rust,
    /// The Rust coverage of every crate.
    Coverage,
    /// One shard of the Rust mutants.
    Mutation(Shard),
    /// Miri on the unsafe code.
    Miri,
    /// Conformance, supply chain and the gates on the repository.
    Gates,
    /// The checks, mutation testing and gates of one TypeScript package.
    Package(Package),
    /// What needs every other part: the unviable mutants of every shard, reviewed and
    /// every review current. Run last, by a job that needs every other one.
    Final,
}

impl Part {
    /// Its name on the command line and in the workflow's matrix: no `/`, which an artifact
    /// name cannot hold.
    #[must_use]
    pub fn name(self) -> String {
        match self {
            Self::Rust => "rust".to_owned(),
            Self::Coverage => "coverage".to_owned(),
            Self::Mutation(shard) => format!("mutation-{}", shard.index()),
            Self::Miri => "miri".to_owned(),
            Self::Gates => "gates".to_owned(),
            Self::Package(package) => package.dir.replace('/', "-"),
            Self::Final => "final".to_owned(),
        }
    }

    /// What it needs prepared.
    #[must_use]
    pub fn needs(self) -> Vec<Setup> {
        match self {
            Self::Gates => vec![Setup::JavaScript],
            Self::Package(package) => [&[Setup::JavaScript], package.needs].concat(),
            Self::Rust | Self::Coverage | Self::Mutation(_) | Self::Miri | Self::Final => {
                Vec::new()
            }
        }
    }
}

/// Every part of the full check, in order, [`Part::Final`] last.
#[must_use]
pub fn full_parts() -> Vec<Part> {
    let mut parts = vec![Part::Rust, Part::Coverage];
    parts.extend(Shard::all().map(Part::Mutation));
    parts.extend([Part::Miri, Part::Gates]);
    parts.extend(PACKAGES.map(Part::Package));
    parts.push(Part::Final);
    parts
}

/// Where the final part reads the unviable mutants of the shard `part` (named as by
/// [`Part::name`]).
///
/// The CI job of each shard uploads its [`UNVIABLE_REPORT`] as the artifact
/// `unviable-<part>`, which the final job downloads below [`SHARD_REPORTS`], one directory
/// per artifact.
#[must_use]
pub fn shard_unviable_report(part: &str) -> String {
    format!("{SHARD_REPORTS}/{UNVIABLE_ARTIFACT}{part}/unviable.txt")
}

/// Where the final part finds the reports of the shards, one directory per artifact.
pub const SHARD_REPORTS: &str = "mutants.out/shards";

/// The prefix of the name of the artifact of each shard's unviable mutants.
pub const UNVIABLE_ARTIFACT: &str = "unviable-";

/// Who runs a step of the full check when it is split into parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    /// Every part: the tools, and the working tree recorded then unchanged.
    Frame,
    /// Every part that needs this preparation.
    Setup(Setup),
    /// That part only.
    Proof(Part),
    /// Every shard ([`Part::Mutation`]): the mutation report emptied.
    EveryShard,
    /// Split among the shards: each runs its own mutants.
    Sharded,
    /// The final part, on what every shard found: the unviable-mutant gate.
    AcrossShards,
}

/// The steps of `part` of the full check, in the order of the full check, with `jobs`
/// cargo-mutants jobs.
#[must_use]
pub fn part_steps(part: Part, jobs: NonZeroUsize) -> Vec<Step> {
    let needs = part.needs();
    roles(Mode::Full { jobs })
        .into_iter()
        .filter_map(|(role, step)| match (role, part) {
            (Role::Frame, _) | (Role::EveryShard, Part::Mutation(_)) => Some(step),
            (Role::Setup(setup), _) => needs.contains(&setup).then_some(step),
            (Role::Proof(owner), _) => (owner == part).then_some(step),
            (Role::Sharded, Part::Mutation(shard)) => Some(Step {
                name: format!("{} (shard {}/{MUTATION_SHARDS})", step.name, shard.index()),
                action: Action::Run(workspace_mutants_command(jobs, Some(shard))),
            }),
            (Role::AcrossShards, Part::Final) => Some(Step {
                name: format!("{} (every shard)", step.name),
                action: Action::ShardsUnviableGate(
                    Shard::all()
                        .map(|shard| shard_unviable_report(&Part::Mutation(shard).name()))
                        .collect(),
                ),
            }),
            (Role::EveryShard | Role::Sharded | Role::AcrossShards, _) => None,
        })
        .collect()
}

/// The arguments that make a cargo-mutants command run one shard: `--shard <k>/<n>`, written
/// as cargo-mutants reads it (no sign, no leading zero), then round-robin sharding.
///
/// Returns the command without them, the shard's index and the count of shards, or `None` if `cmd` holds no such
/// arguments.
fn unsharded(cmd: &Cmd) -> Option<(Cmd, usize, usize)> {
    let at = cmd.args.iter().position(|arg| arg == "--shard")?;
    let [_, value, sharding, ..] = cmd.args.get(at..)? else {
        return None;
    };
    let (index, count) = value.split_once('/')?;
    let (index, count): (usize, usize) = (index.parse().ok()?, count.parse().ok()?);
    if *value != format!("{index}/{count}") || sharding != "--sharding=round-robin" {
        return None;
    }
    let mut args = cmd.args.clone();
    let _removed: Vec<String> = args.drain(at..at.checked_add(3)?).collect();
    Some((
        Cmd {
            args,
            ..cmd.clone()
        },
        index,
        count,
    ))
}

/// What the steps of the parts stand for in the full check ([`parts_gate`]).
#[derive(Debug, Default)]
struct Origins {
    /// For each step of the full check, the parts that run it whole.
    whole: Vec<Vec<String>>,
    /// For each step of the full check run in shards: each shard (its index and the count of
    /// shards), and its part.
    shards: BTreeMap<usize, Vec<(usize, usize, String)>>,
    /// The parts that run a shard, in order.
    shard_parts: Vec<String>,
    /// The reports read by each unviable-mutant gate over the shards.
    across: Vec<Vec<String>>,
}

/// Finds what `step`, a step of `part`, stands for in `full`: its index there, recorded in
/// `origins`, or a problem.
fn origin(
    full: &[Step],
    part: &str,
    step: &Step,
    origins: &mut Origins,
    problems: &mut Vec<String>,
) -> Option<usize> {
    if let Some(index) = full.iter().position(|whole| whole == step) {
        origins.whole.get_mut(index)?.push(part.to_owned());
        return Some(index);
    }
    if let Action::Run(cmd) = &step.action
        && let Some((whole, shard, count)) = unsharded(cmd)
        && let Some(index) = full.iter().position(|of| {
            of.action == Action::Run(whole.clone())
                && step.name == format!("{} (shard {shard}/{count})", of.name)
        })
    {
        if origins.shard_parts.iter().any(|name| name == part) {
            problems.push(format!(
                "{part}: runs a second shard, whose report would replace the first"
            ));
        }
        origins.shard_parts.push(part.to_owned());
        origins
            .shards
            .entry(index)
            .or_default()
            .push((shard, count, part.to_owned()));
        return Some(index);
    }
    if let Action::ShardsUnviableGate(reports) = &step.action
        && let Some(index) = full.iter().position(|of| {
            of.action == Action::UnviableGate(Extent::Complete)
                && step.name == format!("{} (every shard)", of.name)
        })
    {
        origins.across.push(reports.clone());
        return Some(index);
    }
    problems.push(format!(
        "{part}: `{}` is no step of the full check",
        step.name
    ));
    None
}

/// The problems of a step of the full check run in shards: `sharded` (each shard's index,
/// the count of shards, and its part), never whole, and every shard of the same count once.
fn shard_problems(name: &str, whole: &[String], sharded: &[(usize, usize, String)]) -> Vec<String> {
    let mut problems = Vec::new();
    if !whole.is_empty() {
        problems.push(format!(
            "`{name}` runs both whole ({}) and in shards",
            whole.join(", ")
        ));
    }
    let counts: BTreeSet<usize> = sharded.iter().map(|(_, count, _)| *count).collect();
    let [count] = counts.into_iter().collect::<Vec<_>>()[..] else {
        problems.push(format!("`{name}` is split into shards of different counts"));
        return problems;
    };
    for (shard, _, part) in sharded.iter().filter(|(shard, _, _)| *shard >= count) {
        problems.push(format!(
            "{part}: shard {shard}/{count} of `{name}` is no shard"
        ));
    }
    for index in 0..count {
        let parts: Vec<&str> = sharded
            .iter()
            .filter(|(shard, _, _)| *shard == index)
            .map(|(_, _, part)| part.as_str())
            .collect();
        match parts.len() {
            0 => problems.push(format!("shard {index}/{count} of `{name}` runs in no part")),
            1 => {}
            _ => problems.push(format!(
                "shard {index}/{count} of `{name}` runs in {} parts: {}",
                parts.len(),
                parts.join(", ")
            )),
        }
    }
    problems
}

/// Checks that `parts` (each named, with its steps) run every step of `full`, the full check,
/// once.
///
/// Each step of a part stands for one step of `full`, and the steps of a part keep the order
/// of `full`. A step of a part is a step of `full`; or one shard of a cargo-mutants step of
/// `full` (its command with `--shard <k>/<n> --sharding=round-robin` added, named after it
/// with ` (shard <k>/<n>)`); or the unviable-mutant gate over the shards
/// ([`Action::ShardsUnviableGate`], named after the complete [`Action::UnviableGate`] of
/// `full` with ` (every shard)`), which stands for that gate. Every step of `full` runs in
/// exactly one part, except: the `preparation` steps, which run in one part at least; a step
/// run in shards, which runs whole in no part, every shard `k` from 0 to `n - 1` in exactly
/// one part, a part running one shard at most; and the unviable-mutant gate, run over the
/// shards in exactly one part, reading the report of every part that runs a shard
/// ([`shard_unviable_report`]), in order. Part names are unique.
///
/// So no step of the full check is lost or proven twice, and no part runs a step the full
/// check does not (cargo-mutants itself splits the mutants exactly among the shards).
///
/// # Errors
///
/// Returns every step run in no part or in several, every shard missing or repeated, every
/// step of a part that stands for none of `full` or runs out of its order, and every
/// repeated part name.
pub fn parts_gate(
    full: &[Step],
    parts: &[(String, Vec<Step>)],
    preparation: &[String],
) -> Result<(), Failure> {
    let mut problems = Vec::new();
    let mut origins = Origins {
        whole: vec![Vec::new(); full.len()],
        ..Origins::default()
    };
    let mut names = BTreeSet::new();
    for (part, steps) in parts {
        if !names.insert(part) {
            problems.push(format!("two parts are named {part}"));
        }
        let mut last = None;
        for step in steps {
            let Some(index) = origin(full, part, step, &mut origins, &mut problems) else {
                continue;
            };
            if last.is_some_and(|last| last >= index) {
                problems.push(format!(
                    "{part}: `{}` runs out of the order of the full check",
                    step.name
                ));
            }
            last = Some(index);
        }
    }
    let reports: Vec<String> = origins
        .shard_parts
        .iter()
        .map(|part| shard_unviable_report(part))
        .collect();
    for (index, (step, whole)) in full.iter().zip(&origins.whole).enumerate() {
        let name = &step.name;
        if let Some(sharded) = origins.shards.get(&index) {
            problems.extend(shard_problems(name, whole, sharded));
            continue;
        }
        let across = if step.action == Action::UnviableGate(Extent::Complete) {
            origins.across.len()
        } else {
            0
        };
        let runs = whole.len().saturating_add(across);
        if runs == 0 {
            problems.push(format!("`{name}` runs in no part"));
        } else if runs > 1 && !preparation.contains(name) {
            problems.push(format!("`{name}` runs in {runs} parts"));
        }
    }
    problems.extend(
        origins
            .across
            .iter()
            .filter(|read| **read != reports)
            .map(|read| {
                format!(
                    "the unviable-mutant gate over the shards reads {}, not the reports of the \
                     shards: {}",
                    read.join(" "),
                    reports.join(" ")
                )
            }),
    );
    verdict(
        "the parts of the full check do not run each of its steps once",
        problems,
    )
}

/// The names of the steps of the full check that more than one part may run: those that
/// prepare ([`Setup`], and the mutation report emptied by every shard) or frame the others,
/// never a proof.
#[must_use]
pub fn preparation_steps() -> Vec<String> {
    roles(Mode::Full {
        jobs: NonZeroUsize::MIN,
    })
    .into_iter()
    .filter(|(role, _)| matches!(role, Role::Frame | Role::Setup(_) | Role::EveryShard))
    .map(|(_, step)| step.name)
    .collect()
}

/// What the working tree holds that is not committed (the files git reports as changed or
/// untracked, ignored files apart, and the changes of the tracked ones), and what is
/// checked out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeState {
    /// `git status --porcelain --untracked-files=all`.
    pub status: String,
    /// `git diff --no-ext-diff --no-textconv HEAD`: the content of the changes, as they
    /// are (no external diff or text conversion of the user's).
    pub diff: String,
    /// `git rev-parse HEAD --symbolic-full-name HEAD`: the commit checked out, then its
    /// branch (`HEAD` when detached).
    pub head: String,
}

/// Checks that the checks left the working tree as they found it.
///
/// A tree may be dirty (a local check of uncommitted work), but the checks must not change
/// it: a command that rewrites a tracked file (a stale `Cargo.lock` an unlocked `cargo`
/// would update, a generated file) or leaves a file git does not ignore makes CI differ from
/// what was checked. Nor may what is checked out change (a commit made or a branch checked
/// out in the same checkout while the checks run): the steps after it would read another
/// commit than the one recorded, which a push would then send as checked.
///
/// # Errors
///
/// Returns every file whose state changed, a change of what is checked out, or that the
/// tree was not recorded.
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
    if before.head != after.head {
        let words = |head: &str| head.split_whitespace().collect::<Vec<_>>().join(" ");
        problems.push(format!(
            "what is checked out changed, from `{}` to `{}`: the checks read another commit than the one recorded",
            words(&before.head),
            words(&after.head)
        ));
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
        diff: git_output(
            runner,
            &["diff", "--no-ext-diff", "--no-textconv", "HEAD"],
            "read the work tree",
        )?,
        head: git_output(
            runner,
            &["rev-parse", "HEAD", "--symbolic-full-name", "HEAD"],
            "read the work tree",
        )?,
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
                "the lines: cargo {NIGHTLY_TOOLCHAIN} llvm-cov --manifest-path {}/Cargo.toml{lib} --branch --show-missing-lines -- {INCLUDE_IGNORED}",
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

/// The Stryker configuration of every package, relative to it: the one file the gates read
/// ([`stryker_config_gate`], [`scope_gate`]), so the one every Stryker run names.
pub const STRYKER_CONFIG: &str = "stryker.config.json";

/// The only command a package script may run Stryker with.
pub const STRYKER_SCRIPT: &str = "stryker run stryker.config.json";

/// Checks that no script of the package manifest at `path` runs Stryker but as
/// [`STRYKER_SCRIPT`], so that none reads another configuration or narrows the run.
///
/// # Errors
///
/// Returns every script that mentions Stryker otherwise and every script that is not a
/// command, or why the manifest cannot be read.
pub fn stryker_scripts_gate(path: &str, manifest: &str) -> Result<(), Failure> {
    let json: serde_json::Value = serde_json::from_str(manifest)
        .map_err(|error| Failure::from(format!("{path}: invalid manifest: {error}")))?;
    let mut problems = Vec::new();
    let scripts = json
        .get("scripts")
        .and_then(serde_json::Value::as_object)
        .into_iter()
        .flatten();
    for (name, script) in scripts {
        match script.as_str() {
            Some(command) if command.contains("stryker") && command != STRYKER_SCRIPT => {
                problems.push(format!("`{name}`: {command}"));
            }
            Some(_) => {}
            None => problems.push(format!("`{name}` is not a command")),
        }
    }
    verdict(
        &format!("{path} runs Stryker otherwise than `{STRYKER_SCRIPT}`"),
        problems,
    )
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
///
/// A `cfg` that a coverage build sets (`coverage`, and the one of the nightly build) is an unexpected
/// `cfg` in every other build, which the workspace lints refuse; so is any lifting of that
/// lint, and any declaration of an expected `cfg` (in a manifest or a build script), are
/// refused here.
fn bypass_tokens() -> Vec<(String, &'static [&'static str])> {
    let rust: &[&str] = &["rs"];
    let rust_or_manifest: &[&str] = &["rs", "toml"];
    let typescript: &[&str] = &BYPASS_TYPESCRIPT;
    vec![
        (format!("{}::{}", "mutants", "skip"), rust),
        (format!("{}(off)", "coverage"), rust),
        (format!("cfg({}", "coverage"), rust),
        (format!("{}_{}", "coverage", "nightly"), rust),
        (format!("{}_{}", "unexpected", "cfgs"), rust_or_manifest),
        (format!("{}-{}", "check", "cfg"), rust_or_manifest),
        (format!(".{}(", "only"), typescript),
        (format!(".{}(", "skip"), typescript),
        (format!(".{}(", "todo"), typescript),
        (format!(".{}(", "fixme"), typescript),
        (format!("{}If(", "skip"), typescript),
        (format!("{}If(", "run"), typescript),
    ]
}

/// What ignores a Rust test or makes an attribute conditional, as rustc accepts it: an
/// `ignore` attribute, written with any whitespace or as a raw identifier, and any
/// conditional attribute (its condition may name the coverage build or Miri, where the
/// ignored tests run, and it may ignore a test there). Built at run time, like
/// [`bypass_tokens`]; [`bypass_gate`] seeks them in a whole file, without case or whitespace.
fn rust_hints() -> Vec<String> {
    vec![
        format!("#[{}", "ignore"),
        format!("r#{}", "ignore"),
        format!("{}_{}", "cfg", "attr"),
    ]
}

/// The `cfg` Miri sets: a test that names it can leave the Miri run ([`miri_command`]), so
/// [`bypass_gate`] refuses it as a word anywhere in Rust code (a raw identifier included),
/// except in the exact string literal that names the tool.
const MIRI: &str = "miri";

/// Whether `line`, Rust code, holds the word [`MIRI`] outside the string literal of
/// exactly that word: neither preceded nor followed by a letter, digit or underscore.
fn names_miri(line: &str) -> bool {
    let code = line.replace(&format!("{MIRI:?}"), "");
    let word = |character: char| character.is_alphanumeric() || character == '_';
    code.match_indices(MIRI).any(|(at, _)| {
        let before = code.get(..at).and_then(|text| text.chars().next_back());
        let after = code
            .get(at.saturating_add(MIRI.len())..)
            .and_then(|text| text.chars().next());
        !before.into_iter().chain(after).any(word)
    })
}

/// The extensions of the TypeScript and JavaScript files whose bypasses [`bypass_gate`]
/// refuses: those of [`TS_SOURCES`].
const BYPASS_TYPESCRIPT: [&str; 9] = [
    "ts", "tsx", "mts", "cts", "astro", "js", "jsx", "mjs", "cjs",
];

/// What makes a TypeScript coverage tool exclude code: the ignore comments of its
/// providers, which accept any whitespace between their words and any case, and the option
/// excluding methods by name. Built at run time, like [`bypass_tokens`]; [`bypass_gate`]
/// seeks them in a whole file, without case or whitespace.
fn coverage_hints() -> [String; 5] {
    [
        format!("{} {}", "v8", "ignore"),
        format!("{} {}", "c8", "ignore"),
        format!("{} {}", "istanbul", "ignore"),
        format!("{}:{} {}", "node", "coverage", "ignore"),
        format!("{}{}", "ignore", "ClassMethods"),
    ]
}

/// The line (numbered from 0) of `text` where each occurrence of `hint` starts, both
/// compared in lowercase with every whitespace removed, so that no spacing, line break or
/// case hides it.
fn hint_lines(text: &str, hint: &str) -> Vec<usize> {
    let mut line = 0_usize;
    let mut compact = Vec::new();
    for character in text.chars() {
        if character == '\n' {
            line = line.saturating_add(1);
        }
        if !character.is_whitespace() {
            compact.extend(character.to_lowercase().map(|lower| (lower, line)));
        }
    }
    let needle: Vec<char> = hint
        .chars()
        .filter(|character| !character.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    compact
        .windows(needle.len())
        .filter(|window| window.iter().map(|(character, _)| character).eq(&needle))
        .filter_map(|window| window.first().map(|(_, start)| *start))
        .collect()
}

/// The attributes of a doc test that keep it from running: rustdoc never compiles an
/// `ignore` test, even with [`INCLUDE_IGNORED`], and compiles a `no_run` one only.
const SILENT_DOC_TESTS: [&str; 2] = ["ignore", "no_run"];

/// The attribute of [`SILENT_DOC_TESTS`] that `line`, a fence opening a code block of Rust
/// documentation (three backquotes or tildes), carries, if any.
fn silent_doc_test(line: &str) -> Option<&'static str> {
    let fences = ["`".repeat(3), "~".repeat(3)];
    let info = fences
        .iter()
        .find_map(|fence| line.split_once(fence.as_str()))
        .map(|(_, info)| info)?;
    SILENT_DOC_TESTS
        .into_iter()
        .find(|attribute| info.contains(attribute))
}

/// The fixtures of the shared TypeScript policy: each holds on purpose what a rule forbids,
/// to prove the rule rejects it, and is never run as a test.
pub const POLICY_FIXTURES: [&str; 2] = [
    "tooling/config/test/policy/",
    "tooling/config/test/compiler/",
];

/// The tracked files the bypass gate reads (git pathspecs).
pub const BYPASS_SOURCES: [&str; 12] = [
    "*mutants.toml",
    "*Cargo.toml",
    "*.rs",
    "*.ts",
    "*.tsx",
    "*.mts",
    "*.cts",
    "*.astro",
    "*.js",
    "*.jsx",
    "*.mjs",
    "*.cjs",
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
/// occurrence), and every review must be current. The reviews of [`scope_gate`] are its
/// own, but one of a file in no package of [`PACKAGES`] is stale here, since no scope gate
/// reads it.
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
        let lines: Vec<&str> = text.lines().collect();
        for line in &lines {
            for (token, extensions) in &tokens {
                if extensions.contains(&extension) && line.contains(token.as_str()) {
                    keys.push(format!("{path}: {token} at {}", line.trim()));
                }
            }
            if extension == "rs" && names_miri(line) {
                keys.push(format!("{path}: {MIRI} at {}", line.trim()));
            }
            if let Some(attribute) = silent_doc_test(line).filter(|_| extension == "rs") {
                keys.push(format!(
                    "{path}: {}{attribute} at {}",
                    "`".repeat(3),
                    line.trim()
                ));
            }
        }
        let hints = if BYPASS_TYPESCRIPT.contains(&extension) {
            coverage_hints().to_vec()
        } else if extension == "rs" {
            rust_hints()
        } else {
            Vec::new()
        };
        for hint in hints {
            for at in hint_lines(text, &hint) {
                let line = lines.get(at).map_or("", |line| line.trim());
                keys.push(format!("{path}: {hint} at {line}"));
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
    let dirs = PACKAGES.map(|package| package.dir);
    problems.extend(
        reviews_of(reviewed, true)
            .lines()
            .map(review_key)
            .filter(|key| !key.starts_with('#') && !reviews_below(key, &dirs))
            .map(|key| format!("stale review: {key} (a file of no package)")),
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
/// A source the coverage of its package does not measure, one it measures but Stryker
/// does not mutate, and one with statements that the `mutate` globs select but the mutation
/// report lacks (Stryker found no mutant in it, or read another configuration). Every other
/// review of that file belongs to [`bypass_gate`].
pub const SCOPE_REVIEWS: [&str; 3] = [
    ": not measured",
    ": measured, not mutated",
    ": mutated, no mutant reported",
];

/// The key of a review line: what precedes its reason.
fn review_key(line: &str) -> &str {
    line.split_once(" -- ").map_or(line, |(key, _)| key).trim()
}

/// The lines of `reviewed` that are reviews of [`scope_gate`] (`scope`), or the others.
fn reviews_of(reviewed: &str, scope: bool) -> String {
    reviewed
        .lines()
        .filter(|line| {
            SCOPE_REVIEWS
                .iter()
                .any(|suffix| review_key(line).ends_with(suffix))
                == scope
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The reviews of [`scope_gate`] in `reviewed` that name a file below one of `dirs`.
fn scope_reviews_below(reviewed: &str, dirs: &[&str]) -> String {
    reviews_of(reviewed, true)
        .lines()
        .filter(|line| reviews_below(review_key(line), dirs))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether the review key `key` names a file below one of `dirs`.
fn reviews_below(key: &str, dirs: &[&str]) -> bool {
    dirs.iter().any(|dir| {
        key.strip_prefix(dir)
            .is_some_and(|rest| rest.starts_with('/'))
    })
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
    /// The text of its mutation report (Stryker `json`), complete: its `files` are the
    /// sources Stryker read, relative to the package.
    pub mutation: String,
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

/// The Stryker configuration, the coverage summary and the mutation report of `scope`, or
/// the first that cannot be read.
fn parse_scope(scope: &Scope) -> Result<[serde_json::Value; 3], String> {
    let dir = scope.dir;
    let parse = |text: &str, what: &str| {
        serde_json::from_str(text).map_err(|error| format!("{dir}: invalid {what}: {error}"))
    };
    Ok([
        parse(&scope.stryker, "Stryker configuration")?,
        parse(&scope.summary, "coverage summary")?,
        parse(&scope.mutation, "mutation report")?,
    ])
}

/// Checks that every TypeScript source of the packages of `scopes` is both measured and
/// mutated, or reviewed.
///
/// `tracked` are the files tracked with an extension of [`TS_SOURCES`]; `scopes` what each
/// package checked (one, or several, of [`PACKAGES`]) measures (its coverage summary) and mutates (the `mutate` globs of
/// its Stryker configuration, of which only `*` and `**` are understood). The sources of a
/// package are its tracked files but its tests (`test/`, `*.test.ts`, `*.spec.ts`). Each
/// source must be measured and mutated; one measured but not mutated is a
/// `<source>: measured, not mutated` occurrence, one neither measured nor mutated a
/// `<source>: not measured` occurrence, and one mutated, measured with statements (or with
/// no count of them) but absent from the package's mutation report (which a check makes
/// complete, the incremental runs included) a `<source>: mutated, no mutant reported`
/// occurrence: Stryker found no mutant in it, or ran with another configuration than
/// [`STRYKER_CONFIG`]. Each is to be reviewed in `reviewed`
/// ([`BYPASSES_REVIEWED`], whose other reviews are [`bypass_gate`]'s), every review of a
/// file of these packages current (a review of a file in no package is the bypass gate's
/// to refuse, so that each package can be checked alone). So the coverage `include` and the Stryker `mutate` of a package select the same
/// sources, but for reviewed differences, and no source escapes both unseen.
///
/// # Errors
///
/// Returns every source mutated but not measured, every file measured that is not a source
/// of the package, every `mutate` glob that is not understood or matches no source, every
/// unreadable configuration, summary or mutation report, every mutation report without
/// `files`, and every unreviewed occurrence or stale review.
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
        let [stryker, summary, mutation] = match parse_scope(scope) {
            Ok(parsed) => parsed,
            Err(problem) => {
                problems.push(problem);
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
        let Some(reported) = mutation.get("files").and_then(serde_json::Value::as_object) else {
            problems.push(format!("{dir}: the mutation report lists no `files`"));
            continue;
        };
        // Each source measured, with whether it has statements: one without any (only
        // declarations) has nothing to mutate, and Stryker leaves it out of its report.
        let mut measured = BTreeMap::new();
        for (file, entry) in summary
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(key, _)| *key != "total")
        {
            match sources
                .iter()
                .find(|rel| file.ends_with(&format!("/{prefix}{rel}")))
            {
                Some(rel) => {
                    let statements = entry.pointer("/statements/total");
                    let _new = measured.insert(*rel, statements != Some(&0_u64.into()));
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
            match (measured.get(rel), !matching.is_empty()) {
                (Some(true), true) if !reported.contains_key(*rel) => {
                    keys.push(format!("{prefix}{rel}: mutated, no mutant reported"));
                }
                (Some(_), true) => {}
                (None, true) => problems.push(format!(
                    "{prefix}{rel} is mutated but not measured: the coverage `include` misses it"
                )),
                (Some(_), false) => keys.push(format!("{prefix}{rel}: measured, not mutated")),
                (None, false) => keys.push(format!("{prefix}{rel}: not measured")),
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
    let dirs: Vec<&str> = scopes.iter().map(|scope| scope.dir).collect();
    compare_reviews(
        &count(keys),
        Extent::Complete,
        &scope_reviews_below(reviewed, &dirs),
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

/// The only files allowed to lift the workspace lint against unsafe code, each audited.
///
/// The C interface of the WebAssembly module, the counting allocator of the tests, and the
/// floating-point control register of native engine calls.
pub const UNSAFE_FILES: [&str; 3] = [
    "bindings/wasm/src/ffi.rs",
    "crates/engine-alloc-counter/src/lib.rs",
    "crates/engine-fpenv/src/word.rs",
];

/// The files of [`UNSAFE_FILES`] made of inline assembly, which Miri cannot run.
///
/// Their crates leave the Miri step ([`unsafe_crates`]), and are proven by their own tests
/// on each architecture they support, and by the audit written in the file.
pub const ASSEMBLY_FILES: [&str; 1] = ["crates/engine-fpenv/src/word.rs"];

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
/// Every file of [`ASSEMBLY_FILES`], which leaves the Miri step, must still hold inline
/// assembly (`asm!(`): a file that no longer does would be unsafe code Miri could check.
///
/// # Errors
///
/// Returns every other file that lifts the lint, every listed file that no longer does,
/// every file that should forbid it and does not, and every assembly file without
/// assembly.
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
    problems.extend(
        ASSEMBLY_FILES
            .iter()
            .filter(|file| {
                !sources
                    .iter()
                    .any(|(path, text)| path == *file && text.contains("asm!("))
            })
            .map(|file| format!("{file} is listed in ASSEMBLY_FILES but holds no inline assembly")),
    );
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

/// The crates that may depend on `libm` outside their tests (`spec/engine-design.md`,
/// decision 10).
///
/// `engine-core`, whose math layer is the only path from box arithmetic to a math
/// function, and `engine-testkit`, whose measurements must not share the arithmetic of the
/// boxes they measure. A box crate is `no_std`, where `f32` and `f64` have no math
/// method: without `libm`, it can only reach the math layer's. A test may use `libm`
/// directly (a development dependency), for references computed apart from the code
/// under test.
pub const LIBM_CRATES: [&str; 2] = ["engine-core", "engine-testkit"];

/// The sources of the box crates (`engine-objects`), as git pathspecs: the code the math
/// layer gate reads, its `*_tests.rs` files apart.
pub const BOX_SOURCES: [&str; 1] = ["crates/engine-objects/src/*.rs"];

/// The names box code may not write (`spec/engine-design.md`, decision 10).
///
/// The raw float types, whose operators and methods bypass the math layer, `libm`, and
/// `host`, the module of the engine that reads and writes the floats of samples.
pub const BOX_FORBIDDEN_NAMES: [&str; 4] = ["f32", "f64", "libm", "host"];

/// The words of Rust code in `text`, with their line numbers: maximal runs of letters,
/// digits and `_`, outside comments (line and block) and string literals, which are prose.
fn code_words(text: &str) -> Vec<(usize, String)> {
    /// Ends the word being read, if any, at `line`.
    fn end(found: &mut Vec<(usize, String)>, current: &mut String, line: usize) {
        if !current.is_empty() {
            found.push((line, core::mem::take(current)));
        }
    }
    let mut words = Vec::new();
    let (mut line, mut word) = (1_usize, String::new());
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        if character.is_ascii_alphanumeric() || character == '_' {
            word.push(character);
            continue;
        }
        end(&mut words, &mut word, line);
        match (character, chars.peek()) {
            ('/', Some('/')) => {
                for skipped in chars.by_ref() {
                    if skipped == '\n' {
                        line = line.saturating_add(1);
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                let mut last = ' ';
                for skipped in chars.by_ref() {
                    if skipped == '\n' {
                        line = line.saturating_add(1);
                    }
                    if last == '*' && skipped == '/' {
                        break;
                    }
                    last = skipped;
                }
            }
            ('"', _) => {
                let mut escaped = false;
                for skipped in chars.by_ref() {
                    if skipped == '\n' {
                        line = line.saturating_add(1);
                    }
                    if !escaped && skipped == '"' {
                        break;
                    }
                    escaped = !escaped && skipped == '\\';
                }
            }
            ('\n', _) => line = line.saturating_add(1),
            _ => {}
        }
    }
    end(&mut words, &mut word, line);
    words
}

/// Checks that box arithmetic can only go through the math layer.
///
/// No crate but those of [`LIBM_CRATES`] depends on `libm`, as a normal or a build
/// dependency, under any name it is renamed to, and box code names none of
/// [`BOX_FORBIDDEN_NAMES`], nor a float literal suffixed `f32` or `f64`.
///
/// `metadata` is the output of [`metadata_command`]; `sources` the files of [`BOX_SOURCES`]
/// (path and text), of which the `*_tests.rs` files are test code, not read. So a box crate,
/// `no_std` and without `libm`, computes with the engine's sample types alone, whose
/// operators and math functions are the math layer's.
///
/// # Errors
///
/// Returns every other crate that depends on `libm` and every forbidden name of box code,
/// with its file and line, or why the metadata cannot be read.
pub fn math_layer_gate(metadata: &str, sources: &[(String, String)]) -> Result<(), Failure> {
    let json: serde_json::Value = serde_json::from_str(metadata)
        .map_err(|error| Failure::from(format!("invalid cargo metadata: {error}")))?;
    let crates = json
        .get("packages")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| Failure::from("cargo metadata lists no packages".to_owned()))?;
    let mut problems = Vec::new();
    for krate in crates {
        let name = krate
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let reaches_libm = krate
            .get("dependencies")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .any(|dependency| {
                let text = |key: &str| dependency.get(key).and_then(serde_json::Value::as_str);
                text("name") == Some("libm") && text("kind") != Some("dev")
            });
        if reaches_libm && !LIBM_CRATES.contains(&name) {
            problems.push(format!(
                "{name} depends on libm: its arithmetic must go through the math layer of \
                 engine-core"
            ));
        }
    }
    for (path, text) in sources
        .iter()
        .filter(|(path, _)| !path.ends_with("_tests.rs"))
    {
        for (line, word) in code_words(text) {
            let suffixed = word.starts_with(|first: char| first.is_ascii_digit())
                && (word.ends_with("f32") || word.ends_with("f64"));
            if suffixed || BOX_FORBIDDEN_NAMES.contains(&word.as_str()) {
                problems.push(format!(
                    "{path}:{line}: box code names `{word}`: it computes with the engine's \
                     sample types, through the math layer"
                ));
            }
        }
    }
    verdict(
        &format!(
            "box arithmetic only through the math layer ({} only may depend on libm; box code \
             names no raw float, libm or host)",
            LIBM_CRATES.join(", ")
        ),
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

/// The Clippy configuration, whose `disallowed-methods` ban the float methods whose result
/// may differ between targets.
pub const CLIPPY_TOML: &str = "clippy.toml";

/// The sources of the inherent methods of `f32` and `f64` in the standard library.
///
/// Relative to the sysroot of the pinned nightly ([`NIGHTLY_TOOLCHAIN`]), whose `rust-src`
/// component Miri already needs: the pinned release has no sources of its own, since
/// installing them would make the compiler write their local paths into the engine's
/// module, which would no longer be reproducible. The nightly is newer than the release,
/// and each method's `#[stable(since = …)]` says whether the release has it. `core` holds
/// the methods that need no operating system, `std` those that call the platform's math
/// library.
pub const FLOAT_SOURCES: [&str; 4] = [
    "lib/rustlib/src/rust/library/core/src/num/f32.rs",
    "lib/rustlib/src/rust/library/core/src/num/f64.rs",
    "lib/rustlib/src/rust/library/std/src/num/f32.rs",
    "lib/rustlib/src/rust/library/std/src/num/f64.rs",
];

/// The macros a float's `impl` block may call among its methods, each with the source of
/// the nightly that defines it (relative to the sysroot, as [`FLOAT_SOURCES`]).
///
/// A macro called there could define methods the audit would not see, so
/// [`parse_float_methods`] refuses any other, and [`float_gate`] audits each source here:
/// it must define the macro and declare no function nor call a macro, so that the macro
/// defines only constants. `float_impl!`, shared by every float type of `core`, defines
/// their constants.
pub const FLOAT_ITEM_MACROS: [(&str, &str); 1] = [(
    "float_impl",
    "lib/rustlib/src/rust/library/core/src/num/float_macros.rs",
)];

/// The stable float methods reviewed as giving the same result on every target.
///
/// For `f32` and `f64` alike, with the reason. Every other stable method of the pinned toolchain must
/// be banned in [`CLIPPY_TOML`] ([`float_gate`]).
pub const DETERMINISTIC_FLOAT_METHODS: [(&str, &str); 32] = [
    ("abs", "clears the sign bit"),
    ("ceil", "exact: an integer value, rounded toward +infinity"),
    ("classify", "reads the bits"),
    (
        "copysign",
        "copies a sign bit; a NaN's bits, which arithmetic may set differently per target, are outside this audit (roadmap #12, output stage)",
    ),
    (
        "div_euclid",
        "a correctly rounded division, then exact rounding to an integer value",
    ),
    ("floor", "exact: an integer value, rounded toward -infinity"),
    ("fract", "exact: the value less its integer part"),
    ("from_be_bytes", "a bit copy in a fixed byte order"),
    ("from_bits", "a bit copy"),
    ("from_le_bytes", "a bit copy in a fixed byte order"),
    ("is_finite", "reads the bits"),
    ("is_infinite", "reads the bits"),
    ("is_nan", "reads the bits"),
    ("is_normal", "reads the bits"),
    (
        "is_sign_negative",
        "reads the sign bit; a NaN's bits, which arithmetic may set differently per target, are outside this audit (roadmap #12, output stage)",
    ),
    (
        "is_sign_positive",
        "reads the sign bit; a NaN's bits, which arithmetic may set differently per target, are outside this audit (roadmap #12, output stage)",
    ),
    ("is_subnormal", "reads the bits"),
    (
        "midpoint",
        "correctly rounded additions and exact halvings, in a fixed order",
    ),
    (
        "next_down",
        "the next value toward -infinity, from the bits (IEEE 754 nextDown)",
    ),
    (
        "next_up",
        "the next value toward +infinity, from the bits (IEEE 754 nextUp)",
    ),
    ("recip", "a correctly rounded division"),
    (
        "rem_euclid",
        "an exact remainder, then a correctly rounded addition",
    ),
    ("round", "exact: an integer value, ties away from zero"),
    ("round_ties_even", "exact: an integer value, ties to even"),
    ("signum", "a constant of the sign of the value"),
    ("sqrt", "correctly rounded by IEEE 754 on every target"),
    ("to_be_bytes", "a bit copy in a fixed byte order"),
    (
        "to_bits",
        "a bit copy; a NaN's bits, which arithmetic may set differently per target, are outside this audit (roadmap #12, output stage)",
    ),
    (
        "to_int_unchecked",
        "an unsafe function: every crate forbids unsafe code, so none can call it",
    ),
    ("to_le_bytes", "a bit copy in a fixed byte order"),
    ("total_cmp", "orders the bits"),
    ("trunc", "exact: an integer value, rounded toward zero"),
];

/// A release number, `X.Y.Z`, ordered as releases are.
pub type Release = (u32, u32, u32);

/// Reads a release number written `X.Y.Z`, nothing else.
#[must_use]
pub fn parse_release(text: &str) -> Option<Release> {
    let mut parts = text.split('.').map(|part| {
        part.bytes()
            .all(|byte| byte.is_ascii_digit())
            .then(|| part.parse::<u32>().ok())
            .flatten()
    });
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(Some(major)), Some(Some(minor)), Some(Some(patch)), None) => {
            Some((major, minor, patch))
        }
        _ => None,
    }
}

/// One inherent method of `f32` or `f64` found in the standard library's sources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FloatMethod {
    /// `f32` or `f64`.
    pub float: String,
    /// The method's name.
    pub name: String,
    /// The release that stabilized it; `None` when it is unstable, or stabilized by the
    /// nightly only (`since = "CURRENT_RUSTC_VERSION"`).
    pub since: Option<Release>,
    /// Whether its documentation says its result may differ ("non-deterministic",
    /// "Unspecified precision").
    pub documented_varying: bool,
}

/// Reads the inherent methods of `f32` or `f64` from one source of the standard library.
///
/// The single `impl f32 {` or `impl f64 {` block at the start of a line, to its closing `}`
/// at the start of a line; each method is a line `    pub [const] [unsafe] fn <name>`,
/// after its documentation and its `#[stable(…, since = "X.Y.Z")]` (or
/// `"CURRENT_RUSTC_VERSION"`) or `#[unstable(` attribute. A line indented by fewer than
/// eight spaces is among the block's items, not in a method's body: a macro called there
/// could define methods, so it must be one of [`FLOAT_ITEM_MACROS`], whose sources
/// [`float_gate`] audits.
///
/// # Errors
///
/// Returns what is not in that shape: no such block or more than one, a block never closed,
/// a line declaring a function that is not read as a method, a method without a stability
/// attribute, a `since` of another shape, a macro called among the items that is not
/// reviewed, or no method at all. Nothing is skipped.
pub fn parse_float_methods(path: &str, source: &str) -> Result<Vec<FloatMethod>, String> {
    let lines: Vec<&str> = source.lines().collect();
    let starts: Vec<(usize, &str)> = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| match *line {
            "impl f32 {" => Some((index, "f32")),
            "impl f64 {" => Some((index, "f64")),
            _ => None,
        })
        .collect();
    let [(start, float)] = starts.as_slice() else {
        return Err(format!(
            "{path}: {} `impl f32 {{` or `impl f64 {{` blocks, expected one",
            starts.len()
        ));
    };
    let body = lines.get(start.saturating_add(1)..).unwrap_or_default();
    let Some(end) = body.iter().position(|line| *line == "}") else {
        return Err(format!(
            "{path}: the `impl {float} {{` block is never closed"
        ));
    };
    let mut methods = Vec::new();
    let mut doc = String::new();
    let mut stability: Option<Option<Release>> = None;
    for line in body.iter().take(end) {
        let trimmed = line.trim_start();
        if trimmed.starts_with("#[stable(") {
            let since = trimmed
                .split_once("since = \"")
                .and_then(|(_, rest)| rest.split_once('"'))
                .map(|(since, _)| since);
            stability = match since {
                Some("CURRENT_RUSTC_VERSION") => Some(None),
                Some(release) if parse_release(release).is_some() => Some(parse_release(release)),
                _ => {
                    return Err(format!(
                        "{path}: `{}` gives no release the method was stabilized in",
                        trimmed.trim_end()
                    ));
                }
            };
        } else if trimmed.starts_with("#[unstable(") {
            stability = Some(None);
        } else if trimmed.starts_with("///") {
            doc.push_str(trimmed);
            doc.push('\n');
        }
        let item = !line.starts_with("        ");
        let unreviewed = macro_call(trimmed).is_some_and(|name| {
            item && !FLOAT_ITEM_MACROS
                .iter()
                .any(|(reviewed, _)| *reviewed == name)
        });
        if unreviewed {
            return Err(format!(
                "{path}: `{}` calls a macro among the items of the `impl {float} {{` block that is not reviewed as defining only constants (FLOAT_ITEM_MACROS in xtask)",
                trimmed.trim_end()
            ));
        }
        let declares = trimmed.starts_with("pub ") && trimmed.contains("fn ");
        if !declares {
            continue;
        }
        let name = line
            .strip_prefix("    pub ")
            .map(|rest| {
                rest.trim_start_matches("const ")
                    .trim_start_matches("unsafe ")
            })
            .and_then(|rest| rest.strip_prefix("fn "))
            .map(|rest| {
                rest.chars()
                    .take_while(|letter| {
                        letter.is_ascii_lowercase() || letter.is_ascii_digit() || *letter == '_'
                    })
                    .collect::<String>()
            })
            .filter(|name| !name.is_empty());
        let (Some(name), Some(since)) = (name, stability) else {
            return Err(format!(
                "{path}: `{}` is not a method of the expected shape, with its stability",
                line.trim()
            ));
        };
        let documented_varying =
            doc.contains("non-deterministic") || doc.contains("Unspecified precision");
        methods.push(FloatMethod {
            float: (*float).to_owned(),
            name,
            since,
            documented_varying,
        });
        doc.clear();
        stability = None;
    }
    if methods.is_empty() {
        return Err(format!("{path}: no method in the `impl {float} {{` block"));
    }
    Ok(methods)
}

/// The name of the macro a line calls, when it starts with one: `name!` (an empty name
/// when it starts with `!`, which is no Rust item either).
fn macro_call(trimmed: &str) -> Option<&str> {
    let end = trimmed.find(|letter: char| !(letter.is_ascii_alphanumeric() || letter == '_'))?;
    let (name, rest) = trimmed.split_at(end);
    rest.starts_with('!').then_some(name)
}

/// Checks that a macro of [`FLOAT_ITEM_MACROS`] defines only constants.
///
/// Its source must define it (a line holding `macro <name>(`), and no line may declare a
/// function (hold `fn ` outside a comment or an attribute, private ones included) or call a
/// macro, which could declare one.
///
/// # Errors
///
/// Returns the missing definition, or the first line that may define a method.
pub fn audit_float_macro(path: &str, source: &str, name: &str) -> Result<(), String> {
    let definition = format!("macro {name}(");
    if !source.lines().any(|line| line.contains(&definition)) {
        return Err(format!(
            "{path}: no `{definition}` definition, reviewed in FLOAT_ITEM_MACROS"
        ));
    }
    for line in source.lines().map(str::trim) {
        let declares = line.contains("fn ") && !line.starts_with("//") && !line.starts_with("#[");
        if declares || macro_call(line).is_some() {
            return Err(format!(
                "{path}: `{line}` may define a method in `{name}!`, which is reviewed as defining only constants"
            ));
        }
    }
    Ok(())
}

/// The float methods `clippy.toml` bans: every `{ path = "f32::…" }` or `{ path = "f64::…" }`
/// of its `disallowed-methods`, as `f32::name`.
fn banned_float_methods(clippy: &str) -> BTreeSet<String> {
    clippy
        .lines()
        .filter_map(|line| line.trim().strip_prefix("{ path = \""))
        .filter_map(|rest| rest.split('"').next())
        .filter(|path| path.starts_with("f32::") || path.starts_with("f64::"))
        .map(str::to_owned)
        .collect()
}

/// Checks that every stable float method of the pinned release is banned or reviewed.
///
/// A method must give the same result on every target or be banned: each is banned in `clippy` (the text of [`CLIPPY_TOML`]) or
/// reviewed in [`DETERMINISTIC_FLOAT_METHODS`], never both, and never reviewed when its own
/// documentation says its result may differ; every ban and every review names a method
/// that exists. So a new toolchain cannot bring a method no one classified.
///
/// `sources` holds the path and text of each file of [`FLOAT_SOURCES`] and of
/// [`FLOAT_ITEM_MACROS`] (each audited by [`audit_float_macro`]), read from a toolchain whose `rustc --version` is `sources_version`; `release` is the pinned release:
/// its stable methods are those stabilized in it or before.
///
/// # Errors
///
/// Returns sources not newer than the release, every source not in the expected shape
/// ([`parse_float_methods`], [`audit_float_macro`]) or missing, and every method or entry
/// breaking a rule above.
pub fn float_gate(
    sources: &[(String, String)],
    sources_version: &str,
    release: Release,
    clippy: &str,
) -> Result<(), Failure> {
    let mut problems = Vec::new();
    let newer = sources_version
        .split_whitespace()
        .nth(1)
        .and_then(|version| version.split('-').next())
        .and_then(parse_release)
        .is_some_and(|version| version >= release);
    if !newer {
        problems.push(format!(
            "the sources come from `{}`, not from a release at least {}.{}.{}: they may lack its methods",
            sources_version.trim(),
            release.0,
            release.1,
            release.2
        ));
    }
    let mut methods = Vec::new();
    for (path, text) in sources {
        let audited = match FLOAT_ITEM_MACROS
            .iter()
            .find(|(_, source)| path.ends_with(source))
        {
            Some((name, _)) => audit_float_macro(path, text, name).map(|()| Vec::new()),
            None => parse_float_methods(path, text),
        };
        match audited {
            Ok(found) => methods.extend(found),
            Err(problem) => problems.push(problem),
        }
    }
    problems.extend(
        FLOAT_ITEM_MACROS
            .iter()
            .filter(|(_, source)| !sources.iter().any(|(path, _)| path.ends_with(source)))
            .map(|(name, source)| {
                format!("`{name}!` is reviewed as defining only constants, but its source {source} is not audited")
            }),
    );
    let banned = banned_float_methods(clippy);
    let reviewed: BTreeMap<&str, &str> = DETERMINISTIC_FLOAT_METHODS.into_iter().collect();
    let in_release = |method: &FloatMethod| method.since.is_some_and(|since| since <= release);
    for method in methods.iter().filter(|method| in_release(method)) {
        let path = format!("{}::{}", method.float, method.name);
        let is_banned = banned.contains(&path);
        let is_reviewed = reviewed.contains_key(method.name.as_str());
        if is_banned && is_reviewed {
            problems.push(format!(
                "`{path}` is both banned in {CLIPPY_TOML} and reviewed as deterministic"
            ));
        } else if !is_banned && !is_reviewed {
            problems.push(format!(
                "`{path}` is neither banned in {CLIPPY_TOML} nor reviewed as deterministic (DETERMINISTIC_FLOAT_METHODS in xtask)"
            ));
        } else if is_reviewed && method.documented_varying {
            problems.push(format!(
                "`{path}` is reviewed as deterministic, but its documentation says its result may differ: ban it in {CLIPPY_TOML}"
            ));
        }
    }
    let found: BTreeSet<String> = methods
        .iter()
        .map(|method| format!("{}::{}", method.float, method.name))
        .collect();
    problems.extend(
        banned
            .iter()
            .filter(|path| !found.contains(*path))
            .map(|path| {
                format!("{CLIPPY_TOML} bans `{path}`, which the pinned toolchain does not have")
            }),
    );
    problems.extend(
        reviewed
            .keys()
            .filter(|name| {
                !methods
                    .iter()
                    .any(|method| in_release(method) && method.name == **name)
            })
            .map(|name| format!("`{name}` is reviewed as deterministic, but no stable float method has that name")),
    );
    verdict("the float methods of the pinned toolchain", problems)
}

/// Reads the pinned release, the pinned nightly's float sources and `clippy.toml`, then
/// checks them.
fn float_methods(runner: &mut dyn Runner) -> Result<(), Failure> {
    let toolchain = read_file(runner, RUST_TOOLCHAIN_FILE)?;
    let release = toolchain_channel(&toolchain)
        .and_then(parse_release)
        .ok_or_else(|| {
            Failure::from(format!(
                "{RUST_TOOLCHAIN_FILE} must pin one release: channel = \"X.Y.Z\""
            ))
        })?;
    let mut capture = |args: &[&str]| {
        runner
            .capture(&Cmd::new("rustc", args, Dir::Root))
            .map_err(|failure| {
                Failure::from(format!("cannot find the toolchain's sources: {failure}"))
            })
    };
    let version = capture(&[NIGHTLY_TOOLCHAIN, "--version"])?;
    let sysroot = capture(&[NIGHTLY_TOOLCHAIN, "--print", "sysroot"])?;
    let paths = FLOAT_SOURCES
        .iter()
        .chain(FLOAT_ITEM_MACROS.iter().map(|(_, source)| source))
        .map(|source| format!("{}/{source}", sysroot.trim_end()))
        .collect();
    let sources = read_all(runner, paths)?;
    float_gate(
        &sources,
        &version,
        release,
        &read_file(runner, CLIPPY_TOML)?,
    )
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
    /// The Rust compiler, exactly the release of [`RUST_TOOLCHAIN_FILE`]: the second word
    /// of its answer, as in `rustc 1.99.1 (…)`.
    Toolchain,
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
pub const TOOLS: [Tool; 10] = [
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
    Tool {
        program: "rustc",
        args: &["--version"],
        need: Need::Toolchain,
        install: Install::Text(
            "the release of rust-toolchain.toml, which rustup installs: remove what overrides it (`rustup override unset`)",
        ),
    },
];

/// The file pinning the Rust toolchain, which rustup reads.
pub const RUST_TOOLCHAIN_FILE: &str = "rust-toolchain.toml";

/// The variables that change what cargo builds or how it lints, refused by the preflight.
///
/// Set outside the repository, they would make a local check differ from CI (lints lifted,
/// another target, profile or compiler wrapper, another `clippy.toml`).
pub const REFUSED_VARIABLES: [&str; 9] = [
    "RUSTFLAGS",
    "RUSTDOCFLAGS",
    "CARGO_ENCODED_RUSTFLAGS",
    "CARGO_ENCODED_RUSTDOCFLAGS",
    "RUSTC",
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "RUSTDOC",
    "CLIPPY_CONF_DIR",
];

/// The prefixes of the variables that set Cargo's `build`, `target`, `profile` and
/// `unstable` configuration, which the preflight refuses (`CARGO_TARGET_DIR` apart, which
/// only moves the build).
pub const REFUSED_VARIABLE_PREFIXES: [&str; 4] = [
    "CARGO_BUILD_",
    "CARGO_TARGET_",
    "CARGO_PROFILE_",
    "CARGO_UNSTABLE_",
];

/// Whether the variable `name` is one the preflight refuses: one of [`REFUSED_VARIABLES`],
/// or a name starting with one of [`REFUSED_VARIABLE_PREFIXES`] other than
/// `CARGO_TARGET_DIR`.
#[must_use]
pub fn refused_variable(name: &str) -> bool {
    REFUSED_VARIABLES.contains(&name)
        || (name != "CARGO_TARGET_DIR"
            && REFUSED_VARIABLE_PREFIXES
                .iter()
                .any(|prefix| name.starts_with(prefix)))
}

/// What the environment of the checks holds that could change cargo from outside the
/// repository.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Environment {
    /// The names of the environment variables.
    pub variables: Vec<String>,
    /// The Cargo configuration files that exist outside the repository's own
    /// [`CARGO_CONFIG_FILE`] (see [`cargo_config_candidates`]).
    pub cargo_configs: Vec<String>,
}

/// Every Cargo configuration file that cargo would read for a command run at `root`, other
/// than the repository's own [`CARGO_CONFIG_FILE`], sorted.
///
/// `config` and `config.toml` in the Cargo home (`cargo_home`, else `.cargo` in `home`),
/// and in the `.cargo` folder of `root` and of every directory above it.
#[must_use]
pub fn cargo_config_candidates(
    root: &Path,
    cargo_home: Option<OsString>,
    home: Option<OsString>,
) -> Vec<PathBuf> {
    let cargo_home = cargo_home
        .map(PathBuf::from)
        .or_else(|| home.map(|home| PathBuf::from(home).join(".cargo")));
    let own = root.join(CARGO_CONFIG_FILE);
    cargo_home
        .into_iter()
        .chain(root.ancestors().map(|dir| dir.join(".cargo")))
        .flat_map(|dir| [dir.join("config"), dir.join("config.toml")])
        .filter(|path| *path != own)
        .collect::<BTreeSet<PathBuf>>()
        .into_iter()
        .collect()
}

/// The release a toolchain file pins: its line `channel = "<release>"`.
#[must_use]
pub fn toolchain_channel(toolchain: &str) -> Option<&str> {
    toolchain.lines().find_map(|line| {
        line.trim()
            .strip_prefix("channel")?
            .trim_start()
            .strip_prefix('=')?
            .trim()
            .strip_prefix('"')?
            .strip_suffix('"')
            .filter(|channel| !channel.is_empty())
    })
}

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
/// release of `node_version` (the text of [`NODE_VERSION_FILE`], which CI installs, so that a
/// local check differs from CI only by platform), and the Rust compiler at exactly the release of
/// `toolchain` (the text of [`RUST_TOOLCHAIN_FILE`]: an override, a `+toolchain` or
/// `RUSTUP_TOOLCHAIN` would replace it), on a platform the checks support (macOS or Linux;
/// Windows through WSL2), in an `environment` that sets none of the variables of
/// [`refused_variable`] and has no Cargo configuration outside the repository's own.
///
/// # Errors
///
/// Returns one failure listing every problem found, each with how to fix it: an
/// unsupported platform, a tool that did not answer (with its own reason), or a tool at a
/// version the checks do not accept (Node.js included, which must be `node_version`, and
/// the Rust compiler, which must be the channel of `toolchain`), a refused variable, or a
/// Cargo configuration file outside the repository.
pub fn preflight_gate(
    os: &str,
    answers: &[Result<String, CommandFailure>],
    node_version: &str,
    toolchain: &str,
    environment: &Environment,
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
            Need::Toolchain => match toolchain_channel(toolchain) {
                None => problems.push(format!(
                    "{RUST_TOOLCHAIN_FILE} must pin one release: channel = \"X.Y.Z\""
                )),
                Some(channel) => {
                    if version.split_whitespace().nth(1) != Some(channel) {
                        problems.push(format!(
                            "{version} is installed, the checks need {channel} of {RUST_TOOLCHAIN_FILE}: {install}"
                        ));
                    }
                }
            },
            Need::Pinned((name, needed)) => {
                if version.split_whitespace().nth(1) != Some(needed) {
                    problems.push(format!(
                        "{version} is installed, the checks need {name} {needed}: {install}"
                    ));
                }
            }
        }
    }
    problems.extend(
        environment
            .variables
            .iter()
            .filter(|name| refused_variable(name))
            .map(|name| {
                format!("{name} is set, which changes what cargo builds or how it lints: unset it")
            }),
    );
    problems.extend(environment.cargo_configs.iter().map(|path| {
        format!(
            "{path} configures every cargo command here: move it away (the configuration of the repository is {CARGO_CONFIG_FILE})"
        )
    }));
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
/// secrets, build flags that could lift lints, and a matrix exclusion, which could drop a
/// part of the full check.
pub const WORKFLOW_REFUSED: [&str; 6] = [
    "continue-on-error",
    "pull_request_target",
    "workflow_run",
    "secrets.",
    "RUSTFLAGS",
    "exclude:",
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

/// The commands that give every job of the check an audio output, in this order.
///
/// The runner has no sound card, and Firefox plays live only through an audio server
/// (Chromium runs with its audio output disabled, and Playwright's `WebKit` on Linux plays
/// without one): without it, every live test of the engine package and of the playground
/// fails in Firefox alone. The `pulseaudio` server, which the runner's image does not
/// hold, is installed from the OS release at an exact version without its recommended
/// packages (a new Ubuntu update of it fails this step until the version is changed here,
/// rather than changing CI without a commit), started for the whole job with a null sink
/// as the default output, and the last line fails the step, with one line naming the
/// cause, unless that sink is the default.
pub const AUDIO_SINK: [&str; 6] = [
    "sudo apt-get update",
    "sudo apt-get install --yes --no-install-recommends pulseaudio=1:16.1+dfsg1-2ubuntu10.1",
    "pulseaudio --start --exit-idle-time=-1",
    "pactl load-module module-null-sink sink_name=null",
    "pactl set-default-sink null",
    "pactl get-default-sink | grep --quiet --line-regexp null || { echo \"::error::no audio sink: Firefox cannot play live (the runner has no sound card)\"; exit 1; }",
];

/// The problem of `workflow` if its lines, trimmed, do not hold [`AUDIO_SINK`] as
/// consecutive lines.
fn audio_sink_problem(workflow: &str) -> Option<String> {
    let lines: Vec<&str> = workflow.lines().map(str::trim).collect();
    let held = lines
        .windows(AUDIO_SINK.len())
        .any(|window| window == AUDIO_SINK);
    (!held).then(|| {
        format!(
            "{WORKFLOW} lacks the audio sink, these lines in this order: `{}`",
            AUDIO_SINK.join("` `")
        )
    })
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

/// The exact concurrency of the workflow.
///
/// A newer run of the check cancels the older one of the same ref, while a run that the
/// `[ci]` condition of [`WORKFLOW_CONDITIONS`] skips gets a group of its own, so that it
/// never cancels a check in progress and then skips its own.
#[must_use]
pub fn workflow_concurrency() -> [String; 3] {
    let [_, condition] = WORKFLOW_CONDITIONS;
    let condition = condition.strip_prefix("if: ").unwrap_or(condition);
    [
        "concurrency:".to_owned(),
        format!(
            "  group: ${{{{ ({condition}) && format('quality-{{0}}', github.ref) || format('skipped-{{0}}', github.run_id) }}}}"
        ),
        "  cancel-in-progress: true".to_owned(),
    ]
}

/// The lines that make the workflow run the parts of the full check ([`full_parts`]).
///
/// A job `check` runs every part but the final one, one per job of its matrix (all of them
/// whatever fails, as the full check runs every step); each uploads the unviable mutants of
/// its shard, if it ran one, as the artifact `unviable-<part>`. A job that needs it
/// downloads them below [`SHARD_REPORTS`] and runs the final part, which reviews them
/// together ([`shard_unviable_report`]).
#[must_use]
pub fn workflow_parts() -> Vec<String> {
    let matrix: Vec<String> = full_parts()
        .into_iter()
        .filter(|part| *part != Part::Final)
        .map(Part::name)
        .collect();
    let each = "${{ matrix.part }}";
    vec![
        "fail-fast: false".to_owned(),
        format!("part: [{}]", matrix.join(", ")),
        format!("run: cargo xtask check --full --part {each}"),
        format!("name: {UNVIABLE_ARTIFACT}{each}"),
        format!("path: {UNVIABLE_REPORT}"),
        "needs: check".to_owned(),
        format!("pattern: {UNVIABLE_ARTIFACT}*"),
        format!("path: {SHARD_REPORTS}"),
        format!(
            "run: cargo xtask check --full --part {}",
            Part::Final.name()
        ),
    ]
}

/// The problems of the jobs of `workflow`: every job, written `  <name>:` below `jobs:`,
/// must hold the `[ci]` condition of [`WORKFLOW_CONDITIONS`] (at the indentation of its
/// keys), so that a push skips them all or none; any other line at the indentation of a job
/// is refused rather than skipped.
fn job_problems(workflow: &str) -> Vec<String> {
    let [_, condition] = WORKFLOW_CONDITIONS;
    let held = format!("    {condition}");
    let mut problems = Vec::new();
    let mut jobs: Vec<(&str, bool)> = Vec::new();
    let mut in_jobs = false;
    for line in workflow.lines().map(str::trim_end) {
        if line == "jobs:" {
            in_jobs = true;
            continue;
        }
        if !line.is_empty() && !line.starts_with(' ') {
            in_jobs = false;
        }
        if !in_jobs || line.is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if line.starts_with("   ") {
            if line == held
                && let Some(job) = jobs.last_mut()
            {
                job.1 = true;
            }
            continue;
        }
        match line
            .strip_prefix("  ")
            .and_then(|rest| rest.strip_suffix(':'))
            .filter(|name| {
                !name.is_empty()
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
            }) {
            Some(name) => jobs.push((name, false)),
            None => problems.push(format!("`{}`: a job is written `  <name>:`", line.trim())),
        }
    }
    problems.extend(
        jobs.into_iter()
            .filter(|(_, conditioned)| !conditioned)
            .map(|(name, _)| format!("job `{name}` lacks `{condition}`, which every job holds")),
    );
    problems
}

/// Checks that CI cannot change under the project's feet, nor pass without checking.
///
/// `workflows` are the workflow files tracked by git: only [`WORKFLOW`]. In it, every line
/// using an action pins it to a full commit hash, whatever its form (`- uses:`, a flow
/// mapping); nothing of [`WORKFLOW_REFUSED`] appears; the only conditions are those of
/// [`WORKFLOW_CONDITIONS`]; the triggers, the token's permissions and the concurrency are
/// exactly [`WORKFLOW_TRIGGERS`], [`WORKFLOW_PERMISSIONS`] and [`workflow_concurrency`]; every nightly toolchain named is
/// [`NIGHTLY_TOOLCHAIN`]; the checkout keeps no credential; the Cargo tools are
/// [`PINNED_TOOLS`]; Node.js is the release in `.node-version` (at [`NODE_FLOOR`] or later);
/// the runner is an OS release (never `-latest`); the whole history is fetched for the
/// history gate; the jobs get an audio output, as [`AUDIO_SINK`] says; the workflow runs
/// every part of `cargo xtask check --full`, as [`workflow_parts`] says; every job, written `  <name>:` below `jobs:`, holds the `[ci]` condition at the
/// indentation of its keys (any other line at the indentation of a job is refused); its Rust
/// cache is saved even when the run fails; and a failed run uploads every
/// [`failure_uploads`] path.
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
    let concurrency = workflow_concurrency();
    let concurrency: Vec<&str> = concurrency.iter().map(String::as_str).collect();
    for (block, what) in [
        (&WORKFLOW_TRIGGERS[..], "triggers"),
        (&WORKFLOW_PERMISSIONS[..], "permissions"),
        (&concurrency[..], "concurrency"),
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
        "cache-on-failure: true".to_owned(),
    ]
    .into_iter()
    .chain(workflow_parts())
    .chain(failure_uploads())
    {
        if !workflow.lines().any(|line| line.trim() == expected) {
            problems.push(format!("{WORKFLOW} lacks `{expected}`"));
        }
    }
    problems.extend(audio_sink_problem(workflow));
    problems.extend(job_problems(workflow));
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
/// `Revert "…"`): write `revert: …`. The marker [`CI_MARKER`] may only stand alone on a
/// line ([`ci_marker`]).
///
/// # Errors
///
/// Returns a message explaining the expected format, or that the message names
/// [`CI_MARKER`] other than alone on a line.
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
/// on a line of its own, the one way to ask for CI. GitHub's `contains` ignores case
/// (`[CI]` starts CI too), so the marker is read in any case, both sides uppercased.
#[must_use]
pub fn ci_marker(message: &str) -> Option<String> {
    let marker = CI_MARKER.to_uppercase();
    message
        .lines()
        .map(str::to_uppercase)
        .any(|line| line.contains(&marker) && line.trim() != marker)
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

/// Whether `message` is exactly the one GitHub writes for the merge it checks out to run a
/// pull request: `Merge <40 hex> into <40 hex>`, in lowercase, alone (as `git log` writes
/// it, followed by newlines).
fn synthetic_merge(message: &str) -> bool {
    let hash = |text: &str| {
        text.len() == 40
            && text
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    };
    // `git log` ends the message with one newline more than it holds: none or one.
    message
        .trim_end_matches('\n')
        .strip_prefix("Merge ")
        .and_then(|line| line.split_once(" into "))
        .is_some_and(|(head, base)| hash(head) && hash(base))
}

/// Checks the history of commits.
///
/// Every commit message, as recorded, follows the convention ([`commit_msg_gate`]), and every commit
/// since [`CHANGELOG_SINCE`] that changes behavior touches `CHANGELOG.md`.
///
/// One commit is not held to the convention: the first of `messages`, the commit checked
/// out, when its message is exactly the one GitHub writes for the merge it checks out to
/// run a pull request (`Merge <commit> into <commit>`). That merge is never recorded (the
/// repository allows rebase merges only), so the same message anywhere else is refused.
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
    let records = messages.split('\u{1e}').filter(|record| !record.is_empty());
    for (index, record) in records.enumerate() {
        let (hash, message) = record.split_once('\u{1f}').unwrap_or((record, ""));
        if index == 0 && synthetic_merge(message) {
            continue;
        }
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
/// The working tree has nothing uncommitted (`status`, from `git status --porcelain
/// --untracked-files=all --ignore-submodules=none`), and
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

    /// What the environment holds that could change cargo from outside the repository.
    fn environment(&mut self) -> Environment;
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

/// A command for `program` in `dir`, without the variables of [`LOCAL_GIT_VARIABLES`], nor
/// `RUSTUP_TOOLCHAIN`, reading no input (a command waiting for an answer fails instead of
/// blocking the check), and for git with `LC_ALL=C` (its messages untranslated, as
/// [`error_reason`] reads them).
///
/// Rustup sets that variable for xtask itself, to the toolchain it chose for it (`cargo
/// +beta xtask`, `RUSTUP_TOOLCHAIN=stable`): inherited, it would replace the toolchain of
/// [`RUST_TOOLCHAIN_FILE`] for every command. Without it, each command runs with the
/// toolchain of the file (or of an override, which the preflight refuses).
fn command(program: &str, dir: &Path) -> Command {
    let mut command = Command::new(program);
    let _in_dir = command.current_dir(dir).stdin(Stdio::null());
    if program == "git" {
        let _untranslated = command.env("LC_ALL", "C");
    }
    let _pinned = command.env_remove("RUSTUP_TOOLCHAIN");
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
        // Separated by NUL, every name as it is: git quotes a name out of ASCII otherwise.
        let mut args = vec!["ls-files", "-z", "--"];
        args.extend(patterns);
        self.capture(&Cmd::new("git", &args, Dir::Root))
            .map(|text| {
                text.split('\0')
                    .filter(|path| !path.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
    }

    fn write(&mut self, path: &str, text: &str) -> io::Result<()> {
        let file = self.root.join(path);
        file.parent().map_or(Ok(()), std::fs::create_dir_all)?;
        std::fs::write(file, text)
    }

    fn parallelism(&mut self) -> NonZeroUsize {
        std::thread::available_parallelism().unwrap_or(NonZeroUsize::MIN)
    }

    /// The names of xtask's own variables, which every command inherits, and the Cargo
    /// configuration files that exist above the root, resolved (a root that cannot be
    /// resolved is walked up as it is).
    fn environment(&mut self) -> Environment {
        let root = std::fs::canonicalize(&self.root).unwrap_or_else(|_| self.root.clone());
        Environment {
            variables: std::env::vars_os()
                .map(|(name, _)| name.to_string_lossy().into_owned())
                .collect(),
            cargo_configs: cargo_config_candidates(
                &root,
                std::env::var_os("CARGO_HOME"),
                std::env::var_os("HOME"),
            )
            .into_iter()
            .filter(|path| path.is_file())
            .map(|path| path.display().to_string())
            .collect(),
        }
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
    // Every untracked file and submodule change, whatever the user's configuration hides
    // (`status.showUntrackedFiles`, `diff.ignoreSubmodules`): the check would read them.
    let status = git_output(
        runner,
        &[
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--ignore-submodules=none",
        ],
        "read the work tree",
    )?;
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

/// Whether git pushes `object` to delete a ref: the hook then receives an object name of
/// all zeros, 40 of them (SHA-1) or 64 (SHA-256).
fn is_deletion(object: &str) -> bool {
    [40, 64].contains(&object.len()) && object.bytes().all(|byte| byte == b'0')
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
    let node_version = read_file(runner, NODE_VERSION_FILE)?;
    let toolchain = read_file(runner, RUST_TOOLCHAIN_FILE)?;
    preflight_gate(
        std::env::consts::OS,
        &answers,
        &node_version,
        &toolchain,
        &runner.environment(),
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

/// Reads the Stryker configuration of every package and every package manifest, then checks
/// that no configuration hides a mutant and that no script runs Stryker otherwise.
fn stryker_configs(runner: &mut dyn Runner) -> Result<(), Failure> {
    PACKAGES.iter().try_for_each(|package| {
        let path = format!("{}/{STRYKER_CONFIG}", package.dir);
        stryker_config_gate(package.dir, &read_file(runner, &path)?)
    })?;
    package_jsons()
        .iter()
        .try_for_each(|path| stryker_scripts_gate(path, &read_file(runner, path)?))
}

/// Reads what `package` measures and mutates, then checks that every source of it is in
/// both, or reviewed.
fn scope(runner: &mut dyn Runner, package: Package) -> Result<(), Failure> {
    let tracked = list_tracked(runner, &TS_SOURCES, "the TypeScript sources")?;
    let [coverage, mutation] = package.reports();
    let scope = Scope {
        dir: package.dir,
        stryker: read_file(runner, &format!("{}/{STRYKER_CONFIG}", package.dir))?,
        summary: read_file(runner, &format!("{coverage}/coverage-summary.json"))?,
        mutation: read_file(runner, &format!("{mutation}/mutation.json"))?,
    };
    scope_gate(&tracked, &[scope], &read_file(runner, BYPASSES_REVIEWED)?)
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

/// Whether `rel`, a path relative to its crate, is Rust test code: under `tests/` (its
/// fixtures included), or a unit test file beside its source (`*_tests.rs`).
fn rust_test_code(rel: &str) -> bool {
    rel.starts_with("tests/") || rel.ends_with("_tests.rs")
}

/// What every Rust test depends on: a change to a file of one of these names, or below the
/// shared test crate, can let any mutant survive, so the default check then mutates
/// everything.
pub const MUTATE_EVERYTHING_WHEN: [&str; 4] = [
    "Cargo.toml",
    "Cargo.lock",
    RUST_TOOLCHAIN_FILE,
    "crates/engine-testkit/",
];

/// The paths a diff written with the `a/` and `b/` prefixes changes.
fn changed_paths(diff: &str) -> Vec<&str> {
    diff.lines()
        .filter_map(|line| line.strip_prefix("diff --git a/"))
        .filter_map(|rest| rest.split_once(" b/").map(|(_, path)| path))
        .collect()
}

/// Runs cargo-mutants on what changed since the merge base ([`Action::RustMutantsOfChanges`]):
/// the changes of the tracked files, committed or not, and every new Rust file git does not
/// ignore; nothing when nothing changed.
///
/// A change to the test code of a crate can weaken the tests of any of its code, so every
/// source of that crate is then added whole; a change to what every test depends on
/// ([`MUTATE_EVERYTHING_WHEN`]) mutates the whole workspace.
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
            "--no-textconv",
            "--no-renames",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            &base,
        ],
        "read the changes",
    )?;
    let new = git_output(
        runner,
        &[
            "ls-files",
            "-z",
            "--others",
            "--exclude-standard",
            "--",
            "*.rs",
        ],
        "list the new Rust files",
    )?;
    for path in new.split('\0').filter(|path| !path.is_empty()) {
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
    let changed: Vec<String> = changed_paths(&diff)
        .into_iter()
        .map(str::to_owned)
        .collect();
    if let Some(path) = changed.iter().find(|path| {
        let name = path
            .rsplit_once('/')
            .map_or(path.as_str(), |(_, name)| name);
        MUTATE_EVERYTHING_WHEN
            .iter()
            .any(|shared| *shared == name || (shared.ends_with('/') && path.starts_with(shared)))
    }) {
        say(out, &format!("  {path} changed: every Rust mutant"))?;
        return runner
            .run(&mutants_command(&[
                "--workspace",
                "--jobs",
                &jobs.to_string(),
            ]))
            .map_err(|failure| Failure::from(failure.to_string()));
    }
    let tested: Vec<RustCrate> = RUST_CRATES
        .into_iter()
        .filter(|krate| {
            let prefix = format!("{}/", krate.dir);
            changed
                .iter()
                .filter_map(|path| path.strip_prefix(&prefix))
                .any(rust_test_code)
        })
        .collect();
    if !tested.is_empty() {
        let sources = list_tracked(runner, &["*.rs"], "the Rust files")?;
        for krate in tested {
            say(
                out,
                &format!(
                    "  the test code of {} changed: every mutant of {}",
                    krate.name, krate.name
                ),
            )?;
            let prefix = format!("{}/", krate.dir);
            for path in &sources {
                if path
                    .strip_prefix(&prefix)
                    .is_some_and(|rel| !rust_test_code(rel))
                {
                    let text = read_file(runner, path)?;
                    diff.push_str(&new_file_diff(path, &text));
                }
            }
        }
    }
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

/// Reads the unviable mutants of every shard (`reports`) and checks them together, as after
/// a complete run.
fn shards_unviable(runner: &mut dyn Runner, reports: &[String]) -> Result<(), Failure> {
    let mut report = String::new();
    for path in reports {
        report.push_str(&read_file(runner, path)?);
        report.push('\n');
    }
    unviable_gate(
        &report,
        Extent::Complete,
        &read_file(runner, UNVIABLE_REVIEWED)?,
    )
}

/// Checks `parts` against the full check ([`parts_gate`]); the number of cargo-mutants
/// jobs changes neither.
fn parts_of_the_full_check(parts: Vec<Part>) -> Result<(), Failure> {
    let jobs = NonZeroUsize::MIN;
    let parts: Vec<(String, Vec<Step>)> = parts
        .into_iter()
        .map(|part| (part.name(), part_steps(part, jobs)))
        .collect();
    parts_gate(
        &check_steps(Mode::Full { jobs }),
        &parts,
        &preparation_steps(),
    )
}

/// The crates of the workspace and their direct dependencies ([`metadata_command`]).
fn crates_metadata(runner: &mut dyn Runner) -> Result<String, Failure> {
    runner
        .capture(&metadata_command())
        .map_err(|failure| Failure::from(format!("cannot list the crates: {failure}")))
}

/// Runs the fused test build ([`fused_command`]) and checks its option was in force.
fn fused_build(runner: &mut dyn Runner) -> Result<(), Failure> {
    let output = runner
        .capture(&fused_command())
        .map_err(|failure| Failure::from(format!("the fused test build failed: {failure}")))?;
    fused_gate(&output)
}

/// Reads what the math layer gate checks, and runs it ([`math_layer_gate`]).
fn math_layer(runner: &mut dyn Runner) -> Result<(), Failure> {
    let metadata = crates_metadata(runner)?;
    let paths = list_tracked(runner, &BOX_SOURCES, "the box sources")?;
    math_layer_gate(&metadata, &read_all(runner, paths)?)
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
        Action::FloatMethodGate => float_methods(runner),
        Action::SupplyChainGate => supply_chain_gate(
            &SUPPLY_CHAIN_SETTINGS
                .iter()
                .map(|(path, _)| read_file(runner, path))
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Action::FusedBuild => fused_build(runner),
        Action::MathLayerGate => math_layer(runner),
        Action::DependencyGate => {
            let metadata = crates_metadata(runner)?;
            let packages = read_all(runner, package_jsons())?;
            dependency_gate(
                &metadata,
                &packages,
                &read_file(runner, DEPENDENCIES_REVIEWED)?,
            )
        }
        Action::TypeScriptReportsGate(package) => {
            let disabled = disabled_mutants(runner)?;
            let [coverage, mutation] = package.reports();
            let summary = read_file(runner, &format!("{coverage}/coverage-summary.json"))?;
            ts_coverage_gate(package.dir, &summary)?;
            let report = read_file(runner, &format!("{mutation}/mutation.json"))?;
            ts_mutation_gate(package.dir, &report, &disabled, Extent::Complete)
        }
        Action::TypeScriptMutationGate { package, extent } => {
            let disabled = disabled_mutants(runner)?;
            let [_, mutation] = package.reports();
            let report = read_file(runner, &format!("{mutation}/mutation.json"))?;
            ts_mutation_gate(package.dir, &report, &disabled, *extent)
        }
        Action::TypeScriptScopeGate(package) => scope(runner, *package),
        Action::StrykerConfigGate => stryker_configs(runner),
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
        Action::ShardsUnviableGate(reports) => shards_unviable(runner, reports),
        Action::PartsGate => parts_of_the_full_check(full_parts()),
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

/// Runs `steps` in order and stops at the first failure: [`run_steps_then`] with
/// [`OnFailure::Stop`].
///
/// # Errors
///
/// As [`run_steps_then`].
pub fn run_steps(
    steps: &[Step],
    runner: &mut dyn Runner,
    out: &mut dyn Write,
) -> Result<(), Failure> {
    run_steps_then(steps, runner, out, OnFailure::Stop)
}

/// Runs `steps` in order; at a failure, does what `on_failure` says.
///
/// # Errors
///
/// With [`OnFailure::Stop`], the failure of the first step that failed, its summary naming
/// the step. With [`OnFailure::RunTheRest`], once every step has run, a summary counting
/// the failed steps, and in the details, for each failed step in order, its own details
/// then its summary naming the step. In both modes, why the progress could not be written.
pub fn run_steps_then(
    steps: &[Step],
    runner: &mut dyn Runner,
    out: &mut dyn Write,
    on_failure: OnFailure,
) -> Result<(), Failure> {
    let total_steps = steps.len();
    let mut tree = None;
    let mut failures = Vec::new();
    for (index, step) in steps.iter().enumerate() {
        let number = index.saturating_add(1);
        say(out, &format!("▶ [{number}/{total_steps}] {}", step.name))?;
        if let Err(failure) = run_action(&step.action, runner, out, &mut tree) {
            let failure = Failure {
                summary: format!("{}: {}", step.name, failure.summary),
                details: failure.details,
            };
            if on_failure == OnFailure::Stop {
                return Err(failure);
            }
            failures.push(failure);
        }
    }
    if failures.is_empty() {
        return say(out, &format!("✔ all {total_steps} checks passed"));
    }
    Err(Failure {
        summary: format!(
            "{} of {total_steps} checks failed, listed above",
            failures.len()
        ),
        details: failures
            .into_iter()
            .flat_map(|failure| failure.details.into_iter().chain([failure.summary]))
            .collect(),
    })
}

/// Usage of the `xtask` command, in the order of its first line.
pub const USAGE: &str = "\
usage: cargo xtask <check [--full [--part <part>]]|list [--full [--parts]]|mutants <target>|bless|bless-benchmarks|commit-msg <file>|pre-push [pair...]|help>
  check [--full]        run every quality check; mutation testing covers the changes: Rust
                        since the merge base with the upstream, origin/main or main,
                        TypeScript since the last local Stryker run; stops at the first
                        failure; with --full every mutant, as CI does, and every check
                        runs whatever fails, then every failure is reported; with
                        --full --part <part>, only that part, as one CI job runs it
                        (the parts together run every check once; the final part
                        reads the unviable mutants of every mutation shard)
  list [--full]         list the checks of `check` (or `check --full`), in order; with
                        --full --parts, the parts of `check --full`
  mutants <target>      run every mutant of one target: a crate (its name), a Rust file
                        of a crate, a TypeScript package (its directory) or one of its
                        sources (their paths from the root)
  bless                 rewrite conformance/references.json from the current renders
  bless-benchmarks      rewrite spec/benchmark-baseline.json from the current instruction
                        counts of the engine module (spec/benchmarks.md)
  commit-msg <file>     check a commit message (used by the commit-msg hook)
  pre-push [pair...]    check that the tree is clean and the pushed commits are checked
                        out, then run `check`, or `check --full` if a tag is pushed;
                        each pair is a pushed commit and the remote ref it updates
                        (used by the pre-push hook)
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
/// Returns the usage error, in one line: a path that is not plain, test code (Rust or
/// TypeScript, fixtures included), or anything else that is no target.
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
        && let Some((krate, rel)) = RUST_CRATES
            .iter()
            .find_map(|krate| below(krate.dir).map(|rel| (krate, rel)))
    {
        if rust_test_code(rel) {
            return Err(format!("{text:?} is test code: name the source it tests"));
        }
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
                &["exec", "stryker", "run", STRYKER_CONFIG, "--mutate", source],
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

/// The only way to change the benchmark baseline.
///
/// Builds the engine module, counts the instructions of every benchmark and rewrites
/// `spec/benchmark-baseline.json`, refusing to when building plans grows faster than
/// n log n. The commit must explain why the counts changed (`spec/benchmarks.md`).
#[must_use]
pub fn bless_benchmarks_step() -> Step {
    run(
        "Bless the benchmark baseline",
        "pnpm",
        &["run", "benchmarks:bless"],
        Dir::Package("bindings/wasm"),
    )
}

/// A command of `cargo xtask`, read from its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Task {
    /// Every check: every mutant if `true` ([`Mode::Full`]), else the changes.
    Check(bool),
    /// The list of the checks: those of `check --full` if `true`.
    List(bool),
    /// One part of the full check.
    CheckPart(Part),
    /// The list of the parts of the full check.
    ListParts,
    /// Every mutant of one target.
    Mutants(Target),
    /// Rewrite the reference hashes.
    Bless,
    /// Rewrite the benchmark baseline.
    BlessBenchmarks,
    /// Check the commit message in this file.
    CommitMsg(String),
    /// Check what is pushed (each object with the remote ref it updates), then every check:
    /// every mutant if a tag is pushed, since a release requires the full check.
    PrePush(Vec<(String, String)>),
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
    let alone = ["bless", "bless-benchmarks", "help"];
    match args.as_slice() {
        [] => Err("no command given".to_owned()),
        [command] if command == "check" => Ok(Task::Check(false)),
        [command, flag] if command == "check" && flag == "--full" => Ok(Task::Check(true)),
        [command] if command == "list" => Ok(Task::List(false)),
        [command, flag] if command == "list" && flag == "--full" => Ok(Task::List(true)),
        [command, full, flag] if command == "list" && full == "--full" && flag == "--parts" => {
            Ok(Task::ListParts)
        }
        [command, full, flag, name]
            if command == "check" && full == "--full" && flag == "--part" =>
        {
            full_parts()
                .into_iter()
                .find(|part| part.name() == *name)
                .map(Task::CheckPart)
                .ok_or_else(|| {
                    format!(
                        "unknown part \"{name}\" (`cargo xtask list --full --parts` lists them)"
                    )
                })
        }
        [command] if command == "bless" => Ok(Task::Bless),
        [command] if command == "bless-benchmarks" => Ok(Task::BlessBenchmarks),
        [command] if command == "help" => Ok(Task::Help),
        [command, file] if command == "commit-msg" => Ok(Task::CommitMsg(file.clone())),
        [command, target] if command == "mutants" => mutation_target(target).map(Task::Mutants),
        [command, ..] if command == "mutants" => Err("`mutants` takes one target".to_owned()),
        [command, pushed @ ..] if command == "pre-push" => {
            let (pairs, rest) = pushed.as_chunks::<2>();
            if !rest.is_empty() {
                return Err(
                    "`pre-push` takes pairs of a commit and the remote ref it updates".to_owned(),
                );
            }
            Ok(Task::PrePush(
                pairs
                    .iter()
                    .map(|[commit, remote]| (commit.clone(), remote.clone()))
                    .collect(),
            ))
        }
        [command, ..] if alone.contains(&command.as_str()) => {
            Err(format!("`{command}` takes no argument"))
        }
        [command, ..] if command == "check" => {
            Err("`check` takes no argument but `--full`, then `--part <part>`".to_owned())
        }
        [command, ..] if command == "list" => {
            Err("`list` takes no argument but `--full`, then `--parts`".to_owned())
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
/// `err`, nothing on `out`. Only `help` prints the usage, on `out`. The error line and
/// each line of the details go through [`safe_message`]: escaped, and bounded.
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
            drop(writeln!(
                err,
                "error: {} (see: cargo xtask help)",
                safe_message(&usage)
            ));
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
            let mode = mode(full);
            run_steps_then(&check_steps(mode), runner, out, mode.on_failure())
        }
        Task::CheckPart(part) => {
            run_steps_then(&part_steps(part, jobs), runner, out, OnFailure::RunTheRest)
        }
        Task::ListParts => full_parts()
            .into_iter()
            .try_for_each(|part| say(out, &part.name())),
        Task::Bless => run_steps(&[bless_step()], runner, out),
        Task::BlessBenchmarks => run_steps(&[bless_benchmarks_step()], runner, out),
        Task::CommitMsg(path) => {
            read_file(runner, &path).and_then(|message| commit_msg_gate(&message))
        }
        Task::PrePush(pushed) => {
            let commits: Vec<String> = pushed.iter().map(|(commit, _)| commit.clone()).collect();
            let release = pushed
                .iter()
                .any(|(_, remote)| remote.starts_with("refs/tags/"));
            let mode = mode(release);
            pre_push(runner, &commits)
                .and_then(|()| run_steps_then(&check_steps(mode), runner, out, mode.on_failure()))
        }
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
            // Each line of a detail on a line of its own, every line made safe.
            for line in failure.details.iter().flat_map(|detail| detail.lines()) {
                drop(writeln!(out, "{}", safe_message(line)));
            }
            drop(writeln!(err, "error: {}", safe_message(&failure.summary)));
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
