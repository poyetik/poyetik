//! The workspace as the checks see it: its crates, packages, targets, paths and modes.

use core::num::NonZeroUsize;

use crate::{
    parts::Setup,
    steps::{Cmd, Dir, INCLUDE_IGNORED},
};

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
    /// The files of the repository outside its directory that its tests read (`spec/`
    /// vectors, the conformance corpus…), each a file or a directory ending in `/`,
    /// checked by [`test_data_gate`](crate::test_code::test_data_gate): a change to one of them can weaken its tests as much
    /// as a change to its test code.
    pub test_data: &'static [&'static str],
    /// What the rules of the checks take it for ([`CrateRole`]): the one place a crate's
    /// role is written, every list of crates of one role derived from it
    /// ([`crates_in`]).
    pub roles: &'static [CrateRole],
}

/// A role of a crate of [`RUST_CRATES`] that a rule of the checks depends on; a crate may
/// hold several.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrateRole {
    /// A crate of the audio library: a library usable on its own, which needs nothing else
    /// of the project (`spec/engine-design.md`, decision 32; [`library_gate`](crate::dependencies::library_gate)).
    Library,
    /// A crate the tests of the audio library may use beside it (a development dependency),
    /// never its code ([`library_gate`](crate::dependencies::library_gate)).
    LibraryTests,
    /// A crate that may depend on `libm` outside its tests (`spec/engine-design.md`,
    /// decision 10; [`math_layer_gate`](crate::math_layer::math_layer_gate)): the engine core, whose math layer is the only
    /// path from box arithmetic to a math function, and the test kit, whose measurements
    /// must not share the arithmetic of the boxes they measure. A box crate is `no_std`,
    /// where `f32` and `f64` have no math method: without `libm`, it can only reach the
    /// math layer's. A test may use `libm` directly (a development dependency), for
    /// references computed apart from the code under test.
    Libm,
    /// A crate whose sources the math layer gate reads.
    MathLayer,
    /// Code for tests: a change below its directory can let a mutant of any crate whose
    /// tests use it survive, so it widens the default check to all of them. Every crate the
    /// workspace reaches only through development dependencies, directly or through such
    /// crates, holds it ([`shared_tests_gate`](crate::dependencies::shared_tests_gate)).
    SharedTests,
}

/// The names of the crates of [`RUST_CRATES`] that hold `role`, in their order.
#[must_use]
pub fn crates_in(role: CrateRole) -> Vec<&'static str> {
    RUST_CRATES
        .iter()
        .filter(|krate| krate.roles.contains(&role))
        .map(|krate| krate.name)
        .collect()
}

impl RustCrate {
    /// The path of `path` relative to this crate's directory, if the crate holds it (a path
    /// git names, `/`-separated): `crates/audio-core/src/lib.rs` is `src/lib.rs` of
    /// `audio-core`, while `crates/audio-core-x/a.rs` and the directory itself are none
    /// of its files.
    #[must_use]
    pub fn holds<'path>(&self, path: &'path str) -> Option<&'path str> {
        below(self.dir, path)
    }
}

/// The path of `path` relative to the directory `dir`, if it lies below it (paths git
/// names, `/`-separated): `a/b/c` is `c` below `a/b`, while `a/bc` and `a/b` itself are
/// not below it. The one rule of [`RustCrate::holds`] and of the mutation targets.
pub(crate) fn below<'path>(dir: &str, path: &'path str) -> Option<&'path str> {
    path.strip_prefix(dir)?.strip_prefix('/')
}

/// The crates of the Cargo workspace (checked by [`membership_gate`](crate::reviews::membership_gate)): each is measured by
/// its own unit tests, and each binary by the crate's tests.
pub const RUST_CRATES: [RustCrate; 15] = [
    RustCrate {
        name: "audio-core",
        dir: "crates/audio-core",
        binary: false,
        test_data: &["conformance/test-matrix.json"],
        roles: &[CrateRole::Library, CrateRole::Libm, CrateRole::MathLayer],
    },
    RustCrate {
        name: "audio-boxes",
        dir: "crates/audio-boxes",
        binary: false,
        test_data: &[
            "spec/benchmark-baseline.json",
            "spec/measurement-vectors.json",
        ],
        roles: &[CrateRole::Library, CrateRole::MathLayer],
    },
    RustCrate {
        name: "audio-patch",
        dir: "crates/audio-patch",
        binary: false,
        test_data: &["spec/safe-message-vectors.json"],
        roles: &[CrateRole::Library],
    },
    RustCrate {
        name: "audio-hash",
        dir: "crates/audio-hash",
        binary: false,
        test_data: &["spec/render-hash-vectors.json"],
        roles: &[CrateRole::Library],
    },
    RustCrate {
        name: "visual-core",
        dir: "crates/visual-core",
        binary: false,
        test_data: &[],
        roles: &[],
    },
    RustCrate {
        name: "visual-boxes",
        dir: "crates/visual-boxes",
        binary: false,
        test_data: &[],
        roles: &[],
    },
    RustCrate {
        name: "visual-hash",
        dir: "crates/visual-hash",
        binary: false,
        test_data: &["spec/image-hash-vectors.json"],
        roles: &[],
    },
    RustCrate {
        name: "visual-testkit",
        dir: "crates/visual-testkit",
        binary: false,
        test_data: &["conformance/visual/test-matrix.json"],
        roles: &[],
    },
    RustCrate {
        name: "alloc-counter",
        dir: "crates/alloc-counter",
        binary: false,
        test_data: &[],
        roles: &[CrateRole::SharedTests],
    },
    RustCrate {
        name: "audio-fpenv",
        dir: "crates/audio-fpenv",
        binary: false,
        test_data: &["conformance/"],
        roles: &[CrateRole::Library],
    },
    RustCrate {
        name: "audio-battery",
        dir: "crates/audio-battery",
        binary: false,
        test_data: &["conformance/test-matrix.json"],
        roles: &[CrateRole::SharedTests],
    },
    RustCrate {
        name: "audio-testkit",
        dir: "crates/audio-testkit",
        binary: false,
        test_data: &["spec/measurement-vectors.json"],
        roles: &[
            CrateRole::LibraryTests,
            CrateRole::Libm,
            CrateRole::SharedTests,
        ],
    },
    RustCrate {
        name: "audio-cli",
        dir: "crates/audio-cli",
        binary: true,
        test_data: &["conformance/"],
        roles: &[],
    },
    RustCrate {
        name: "audio-wasm",
        dir: "bindings/audio-wasm",
        binary: false,
        test_data: &[],
        roles: &[CrateRole::Library],
    },
    RustCrate {
        name: "xtask",
        dir: "crates/xtask",
        binary: true,
        // Its tests read fixtures of their own, under its directory, the repository's
        // hooks, which they run, and the workflow, the one file whose whole text a gate
        // decides; the gates that read the rest of the repository are steps of every
        // check, not tests that kill its mutants.
        test_data: &[".githooks/", ".github/workflows/quality.yml"],
        roles: &[],
    },
];

