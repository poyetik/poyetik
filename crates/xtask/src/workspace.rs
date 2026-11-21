//! The workspace as the checks see it: its crates, packages, targets, paths and modes.

use core::num::NonZeroUsize;

use crate::{
    parts::Setup,
    rust_source::{RustSource, same_tokens, tokens_of},
    steps::{Cmd, Dir, Failure, INCLUDE_IGNORED, step_verdict, verdict},
};

/// Pinned nightly toolchain, used only to measure branch coverage and to run Miri.
pub const NIGHTLY_TOOLCHAIN: &str = "+nightly-2026-11-01";

/// Where the coverage reports of the crates are written, one per measurement.
pub const COVERAGE_DIR: &str = "target/coverage";

/// Where the fixtures of xtask's own checks live: workspaces of their own, which are not
/// parts of the project.
pub const FIXTURES: &str = "crates/xtask/tests/fixtures/";

/// What of a kind of tracked text file the gates of prose read ([`SourceKind::reading`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reading {
    /// Markdown: every line outside code blocks, code spans and link targets apart.
    Markdown,
    /// A text file, a reviewed list: every line.
    Text,
    /// Rust: its comments and string literals.
    Rust,
    /// TypeScript or JavaScript: its comments and string literals.
    Script,
    /// JSON: its string values.
    Json,
    /// A file whose comments open with `#` and a space (configuration, a git hook).
    Hash,
    /// A file whose comments open with `//` or `/*` and whose strings no gate reads (a
    /// shader, a style sheet, a page).
    Comments,
    /// Nothing: a file a tool writes, data or a license, which no sentence of the project's
    /// is written in.
    Nothing,
}

/// What the checks take a kind of tracked text file for ([`SourceKind::role`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceRole {
    /// Rust code, which every gate of code reads.
    Rust,
    /// TypeScript or JavaScript code a package's coverage and mutation testing could reach,
    /// with whether Stryker mutates it (an Astro template it does not).
    Script {
        /// Whether Stryker mutates it.
        mutated: bool,
    },
    /// Not code: why no gate of code reads it.
    Other(&'static str),
}

/// One kind of tracked text file: its pattern, what of it is prose, and what it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceKind {
    /// Its git pathspec, written with one `*` at most, which stands for any text, `/`
    /// included.
    pub pattern: &'static str,
    /// What of it the gates of prose read.
    pub reading: Reading,
    /// Code, and of which language, or why no gate of code reads it.
    pub role: SourceRole,
}

/// A TypeScript or JavaScript source Stryker mutates.
const MUTATED: SourceRole = SourceRole::Script { mutated: true };

/// Every kind of tracked text file, its pattern, what of it is prose and what it is
/// ([`SourceKind`], read through [`source_kinds`]): the one table every gate's list of files
/// is derived from.
///
/// [`code_sources`], [`ts_sources`], [`stryker_sources`],
/// [`vocabulary_sources`](crate::vocabulary::vocabulary_sources),
/// [`citation_sources`](crate::citations::citation_sources) and
/// [`bypass_sources`](crate::reviews::bypass_sources) filter it by what each gate reads,
/// and each reader of prose asks it how a file is read ([`reading_of`]).
///
/// [`source_kinds_gate`] refuses a tracked text file of no kind, so that a new kind of file
/// is classified before a gate misses it.
pub const SOURCE_KINDS: [(&str, Reading, SourceRole); 29] = [
    ("*.rs", Reading::Rust, SourceRole::Rust),
    ("*.ts", Reading::Script, MUTATED),
    ("*.tsx", Reading::Script, MUTATED),
    ("*.mts", Reading::Script, MUTATED),
    ("*.cts", Reading::Script, MUTATED),
    ("*.js", Reading::Script, MUTATED),
    ("*.jsx", Reading::Script, MUTATED),
    ("*.mjs", Reading::Script, MUTATED),
    ("*.cjs", Reading::Script, MUTATED),
    (
        "*.astro",
        Reading::Script,
        SourceRole::Script { mutated: false },
    ),
    (
        "*.md",
        Reading::Markdown,
        SourceRole::Other("documentation, read by the gates of prose"),
    ),
    (
        "*.txt",
        Reading::Text,
        SourceRole::Other("a reviewed list, read by its gate"),
    ),
    (
        "*.json",
        Reading::Json,
        SourceRole::Other("data and configuration"),
    ),
    ("*.toml", Reading::Hash, SourceRole::Other("configuration")),
    ("*.yml", Reading::Hash, SourceRole::Other("configuration")),
    ("*.yaml", Reading::Hash, SourceRole::Other("configuration")),
    (
        "*.lock",
        Reading::Nothing,
        SourceRole::Other("a lockfile its tool writes"),
    ),
    ("*.svg", Reading::Nothing, SourceRole::Other("an image")),
    (
        "*.wgsl",
        Reading::Comments,
        SourceRole::Other("a shader, which gives no message"),
    ),
    (
        "*.html",
        Reading::Comments,
        SourceRole::Other("a page, whose scripts are code files of their own"),
    ),
    (
        "*.css",
        Reading::Comments,
        SourceRole::Other("a style sheet"),
    ),
    (
        "*.inc",
        Reading::Nothing,
        SourceRole::Other("data a fixture includes"),
    ),
    (
        "*.gitignore",
        Reading::Hash,
        SourceRole::Other("git's configuration"),
    ),
    (
        "*.gitattributes",
        Reading::Hash,
        SourceRole::Other("git's configuration"),
    ),
    (
        "*.editorconfig",
        Reading::Hash,
        SourceRole::Other("editors' configuration"),
    ),
    (
        ".node-version",
        Reading::Nothing,
        SourceRole::Other("the pinned version of Node.js"),
    ),
    (
        "LICENSE-*",
        Reading::Nothing,
        SourceRole::Other("a license text"),
    ),
    (
        "*/LICENSE",
        Reading::Nothing,
        SourceRole::Other("a license text"),
    ),
    (
        ".githooks/*",
        Reading::Hash,
        SourceRole::Other("a git hook, which runs `cargo xtask` and gives no message of its own"),
    ),
];

