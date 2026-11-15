//! The steps of the check: commands, actions, their verdicts, and the order they run in.

use core::num::NonZeroUsize;
use std::path::Path;
use std::path::PathBuf;

use crate::{
    parts::{MUTATION_SHARDS, Part, Role, Setup, Shard},
    reviews::STRYKER_CONFIG,
    unsafety::{ASSEMBLY_FILES, UNSAFE_FILES},
    workspace::{
        CHANGES_DIFF, CONFORMANCE_EXPORT, COVERAGE_DIR, Coverage, ENGINE_PACKAGE, Extent,
        MUTANTS_OUT, Measured, Mode, NATIVE_PACKAGES, NATIVE_TARGETS, NIGHTLY_TOOLCHAIN, PACKAGES,
        PORTABILITY_PACKAGES, PORTABILITY_TARGETS, Package, RUST_CRATES, STRYKER_INCREMENTAL,
        build_outputs, package_args, typescript_reports,
    },
};

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
    /// Measure [`COVERAGE_LOCALITY`](crate::workspace::COVERAGE_LOCALITY) and require the coverage gate to refuse it.
    CoverageLocality,
    /// Run cargo-mutants on the changes of [`Mode::Changes`]: the diff since the merge base
    /// of [`MUTATION_BASES`](crate::workspace::MUTATION_BASES), written to [`CHANGES_DIFF`]; nothing when there is no change.
    RustMutantsOfChanges {
        /// The number of cargo-mutants jobs.
        jobs: NonZeroUsize,
    },
    /// Run cargo-mutants on the Rust lines changed since the merge base only, not widened
    /// (`cargo xtask mutants --changed`); nothing when no Rust line changed.
    RustMutantsOfChangedLines {
        /// The number of cargo-mutants jobs.
        jobs: NonZeroUsize,
    },
    /// Check the conformance references natively: read the record at the merge base of
    /// [`MAIN_BASES`](crate::workspace::MAIN_BASES), then run [`conformance_command`](crate::cli::conformance_command), which also compares the
    /// fingerprints with it.
    Conformance,
    /// Bless the conformance references ([`bless_step`](crate::cli::bless_step)): read the record at the merge base
    /// of [`MAIN_BASES`](crate::workspace::MAIN_BASES), then run `audio-cli bless` with it as the versions spent and
    /// these options, as `cargo xtask bless` was given them.
    Bless(Vec<String>),
    /// Record the proof of the Rust mutation step of the default check that passed
    /// ([`ProofRecord`](crate::proof_record::ProofRecord)), when the tree is clean.
    MutationProofRecorded,
    /// Require every unviable mutant of the last run to be reviewed; after a
    /// [`Extent::Complete`] run, also every review to be current.
    UnviableGate(Extent),
    /// Require every review of unviable mutants to name a mutant of the workspace
    /// ([`unviable_reviews_gate`](crate::reviews::unviable_reviews_gate)): the stale reviews a partial run cannot see.
    UnviableReviewsCurrent,
    /// Require every mutant of the mutation report of `package` to be killed
    /// ([`ts_mutation_gate`](crate::reviews::ts_mutation_gate)), after a run of `extent`.
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
    /// (its format; the changelog entry of each change is the [`Action::HistoryGate`]'s),
    /// whose released sections are as `main` holds them ([`released_sections_gate`](crate::release::released_sections_gate)).
    VersionGate,
    /// Empty each of these directories (relative to the root), creating them if needed:
    /// what a later step reads there is then produced by this run only.
    FreshDirs(Vec<String>),
    /// Require 100% coverage and every mutant killed in the reports of a TypeScript package.
    TypeScriptReportsGate(Package),
    /// Require every Stryker configuration to hide no mutant.
    StrykerConfigGate,
    /// Require the `check` script of every package to run exactly the scripts its checks
    /// run ([`Package::checks`](crate::workspace::Package::checks)).
    PackageCheckGate,
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
    /// synonyms [`VOCABULARY`](crate::vocabulary::VOCABULARY) lists.
    VocabularyGate,
    /// Require no tracked text file to hold a conflict marker git left
    /// ([`conflict_marker_gate`](crate::conflicts::conflict_marker_gate)).
    ConflictMarkerGate,
    /// Require every JavaScript dependency under an allowed license: the strict list for
    /// what the apps ship, the wider one for development tooling.
    LicenseGate,
    /// Require the lint against unsafe code to be lifted only in the audited files, and
    /// forbidden in every crate root.
    UnsafeGate,
    /// Require the Cargo configuration to be exactly [`CARGO_CONFIG`](crate::dependencies::CARGO_CONFIG).
    CargoConfigGate,
    /// Require Rust test code to live only in `tests/` and declared `*_tests.rs` files
    /// ([`test_code_gate`](crate::test_code::test_code_gate)).
    TestCodeGate,
    /// Require the tests of each crate to read only its declared test data
    /// ([`test_data_gate`](crate::test_code::test_data_gate)).
    TestDataGate,
    /// Require every target that turns its tests off to hold no test, and every library to
    /// keep its doc tests ([`untested_targets_gate`](crate::test_code::untested_targets_gate)).
    UntestedTargetsGate,
    /// Require every stable float method of the pinned toolchain to be banned in
    /// `clippy.toml` or reviewed as deterministic ([`float_gate`](crate::float::float_gate)).
    FloatMethodGate,
    /// Require every direct dependency to be reviewed, and every review current.
    DependencyGate,
    /// Require every bit-defining dependency to be pinned to one exact version
    /// ([`pin_gate`](crate::dependencies::pin_gate)).
    PinGate,
    /// The rules of [`library_gate`](crate::dependencies::library_gate) (what each library may depend on) and of
    /// [`shared_tests_gate`](crate::dependencies::shared_tests_gate) (which crates only tests use).
    LibraryGate,
    /// Require every part to be under the license of its folder, and no open part to depend
    /// on an app ([`part_license_gate`](crate::licenses::part_license_gate)).
    PartLicenseGate,
    /// Require every roadmap step a tracked file cites to exist on the board
    /// ([`citation_gate`](crate::citations::citation_gate)).
    CitationGate,
    /// Build and test the engine core and the boxes with the math layer's fused multiply-add
    /// option ([`fused_command`]), and require the option to have been in force
    /// ([`fused_gate`]).
    FusedBuild,
    /// Require every function a review of an unviable mutant names to exist
    /// ([`unviable_names_gate`](crate::test_code::unviable_names_gate)).
    UnviableNamesGate,
    /// Read the files rustc loaded for each build of the workspace ([`loaded_files_commands`](crate::loaded_files::loaded_files_commands))
    /// and require them where the gates reading sources by path assume ([`loaded_files_gate`](crate::loaded_files::loaded_files_gate)).
    LoadedFilesGate,
    /// Require `libm` to be a dependency of the crates of [`CrateRole::Libm`](crate::workspace::CrateRole::Libm) only, outside
    /// tests: boxes reach math functions through the engine's math layer alone
    /// ([`math_layer_gate`](crate::math_layer::math_layer_gate)).
    MathLayerGate,
    /// Require the supply-chain configuration xtask defines ([`supply_chain_gate`](crate::supply_chain::supply_chain_gate)).
    SupplyChainGate,
    /// Require CI to run exactly the workflow xtask defines ([`workflow_gate`](crate::workflow::workflow_gate)).
    WorkflowGate,
    /// Require the parts of the full check to run every step of it once ([`parts_gate`](crate::parts::parts_gate)).
    PartsGate,
    /// Require every unviable mutant of the shards to be reviewed, and every review
    /// current: [`Action::UnviableGate`] after a complete run, on the reports of every shard
    /// (these paths), together.
    ShardsUnviableGate(Vec<String>),
    /// Record the state of the working tree ([`TreeState`](crate::coverage::TreeState)), for [`Action::TreeUnchanged`].
    TreeRecorded,
    /// Require the working tree to be as [`Action::TreeRecorded`] found it: the checks
    /// change no tracked file and leave no new untracked one.
    TreeUnchanged,
    /// Require every commit message to follow the convention, and every change of behavior
    /// since [`CHANGELOG_SINCE`](crate::history::CHANGELOG_SINCE) to update the changelog.
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
    /// Whether it is a usage error that a command xtask passed its arguments on to reported
    /// (`audio-cli bless`, exit code 2): xtask then exits with 2 too, pointing to its usage.
    pub usage: bool,
}

