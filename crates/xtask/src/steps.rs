//! The steps of the check: commands, actions, their verdicts, and the order they run in.

use core::num::NonZeroUsize;
use std::path::Path;
use std::path::PathBuf;

use crate::{
    cli::Revision,
    corpus::corpus_steps,
    parts::{MUTATION_SHARDS, Part, Role, Setup, Shard},
    preflight::GpuDriver,
    reviews::STRYKER_CONFIG,
    unsafety::{ASSEMBLY_FILES, UNSAFE_FILES},
    workspace::{
        CHANGES_DIFF, CONFORMANCE_EXPORT, COVERAGE_DIR, CargoWorkspace, Coverage, Extent,
        MUTANTS_OUT, Measured, Mode, NATIVE_PACKAGES, NATIVE_TARGETS, NIGHTLY_TOOLCHAIN, PACKAGES,
        PORTABILITY_PACKAGES, PORTABILITY_TARGETS, Package, RUST_CRATES, RustCrate,
        STRYKER_INCREMENTAL, build_outputs, needed_builds, package_args, typescript_reports,
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
    /// The variables set for it alone, each name with its value, on top of those every
    /// command of the runner gets.
    pub env: Vec<(&'static str, String)>,
}

impl Cmd {
    /// `program` with `args`, run in `dir`, with no variable of its own.
    #[must_use]
    pub fn new(program: &'static str, args: &[&str], dir: Dir) -> Self {
        Self::of(program, args, dir)
    }

    /// [`Cmd::new`]'s body, a function of its own so that mutation testing reaches it:
    /// cargo-mutants never mutates a function named `new`.
    fn of(program: &'static str, args: &[&str], dir: Dir) -> Self {
        Self {
            program,
            args: args.iter().map(|&arg| (*arg).to_owned()).collect(),
            dir,
            env: Vec::new(),
        }
    }

    /// The command, with the variable `name` set to `value` for it alone.
    #[must_use]
    pub fn with_env(mut self, name: &'static str, value: String) -> Self {
        self.env.push((name, value));
        self
    }

    /// The command line, as a shell would show it (in its directory): its own variables,
    /// `NAME=value`, then the program and its arguments.
    #[must_use]
    pub fn line(&self) -> String {
        let mut line = String::new();
        for (name, value) in &self.env {
            line.push_str(name);
            line.push('=');
            line.push_str(value);
            line.push(' ');
        }
        line.push_str(self.program);
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
    /// Run an external tool whose verdict is a static gate ([`Action::is_static_gate`]): it
    /// reads the repository, or installs what the gates after it read, and builds or runs
    /// no code under test (rustdoc's lints, cargo-deny, a frozen install, `pnpm audit`); the
    /// step fails if it exits with an error, as [`Action::Run`].
    ToolGate(Cmd),
    /// Measure a crate and require 100% line, function and branch coverage of it.
    Coverage(Coverage),
    /// Measure [`COVERAGE_LOCALITY`](crate::workspace::COVERAGE_LOCALITY) and require the coverage gate to refuse it.
    CoverageLocality,
    /// Run a cargo-mutants command of the root's workspace, its report in
    /// [`MUTANTS_OUT`], then the doc-test pass of the
    /// mutants it missed (`mutants_with_doc_tests`, `doc_test_pass_command`).
    RustMutants(Cmd),
    /// Run every mutant of the GPU workspace ([`gpu_mutants_command`]), then add its
    /// unviable mutants to the root's report of a run of this extent, their paths from the
    /// root.
    GpuMutants(Extent),
    /// Run cargo-mutants on the changes of [`Mode::Changes`]: the diff since the merge base
    /// of [`MUTATION_BASES`](crate::workspace::MUTATION_BASES), written to [`CHANGES_DIFF`]; nothing when there is no change.
    RustMutantsOfChanges {
        /// The number of cargo-mutants jobs.
        jobs: NonZeroUsize,
    },
    /// Mutate the CPU reference's code that writes image bytes with the visual corpus's check
    /// as the only test, then require every mutant it missed to be reviewed in
    /// [`CORPUS_EXEMPT`](crate::corpus::CORPUS_EXEMPT), and every review to be current
    /// (`corpus_mutants`); in [`Mode::Changes`] only when a change reaches it
    /// ([`corpus_reached`](crate::corpus::corpus_reached)).
    CorpusMutants(Mode),
    /// Run cargo-mutants on the Rust lines changed since the merge base of `HEAD` with a
    /// revision only, not widened (`cargo xtask mutants --changed <revision>`); nothing when
    /// no Rust line changed.
    RustMutantsOfChangedLines {
        /// The number of cargo-mutants jobs.
        jobs: NonZeroUsize,
        /// The revision the changes are measured from.
        since: Revision,
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
    /// Require no code file to hold a pattern a fix retired
    /// ([`retired_patterns_gate`](crate::retired::retired_patterns_gate)).
    RetiredPatternsGate,
    /// Require every hint a message gives to name a runnable command
    /// ([`runnable_hints_gate`](crate::hints::runnable_hints_gate)).
    RunnableHintsGate,
    /// Require every place that starts a process to be listed, and every copy of git's
    /// variables to be xtask's ([`hermetic_git_gate`](crate::hermetic::hermetic_git_gate)).
    HermeticGitGate,
    /// Require every `fn new` of a crate's sources to be one call to a function of another
    /// name, which mutation testing reaches
    /// ([`delegating_new_gate`](crate::constructors::delegating_new_gate)).
    DelegatingNewGate,
    /// Require the reasons of the registers of proof bypasses and equivalent mutants to name
    /// what exists ([`register_names_gate`](crate::reviews::register_names_gate)).
    RegisterNamesGate,
    /// Require no tracked text file to hold a conflict marker git left
    /// ([`conflict_marker_gate`](crate::conflicts::conflict_marker_gate)).
    ConflictMarkerGate,
    /// Require every tracked text file to be of a known kind
    /// ([`source_kinds_gate`](crate::workspace::source_kinds_gate)).
    SourceKindsGate,
    /// Require no sentence of prose to write how many entries a typed list holds
    /// ([`list_counts_gate`](crate::counts::list_counts_gate)).
    ListCountsGate,
    /// Require every JavaScript dependency under an allowed license: the strict list for
    /// what the apps ship, the wider one for development tooling.
    LicenseGate,
    /// Require the lint against unsafe code to be lifted only in the audited files, and
    /// forbidden in every crate root ([`unsafe_gate`](crate::unsafety::unsafe_gate)).
    UnsafeGate,
    /// Require every root of both libraries to hold their lints
    /// ([`library_lints_gate`](crate::workspace::library_lints_gate)), and every `no_std`
    /// crate to be built for every portability target or listed with why not
    /// ([`no_std_portability_gate`](crate::workspace::no_std_portability_gate)): both read
    /// the leading attributes of the same sources, read once, and the step names both.
    LibraryLintsGate,
    /// Require the Cargo configuration to be exactly [`CARGO_CONFIG`](crate::dependencies::CARGO_CONFIG),
    /// and the nextest configuration to hold only its slow group
    /// ([`nextest_configs_gate`](crate::test_order::nextest_configs_gate)).
    CargoConfigGate,
    /// Require Rust test code to live only in `tests/` and declared `*_tests.rs` files
    /// ([`test_code_gate`](crate::test_code::test_code_gate)), and to assert an error whole
    /// ([`exact_errors_gate`](crate::exact_errors::exact_errors_gate)).
    TestCodeGate,
    /// Require the tests of each crate to read only its declared test data
    /// ([`test_data_gate`](crate::test_code::test_data_gate)), and the shared vectors of
    /// escaped messages to be read by each tool's ([`safe_vectors_gate`](crate::test_code::safe_vectors_gate)).
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
    /// Require every third-party build script and procedural macro to be reviewed
    /// ([`build_time_gate`](crate::dependencies::build_time_gate)), and the crates both
    /// Cargo workspaces resolve to be at one version in both lock files
    /// ([`shared_versions_gate`](crate::dependencies::shared_versions_gate)): both read
    /// every workspace's resolved metadata, read once.
    BuildTimeGate,
    /// The rules of [`library_gate`](crate::dependencies::library_gate) (what each library may depend on) and of
    /// [`shared_tests_gate`](crate::dependencies::shared_tests_gate) (which crates only tests use).
    LibraryGate,
    /// Require every part to be under the license of its folder, and no open part to depend
    /// on an app ([`part_license_gate`](crate::licenses::part_license_gate)).
    PartLicenseGate,
    /// Require every roadmap step a tracked file cites to exist on the board
    /// ([`citation_gate`](crate::citations::citation_gate)).
    CitationGate,
    /// Run the doc tests of the workspace with the error code of each `compile_fail` checked
    /// ([`doc_tests_command`]), after proving on [`DOC_CODES_FIXTURE`] that the code is
    /// checked: its doc test, failing with another code than the one it names, must fail.
    DocTests,
    /// Build and test the engine core and the boxes with the math layer's fused multiply-add
    /// option ([`fused_command`]), and require the option to have been in force
    /// ([`fused_gate`]).
    FusedBuild,
    /// Require every function a review of an unviable mutant names to exist
    /// ([`unviable_names_gate`](crate::test_code::unviable_names_gate)), each review added
    /// since the last version to name its test
    /// ([`unviable_reasons_gate`](crate::test_code::unviable_reasons_gate)), every test a
    /// specification names to exist ([`spec_names_gate`](crate::test_code::spec_names_gate)),
    /// and the index of the specifications to name the test of each reader of a file only a
    /// tool writes ([`reader_tests_gate`](crate::test_code::reader_tests_gate)).
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
    /// Require the GPU workspace to build the CPU reference as the root does
    /// ([`cpu_reference_gate`](crate::supply_chain::cpu_reference_gate)).
    CpuReferenceGate,
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
    /// platform, and a Vulkan driver when this says so ([`preflighted`]): checked first, so
    /// that a missing tool is named before anything runs.
    Preflight(GpuDriver),
    /// Require no source of the GPU workspace, which its mutation run mutates in place, to
    /// hold a mutant a stopped run left there (`preflight::mutant_left_gate`): checked right
    /// after the preflight, before anything reads, records or tests the tree.
    MutantLeftGate,
}

impl Action {
    /// Whether the action is a static gate: it checks the files of the repository and what
    /// tools say of them, a tool's own run included ([`Action::ToolGate`]), never a test, a
    /// mutant, a coverage measurement, a render or a package check, so that `cargo xtask
    /// check --gates` ([`gates_steps`]), part of every targeted check of a change, runs it
    /// without the long suites.
    ///
    /// Each action is classed here, with no wildcard, so that a new one does not compile
    /// before it is; a command step is classed by its variant, [`Action::ToolGate`] or
    /// [`Action::Run`]. Some build, install or fetch first: the loaded-files gate builds the
    /// workspace to read rustc's record of what it loaded, rustdoc documents it, the license
    /// gate reads the JavaScript dependencies the frozen install put in place, cargo-deny and
    /// `pnpm audit` read the advisory databases (so `--gates` needs the network, and fails
    /// without it), and the review of unviable mutants lists them (`cargo mutants --list`,
    /// no build).
    #[must_use]
    pub const fn is_static_gate(&self) -> bool {
        match self {
            Self::FloatMethodGate
            | Self::LoadedFilesGate
            | Self::EquivalentGate
            | Self::BypassGate
            | Self::UnsafeGate
            | Self::LibraryLintsGate
            | Self::CargoConfigGate
            | Self::TestCodeGate
            | Self::TestDataGate
            | Self::UntestedTargetsGate
            | Self::UnviableNamesGate
            | Self::DependencyGate
            | Self::PinGate
            | Self::BuildTimeGate
            | Self::LibraryGate
            | Self::PartLicenseGate
            | Self::MathLayerGate
            | Self::SupplyChainGate
            | Self::CpuReferenceGate
            | Self::MembershipGate
            | Self::ReadmeGate
            | Self::CitationGate
            | Self::VocabularyGate
            | Self::ConflictMarkerGate
            | Self::SourceKindsGate
            | Self::ListCountsGate
            | Self::RetiredPatternsGate
            | Self::RunnableHintsGate
            | Self::HermeticGitGate
            | Self::DelegatingNewGate
            | Self::RegisterNamesGate
            | Self::VersionGate
            | Self::HistoryGate
            | Self::WorkflowGate
            | Self::PartsGate
            | Self::LicenseGate
            | Self::StrykerConfigGate
            | Self::PackageCheckGate
            | Self::ToolGate(_)
            | Self::MutantLeftGate
            | Self::UnviableReviewsCurrent => true,
            Self::Run(_)
            | Self::Coverage(_)
            | Self::CoverageLocality
            | Self::RustMutants(_)
            | Self::RustMutantsOfChanges { .. }
            | Self::GpuMutants(_)
            | Self::CorpusMutants(_)
            | Self::RustMutantsOfChangedLines { .. }
            | Self::Conformance
            | Self::Bless(_)
            | Self::MutationProofRecorded
            | Self::UnviableGate(_)
            | Self::TypeScriptMutationGate { .. }
            | Self::FreshDirs(_)
            | Self::TypeScriptReportsGate(_)
            | Self::TypeScriptScopeGate(_)
            | Self::FusedBuild
            | Self::DocTests
            | Self::ShardsUnviableGate(_)
            | Self::TreeRecorded
            | Self::TreeUnchanged
            | Self::Preflight(_) => false,
        }
    }

    /// Whether the action runs code of the GPU workspace, which renders through Vulkan on
    /// Linux: its tests, its benchmark, its doc tests, its coverage or its mutants (a
    /// mutation run of the changes or of changed lines among them, which runs its mutants
    /// when the changes reach it).
    ///
    /// Each action is classed here, with no wildcard, so that a new one does not compile
    /// before it is. A command is classed by what it runs ([`runs_gpu_command`]), a static
    /// gate's tool ([`Action::ToolGate`]) too: none of today's (`cargo doc`, `cargo deny`,
    /// `pnpm install`, `pnpm audit`) runs code of the GPU workspace, and one that would is
    /// classed as such.
    #[must_use]
    pub fn runs_gpu_code(&self) -> bool {
        match self {
            Self::Run(cmd) | Self::ToolGate(cmd) => runs_gpu_command(cmd),
            Self::Coverage(coverage) => coverage.krate.workspace == CargoWorkspace::Gpu,
            Self::GpuMutants(_)
            | Self::DocTests
            | Self::RustMutantsOfChanges { .. }
            | Self::RustMutantsOfChangedLines { .. } => true,
            Self::CoverageLocality
            | Self::RustMutants(_)
            | Self::CorpusMutants(_)
            | Self::Conformance
            | Self::Bless(_)
            | Self::MutationProofRecorded
            | Self::UnviableGate(_)
            | Self::UnviableReviewsCurrent
            | Self::TypeScriptMutationGate { .. }
            | Self::EquivalentGate
            | Self::VersionGate
            | Self::FreshDirs(_)
            | Self::TypeScriptReportsGate(_)
            | Self::StrykerConfigGate
            | Self::PackageCheckGate
            | Self::TypeScriptScopeGate(_)
            | Self::BypassGate
            | Self::MembershipGate
            | Self::ReadmeGate
            | Self::VocabularyGate
            | Self::RetiredPatternsGate
            | Self::RunnableHintsGate
            | Self::HermeticGitGate
            | Self::DelegatingNewGate
            | Self::RegisterNamesGate
            | Self::ConflictMarkerGate
            | Self::SourceKindsGate
            | Self::ListCountsGate
            | Self::LicenseGate
            | Self::UnsafeGate
            | Self::LibraryLintsGate
            | Self::CargoConfigGate
            | Self::TestCodeGate
            | Self::TestDataGate
            | Self::UntestedTargetsGate
            | Self::FloatMethodGate
            | Self::DependencyGate
            | Self::PinGate
            | Self::BuildTimeGate
            | Self::LibraryGate
            | Self::PartLicenseGate
            | Self::CitationGate
            | Self::FusedBuild
            | Self::UnviableNamesGate
            | Self::LoadedFilesGate
            | Self::MathLayerGate
            | Self::SupplyChainGate
            | Self::CpuReferenceGate
            | Self::WorkflowGate
            | Self::PartsGate
            | Self::ShardsUnviableGate(_)
            | Self::TreeRecorded
            | Self::TreeUnchanged
            | Self::HistoryGate
            | Self::Preflight(_)
            | Self::MutantLeftGate => false,
        }
    }
}

/// The cargo subcommands that run the code they build: tests, a binary, coverage, mutants,
/// Miri.
pub const RUNNING_SUBCOMMANDS: [&str; 6] =
    ["test", "run", "nextest", "llvm-cov", "mutants", "miri"];

/// Whether `cmd` runs code of the GPU workspace: a cargo command given its manifest
/// ([`CargoWorkspace::manifest_args`]) whose subcommand, after any `+toolchain`, is one of
/// [`RUNNING_SUBCOMMANDS`].
#[must_use]
pub fn runs_gpu_command(cmd: &Cmd) -> bool {
    cmd.program == "cargo"
        && cmd
            .args
            .windows(2)
            .any(|pair| pair == CargoWorkspace::Gpu.manifest_args())
        && cmd
            .args
            .iter()
            .find(|arg| !arg.starts_with('+'))
            .is_some_and(|subcommand| RUNNING_SUBCOMMANDS.contains(&subcommand.as_str()))
}

/// `steps`, each preflight among them requiring a Vulkan driver exactly when one of them
/// runs code of the GPU workspace.
///
/// [`Action::runs_gpu_code`] says which do: what a part of the full check requires is
/// derived from what it runs, never listed.
#[must_use]
pub fn preflighted(steps: Vec<Step>) -> Vec<Step> {
    let driver = if steps.iter().any(|step| step.action.runs_gpu_code()) {
        GpuDriver::Required
    } else {
        GpuDriver::Unneeded
    };
    steps
        .into_iter()
        .map(|step| {
            if matches!(step.action, Action::Preflight(_)) {
                Step {
                    action: Action::Preflight(driver),
                    ..step
                }
            } else {
                step
            }
        })
        .collect()
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
    /// Whether it is a usage error: xtask's own, or one that a command xtask passed its
    /// arguments on to reported (`audio-cli bless`, exit code 2; git resolving no commit from
    /// the revision of `mutants --changed`): xtask then exits with 2, pointing to its usage.
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
/// message, a file, an argument, a path), so [`cli`](crate::cli::cli) passes its error line
/// and each line of the details through it, and so do the lines a step says that quote a path
/// or a reason (`not recorded: …`, a change that widens a mutation run); the usage and the
/// lists CI reads are written as they are.
#[must_use]
pub fn safe_message(message: &str) -> String {
    bounded(message, MAX_MESSAGE_CHARS)
}

/// The room [`remedied`] leaves, under [`MAX_MESSAGE_CHARS`], for what a line says before
/// its reason: a step's name, a gate's context and the pre-push hook's, each with its `: `
/// (a test holds the longest step's name and the longest contexts within it).
pub(crate) const CONTEXT_ROOM: usize = 400;

/// `reason` then `remedy`: the reason made safe and cut to the room the remedy and
/// [`CONTEXT_ROOM`] leave under [`MAX_MESSAGE_CHARS`], so that [`safe_message`] leaves the
/// remedy whole. A remedy that leaves the reason no room, or only its `…`, keeps the reason
/// whole (never said empty), which [`safe_message`] then cuts with the remedy: a known gap
/// (#24 step 109), from a path of 205 characters in the remedy of a file hidden by both marks.
pub(crate) fn remedied(reason: &str, remedy: &str) -> String {
    let taken = bounded(remedy, usize::MAX).chars().count();
    let room = MAX_MESSAGE_CHARS
        .checked_sub(taken.saturating_add(CONTEXT_ROOM))
        .filter(|room| *room > 1);
    room.map_or_else(|| reason.to_owned(), |room| bounded(reason, room)) + remedy
}

/// The remedy of an untracked file the checks read, `path` from the repository's root: the
/// command that adds it whatever ignores it (`--force`, which an ignore file of the user's
/// requires), its name quoted for a POSIX shell, or its removal.
pub(crate) fn add_untracked(path: &str) -> String {
    let quoted = path.replace('\'', r"'\''");
    format!("add it (see: git add --force -- ':/{quoted}'), or remove it")
}

/// [`safe_message`], bounded by `limit` characters, at least 2 (`remedied` keeps a reason
/// whole under a room of 2), a cut text ending with `…`.
fn bounded(message: &str, limit: usize) -> String {
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
        if used > limit {
            safe.truncate(cut);
            safe.push('…');
            return safe;
        }
        safe.push_str(&piece);
        if used < limit {
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

/// The details of a failure merging `failures`, in order: each one's own details, then its
/// summary, so that a summary saying its problems are "listed above" ([`verdict`]) comes
/// after them as printed. The one form of [`all_of`] and of the full check's report
/// ([`run_steps_then`](crate::actions::run_steps_then)).
pub(crate) fn listed_above(failures: Vec<Failure>) -> Vec<String> {
    failures
        .into_iter()
        .flat_map(|failure| failure.details.into_iter().chain([failure.summary]))
        .collect()
}

/// The verdict of gates independent of each other, each run whatever the others found
/// (`results`, evaluated before this is called): none failed passes; one failure is
/// returned as it is; several are counted as gates in the summary, and listed in the
/// details ([`listed_above`]), so that one step reports every failure of its gates at
/// once, as the full check reports every failure of its steps.
///
/// Two gates of one step are chained (`?`) only when the second reads what the first
/// proves (a precondition, stated where they are chained); otherwise they go through this.
/// It merges gates, never a usage error.
pub(crate) fn all_of(
    results: impl IntoIterator<Item = Result<(), Failure>>,
) -> Result<(), Failure> {
    let failures: Vec<Failure> = results.into_iter().filter_map(Result::err).collect();
    if failures.len() < 2 {
        return failures.into_iter().next().map_or(Ok(()), Err);
    }
    Err(Failure {
        summary: format!("{} gates failed, listed above", failures.len()),
        details: listed_above(failures),
        usage: false,
    })
}

/// Builds a command step.
pub(crate) fn run(name: &str, program: &'static str, args: &[&str], dir: Dir) -> Step {
    Step {
        name: name.to_owned(),
        action: Action::Run(Cmd::new(program, args, dir)),
    }
}

/// Builds a command step whose verdict is a static gate ([`Action::ToolGate`]).
pub(crate) fn tool_gate(name: &str, program: &'static str, args: &[&str], dir: Dir) -> Step {
    Step {
        name: name.to_owned(),
        action: Action::ToolGate(Cmd::new(program, args, dir)),
    }
}

/// The pinned tools, the platform and an environment that changes no build: the first step
/// of every command that starts the tools of the checks (every check, every mutation run of
/// `cargo xtask mutants`, and every bless, whose output later checks compare with).
///
/// It is the whole preflight in each, every tool of [`TOOLS`](crate::preflight::TOOLS)
/// required even where the command starts only some of them (a bless runs neither
/// cargo-mutants, cargo-deny, the coverage nor Miri): a machine that can bless or mutate can
/// then also check what it produced. A preflight of only the tools each command starts is
/// #24 step 153, of which the Vulkan driver is a first slice: each part of the full check,
/// which CI runs as a job of its own, requires it only when its steps run code of the GPU
/// workspace ([`preflighted`]).
pub(crate) fn preflight_step() -> Step {
    Step {
        name: "Tools and platform".to_owned(),
        action: Action::Preflight(GpuDriver::Required),
    }
}

/// What every command starting the tools of the checks opens with: the preflight
/// ([`preflight_step`]), then the refusal of a mutant left in place
/// ([`Action::MutantLeftGate`]), so that no check, mutation run or bless reads a source a
/// stopped run left mutated.
pub(crate) fn opening_gates() -> [Step; 2] {
    [
        preflight_step(),
        Step {
            name: "No mutant left in place in the GPU workspace".to_owned(),
            action: Action::MutantLeftGate,
        },
    ]
}

/// The tools and the sources first, then the state of the working tree: what every part
/// starts with.
pub(crate) fn opening_steps() -> Vec<(Role, Step)> {
    let [preflight, mutant_left] = opening_gates();
    vec![
        (Role::Frame, preflight),
        (Role::Frame, mutant_left),
        (Role::Frame, tree_recorded_step()),
    ]
}

/// The state of the working tree recorded ([`Action::TreeRecorded`]), for
/// [`tree_unchanged_step`].
fn tree_recorded_step() -> Step {
    Step {
        name: "Working tree recorded".to_owned(),
        action: Action::TreeRecorded,
    }
}

/// Tags each of `steps` with `role`.
pub(crate) fn tagged(role: Role, steps: Vec<Step>) -> Vec<(Role, Step)> {
    steps.into_iter().map(|step| (role, step)).collect()
}

/// A cargo step named `name` (ended by [`CargoWorkspace::label`]) running cargo's
/// `subcommand` with `args` in `workspace`.
pub(crate) fn cargo_step(
    name: &str,
    workspace: CargoWorkspace,
    subcommand: &str,
    args: &[&str],
) -> Step {
    run(
        &format!("{name}{}", workspace.label()),
        "cargo",
        &[&[subcommand], workspace.manifest_args(), args].concat(),
        Dir::Root,
    )
}

/// A cargo command of `workspace` whose verdict is a static gate ([`tool_gate`]), named as
/// [`cargo_step`] names it.
fn cargo_gate(name: &str, workspace: CargoWorkspace, subcommand: &str, args: &[&str]) -> Step {
    tool_gate(
        &format!("{name}{}", workspace.label()),
        "cargo",
        &[&[subcommand], workspace.manifest_args(), args].concat(),
        Dir::Root,
    )
}

/// The formatting, lints, documentation and tests of `workspace`, each workspace's the
/// same (the root's lints then its float methods gate).
fn workspace_static_steps(workspace: CargoWorkspace) -> Vec<Step> {
    let mut steps = vec![
        cargo_step("Rust formatting", workspace, "fmt", &["--all", "--check"]),
        cargo_step(
            "Rust lints",
            workspace,
            "clippy",
            &["--workspace", "--all-targets", "--locked"],
        ),
    ];
    if workspace == CargoWorkspace::Root {
        steps.push(Step {
            name: "Float methods of the pinned toolchain banned or reviewed as deterministic"
                .to_owned(),
            action: Action::FloatMethodGate,
        });
    }
    steps.extend([
        cargo_gate(
            "Rust documentation",
            workspace,
            "doc",
            &[
                "--workspace",
                "--no-deps",
                "--document-private-items",
                "--locked",
            ],
        ),
        workspace_tests_step(workspace),
    ]);
    steps
}

/// The tests of every crate of `workspace`, the ignored ones included.
fn workspace_tests_step(workspace: CargoWorkspace) -> Step {
    cargo_step(
        "Rust tests",
        workspace,
        "test",
        &["--workspace", "--locked", "--", INCLUDE_IGNORED],
    )
}

/// The GPU's time benchmark, its time checked against the baseline only on a GPU the
/// baseline holds (`visual-gpu check`).
fn gpu_time_step() -> Step {
    cargo_step(
        "Visual GPU time benchmark, checked only on a GPU the baseline holds",
        CargoWorkspace::Gpu,
        "run",
        &["--locked", "--quiet", "--", "check"],
    )
}

/// Formatting, lints, documentation and tests, of every workspace.
pub(crate) fn static_steps() -> Vec<Step> {
    let mut steps: Vec<Step> = CargoWorkspace::ALL
        .into_iter()
        .flat_map(workspace_static_steps)
        .collect();
    steps.push(gpu_time_step());
    steps.extend([
        Step {
            name: "Rust doc tests, the error code of each compile_fail checked".to_owned(),
            action: Action::DocTests,
        },
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
    ]);
    steps
}

/// The target directory of the doc tests whose error codes are checked.
///
/// They ([`doc_tests_command`]) build apart from the main build: what the variable of that
/// step changes in a build (a dependency's build script reading it) never reaches another.
pub const DOC_TESTS_TARGET: &str = "target/doc-codes";

/// The variable that lets the pinned compiler act as a nightly one for the crates it names:
/// rustdoc checks the error code of a `compile_fail` doc test only then.
pub const BOOTSTRAP_VARIABLE: &str = "RUSTC_BOOTSTRAP";

/// A crate whose one doc test names another error code than the one it fails with.
///
/// Its doc test is `compile_fail`: run as the workspace's doc tests are, it must fail,
/// proving that the codes are checked ([`Action::DocTests`]).
pub const DOC_CODES_FIXTURE: &str = "crates/xtask/tests/fixtures/doc-codes";

/// What rustdoc says of a `compile_fail` doc test that failed with other codes than those
/// it names, when it checks them.
pub const UNCHECKED_CODE: &str = "Some expected error codes were not found";

/// The value of [`BOOTSTRAP_VARIABLE`] for `crates`: their names as rustc reads them (`-`
/// written `_`), comma-separated, so that only they, never a dependency, see a nightly
/// compiler.
#[must_use]
pub fn bootstrapped(crates: &[&str]) -> String {
    crates
        .iter()
        .map(|name| name.replace('-', "_"))
        .collect::<Vec<_>>()
        .join(",")
}

/// The doc tests `selection` names (cargo's arguments choosing the packages) in
/// `workspace`, with the error code of each `compile_fail` checked: run on the pinned
/// compiler under [`BOOTSTRAP_VARIABLE`] for that workspace's crates ([`bootstrapped`] of
/// the crates of [`RUST_CRATES`] in it, the ones this command builds) and `others`, in
/// [`DOC_TESTS_TARGET`], the ignored ones included.
///
/// The one form of the workspace's run and of its fixture's, so that the fixture proves the
/// codes are checked in the configuration of the step.
fn checked_doc_tests(workspace: CargoWorkspace, selection: &[&str], others: &[&str]) -> Cmd {
    let crates: Vec<&str> = RUST_CRATES
        .iter()
        .filter(|krate| krate.workspace == workspace)
        .map(|krate| krate.name)
        .chain(others.iter().copied())
        .collect();
    let mut args = vec!["test"];
    args.extend(selection);
    args.extend([
        "--doc",
        "--locked",
        "--target-dir",
        DOC_TESTS_TARGET,
        "--",
        INCLUDE_IGNORED,
    ]);
    Cmd::new("cargo", &args, Dir::Root).with_env(BOOTSTRAP_VARIABLE, bootstrapped(&crates))
}

/// The doc tests of `workspace` with the error code of each `compile_fail` checked.
///
/// Rustdoc checks the code a `compile_fail` doc test names only on a nightly compiler: on
/// the pinned stable one, as the step "Rust tests" runs them, a doc test failing to compile
/// for another reason (a name renamed, a path moved) passes. This runs them on the pinned
/// compiler under [`BOOTSTRAP_VARIABLE`] for that workspace's crates only ([`bootstrapped`]
/// of the crates of [`RUST_CRATES`] in it, the ones this command builds), in
/// [`DOC_TESTS_TARGET`]; the preflight refuses the variable from the environment, so that
/// no other build sees it. Each workspace's are run, the GPU renderer's as the root's.
#[must_use]
pub fn doc_tests_command(workspace: CargoWorkspace) -> Cmd {
    checked_doc_tests(
        workspace,
        &[workspace.manifest_args(), &["--workspace"]].concat(),
        &[],
    )
}

/// The doc tests of [`DOC_CODES_FIXTURE`], run as [`doc_tests_command`] runs the
/// workspace's (`checked_doc_tests`, the fixture's crate named after the workspace's):
/// they must fail, saying [`UNCHECKED_CODE`].
#[must_use]
pub fn doc_codes_fixture_command() -> Cmd {
    let manifest = format!("{DOC_CODES_FIXTURE}/Cargo.toml");
    checked_doc_tests(
        CargoWorkspace::Root,
        &["--manifest-path", &manifest],
        &["doc-codes"],
    )
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

/// The native shell (`audio-cli`) and the floating-point mode it uses
/// (`fpenv`), whose code differs by architecture, linted then built for each of
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
            &format!(
                "Native code of audio-cli and fpenv built, inline assembly included ({target})"
            ),
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

/// Where every mutation run writes its report, as [`MUTANTS_ARGS`] gives it: the root, as
/// `mutants.out/` ([`mutants_args_to`] gives another directory).
pub(crate) const MUTANTS_OUTPUT: &str = "--output=.";

/// The arguments of every cargo-mutants run: no configuration file is read (`--no-config`)
/// and its report goes to `mutants.out` at the root (`--output=.`), whatever an ignored
/// `.cargo/mutants.toml` or `CARGO_MUTANTS_OUTPUT` says, which neither the bypass gate nor
/// the cleanliness checks see (the bypass gate still refuses a tracked `mutants.toml`,
/// dead under `--no-config`, so that a reviewer is told); lints are capped while building
/// mutants (otherwise a mutant that merely leaves a variable unused fails to compile and is
/// silently skipped instead of tested), the ignored files are not copied, and every cargo
/// command it runs keeps `Cargo.lock` as it is (`--locked`, as every other cargo command of
/// the checks) and builds without debug information ([`MUTANT_BUILD_CONFIG`]); on Linux,
/// each test program runs under a memory limit ([`MUTANT_TEST_RUNNER`]); each mutant's
/// tests run under nextest and stop at their first failure ([`MUTANT_TEST_TOOL`],
/// [`MUTANT_FAIL_FAST`]).
pub(crate) const MUTANTS_ARGS: [&str; 11] = [
    "mutants",
    "--no-config",
    MUTANTS_OUTPUT,
    "--cap-lints=true",
    "--gitignore=true",
    "--cargo-arg=--locked",
    MUTANT_BUILD_CONFIG,
    MUTANT_TEST_RUNNER,
    MUTANT_TEST_TOOL,
    MUTANT_FAIL_FAST,
    MUTANT_NO_TESTS,
];

/// The test runner of every cargo-mutants run: cargo-nextest (pinned in
/// [`PINNED_TOOLS`](crate::PINNED_TOOLS)), each test in its own process.
///
/// Chosen for [`MUTANT_FAIL_FAST`], which `cargo test` cannot do on a stable toolchain: its
/// harness lets every test already started run to its end after a failure. A measured case
/// (2026-11-18): in 151 of 214 mutants of audio-core, all caught, the slow tests of the
/// limiter ran over 60 seconds after the failure that caught the mutant.
///
/// What it changes in what a run proves, reviewed: a mutant's verdict is unchanged (caught
/// when a test fails, missed when none does, and a missed mutant still runs every test; a
/// run of no test at all is missed, as under `cargo test`, through [`MUTANT_NO_TESTS`]);
/// a test in its own process depends on no other test, so a test that did fails the
/// unmutated baseline, loudly; the harness argument [`INCLUDE_IGNORED`] keeps its meaning
/// (nextest reads it as `--run-ignored all`); Cargo's runner ([`MUTANT_TEST_RUNNER`]) still
/// starts every test program, as nextest runs them through it. Nextest does not run doc
/// tests, so every mutant it reports missed is tested again by `cargo test`, doc tests
/// included (`doc_test_pass_command`): a mutant only a doc test catches (one that removes
/// a check a `compile_fail` example proves, such as `replace engine_block with ()` in
/// audio-core) is caught as it was before nextest. Each workspace's slow tests run last
/// ([`slow_group`](crate::test_order::slow_group)), so that a mutant a fast test catches
/// stops in seconds; a priority only orders the tests, so that no verdict depends on it.
pub const MUTANT_TEST_TOOL: &str = "--test-tool=nextest";

/// Each mutant's tests stop at their first failure, the tests still running terminated:
/// the failure already decides that the mutant is caught.
///
/// Given to the test runs only (`--cargo-test-arg`): nextest refuses it while building
/// (`--no-run`). The unmutated baseline runs with it too, and fails as soon as a test fails.
pub const MUTANT_FAIL_FAST: &str = "--cargo-test-arg=--max-fail=1:immediate";

/// A mutant whose tests run no test is missed, never caught.
///
/// Nextest's default ends a run that finds no test with code 4, which cargo-mutants reads
/// as a failing test, so a mutant of a package with no test of its own would count as
/// caught (measured with nextest 0.9.152); `cargo test` succeeds there, so `warn` gives
/// nextest the same verdict, with a warning in the log. Given to the test runs only, as
/// [`MUTANT_FAIL_FAST`]; the doc-test pass, run by `cargo test`, drops it.
pub const MUTANT_NO_TESTS: &str = "--cargo-test-arg=--no-tests=warn";

/// The address space a test program of cargo-mutants may take on Linux, in bytes: 4 GiB.
///
/// A real incident (2026-11-16): CI's mutation shards 1 and 4 lost their runner three
/// times, a 16 GB machine; the probable cause is mutants that inflate a size, such as
/// `replace / with * in limiter::frames` or those of `Binary64::rounded_count`, whose
/// tests reserve gigabytes, at several jobs at once, before cargo-mutants could see them
/// fail. Under the limit such a test's allocation fails, the program aborts, and the mutant
/// is caught as any failing test is. It cannot hide a surviving mutant: the unmutated
/// baseline runs under the same limit, so a test that needs more fails the baseline and
/// with it the whole run, loudly, and a mutant that only needs more memory than the
/// unmutated code behaves differently. That the root workspace's test programs fit is
/// shown by the baseline of each run, on CI's Linux runners. The GPU workspace's mutants
/// run under the same limit, but have not run on CI yet: the limit counts reserved address
/// space, which a Vulkan driver inflates (lavapipe's threads and LLVM), so their baseline
/// there is measured at #22 step 7, with a data limit (`prlimit --data`) if it does not fit.
pub const MUTANT_MEMORY_LIMIT: u64 = 4 << 30;

/// The test runner of every cargo-mutants build on Linux: [`MUTANT_MEMORY_LIMIT`].
///
/// `prlimit` (util-linux) sets it as the address-space limit (`RLIMIT_AS`) of each test
/// program cargo runs, inherited by what that program starts; the preflight requires it on
/// Linux (`Need::PresentOnLinux`), with how to install it.
///
/// A runner, given on the command line (`--config`), applies to the test programs only,
/// never to rustc or cargo, whose builds keep the machine's memory; it changes no build
/// (Cargo's runner is not an input of what it compiles). Doc tests, which rustdoc runs
/// itself, are outside it. Elsewhere (macOS) the `cfg` matches nothing and the tests run
/// as before: the incident is CI's, whose runners are Linux.
pub const MUTANT_TEST_RUNNER: &str = "--cargo-arg=--config=target.'cfg(target_os = \"linux\")'.runner=[\"prlimit\", \"--as=4294967296\"]";

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
/// cargo-mutants' jobserver with every other build, or testing, each test run (nextest's,
/// [`MUTANT_TEST_TOOL`]) running as many tests at once as there are processors; so at most
/// `jobs` test runs' worth of work wants the processors at once, and under no outside load a test runs about `jobs` times
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

/// cargo-mutants' exit code when a run found mutants no test caught and nothing worse: no
/// timeout (exit code 3, which takes precedence) and a baseline that passed (4).
pub(crate) const MUTANTS_MISSED_EXIT: i32 = 2;

/// The directory, below the report of a cargo-mutants run, of the report of its doc-test
/// pass ([`doc_test_pass_command`]): git-ignored with the report, emptied with it.
pub(crate) const DOC_TEST_PASS: &str = "doc-tests";

/// The doc-test pass runs the doc tests alone (`cargo test --doc`): every other test of a
/// mutant it is given already ran under nextest, which missed it, so only the doc tests can
/// still catch it, and its baseline runs them alone too.
pub(crate) const DOC_TESTS_ONLY: &str = "--cargo-test-arg=--doc";

/// The characters the regular expressions of cargo-mutants (Rust's `regex`) give a
/// meaning: each is escaped to match itself. No other is, `\<` and `\>` being word
/// boundaries there.
pub(crate) const REGEX_META: &str = "\\.+*?()|[]{}^$#&-~";

/// The regular expression that matches the mutant `name` (as `--list` and `missed.txt` write
/// it, its line and column included) and nothing else: anchored, every character of
/// [`REGEX_META`] escaped.
#[must_use]
pub(crate) fn mutant_name_re(name: &str) -> String {
    let mut re = String::from("^");
    for character in name.chars() {
        if REGEX_META.contains(character) {
            re.push('\\');
        }
        re.push(character);
    }
    re.push('$');
    re
}

/// The doc-test pass of `first`, a cargo-mutants run whose report is `report` (the
/// directory holding its `missed.txt`), which missed the mutants `missed`.
///
/// Those mutants alone (`--re`, [`mutant_name_re`]), tested by `cargo test` on the doc tests
/// alone ([`DOC_TESTS_ONLY`]), which nextest leaves out ([`MUTANT_TEST_TOOL`]): `first`'s arguments but the test
/// tool, [`MUTANT_FAIL_FAST`] and [`MUTANT_NO_TESTS`], nextest's own, so that every test
/// runs; but its shard (`--shard`,
/// `--sharding`), which would select among those mutants only some; and its report written
/// to [`DOC_TEST_PASS`] below `first`'s, which the unviable gates read untouched. The tree,
/// filters (`--in-diff`, `--package`, `--file`), memory limit, build settings and timeouts
/// are `first`'s: a mutant `first` built builds again.
#[must_use]
pub(crate) fn doc_test_pass_command(first: &Cmd, report: &str, missed: &[&str]) -> Cmd {
    let mut all = first.args.iter();
    let mut options = all.by_ref().take_while(|arg| *arg != "--");
    let mut args: Vec<String> = Vec::new();
    while let Some(arg) = options.next() {
        match arg.as_str() {
            MUTANT_TEST_TOOL | MUTANT_FAIL_FAST | MUTANT_NO_TESTS => {}
            "--shard" => {
                let _index = options.next();
            }
            sharding if sharding.starts_with("--sharding=") => {}
            output if output.starts_with("--output=") => {
                args.push(format!("--output={report}/{DOC_TEST_PASS}"));
            }
            _ => args.push(arg.clone()),
        }
    }
    args.push(DOC_TESTS_ONLY.to_owned());
    args.extend(
        missed
            .iter()
            .map(|name| format!("--re={}", mutant_name_re(name))),
    );
    args.push("--".to_owned());
    args.extend(all.cloned());
    Cmd {
        args,
        ..first.clone()
    }
}

/// [`MUTANTS_ARGS`] with the report written to `out`, where every run's arguments write it
/// to the root (`--output=.`).
pub(crate) fn mutants_args_to(out: &str) -> Vec<String> {
    MUTANTS_ARGS
        .iter()
        .map(|&arg| {
            if arg == MUTANTS_OUTPUT {
                format!("--output={out}")
            } else {
                arg.to_owned()
            }
        })
        .collect()
}

/// The cargo-mutants command of the GPU workspace ([`CargoWorkspace::Gpu`]).
///
/// Every mutant, written to [`GPU_MUTANTS_OUT`](crate::workspace::GPU_MUTANTS_OUT), tested in place one
/// at a time (a copy of that workspace alone would lose its path dependencies on the
/// visual library, which cargo-mutants does not rewrite in development dependencies). A
/// run stopped before it restored its mutant leaves it in place, which every command
/// starting the tools of the checks then refuses ([`Action::MutantLeftGate`]). Nothing is
/// copied, so the copy's `--gitignore` is dropped: cargo-mutants refuses it with
/// `--in-place`.
#[must_use]
pub fn gpu_mutants_command() -> Cmd {
    let args = mutants_args_to(crate::workspace::GPU_MUTANTS_OUT);
    let args: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|&arg| arg != "--gitignore=true")
        .chain(CargoWorkspace::Gpu.manifest_args().iter().copied())
        .chain(["--in-place"])
        .collect();
    Cmd::new(
        "cargo",
        &[&args[..], &["--", "--", INCLUDE_IGNORED]].concat(),
        Dir::Root,
    )
}

/// The list of every mutant of the GPU workspace, each path from that workspace's root.
///
/// As [`gpu_mutants_command`] would run them: its arguments, but `--gitignore`, which
/// cargo-mutants refuses with `--in-place` there.
#[must_use]
pub fn gpu_mutants_list_command() -> Cmd {
    let args: Vec<&str> = MUTANTS_ARGS
        .iter()
        .copied()
        .filter(|&arg| arg != "--gitignore=true")
        .chain(CargoWorkspace::Gpu.manifest_args().iter().copied())
        .chain(["--list"])
        .collect();
    Cmd::new("cargo", &args, Dir::Root)
}

/// The cargo-mutants command of [`Mode::Changes`]: the workspace in `jobs` jobs, filtered
/// by [`CHANGES_DIFF`].
#[must_use]
pub fn changes_mutants_command(jobs: NonZeroUsize) -> Cmd {
    tested_mutants_command(&["--workspace", "--in-diff", CHANGES_DIFF], jobs)
}

/// The cargo-mutants command that lists, one per line, the mutants
/// [`changes_mutants_command`] tests, without testing them (`--list`, with the same
/// arguments but those of the tests' jobs and timeouts).
#[must_use]
pub fn changes_list_command() -> Cmd {
    mutants_command(&["--workspace", "--in-diff", CHANGES_DIFF, "--list"])
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

/// Empties [`MUTANTS_OUT`], so that the unviable gate reads the report of this run only.
fn fresh_mutation_report_step() -> Step {
    Step {
        name: "Fresh Rust mutation report".to_owned(),
        action: Action::FreshDirs(vec![MUTANTS_OUT.to_owned()]),
    }
}

/// Every unviable mutant of a run of `extent` reviewed ([`Action::UnviableGate`]).
fn unviable_step(extent: Extent) -> Step {
    Step {
        name: "Rust unviable mutants reviewed".to_owned(),
        action: Action::UnviableGate(extent),
    }
}

/// Every mutant of the GPU workspace, its unviable ones added to the root's report of a
/// run of `extent` ([`Action::GpuMutants`]).
fn gpu_mutants_step(extent: Extent) -> Step {
    Step {
        name: format!("Rust mutation testing{}", CargoWorkspace::Gpu.label()),
        action: Action::GpuMutants(extent),
    }
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
            Action::RustMutants(workspace_mutants_command(jobs, None)),
            Extent::Complete,
        ),
    };
    let mut steps = vec![
        (Role::EveryShard, fresh_mutation_report_step()),
        (
            Role::Sharded,
            Step {
                name: name.to_owned(),
                action,
            },
        ),
        (Role::AcrossShards, unviable_step(extent)),
    ];
    // The GPU workspace's mutants: every one in a full check (in the first shard of CI),
    // after the root's, whose report they add to; in the default check, only when the
    // changes reach its crate, by the changes' own run (`rust_mutants_of_changes`).
    if matches!(mode, Mode::Full { .. }) {
        steps.insert(
            2,
            (
                Role::Proof(Part::Mutation(Shard::FIRST)),
                gpu_mutants_step(extent),
            ),
        );
    }
    // The corpus's adequacy: its own report, in the first shard of CI with the GPU
    // workspace's mutants (`corpus`).
    steps.extend(tagged(
        Role::Proof(Part::Mutation(Shard::FIRST)),
        corpus_steps(mode).to_vec(),
    ));
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
    let gates =
        vec![
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
            name: "Every root of both libraries denies its lints; every `no_std` crate built for every portability target or listed".to_owned(),
            action: Action::LibraryLintsGate,
        },
        Step {
            name: "Cargo configuration: no build flag, a locked alias, the GPU workspace sharing the root's settings".to_owned(),
            action: Action::CargoConfigGate,
        },
        Step {
            name: "Test code only in tests/ and declared *_tests.rs files; a Rust test compares an error whole".to_owned(),
            action: Action::TestCodeGate,
        },
        Step {
            name: "The tests of each crate read only its declared test data; each Rust command-line tool's tests read the shared vectors".to_owned(),
            action: Action::TestDataGate,
        },
        Step {
            name: "Tests turned off only where no test can hide, doc tests kept, each compile_fail \
             naming its error code".to_owned(),
            action: Action::UntestedTargetsGate,
        },
        Step {
            name: "Every function a review or a specification names exists, each new review naming its test; each reader of a file only a tool writes is named in the index".to_owned(),
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
        cargo_gate(
            "Rust supply chain",
            CargoWorkspace::Root,
            "deny",
            &["--locked", "check"],
        ),
        cargo_gate(
            "Rust supply chain",
            CargoWorkspace::Gpu,
            "deny",
            &["--locked", "check"],
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
            name: "Every third-party build script and procedural macro reviewed; the crates both lock files share at one version".to_owned(),
            action: Action::BuildTimeGate,
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
/// unoptimized and half a second optimized (measured once on the maintainer's Mac,
/// 2026-11-15, not by a benchmark); the arithmetic is checked
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

/// The gates over the tracked text files: citations, counts, words, conflict markers,
/// kinds, retired patterns, hints, the git a test starts and the reasons of the registers.
fn text_steps() -> [Step; 10] {
    [
        Step {
            name: "Every roadmap step cited exists on the board".to_owned(),
            action: Action::CitationGate,
        },
        Step {
            name: "No prose counts a typed list".to_owned(),
            action: Action::ListCountsGate,
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
            name: "Every tracked text file of a known kind".to_owned(),
            action: Action::SourceKindsGate,
        },
        Step {
            name: "No pattern a fix retired, in the code it was retired from".to_owned(),
            action: Action::RetiredPatternsGate,
        },
        Step {
            name: "Every hint a message gives names a command a user can run".to_owned(),
            action: Action::RunnableHintsGate,
        },
        Step {
            name: "Every place that starts a process is listed; every copy of git's variables is xtask's"
                .to_owned(),
            action: Action::HermeticGitGate,
        },
        Step {
            name: "Every new of a crate is one call to a function mutation testing reaches"
                .to_owned(),
            action: Action::DelegatingNewGate,
        },
        Step {
            name: "Every reason of the registers of proof bypasses and equivalent mutants names what exists".to_owned(),
            action: Action::RegisterNamesGate,
        },
    ]
}

/// The native conformance check of the audio corpus ([`Action::Conformance`]).
fn conformance_step() -> Step {
    Step {
        name: "Conformance (native)".to_owned(),
        action: Action::Conformance,
    }
}

/// Conformance, supply chain, then the consistency of the repository: membership, READMEs,
/// versions, history and the CI workflow.
pub(crate) fn consistency_steps() -> Vec<(Role, Step)> {
    let conformance = conformance_step();
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
            name: "The CPU reference built in the GPU workspace as at the root".to_owned(),
            action: Action::CpuReferenceGate,
        },
        Step {
            name: "Packages and crates known to the checks".to_owned(),
            action: Action::MembershipGate,
        },
        Step {
            name: "Every part described in a README".to_owned(),
            action: Action::ReadmeGate,
        },
    ]);
    gates.extend(text_steps());
    gates.extend([
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
            tool_gate(
                "JavaScript dependencies",
                "pnpm",
                &["install", "--frozen-lockfile"],
                Dir::Root,
            ),
        ),
        (
            gates,
            tool_gate(
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
    ];
    // Built before any package, each after the builds it uses (in the order of PACKAGES), so
    // that a package using them can be checked without the checks of the packages built (as
    // a part of the full check does).
    for built in needed_builds() {
        steps.push((
            Role::Setup(Setup::Build(built.dir)),
            run(
                &format!("Build of {}, for the packages that use it", built.dir),
                "pnpm",
                &["run", "build"],
                Dir::Package(built.dir),
            ),
        ));
    }
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

/// What every part closes with: the working tree as the opening recorded it
/// ([`Action::TreeUnchanged`]).
fn tree_unchanged_step() -> Step {
    Step {
        name: "Working tree unchanged by the checks".to_owned(),
        action: Action::TreeUnchanged,
    }
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
        vec![(Role::Frame, tree_unchanged_step())],
    ]
    .concat()
}

/// Every check of the repository in `mode`, in the order they run.
#[must_use]
pub fn check_steps(mode: Mode) -> Vec<Step> {
    roles(mode).into_iter().map(|(_, step)| step).collect()
}

/// The steps of `cargo xtask check --gates`: the preflight, then the static gates.
///
/// They are every static gate of either mode ([`Action::is_static_gate`]): those of the full
/// check, taken from its steps in their order, then those only the default check runs (the
/// review of unviable mutants against cargo-mutants' list), so that a gate added to either
/// check is among them by construction.
#[must_use]
pub fn gates_steps() -> Vec<Step> {
    let jobs = NonZeroUsize::MIN;
    let full = check_steps(Mode::Full { jobs });
    let only_changes: Vec<Step> = check_steps(Mode::Changes { jobs })
        .into_iter()
        .filter(|step| !full.contains(step))
        .collect();
    core::iter::once(preflight_step())
        .chain(
            full.into_iter()
                .chain(only_changes)
                .filter(|step| step.action.is_static_gate()),
        )
        .collect()
}

/// A part of `cargo xtask check --native`: what CI's runner cannot prove, run on this
/// machine.
///
/// CI runs on `x86_64` Linux, with no GPU of its own; each part runs only when a change
/// since `main` reaches a crate it runs ([`NativePart::runs`]), and every part when what
/// decides xtask's checks or builds xtask changes (`changes::native_reached`), selected by
/// `changes::native_parts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativePart {
    /// Every test of the root workspace on this machine's architecture: the floating-point
    /// mode's code differs by architecture, and every other crate's laws and property tests
    /// run in CI on `x86_64` only.
    Tests,
    /// The audio corpus's native conformance.
    Audio,
    /// The visual corpus's native conformance.
    Visual,
    /// The GPU workspace, on this machine's GPU: its tests, its time against the baseline,
    /// then its mutants, run in place.
    Gpu,
}

impl NativePart {
    /// Every part, in the order they run: the GPU's last, since its mutants take longest.
    pub const ALL: [Self; 4] = [Self::Tests, Self::Audio, Self::Visual, Self::Gpu];

    /// The part's name, as the check says it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Tests => "Rust tests",
            Self::Audio => "audio",
            Self::Visual => "visual",
            Self::Gpu => "GPU",
        }
    }

    /// Whether the part runs `krate`'s code or data: a change to one of its inputs (a crate
    /// it builds, its declared test data) or one widened to it selects the part. The tests
    /// part runs every crate of the root workspace; each corpus, the command checking it.
    #[must_use]
    pub fn runs(self, krate: &RustCrate) -> bool {
        match self {
            Self::Tests => krate.workspace == CargoWorkspace::Root,
            Self::Audio => krate.name == "audio-cli",
            Self::Visual => krate.name == "visual-cli",
            Self::Gpu => krate.name == "visual-gpu",
        }
    }

    /// The steps of the part, each of them a step of the check.
    #[must_use]
    pub fn steps(self) -> Vec<Step> {
        match self {
            Self::Tests => vec![workspace_tests_step(CargoWorkspace::Root)],
            Self::Audio => vec![conformance_step()],
            Self::Visual => vec![visual_conformance_step()],
            // The time before the mutants, which load the machine; the mutants as a partial
            // run, the root's not run: their unviable ones reviewed, none called stale.
            Self::Gpu => vec![
                workspace_tests_step(CargoWorkspace::Gpu),
                gpu_time_step(),
                fresh_mutation_report_step(),
                gpu_mutants_step(Extent::Partial),
                unviable_step(Extent::Partial),
            ],
        }
    }
}

/// The steps of `cargo xtask check --native` running `parts`, after its opening gates.
///
/// The opening gates (`opening_gates`) and the refusal of changes not committed run before
/// the changes are read; then the working tree recorded, the steps of each part in order,
/// and the tree unchanged, as every part of the checks frames itself.
#[must_use]
pub fn native_steps(parts: &[NativePart]) -> Vec<Step> {
    core::iter::once(tree_recorded_step())
        .chain(parts.iter().flat_map(|part| part.steps()))
        .chain([tree_unchanged_step()])
        .collect()
}

#[cfg(test)]
#[path = "steps_tests.rs"]
mod tests;