/// The kinds of [`SOURCE_KINDS`], in its order.
#[must_use = "an iterator does nothing unless consumed"]
pub fn source_kinds() -> impl Iterator<Item = SourceKind> {
    SOURCE_KINDS
        .into_iter()
        .map(|(pattern, reading, role)| SourceKind {
            pattern,
            reading,
            role,
        })
}

/// The patterns of the kinds of [`SOURCE_KINDS`] that `keep` keeps, in its order.
pub(crate) fn sources_where(keep: impl Fn(&SourceKind) -> bool) -> Vec<&'static str> {
    source_kinds()
        .filter(|kind| keep(kind))
        .map(|kind| kind.pattern)
        .collect()
}

/// The tracked files of code (git pathspecs), where messages and their hints are written:
/// Rust, and every language a TypeScript package's coverage and mutation testing reaches
/// ([`ts_sources`]).
#[must_use]
pub fn code_sources() -> Vec<&'static str> {
    sources_where(|kind| !matches!(kind.role, SourceRole::Other(_)))
}

/// The tracked TypeScript and JavaScript sources (git pathspecs): every language a
/// package's coverage and mutation testing could reach.
#[must_use]
pub fn ts_sources() -> Vec<&'static str> {
    sources_where(|kind| matches!(kind.role, SourceRole::Script { .. }))
}

/// The tracked sources Stryker mutates (git pathspecs): those of [`ts_sources`] but the
/// Astro templates, where a comment disabling a mutant can be written.
#[must_use]
pub fn stryker_sources() -> Vec<&'static str> {
    sources_where(|kind| kind.role == MUTATED)
}

/// How the gates of prose read the file at `path`: the reading of the first kind of
/// [`SOURCE_KINDS`] its path matches, [`Reading::Nothing`] for a file of no kind (which
/// [`source_kinds_gate`] refuses).
#[must_use]
pub fn reading_of(path: &str) -> Reading {
    SOURCE_KINDS
        .into_iter()
        .find(|(pattern, ..)| pathspec_matches(pattern, path))
        .map_or(Reading::Nothing, |(_, reading, _)| reading)
}

/// Whether `path` matches the git pathspec `pattern`, written with one `*` at most, which
/// stands for any text, `/` included (git's default).
pub(crate) fn pathspec_matches(pattern: &str, path: &str) -> bool {
    pattern
        .split_once('*')
        .map_or(path == pattern, |(prefix, suffix)| {
            path.len() >= prefix.len().saturating_add(suffix.len())
                && path.starts_with(prefix)
                && path.ends_with(suffix)
        })
}