impl From<String> for Failure {
    fn from(summary: String) -> Self {
        Self {
            summary,
            details: Vec::new(),
            usage: false,
        }
    }
}

/// Longest line [`safe_message`] returns, in characters after escaping: the same bound as
/// `audio_patch::MAX_MESSAGE_CHARS`.
pub const MAX_MESSAGE_CHARS: usize = 1_000;

/// Whether `character` must not reach a terminal as is: control characters (a newline
/// would forge a second line, ESC starts terminal sequences), bidirectional formatting
/// characters (they reorder what is shown) and the Unicode line and paragraph separators.
pub(crate) const fn needs_escape(character: char) -> bool {
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
/// cut and marked with `…`. The algorithm of `audio_patch::safe_message`, copied: xtask
/// depends on no engine crate. The messages of xtask quote untrusted text (a commit
/// message, a file, an argument), so [`cli`](crate::cli::cli) passes every line it writes through it.
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
pub(crate) fn verdict(context: &str, problems: Vec<String>) -> Result<(), Failure> {
    match problems.as_slice() {
        [] => Ok(()),
        [only] => Err(Failure::from(format!("{context}: {only}"))),
        _ => Err(Failure {
            summary: format!("{context}: {} problems, listed above", problems.len()),
            details: problems,
            usage: false,
        }),
    }
}

/// The verdict of a gate that only runs as a step: its problems alone, the step's name
/// being the one source of what it checks (`run_steps_then` opens the summary with it):
/// the one problem inline, or how many, listed above.
pub(crate) fn step_verdict(problems: Vec<String>) -> Result<(), Failure> {
    match problems.as_slice() {
        [] => Ok(()),
        [only] => Err(Failure::from(only.clone())),
        _ => Err(Failure {
            summary: format!("{} problems, listed above", problems.len()),
            details: problems,
            usage: false,
        }),
    }
}

/// Builds a command step.
pub(crate) fn run(name: &str, program: &'static str, args: &[&str], dir: Dir) -> Step {
    Step {
        name: name.to_owned(),
        action: Action::Run(Cmd::new(program, args, dir)),
    }
}

/// The pinned tools, the platform and an environment that changes no build: the first step
/// of every check, and of every bless, whose output later checks compare with.
pub(crate) fn preflight_step() -> Step {
    Step {
        name: "Tools and platform".to_owned(),
        action: Action::Preflight,
    }
}

/// The tools first, then the state of the working tree: what every part starts with.
pub(crate) fn opening_steps() -> Vec<(Role, Step)> {
    vec![
        (Role::Frame, preflight_step()),
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
pub(crate) fn tagged(role: Role, steps: Vec<Step>) -> Vec<(Role, Step)> {
    steps.into_iter().map(|step| (role, step)).collect()
}

/// Formatting, lints, documentation and tests.
pub(crate) fn static_steps() -> Vec<Step> {
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
            name: "Files rustc loads: tracked, in their crate, those of tests alone named as \
                   test code"
                .to_owned(),
            action: Action::LoadedFilesGate,
        },
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
        &[fused_args(), vec!["--", INCLUDE_IGNORED]].concat(),
        Dir::Root,
    )
}

/// The arguments of the fused test build ([`fused_command`]) that choose what it builds,
/// which the loaded-files gate builds too ([`loaded_files_commands`](crate::loaded_files::loaded_files_commands)).
pub(crate) fn fused_args() -> Vec<&'static str> {
    let mut args = vec!["test", "--locked"];
    args.extend(package_args(&["audio-core", "audio-boxes"]));
    args.extend(["--target-dir", FUSED_TARGET, "--config", FUSED_CONFIG]);
    args
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

/// The engine crates, audio and visual, build for every portability target.
pub(crate) fn portability_steps() -> Vec<Step> {
    PORTABILITY_TARGETS
        .iter()
        .map(|&target| {
            run(
                &format!("Library portability ({target})"),
                "cargo",
                &[
                    vec!["build"],
                    package_args(&PORTABILITY_PACKAGES),
                    vec!["--locked", "--target", target],
                ]
                .concat(),
                Dir::Root,
            )
        })
        .collect()
}

/// The native shell (`audio-cli`) and the library's floating-point mode it uses
/// (`audio-fpenv`), whose code differs by architecture, linted then built for each of
/// [`NATIVE_TARGETS`].
///
/// Clippy type-checks every target of theirs, tests included, without generating code, so
/// it never assembles inline assembly; the build of their libraries generates and assembles
/// it (an invalid instruction or register of the other architecture fails there), and
/// needs no linker of the other target, since a library is not linked.
pub(crate) fn native_lint_steps() -> Vec<Step> {
    let lints = NATIVE_TARGETS.iter().map(|&target| {
        run(
            &format!("Native shell lints ({target})"),
            "cargo",
            &[
                vec!["clippy"],
                package_args(&NATIVE_PACKAGES),
                vec!["--all-targets", "--locked", "--target", target],
            ]
            .concat(),
            Dir::Root,
        )
    });
    let builds = NATIVE_TARGETS.iter().map(|&target| {
        run(
            &format!("Native code of audio-cli and audio-fpenv built, inline assembly included ({target})"),
            "cargo",
            &[
                vec!["build", "--lib"],
                package_args(&NATIVE_PACKAGES),
                vec!["--locked", "--target", target],
            ]
            .concat(),
            Dir::Root,
        )
    });
    lints.chain(builds).collect()
}

/// The test harness argument that runs the ignored tests too.
///
/// Given to every Rust test run that proves something (the tests, coverage, mutation
/// testing, Miri): an ignored test is then inert, never a way to remove a test from a proof.
/// Rustdoc does not compile an `ignore` doc test even so, nor run a `no_run` one, and a
/// conditional attribute or a `cfg` naming Miri can still remove a test from one of these
/// runs: the bypass gate refuses all of them.
pub const INCLUDE_IGNORED: &str = "--include-ignored";

/// The arguments of every cargo-mutants run: no configuration file is read (`--no-config`)
/// and its report goes to `mutants.out` at the root (`--output=.`), whatever an ignored
/// `.cargo/mutants.toml` or `CARGO_MUTANTS_OUTPUT` says, which neither the bypass gate nor
/// the cleanliness checks see (the bypass gate still refuses a tracked `mutants.toml`,
/// dead under `--no-config`, so that a reviewer is told); lints are capped while building
/// mutants (otherwise a mutant that merely leaves a variable unused fails to compile and is
/// silently skipped instead of tested), the ignored files are not copied, and every cargo
/// command it runs keeps `Cargo.lock` as it is (`--locked`, as every other cargo command of
/// the checks) and builds without debug information ([`MUTANT_BUILD_CONFIG`]).
pub(crate) const MUTANTS_ARGS: [&str; 7] = [
    "mutants",
    "--no-config",
    "--output=.",
    "--cap-lints=true",
    "--gitignore=true",
    "--cargo-arg=--locked",
    MUTANT_BUILD_CONFIG,
];

/// The one profile setting of cargo-mutants' builds that differs from the other builds of
/// the checks: no debug information in the test profile.
///
/// Every build `cargo test` makes uses that profile; without debug information, Cargo's
/// default also strips it. The other difference of those builds is the lint cap of
/// `MUTANTS_ARGS` (`--cap-lints`, which caps `forbid` too: a mutant replaces a body by a
/// value or an operator by another, never writing `unsafe`), which changes no generated code
/// either (reviewed); they also build a copy of the tree, without its ignored files, at
/// another path.
///
/// It changes what the compiler is told only by its debug information options
/// (`-C debuginfo`, `-C split-debuginfo`, `-C strip=debuginfo`) and the hashes Cargo names
/// its outputs with, compared on every command line of `cargo test --no-run -v`, so no
/// code it compiles differs. Debug information changes no compiled code; only a
/// backtrace's file and line names depend on it, and no code of the workspace reads a
/// backtrace (reviewed). It makes the builds and test programs of the mutants smaller, so
/// they are written, linked and started faster.
/// Given on the command line (`--config`), never in [`CARGO_CONFIG_FILE`](crate::preflight::CARGO_CONFIG_FILE), which holds no
/// build setting; the full check and every part of it build mutants the same way.
pub const MUTANT_BUILD_CONFIG: &str = "--cargo-arg=--config=profile.test.debug=false";

/// A cargo-mutants command: [`MUTANTS_ARGS`], then `args`, then [`INCLUDE_IGNORED`] for the
/// test harness (cargo-mutants passes what follows its `--` to `cargo test`, which passes
/// what follows its own `--` to the tests).
pub(crate) fn mutants_command(args: &[&str]) -> Cmd {
    Cmd::new(
        "cargo",
        &[&MUTANTS_ARGS[..], args, &["--", "--", INCLUDE_IGNORED]].concat(),
        Dir::Root,
    )
}

/// How many times longer than the unmutated tests a mutant's tests may run alone.
///
/// Past it they count as hung. It is cargo-mutants' own default margin, for the work a
/// mutant may legitimately add (a proptest configuration mutated back to its default runs every
/// property test at proptest's default count of cases).
pub const MUTANT_TIMEOUT_MARGIN: usize = 5;

/// The least time, in seconds, a mutant's tests may run alone before they count as hung:
/// cargo-mutants' own default floor, for tests too short for a margin to mean anything.
pub const MUTANT_TIMEOUT_FLOOR: usize = 20;

/// The timeout arguments of a cargo-mutants run in `jobs` jobs.
///
/// They are `--jobs`, then the test timeout derived from the unmutated tests' time that
/// cargo-mutants measures first (its baseline, run alone), `max(floor, baseline × margin)`,
/// both scaled by `jobs`.
///
/// Why scaled: the baseline runs alone, a mutant's tests share the processors with the
/// other jobs. A job is either building, its builds sharing the processors' tokens of
/// cargo-mutants' jobserver with every other build, or testing, each test binary running
/// as many threads as there are processors; so at most `jobs` binaries' worth of work wants
/// the processors at once, and under no outside load a test runs about `jobs` times
/// slower than alone at most (a model, not a bound: a timeout it gets wrong fails the
/// check, never passes it). Without
/// it, a mutant caught by a fast test but whose binary still runs its slow tests to the end
/// was reported as a timeout under load (audio-testkit's `config` mutants). A mutant that really
/// hangs is still stopped and reported as a timeout, which fails the check: the timeout only bounds
/// how long that failing run takes.
#[must_use]
pub fn mutant_timeout_args(jobs: NonZeroUsize) -> [String; 6] {
    [
        "--jobs".to_owned(),
        jobs.to_string(),
        "--timeout-multiplier".to_owned(),
        MUTANT_TIMEOUT_MARGIN.saturating_mul(jobs.get()).to_string(),
        "--minimum-test-timeout".to_owned(),
        MUTANT_TIMEOUT_FLOOR.saturating_mul(jobs.get()).to_string(),
    ]
}

/// A cargo-mutants command that tests mutants: [`mutants_command`] of `args` then the
/// arguments of [`mutant_timeout_args`] in `jobs` jobs.
pub(crate) fn tested_mutants_command(args: &[&str], jobs: NonZeroUsize) -> Cmd {
    let timeouts = mutant_timeout_args(jobs);
    let mut all = args.to_vec();
    all.extend(timeouts.iter().map(String::as_str));
    mutants_command(&all)
}

/// The cargo-mutants command of [`Mode::Changes`]: the workspace in `jobs` jobs, filtered
/// by [`CHANGES_DIFF`].
#[must_use]
pub fn changes_mutants_command(jobs: NonZeroUsize) -> Cmd {
    tested_mutants_command(&["--workspace", "--in-diff", CHANGES_DIFF], jobs)
}

/// The cargo-mutants command of [`Mode::Full`]: every mutant of the workspace in `jobs` jobs.
///
/// With a `shard`, only its mutants (cargo-mutants' `--shard <index>/<count>`, mutant `i` on
/// shard `i % count`: round-robin, so that each shard takes its share of every file).
#[must_use]
pub fn workspace_mutants_command(jobs: NonZeroUsize, shard: Option<Shard>) -> Cmd {
    let mut args = vec!["--workspace"];
    let sharded = shard.map(|shard| format!("{}/{MUTATION_SHARDS}", shard.index()));
    if let Some(sharded) = &sharded {
        args.extend(["--shard", sharded, "--sharding=round-robin"]);
    }
    tested_mutants_command(&args, jobs)
}

/// Rust mutation testing in `mode`, after emptying [`MUTANTS_OUT`], then its gate.
pub(crate) fn rust_mutation_steps(mode: Mode) -> Vec<(Role, Step)> {
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
    let mut steps = vec![
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
    ];
    if matches!(mode, Mode::Changes { .. }) {
        steps.push((
            Role::AcrossShards,
            Step {
                name: "Rust unviable mutant reviews all current".to_owned(),
                action: Action::UnviableReviewsCurrent,
            },
        ));
        steps.push((
            Role::AcrossShards,
            Step {
                name: "Rust mutation proof recorded for the next default check".to_owned(),
                action: Action::MutationProofRecorded,
            },
        ));
    }
    steps
}

/// Coverage and mutation testing in `mode`, with their gates, and Miri.
pub(crate) fn proof_steps(mode: Mode) -> Vec<(Role, Step)> {
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
        Step {
            name: "Test code only in tests/ and declared *_tests.rs files".to_owned(),
            action: Action::TestCodeGate,
        },
        Step {
            name: "The tests of each crate read only its declared test data".to_owned(),
            action: Action::TestDataGate,
        },
        Step {
            name: "Tests turned off only where no test can hide, doc tests kept".to_owned(),
            action: Action::UntestedTargetsGate,
        },
        Step {
            name: "Every function a review of an unviable mutant names exists".to_owned(),
            action: Action::UnviableNamesGate,
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
                .any(|file| krate.holds(file).is_some())
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

/// The supply chain and the dependencies: every one reviewed, the bit-defining ones pinned,
/// each library standing alone.
pub(crate) fn dependency_steps() -> Vec<Step> {
    vec![
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
            name: "Bit-defining dependencies pinned to one exact version".to_owned(),
            action: Action::PinGate,
        },
        Step {
            name: "Each library depends only on its own crates, its tests also on its test \
                   crates; every crate only tests use is shared test code"
                .to_owned(),
            action: Action::LibraryGate,
        },
        Step {
            name: "Every part under the license of its folder, no open part depending on an app"
                .to_owned(),
            action: Action::PartLicenseGate,
        },
    ]
}

/// The visual conformance corpus, read by `visual-cli` and by the visual WebAssembly
/// module's tests.
pub const VISUAL_CONFORMANCE: &str = "conformance/visual";

/// The native visual conformance check: `visual-cli` renders every visual case with the
/// exact CPU reference renderer and compares its image hash with its reference.
///
/// Built optimized: the largest image of the corpus (2²⁵ pixels) takes about 15 s
/// unoptimized and half a second optimized (measured 2026-11-15); the arithmetic is checked
/// either way, since the library denies `arithmetic_side_effects` and the results are the
/// same bits (the corpus is also checked unoptimized by the tests of `visual-cli`, its
/// largest image apart, and in WebAssembly).
fn visual_conformance_step() -> Step {
    run(
        "Visual conformance (native)",
        "cargo",
        &[
            "run",
            "--release",
            "--package",
            "visual-cli",
            "--locked",
            "--quiet",
            "--",
            "check",
            VISUAL_CONFORMANCE,
        ],
        Dir::Root,
    )
}

/// Conformance, supply chain, then the consistency of the repository: membership, READMEs,
/// versions, history and the CI workflow.
pub(crate) fn consistency_steps() -> Vec<(Role, Step)> {
    let conformance = Step {
        name: "Conformance (native)".to_owned(),
        action: Action::Conformance,
    };
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
                "audio-cli",
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
    let mut gates = dependency_steps();
    gates.extend([
        Step {
            name: "Box arithmetic only through the math layer: libm in audio-core and \
                   audio-testkit only, no raw float in box code, no iterator reduction \
                   outside the math layer"
                .to_owned(),
            action: Action::MathLayerGate,
        },
        Step {
            name: "Supply-chain configuration as xtask defines it".to_owned(),
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
            name: "Every roadmap step cited exists on the board".to_owned(),
            action: Action::CitationGate,
        },
        Step {
            name: "The words of the glossary, not their synonyms".to_owned(),
            action: Action::VocabularyGate,
        },
        Step {
            name: "No conflict marker left in a tracked text file".to_owned(),
            action: Action::ConflictMarkerGate,
        },
        Step {
            name: "One version everywhere, changelog well formed, released sections as on main"
                .to_owned(),
            action: Action::VersionGate,
        },
        Step {
            name: "Commit messages, and a changelog entry for each change of behavior".to_owned(),
            action: Action::HistoryGate,
        },
        Step {
            name: "CI runs exactly the workflow xtask defines".to_owned(),
            action: Action::WorkflowGate,
        },
        Step {
            name: "The parts of CI run every check of the full check once".to_owned(),
            action: Action::PartsGate,
        },
    ]);
    let mut steps = vec![
        (Role::Proof(Part::Gates), conformance),
        (Role::Proof(Part::Gates), visual_conformance_step()),
    ];
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
pub(crate) fn typescript_steps(mode: Mode) -> Vec<(Role, Step)> {
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
            run(
                "JavaScript vulnerabilities",
                "pnpm",
                &["audit", "--audit-level=low"],
                Dir::Root,
            ),
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
            gates,
            Step {
                name: "Each package's check runs exactly the scripts the checks time".to_owned(),
                action: Action::PackageCheckGate,
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
        steps.extend(tagged(
            Role::Proof(Part::Package(package)),
            package_steps(package, mode),
        ));
    }
    steps
}

/// The steps of `package` in `mode`: each script of its checks, then its mutation testing and
/// the gates over its reports.
fn package_steps(package: Package, mode: Mode) -> Vec<Step> {
    let mut owned: Vec<Step> = package
        .checks
        .iter()
        .map(|script| {
            run(
                &format!("{}: {script}", package.step),
                "pnpm",
                &["run", script],
                Dir::Package(package.dir),
            )
        })
        .collect();
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
    owned
}

/// Every check of the repository in `mode`, in the order they run, each with its role
/// in the parts of the full check: the order of the full check ([`full_order`]), which the
/// default check runs cheapest first ([`cheapest_first`]).
pub(crate) fn roles(mode: Mode) -> Vec<(Role, Step)> {
    match mode {
        Mode::Full { .. } => full_order(mode),
        Mode::Changes { .. } => cheapest_first(full_order(mode)),
    }
}

/// Where the steps of a part come in the default check ([`cheapest_first`]): the gates on
/// the repository, then the Rust checks, coverage, Miri, the TypeScript packages, and the
/// Rust mutation testing, by far the longest, last. `None` for the steps every part frames
/// itself with and for the preparation, which come where they are needed.
pub(crate) const fn default_rank(role: Role) -> Option<u8> {
    match role {
        Role::Frame | Role::Setup(_) => None,
        Role::Proof(Part::Gates) => Some(0),
        Role::Proof(Part::Rust) => Some(1),
        Role::Proof(Part::Coverage) => Some(2),
        Role::Proof(Part::Miri) => Some(3),
        Role::Proof(Part::Package(_)) => Some(4),
        Role::Proof(Part::Mutation(_) | Part::Final)
        | Role::EveryShard
        | Role::Sharded
        | Role::AcrossShards => Some(5),
    }
}

/// The part a step of `role` belongs to, whose needs ([`Part::needs`]) come before it.
pub(crate) const fn owner(role: Role) -> Option<Part> {
    match role {
        Role::Proof(part) => Some(part),
        Role::Frame | Role::Setup(_) | Role::EveryShard | Role::Sharded | Role::AcrossShards => {
            None
        }
    }
}

/// `steps`, in the order of the full check, reordered so that the default check, which
/// stops at the first failure, fails as early as it can: the steps that open every part
/// first and the ones that close it last, as they were; between them, the steps of each
/// part in the order of [`default_rank`] (each part's own steps in their order), each
/// preparation ([`Setup`]) just before the first step of a part that needs it.
///
/// The order changes no outcome: the parts of the full check run alone in CI, each after
/// its preparation only ([`part_steps`](crate::parts::part_steps)), so no step of a part depends on a step of another
/// part; only the Rust mutation report's steps depend on each other, and they keep their
/// order.
pub(crate) fn cheapest_first(steps: Vec<(Role, Step)>) -> Vec<(Role, Step)> {
    let opening = steps
        .iter()
        .take_while(|(role, _)| *role == Role::Frame)
        .count();
    let mut rest = steps;
    let mut ordered: Vec<(Role, Step)> = rest.drain(..opening).collect();
    let (closing, middle): (Vec<_>, Vec<_>) =
        rest.into_iter().partition(|(role, _)| *role == Role::Frame);
    let (setups, mut proofs): (Vec<_>, Vec<_>) = middle
        .into_iter()
        .partition(|(role, _)| default_rank(*role).is_none());
    proofs.sort_by_key(|(role, _)| default_rank(*role));
    let mut prepared: Vec<Setup> = Vec::new();
    for (role, step) in proofs {
        for setup in owner(role).map(Part::needs).unwrap_or_default() {
            if !prepared.contains(&setup) {
                prepared.push(setup);
                ordered.extend(
                    setups
                        .iter()
                        .filter(|(of, _)| *of == Role::Setup(setup))
                        .cloned(),
                );
            }
        }
        ordered.push((role, step));
    }
    ordered.extend(closing);
    ordered
}

/// Every check of the repository in `mode`, in the order of the full check, each with its
/// role in the parts of the full check.
pub(crate) fn full_order(mode: Mode) -> Vec<(Role, Step)> {
    let rust = Role::Proof(Part::Rust);
    [
        opening_steps(),
        tagged(rust, static_steps()),
        tagged(rust, portability_steps()),
        tagged(rust, native_lint_steps()),
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

#[cfg(test)]
#[path = "steps_tests.rs"]
mod tests;