/// A crate whose unit tests reach one function and whose integration test reaches both:
/// the coverage gate must refuse it, proving that only a crate's own unit tests count.
pub const COVERAGE_LOCALITY: RustCrate = RustCrate {
    name: "coverage-locality",
    dir: "crates/xtask/tests/fixtures/coverage-locality",
    binary: false,
    test_data: &[],
    roles: &[],
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
/// upstream first, so that before a push the changes are what the push adds; then
/// `origin/main`, then `main`, for a branch without an upstream. With none of them, the
/// check fails and says to run `--full`.
pub const MUTATION_BASES: [&str; 3] = ["@{upstream}", "origin/main", "main"];

/// What reached `main` is read against, in order.
///
/// What reached `main`: the algorithm versions spent (`spec/engine-design.md`, decision 29:
/// a version number is spent when it reaches `main`) and the released sections of the
/// changelog ([`released_sections_gate`](crate::release::released_sections_gate)).
/// The merge base of `HEAD` with the first of these that git resolves: `main` as the
/// remote holds it, else as this clone does. Never the branch's upstream, unlike
/// [`MUTATION_BASES`], which answer another question (what changed): a version pushed on a
/// branch has not reached `main`, and successive changes on that branch share it.
pub const MAIN_BASES: [&str; 2] = ["origin/main", "main"];

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

/// What [`run_steps_then`](crate::actions::run_steps_then) does when a step fails.
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
    /// under [`SHIPPED_LICENSES`](crate::licenses::SHIPPED_LICENSES).
    pub shipped: bool,
    /// Its build output directories that its build does not empty itself (`tsc` never
    /// removes an output whose source is gone), relative to its directory: emptied before
    /// the checks, so that no stale output of an earlier run is used by a later step.
    pub outputs: &'static [&'static str],
    /// What its checks need besides the JavaScript dependencies, which every package needs:
    /// prepared again in its part of the full check ([`Part::needs`](crate::parts::Part::needs)), since its checks may
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
        dir: "bindings/audio-wasm",
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

/// The WebAssembly target: the engine's on the web, [`WASM_PACKAGE`]'s.
pub const WASM_TARGET: &str = "wasm32-unknown-unknown";

/// Targets the engine core must build for: proof that it depends on no environment.
pub const PORTABILITY_TARGETS: [&str; 4] = [
    "aarch64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    WASM_TARGET,
    "thumbv7em-none-eabihf",
];

/// The crates of the engine core and of the visual library's core, built for every one of
/// [`PORTABILITY_TARGETS`].
pub const PORTABILITY_PACKAGES: [&str; 7] = [
    "audio-core",
    "audio-boxes",
    "audio-patch",
    "audio-hash",
    "visual-core",
    "visual-boxes",
    "visual-hash",
];

/// The engine's crate for WebAssembly, which the engine package builds for [`WASM_TARGET`].
pub const WASM_PACKAGE: &str = "audio-wasm";

/// The native shell and the library's floating-point mode it uses, whose code differs by
/// architecture: built for each of [`NATIVE_TARGETS`].
pub const NATIVE_PACKAGES: [&str; 2] = ["audio-fpenv", "audio-cli"];

/// Cargo's arguments selecting `packages`: `--package <name>` for each, in order.
pub(crate) fn package_args<'name>(packages: &[&'name str]) -> Vec<&'name str> {
    packages
        .iter()
        .flat_map(|&package| ["--package", package])
        .collect()
}

/// Targets of the native shell (`audio-cli`) and of its floating-point mode
/// (`audio-fpenv`), whose code differs by architecture.
///
/// Each is linted for both, whatever the machine, so that CI (`x86_64` Linux) compiles the
/// aarch64 code and a Mac the `x86_64` code.
pub const NATIVE_TARGETS: [&str; 2] = ["aarch64-apple-darwin", "x86_64-unknown-linux-gnu"];