/// Checks that every tracked text file of `paths` is of a known kind ([`SOURCE_KINDS`]):
/// code, or another kind with why no gate of code reads it.
///
/// # Errors
///
/// Returns every file of neither kind.
pub fn source_kinds_gate(paths: &[String]) -> Result<(), Failure> {
    let problems = paths
        .iter()
        .filter(|path| {
            !SOURCE_KINDS
                .iter()
                .any(|(pattern, ..)| pathspec_matches(pattern, path))
        })
        .map(|path| {
            format!(
                "{path}: a tracked text file of no known kind: add its pattern to SOURCE_KINDS, \
                 with what of it is prose and whether it is code, or why no gate of code reads \
                 it (crates/xtask/src/workspace.rs)"
            )
        })
        .collect();
    step_verdict(problems)
}

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
    /// vectors, the conformance corpus…), and the files of its own that are not Rust a
    /// build of its tests loads, as the loaded-files gate asks (`audio-wasm`'s
    /// `ts/blocks.ts`), each a file or a directory ending in `/`,
    /// checked by [`test_data_gate`](crate::test_code::test_data_gate): a change to one of them can weaken its tests as much
    /// as a change to its test code. A directory covers what is below it, another crate's
    /// test data included: `conformance/`, declared by the audio command line and its
    /// floating-point crate, holds `conformance/visual/`, which they do not read, so a
    /// visual case changed mutates them too (accepted for now: narrowing a root to what a
    /// crate reads is roadmap #24 step 102).
    pub test_data: &'static [&'static str],
    /// What the rules of the checks take it for ([`CrateRole`]): the one place a crate's
    /// role is written, every list of crates of one role derived from it
    /// ([`crates_in`]).
    pub roles: &'static [CrateRole],
    /// The Cargo workspace it belongs to: the root's, or its own ([`CargoWorkspace`]).
    pub workspace: CargoWorkspace,
}

/// A Cargo workspace of the repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CargoWorkspace {
    /// The root workspace (`Cargo.toml`): every crate but the GPU renderer.
    Root,
    /// The GPU renderer's own workspace ([`GPU_WORKSPACE`]): wgpu's tree turns on features
    /// of crates the engine shares (libm's `arch`), which one workspace would unify into
    /// every build of the engine, and holds licenses the root's policy does not allow
    /// (`crates/visual-gpu/Cargo.toml` says why; its configuration is the root's, checked).
    Gpu,
}

/// The directory of the GPU renderer's own workspace, as a literal: every path of that
/// workspace is built from it (`concat!`), so that none is typed twice.
macro_rules! gpu_workspace {
    () => {
        "crates/visual-gpu"
    };
}
pub(crate) use gpu_workspace;

/// The directory of the GPU renderer's own workspace, [`CargoWorkspace::Gpu`].
pub const GPU_WORKSPACE: &str = gpu_workspace!();

/// Where cargo-mutants writes the report of the GPU workspace's run.
///
/// Below [`MUTANTS_OUT`], so that emptying it empties both; its unviable mutants are then added to
/// [`UNVIABLE_REPORT`], their paths from the root (`actions::gpu_mutants`).
pub const GPU_MUTANTS_OUT: &str = "mutants.out/gpu";

/// The report cargo-mutants writes in [`GPU_MUTANTS_OUT`]: its `mutants.out` there.
pub const GPU_MUTANTS_REPORT: &str = "mutants.out/gpu/mutants.out";

impl CargoWorkspace {
    /// Every workspace, the root first.
    pub const ALL: [Self; 2] = [Self::Root, Self::Gpu];

    /// The arguments that make a cargo command started at the root work in it: none for
    /// the root, its manifest for another.
    #[must_use]
    pub const fn manifest_args(self) -> &'static [&'static str] {
        match self {
            Self::Root => &[],
            Self::Gpu => &["--manifest-path", crate::preflight::GPU_MANIFEST],
        }
    }

    /// The report of its mutation runs: the directory of their `missed.txt` and of the
    /// log of their unmutated baseline (`log/baseline.log`).
    #[must_use]
    pub const fn mutants_report(self) -> &'static str {
        match self {
            Self::Root => MUTANTS_OUT,
            Self::Gpu => GPU_MUTANTS_REPORT,
        }
    }

    /// Its nextest configuration, at its root, where nextest reads it: the slow group of
    /// its tests, run last in every mutant run ([`slow_group`](crate::test_order::slow_group)).
    #[must_use]
    pub const fn nextest_config(self) -> &'static str {
        match self {
            Self::Root => ".config/nextest.toml",
            Self::Gpu => concat!(gpu_workspace!(), "/.config/nextest.toml"),
        }
    }

    /// Its directory, from the root: `.` for the root itself.
    #[must_use]
    pub const fn dir(self) -> &'static str {
        match self {
            Self::Root => ".",
            Self::Gpu => GPU_WORKSPACE,
        }
    }

    /// What the name of a step working in it ends with: nothing for the root.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Root => "",
            Self::Gpu => " (GPU workspace)",
        }
    }
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
    /// A crate of the visual library: a sibling library standing alone the same way as the
    /// audio library (`spec/engine-design.md`, decision 32; `spec/visual-design.md`,
    /// section 1; [`library_gate`](crate::dependencies::library_gate)).
    VisualLibrary,
    /// A crate the tests of the visual library may use beside it (a development
    /// dependency), never its code ([`library_gate`](crate::dependencies::library_gate)).
    VisualLibraryTests,
    /// A tool of the visual library, its command line: it depends, within the project,
    /// only on the visual library as the library's crates do, so that it stands alone
    /// with it, but it is not the library: it is not shipped, and the gate pins no
    /// dependency for that role ([`library_gate`](crate::dependencies::library_gate)): its
    /// case reader is pinned for its role of [`CrateRole::CaseReader`].
    VisualTool,
    /// A command-line tool reading conformance cases: its normal and build dependencies
    /// read the values it renders and hashes, so [`pin_gate`](crate::dependencies::pin_gate)
    /// requires each to be reviewed as bit-defining and pinned, as a library's.
    CaseReader,
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
pub const RUST_CRATES: [RustCrate; 22] = [
    RustCrate {
        name: "audio-core",
        dir: "crates/audio-core",
        binary: false,
        test_data: &["conformance/test-matrix.json"],
        roles: &[CrateRole::Library, CrateRole::Libm, CrateRole::MathLayer],
        workspace: CargoWorkspace::Root,
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
        workspace: CargoWorkspace::Root,
    },
    RustCrate {
        name: "audio-patch",
        dir: "crates/audio-patch",
        binary: false,
        test_data: &["spec/safe-message-vectors.json"],
        roles: &[CrateRole::Library],
        workspace: CargoWorkspace::Root,
    },
    RustCrate {
        name: "audio-hash",
        dir: "crates/audio-hash",
        binary: false,
        test_data: &["spec/render-hash-vectors.json"],
        roles: &[CrateRole::Library],
        workspace: CargoWorkspace::Root,
    },
    RustCrate {
        name: "audio-host",
        dir: "crates/audio-host",
        binary: false,
        test_data: &[
            "conformance/cases/",
            "conformance/patches/",
            "conformance/live-limiter.json",
            "conformance/live-references.json",
            "conformance/invalid/live/",
        ],
        roles: &[CrateRole::Library],
        workspace: CargoWorkspace::Root,
    },
    RustCrate {
        name: "visual-core",
        dir: "crates/visual-core",
        binary: false,
        test_data: &[],
        roles: &[CrateRole::VisualLibrary],
        workspace: CargoWorkspace::Root,
    },
    RustCrate {
        name: "visual-boxes",
        dir: "crates/visual-boxes",
        binary: false,
        test_data: &[],
        roles: &[CrateRole::VisualLibrary],
        workspace: CargoWorkspace::Root,
    },
    RustCrate {
        name: "visual-hash",
        dir: "crates/visual-hash",
        binary: false,
        test_data: &["spec/image-hash-vectors.json"],
        roles: &[CrateRole::VisualLibrary],
        workspace: CargoWorkspace::Root,
    },
    RustCrate {
        name: "visual-testkit",
        dir: "crates/visual-testkit",
        binary: false,
        test_data: &["conformance/visual/test-matrix.json"],
        roles: &[CrateRole::VisualLibraryTests, CrateRole::SharedTests],
        workspace: CargoWorkspace::Root,
    },
    RustCrate {
        name: "visual-cli",
        dir: "crates/visual-cli",
        binary: true,
        test_data: &["conformance/visual/", "spec/safe-message-vectors.json"],
        roles: &[CrateRole::VisualTool, CrateRole::CaseReader],
        workspace: CargoWorkspace::Root,
    },
    RustCrate {
        name: "visual-host",
        dir: "crates/visual-host",
        binary: false,
        test_data: &[],
        roles: &[CrateRole::VisualLibrary],
        workspace: CargoWorkspace::Root,
    },
    RustCrate {
        name: "visual-gpu",
        dir: GPU_WORKSPACE,
        binary: true,
        test_data: &[
            "bindings/visual-wasm/benchmarks/suite.ts",
            "conformance/visual/display.json",
            "spec/benchmarks.md",
            "spec/gpu-time-baseline.txt",
        ],
        roles: &[CrateRole::VisualLibrary],
        workspace: CargoWorkspace::Gpu,
    },
    RustCrate {
        name: "visual-wasm",
        dir: "bindings/visual-wasm",
        binary: false,
        test_data: &[],
        roles: &[CrateRole::VisualLibrary],
        workspace: CargoWorkspace::Root,
    },
    RustCrate {
        name: "alloc-counter",
        dir: "crates/alloc-counter",
        binary: false,
        test_data: &[],
        roles: &[
            CrateRole::SharedTests,
            CrateRole::LibraryTests,
            CrateRole::VisualLibraryTests,
        ],
        workspace: CargoWorkspace::Root,
    },
    RustCrate {
        name: "fpenv",
        dir: "crates/fpenv",
        binary: false,
        test_data: &[],
        roles: &[
            CrateRole::SharedTests,
            CrateRole::LibraryTests,
            CrateRole::VisualLibraryTests,
        ],
        workspace: CargoWorkspace::Root,
    },
    RustCrate {
        name: "audio-battery",
        dir: "crates/audio-battery",
        binary: false,
        test_data: &["conformance/test-matrix.json"],
        roles: &[CrateRole::SharedTests, CrateRole::LibraryTests],
        workspace: CargoWorkspace::Root,
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
        workspace: CargoWorkspace::Root,
    },
    RustCrate {
        name: "audio-cli",
        dir: "crates/audio-cli",
        binary: true,
        test_data: &["conformance/"],
        roles: &[],
        workspace: CargoWorkspace::Root,
    },
    RustCrate {
        name: "audio-wasm",
        dir: "bindings/audio-wasm",
        binary: false,
        test_data: &["bindings/audio-wasm/ts/blocks.ts"],
        roles: &[CrateRole::Library],
        workspace: CargoWorkspace::Root,
    },
    RustCrate {
        name: "coordinator",
        dir: "crates/coordinator",
        binary: false,
        test_data: &[],
        roles: &[],
        workspace: CargoWorkspace::Root,
    },
    RustCrate {
        name: "web-page",
        dir: "bindings/web-page",
        binary: false,
        test_data: &[],
        roles: &[],
        workspace: CargoWorkspace::Root,
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
        workspace: CargoWorkspace::Root,
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
    workspace: CargoWorkspace::Root,
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

    /// The command emptying the coverage build of its crate's workspace first. The measured
    /// crates of a workspace share one build directory: an object left there by an earlier
    /// measurement (the same crate compiled as a dependency of another) would enter the
    /// report as a second copy of its code, never run, and halve its coverage; each
    /// measurement starts from nothing.
    #[must_use]
    pub fn clean_command(self) -> Cmd {
        Cmd::new(
            "cargo",
            &[
                &[NIGHTLY_TOOLCHAIN, "llvm-cov", "clean", "--workspace"],
                self.krate.workspace.manifest_args(),
            ]
            .concat(),
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

/// Where the changes whose Rust mutants are tested are written, for `cargo mutants --in-diff`.
///
/// The default check writes the `git diff` against the base of [`MUTATION_BASES`];
/// `cargo xtask mutants --changed <revision>` writes the one against the merge base of `HEAD`
/// with the revision named. New Rust files are included in both.
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
    /// The name of its checks: each of their steps is named `<step>: <script>`.
    pub step: &'static str,
    /// The scripts its `check` script runs, in order (`pnpm <script> && …`): each a step of
    /// its own, run in its directory, so that each says its time; the package check gate
    /// ([`package_check_gate`](crate::reviews::package_check_gate)) ties them to its manifest.
    /// In `--full`, which runs every step whatever fails, a failed script does not stop the
    /// later ones, as `&&` would: more failures for one cause, the same verdict.
    pub checks: &'static [&'static str],
    /// Whether what it ships reaches users, so that its production dependencies must be
    /// under [`SHIPPED_LICENSES`](crate::licenses::SHIPPED_LICENSES).
    pub shipped: bool,
    /// Its build output directories that its build does not empty itself (`tsc` never
    /// removes an output whose source is gone), relative to its directory: emptied before
    /// the checks, so that no stale output of an earlier run is used by a later step.
    pub outputs: &'static [&'static str],
    /// What its checks need besides the JavaScript dependencies, which every package needs:
    /// prepared again in its part of the full check ([`Part::needs`](crate::parts::Part::needs)), since its checks may
    /// run there without the others. Among them, the builds of the workspace packages it uses,
    /// directly or not, that have build outputs, in the order of [`PACKAGES`], exactly: the
    /// package order gate ([`package_order_gate`](crate::reviews::package_order_gate)) derives
    /// them from the manifests.
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

/// The TypeScript packages, in the order their checks run: each after every workspace
/// package it depends on (`@poyetik/*` in its manifest), as the package order gate
/// ([`package_order_gate`](crate::reviews::package_order_gate)) requires.
///
/// The shared policy, the oracle, the shared build of the WebAssembly modules, the engine in
/// the browser, the visual library's own module and the page module around it, tested in
/// Node (the product never loads two library modules side by side, but one module per
/// browser thread, decision 33), the playground built on both, then the roadmap. Each writes
/// its coverage summary and its mutation report, both checked by xtask rather than trusted
/// to their own thresholds.
pub const PACKAGES: [Package; 8] = [
    Package {
        dir: "tooling/config",
        step: "Shared TypeScript policy (each rule proven to reject what it forbids)",
        checks: &["typecheck", "lint", "test"],
        shipped: false,
        outputs: &[],
        needs: &[],
    },
    Package {
        dir: "tooling/oracle",
        step: "Conformance oracle (independent, TypeScript)",
        checks: &["typecheck", "lint", "test", "oracle"],
        shipped: false,
        outputs: &[],
        needs: &[Setup::ConformanceExport],
    },
    Package {
        dir: "tooling/wasm-build",
        step: "The tool that builds the WebAssembly modules reproducibly (every \
               WebAssembly package's)",
        checks: &["typecheck", "lint", "test"],
        shipped: false,
        outputs: &[],
        needs: &[],
    },
    Package {
        dir: "bindings/audio-wasm",
        step: "Engine WASM checks",
        checks: &[
            "build",
            "typecheck",
            "lint",
            "test",
            "benchmarks:check",
            "test:allocation",
            "test:reproducible",
            "test:e2e",
        ],
        shipped: true,
        outputs: &["dist"],
        needs: &[],
    },
    Package {
        dir: "bindings/visual-wasm",
        step: "Visual WASM checks (the same image hashes as the native renderer)",
        checks: &[
            "build",
            "typecheck",
            "lint",
            "test",
            "test:reproducible",
            "benchmarks:check",
        ],
        shipped: true,
        outputs: &["dist"],
        needs: &[],
    },
    Package {
        dir: "bindings/web-page",
        step: "Page module checks (the visual library and the coordinating layer in one module)",
        checks: &["build", "typecheck", "lint", "test", "test:reproducible"],
        shipped: true,
        outputs: &["dist"],
        needs: &[Setup::Build("bindings/visual-wasm")],
    },
    Package {
        dir: "apps/playground",
        step: "Playground checks",
        checks: &["typecheck", "lint", "test", "build", "test:e2e"],
        shipped: true,
        outputs: &[],
        needs: &[
            Setup::Build("bindings/audio-wasm"),
            Setup::Build("bindings/visual-wasm"),
            Setup::Build("bindings/web-page"),
        ],
    },
    Package {
        dir: "apps/roadmap",
        step: "Roadmap checks",
        checks: &["typecheck", "lint", "test", "build", "test:e2e"],
        shipped: true,
        outputs: &[],
        needs: &[],
    },
];

/// The packages of [`PACKAGES`] whose build another package needs ([`Setup::Build`]), in
/// that order: each built before the packages that use it, and after the builds it uses.
#[must_use]
pub fn needed_builds() -> Vec<Package> {
    PACKAGES
        .into_iter()
        .filter(|built| {
            PACKAGES
                .iter()
                .any(|package| package.needs.contains(&Setup::Build(built.dir)))
        })
        .collect()
}

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

/// The WebAssembly target: that of the libraries on the web, [`WASM_PACKAGES`].
pub const WASM_TARGET: &str = "wasm32-unknown-unknown";

/// A library of the project, standing alone (`spec/engine-design.md`, decision 32).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Library {
    /// The role of its crates.
    pub crates: CrateRole,
    /// The role of the crates its tests may also use.
    pub tests: CrateRole,
    /// The role of its tools, held to the same dependencies within the project as its crates
    /// (a tool's third-party dependencies are pinned only for a role of their own, as
    /// [`CrateRole::CaseReader`]), if it has any.
    pub tools: Option<CrateRole>,
    /// Its name.
    pub name: &'static str,
}

/// The libraries of the project, each standing alone (`spec/engine-design.md`, decision
/// 32).
pub const LIBRARIES: [Library; 2] = [
    Library {
        crates: CrateRole::Library,
        tests: CrateRole::LibraryTests,
        tools: None,
        name: "audio library",
    },
    Library {
        crates: CrateRole::VisualLibrary,
        tests: CrateRole::VisualLibraryTests,
        tools: Some(CrateRole::VisualTool),
        name: "visual library",
    },
];

/// Targets the crates of both libraries must build for ([`PORTABILITY_PACKAGES`], their
/// shells apart): proof that they depend on no environment.
pub const PORTABILITY_TARGETS: [&str; 4] = [
    "aarch64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    WASM_TARGET,
    "thumbv7em-none-eabihf",
];

/// The crates built for every one of [`PORTABILITY_TARGETS`].
///
/// The engine core, the visual library's core, and the coordinating layer above both,
/// linked into every product. Every `no_std` crate is among them, or says why it is not
/// ([`NO_STD_NOT_PORTABLE`], [`no_std_portability_gate`]).
pub const PORTABILITY_PACKAGES: [&str; 10] = [
    "audio-core",
    "audio-boxes",
    "audio-patch",
    "audio-hash",
    "audio-host",
    "visual-core",
    "visual-boxes",
    "visual-hash",
    "visual-host",
    "coordinator",
];

/// The `no_std` crates not built for every one of [`PORTABILITY_TARGETS`], each with why
/// ([`no_std_portability_gate`]); none is one of [`PORTABILITY_PACKAGES`].
pub const NO_STD_NOT_PORTABLE: [(&str, &str); 1] = [(
    "fpenv",
    "the default floating-point mode of the native calls: its code is written per \
     architecture, built for each of NATIVE_TARGETS",
)];

/// The attribute that makes a crate `no_std`, as its root's leading inner attributes hold it.
const NO_STD: &str = "#![no_std]";

/// The crates built for WebAssembly, the engine's, the visual library's and the page
/// module's, which their packages build for [`WASM_TARGET`].
pub const WASM_PACKAGES: [&str; 3] = ["audio-wasm", "visual-wasm", "web-page"];

/// The native shell and the floating-point mode it uses, whose code differs by
/// architecture: built for each of [`NATIVE_TARGETS`].
pub const NATIVE_PACKAGES: [&str; 2] = ["fpenv", "audio-cli"];

/// Cargo's arguments selecting `packages`: `--package <name>` for each, in order.
pub(crate) fn package_args<'name>(packages: &[&'name str]) -> Vec<&'name str> {
    packages
        .iter()
        .flat_map(|&package| ["--package", package])
        .collect()
}

/// Targets of the native shell (`audio-cli`) and of its floating-point mode
/// (`fpenv`), whose code differs by architecture.
///
/// Each is linted for both, whatever the machine, so that CI (`x86_64` Linux) compiles the
/// aarch64 code and a Mac the `x86_64` code.
pub const NATIVE_TARGETS: [&str; 2] = ["aarch64-apple-darwin", "x86_64-unknown-linux-gnu"];

/// The lints every root of both libraries denies among its leading inner attributes
/// ([`library_lints_gate`]).
///
/// The libraries are the crates holding [`CrateRole::Library`] or
/// [`CrateRole::VisualLibrary`]: a public enum of them, and a public struct of public
/// fields, is non-exhaustive, or says where it is declared why it is closed.
pub const LIBRARY_LINTS: [&str; 2] = [
    "#![deny(clippy::exhaustive_enums)]",
    "#![deny(clippy::exhaustive_structs)]",
];

/// Checks that the root of every crate of both libraries, the audio and the visual one, holds
/// each of [`LIBRARY_LINTS`] among its leading inner attributes.
///
/// So that what the changelog and the crates' docs claim of each whole library, its enums
/// and structs non-exhaustive, holds of a crate added to it too; read as the tokens of a real
/// leading inner attribute (`RustSource::leading_attribute`), which the compiler applies to the
/// whole crate.
///
/// # Errors
///
/// Returns every root of the library that is missing among `parsed`, and every lint a root
/// does not hold.
pub fn library_lints_gate(parsed: &[RustSource]) -> Result<(), Failure> {
    let mut problems = Vec::new();
    for krate in RUST_CRATES.iter().filter(|krate| {
        krate.roles.contains(&CrateRole::Library) || krate.roles.contains(&CrateRole::VisualLibrary)
    }) {
        let root = format!("{}/src/lib.rs", krate.dir);
        let Some(source) = parsed.iter().find(|source| source.path == root) else {
            problems.push(format!("{root}: the root of {} is not read", krate.name));
            continue;
        };
        problems.extend(
            LIBRARY_LINTS
                .iter()
                .filter(|lint| source.leading_attribute(lint).is_none())
                .map(|lint| {
                    format!(
                        "{root} holds no leading attribute exactly `{lint}` (one lint per \
                         attribute)"
                    )
                }),
        );
    }
    verdict("the lints of the libraries", problems)
}

/// Checks that every `no_std` crate is built for every portability target, or says why not.
///
/// Every crate whose root is `no_std` is one of [`PORTABILITY_PACKAGES`], built for every
/// one of [`PORTABILITY_TARGETS`], or one of [`NO_STD_NOT_PORTABLE`], each of which is still
/// `no_std`.
///
/// So that a crate claiming no environment, as the coordinating layer linked into every
/// product does, proves it on the bare-metal target too: a dependency needing `std` would
/// otherwise build natively and for WebAssembly, unseen until an embedded export. Read as
/// the tokens of a real leading inner attribute (`RustSource::leading_attribute`).
///
/// # Errors
///
/// Returns every `no_std` crate built for fewer targets and not listed, and every listed
/// crate whose root is not read or is no longer `no_std` (a listed crate is never one of
/// [`PORTABILITY_PACKAGES`], which a test holds).
pub fn no_std_portability_gate(parsed: &[RustSource]) -> Result<(), Failure> {
    let no_std = |root: &str| {
        parsed
            .iter()
            .find(|source| source.path == root)
            .map(|source| source.leading_attribute(NO_STD).is_some())
    };
    let roots: Vec<String> = RUST_CRATES
        .iter()
        .map(|krate| format!("{}/src/lib.rs", krate.dir))
        .collect();
    let mut problems = Vec::new();
    // `no_std` in its one form, at a crate's root only: any other spelling (a conditional
    // one), or a root the gate does not read, would pass as `std` unseen.
    for source in parsed {
        let mut declared = source.leading_attributes().into_iter().filter(|range| {
            source
                .tokens
                .get(range.clone())
                .unwrap_or_default()
                .iter()
                .any(|token| token.is_ident("no_std"))
        });
        if declared.clone().any(|range| {
            !same_tokens(
                source.tokens.get(range).unwrap_or_default(),
                &tokens_of(NO_STD),
            )
        }) {
            problems.push(format!(
                "{} declares `no_std` in another form than `{NO_STD}`, the one form this gate \
                 reads",
                source.path
            ));
        } else if declared.next().is_some() && !roots.contains(&source.path) {
            problems.push(format!(
                "{} declares `no_std` but is not the root `src/lib.rs` of a crate of \
                 RUST_CRATES, the one root this gate reads",
                source.path
            ));
        }
    }
    for (krate, root) in RUST_CRATES.iter().zip(roots.iter()) {
        let listed = NO_STD_NOT_PORTABLE
            .iter()
            .any(|(name, _)| *name == krate.name);
        let portable = PORTABILITY_PACKAGES.contains(&krate.name);
        match (no_std(root), listed) {
            (Some(true), false) if !portable => problems.push(format!(
                "{root} is `no_std` but {} is not built for every portability target \
                 (PORTABILITY_PACKAGES)",
                krate.name
            )),
            (None, true) => problems.push(format!(
                "{root}: the root of {}, listed in NO_STD_NOT_PORTABLE, is not read",
                krate.name
            )),
            (Some(false), true) => problems.push(format!(
                "{} is listed in NO_STD_NOT_PORTABLE but its root is not `no_std`",
                krate.name
            )),
            _ => {}
        }
    }
    verdict("the portability of the `no_std` crates", problems)
}

#[cfg(test)]
#[path = "workspace_tests.rs"]
mod tests;
