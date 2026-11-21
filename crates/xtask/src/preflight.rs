//! The pinned tools and the environment the check refuses, checked before any step.

use alloc::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::path::PathBuf;

use crate::{
    dependencies::CARGO_CONFIG,
    release::Version,
    runner::CommandFailure,
    steps::{Cmd, Dir, Failure, verdict},
    workflow::{NIGHTLY_COMPONENTS, NODE_VERSION_FILE},
    workspace::NIGHTLY_TOOLCHAIN,
};

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
            usage: false,
        })
    }
}

/// The tables of the root manifest the GPU workspace's manifest holds exactly: each from
/// its header to the header that follows it in the root's (the package fields its crates
/// inherit, then every lint).
pub const SHARED_MANIFEST_TABLES: [(&str, &str); 2] = [
    ("[workspace.package]", "[workspace.dependencies]"),
    ("[workspace.lints.rust]", "[profile.release]"),
];

/// The sections of the GPU workspace's manifest, exactly, in this order: its workspace, the
/// root's tables ([`SHARED_MANIFEST_TABLES`]), then its one crate.
pub(crate) const GPU_MANIFEST_SECTIONS: [&str; 9] = [
    "[workspace]",
    "[workspace.package]",
    "[workspace.lints.rust]",
    "[workspace.lints.rustdoc]",
    "[workspace.lints.clippy]",
    "[package]",
    "[dependencies]",
    "[dev-dependencies]",
    "[lints]",
];

/// The body of the GPU manifest's `[workspace]` table, exactly: its one crate.
const GPU_WORKSPACE_TABLE: &str = "resolver = \"3\"\nmembers = [\".\"]\n\n";

/// The body of the GPU crate's `[lints]` table, exactly: the workspace's.
const GPU_LINTS_TABLE: &str = "workspace = true\n";

/// The text of `manifest` from the line `start` to the line `end`, or to its end with no
/// `end`: `start`'s line included, `end`'s not.
fn table<'text>(manifest: &'text str, start: &str, end: Option<&str>) -> Option<&'text str> {
    let (_, from) = manifest.split_once(&format!("{start}\n"))?;
    let body = match end {
        Some(end) => from.split_once(&format!("{end}\n"))?.0,
        None => from,
    };
    Some(body)
}

/// Checks that the GPU workspace's manifest shares the root's settings and holds nothing
/// else but its one crate.
///
/// The GPU workspace's manifest (`gpu`) holds exactly its sections (`GPU_MANIFEST_SECTIONS`), a header
/// being any line whose text starts with `[`, and nothing but comments and blank lines
/// before the first (a key there would be a top-level key, such as a dotted
/// `patch.crates-io.<crate>`); from `[workspace.package]` to
/// `[workspace.lints.rust]`, and from there to `[package]`, exactly the root's
/// [`SHARED_MANIFEST_TABLES`] (`root`), so that its crate shares the root's version,
/// edition, license and lints, with no lint added after them; and its `[workspace]` and
/// `[lints]` tables exactly its one crate and the workspace's lints. Its crate's fields
/// and dependencies are its own.
///
/// # Errors
///
/// Returns each table the root does not hold as expected, and each way the GPU manifest
/// departs from that form.
pub fn gpu_manifest_gate(root: &str, gpu: &str) -> Result<(), Failure> {
    let mut problems: Vec<String> = Vec::new();
    let headers: Vec<&str> = gpu
        .lines()
        .filter(|line| line.trim_start().starts_with('['))
        .collect();
    if headers != GPU_MANIFEST_SECTIONS {
        problems.push(format!(
            "{GPU_MANIFEST} holds other sections than {}, in that order",
            GPU_MANIFEST_SECTIONS.join(", ")
        ));
    }
    // Keys before the first table are top-level keys, which no table above reads: a dotted
    // one there (`patch.crates-io.wgpu = …`, `profile.test.overflow-checks = false`) would
    // patch a dependency or set a profile unseen.
    let opening = gpu
        .lines()
        .take_while(|line| !line.trim_start().starts_with('['))
        .map(str::trim);
    if opening
        .clone()
        .any(|line| !line.is_empty() && !line.starts_with('#'))
    {
        problems.push(format!(
            "{GPU_MANIFEST} holds more than comments and blank lines before its first table, \
             {}",
            GPU_MANIFEST_SECTIONS[0]
        ));
    }
    for (&(start, end), next) in SHARED_MANIFEST_TABLES
        .iter()
        .zip(["[workspace.lints.rust]", "[package]"])
    {
        match table(root, start, Some(end)) {
            None => problems.push(format!("Cargo.toml has no {start} table followed by {end}")),
            Some(shared) if table(gpu, start, Some(next)) == Some(shared) => {}
            Some(_) => problems.push(format!(
                "{GPU_MANIFEST} does not hold the root's {start} table exactly"
            )),
        }
    }
    if table(gpu, "[workspace]", Some("[workspace.package]")) != Some(GPU_WORKSPACE_TABLE) {
        problems.push(format!(
            "{GPU_MANIFEST}: its [workspace] table is not exactly `resolver = \"3\"` and \
             `members = [\".\"]`"
        ));
    }
    if table(gpu, "[lints]", None) != Some(GPU_LINTS_TABLE) {
        problems.push(format!(
            "{GPU_MANIFEST}: its [lints] table is not exactly `workspace = true`"
        ));
    }
    verdict(
        "the GPU workspace shares the root's package fields and lints",
        problems,
    )
}

/// The manifest of the GPU workspace.
pub const GPU_MANIFEST: &str = concat!(crate::workspace::gpu_workspace!(), "/Cargo.toml");

/// The Cargo tools the checks run, at the exact version they need (as `cargo install`
/// names them): the preflight refuses another version, and the CI workflow installs these.
pub const PINNED_TOOLS: [(&str, &str); 4] = [
    ("cargo-llvm-cov", "0.9.1"),
    ("cargo-mutants", "27.1.0"),
    ("cargo-deny", "0.20.2"),
    ("cargo-nextest", "0.9.152"),
];

/// What the pinned cargo-mutants ([`PINNED_TOOLS`], 27.1.0: `Mutant::mutated_code`) writes
/// after every span it mutates. Another version may write another: [`mutant_left_gate`]
/// reads this one, so a change of the pin checks it again.
pub(crate) const MUTANT_MARKER: &str = "/* ~ changed by cargo-mutants ~ */";

/// The search for [`MUTANT_MARKER`] in the Rust files of the GPU workspace, which its
/// mutation run mutates in place ([`gpu_mutants_command`](crate::gpu_mutants_command)),
/// tracked or not, its build directory apart: grep's lines `<file>:<line>:<text>`, and its
/// exit code 1 when it finds none. On the files themselves, not through git: cargo-mutants
/// copies no `.git` into the trees where it runs the tests of xtask.
pub(crate) fn mutant_search_command() -> Cmd {
    Cmd::new(
        "grep",
        &[
            "-rnF",
            "--include=*.rs",
            "--exclude-dir=target",
            "-e",
            MUTANT_MARKER,
            "--",
            crate::workspace::gpu_workspace!(),
        ],
        Dir::Root,
    )
}

/// Checks that `found`, the lines of [`mutant_search_command`], names no file: a mutation
/// run stopped before it restored its mutant (killed, out of memory, its terminal closed)
/// leaves it in place, where the next check would test it as the code. An honest edit never
/// writes the marker, so an uncommitted change passes.
///
/// # Errors
///
/// Returns every line holding the marker, with its file.
pub(crate) fn mutant_left_gate(found: &str) -> Result<(), Failure> {
    let problems = found
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let mut fields = line.splitn(3, ':');
            let place = match (fields.next(), fields.next()) {
                (Some(path), Some(number)) => format!("{path}:{number}"),
                _ => line.to_owned(),
            };
            format!("{place} holds cargo-mutants' marker")
        })
        .collect();
    verdict(
        "a mutant left in place by a stopped mutation run (restore each file named, with \
         `git checkout -- <file>` when it holds no change of yours)",
        problems,
    )
}

/// Which version of a tool the checks need.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// Any version: the tool only has to answer.
    Present,
    /// Any version, on Linux only, where the mutation tests of the checks and of
    /// `cargo xtask mutants` run it (their memory limit,
    /// [`MUTANT_TEST_RUNNER`](crate::MUTANT_TEST_RUNNER)): elsewhere its answer is not
    /// read. A bless, which mutates nothing, requires it too, as every tool of the
    /// preflight, which every command starting the tools of the checks runs whole.
    PresentOnLinux,
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
pub const TOOLS: [Tool; 12] = [
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
        install: Install::Nightly(&NIGHTLY_COMPONENTS),
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
        program: "cargo",
        args: &["nextest", "--version"],
        need: Need::Pinned(PINNED_TOOLS[3]),
        install: Install::Cargo(PINNED_TOOLS[3]),
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
        program: "prlimit",
        args: &["--version"],
        need: Need::PresentOnLinux,
        install: Install::Text(
            "install util-linux (its prlimit bounds the memory of each test program of the mutation tests)",
        ),
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
/// another target, profile or compiler wrapper, another `clippy.toml`, a compiler acting as
/// a nightly one, which changes what a dependency's build script enables). The doc tests
/// whose error codes are checked set [`BOOTSTRAP_VARIABLE`](crate::steps::BOOTSTRAP_VARIABLE)
/// for themselves alone.
pub const REFUSED_VARIABLES: [&str; 10] = [
    "RUSTFLAGS",
    "RUSTDOCFLAGS",
    "CARGO_ENCODED_RUSTFLAGS",
    "CARGO_ENCODED_RUSTDOCFLAGS",
    "RUSTC",
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "RUSTDOC",
    "CLIPPY_CONF_DIR",
    crate::steps::BOOTSTRAP_VARIABLE,
];

/// The prefixes of the variables the preflight refuses.
///
/// Those of Cargo's `build`, `target`, `profile` and `unstable` configuration
/// (`CARGO_TARGET_DIR` apart, which only moves the build), and nextest's own settings.
/// Nextest, the test runner of every mutant run, reads its profile, retries, threads,
/// handling of a run with no test and the form of its output from `NEXTEST_` variables:
/// set outside the repository, one would retry a flaky test until it passes, run another
/// profile than the repository's, or change the log the test order is read from.
pub const REFUSED_VARIABLE_PREFIXES: [&str; 5] = [
    "CARGO_BUILD_",
    "CARGO_TARGET_",
    "CARGO_PROFILE_",
    "CARGO_UNSTABLE_",
    "NEXTEST_",
];

/// The variables of the runtimes the checks run on besides cargo's, refused by the
/// preflight, each with its runtime.
///
/// Each reaches every TypeScript check and every process their tests start, so that a
/// local check would differ from CI: Node.js's options (a preloaded module, an inspector, a
/// flag), its mode (`NODE_ENV`) and where and how it finds modules (`NODE_PATH`,
/// `NODE_PRESERVE_SYMLINKS`); each with its runtime and what it changes. The runtimes
/// covered: Node.js, npm and pnpm ([`REFUSED_RUNTIME_PREFIXES`]); the Rust toolchain's
/// variables are [`REFUSED_VARIABLES`] and [`REFUSED_VARIABLE_PREFIXES`].
pub const REFUSED_RUNTIME_VARIABLES: [(&str, &str, &str); 4] = [
    (
        "NODE_OPTIONS",
        "Node.js",
        "a preloaded module, an inspector, another flag",
    ),
    (
        "NODE_ENV",
        "Node.js",
        "the mode tools and packages pick, production or development",
    ),
    (
        "NODE_PATH",
        "Node.js",
        "modules found outside the workspace's own",
    ),
    (
        "NODE_PRESERVE_SYMLINKS",
        "Node.js",
        "how the workspace's linked modules resolve",
    ),
];

/// The prefixes of the variables npm and pnpm read their settings from.
///
/// Refused by the preflight in any case, as npm reads them (`npm_config_registry`, `NPM_CONFIG_REGISTRY`),
/// each with its runtime and what it changes: the workspace's settings are its own
/// (`pnpm-workspace.yaml`, checked by the supply-chain gate).
pub const REFUSED_RUNTIME_PREFIXES: [(&str, &str, &str); 2] = [
    (
        "npm_config_",
        "npm",
        "a setting of every npm and pnpm command, read in any case",
    ),
    (
        "pnpm_config_",
        "pnpm",
        "a setting of every pnpm command, read in any case",
    ),
];

/// The runtime of `name` among [`REFUSED_RUNTIME_VARIABLES`] and
/// [`REFUSED_RUNTIME_PREFIXES`], with what it changes, if it is one of them.
fn refused_runtime(name: &str) -> Option<(&'static str, &'static str)> {
    REFUSED_RUNTIME_VARIABLES
        .iter()
        .find(|(variable, ..)| *variable == name)
        .or_else(|| {
            REFUSED_RUNTIME_PREFIXES.iter().find(|(prefix, ..)| {
                name.get(..prefix.len())
                    .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
            })
        })
        .map(|(_, runtime, what)| (*runtime, *what))
}

/// Whether the variable `name` is one the preflight refuses.
///
/// It refuses one of [`REFUSED_VARIABLES`] or [`REFUSED_RUNTIME_VARIABLES`], a name opening
/// with one of [`REFUSED_RUNTIME_PREFIXES`] in any case, a variable a crate reads at build time
/// (`BUILD_VARIABLES`: the engine block a build names, where the checks build every engine
/// block), or a name starting with one of [`REFUSED_VARIABLE_PREFIXES`] other than
/// `CARGO_TARGET_DIR`.
#[must_use]
pub fn refused_variable(name: &str) -> bool {
    REFUSED_VARIABLES.contains(&name)
        || refused_runtime(name).is_some()
        || crate::test_code::BUILD_VARIABLES.contains(&name)
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
    /// The npm and pnpm configuration files of the user and the machine that exist (see
    /// [`node_config_candidates`]).
    pub node_configs: Vec<String>,
    /// The directories the Vulkan loader searches for a driver, when none holds one and no
    /// variable names one (`vulkan_searched`): none when a driver is declared.
    pub vulkan_searched: Option<Vec<String>>,
}

/// Whether a preflight requires a Vulkan driver on Linux.
///
/// Every command's does, but a part of the full check's, derived from its steps
/// ([`preflighted`](crate::steps::preflighted)), so that a part that runs no code of the
/// GPU workspace starts in a CI job that installs no driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuDriver {
    /// One of its steps runs code of the GPU workspace, which renders through Vulkan on
    /// Linux ([`Action::runs_gpu_code`](crate::Action::runs_gpu_code)).
    Required,
    /// None of its steps does.
    Unneeded,
}

/// The variables that name Vulkan driver manifests to the loader directly, in place of its
/// directories or beside them.
pub(crate) const VULKAN_DRIVER_VARIABLES: [&str; 3] =
    ["VK_DRIVER_FILES", "VK_ICD_FILENAMES", "VK_ADD_DRIVER_FILES"];

/// What the GPU workspace's tests need on Linux, where they render through Vulkan.
const VULKAN_INSTALL: &str = "install a Vulkan driver, which the GPU workspace's tests render \
                              on (on Ubuntu, mesa-vulkan-drivers: lavapipe renders on the \
                              CPU), or name its manifest with VK_DRIVER_FILES if your \
                              loader searches other system directories than these";

/// The directories the Vulkan loader searches for driver manifests on Linux, in its order,
/// each `vulkan/icd.d` below: the XDG configuration home (`XDG_CONFIG_HOME`, else
/// `$HOME/.config`), the XDG configuration directories (`XDG_CONFIG_DIRS`, else
/// `/etc/xdg`), the system's (`/usr/local/etc`, `/etc`), the XDG data home
/// (`XDG_DATA_HOME`, else `$HOME/.local/share`) and the XDG data directories
/// (`XDG_DATA_DIRS`, else `/usr/local/share:/usr/share`); `variable` reads the
/// environment, an empty variable, a home or a list, being unset as XDG says.
///
/// The system directories are those of the upstream loader as Debian and Ubuntu build it:
/// a loader built with others (`SYSCONFDIR`, a distribution's own) is told its driver
/// through `VK_DRIVER_FILES`, which the refusal names.
pub(crate) fn vulkan_driver_dirs(variable: &dyn Fn(&str) -> Option<OsString>) -> Vec<PathBuf> {
    let home = variable("HOME").map(PathBuf::from);
    let one = |name: &str, below_home: &str| {
        variable(name)
            .filter(|set| !set.is_empty())
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|home| home.join(below_home)))
    };
    let many = |name: &str, default: &str| -> Vec<PathBuf> {
        let listed = variable(name)
            .filter(|listed| !listed.is_empty())
            .unwrap_or_else(|| OsString::from(default));
        std::env::split_paths(&listed).collect()
    };
    one("XDG_CONFIG_HOME", ".config")
        .into_iter()
        .chain(many("XDG_CONFIG_DIRS", "/etc/xdg"))
        .chain(["/usr/local/etc", "/etc"].map(PathBuf::from))
        .chain(one("XDG_DATA_HOME", ".local/share"))
        .chain(many("XDG_DATA_DIRS", "/usr/local/share:/usr/share"))
        .map(|dir| dir.join("vulkan/icd.d"))
        .collect()
}

/// Whether `dir` holds a driver manifest, a `.json` file: a link to one too, which the
/// loader follows as it opens each by its path, but neither a dangling link nor a
/// directory.
pub(crate) fn holds_a_manifest(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            let path = entry.path();
            path.extension()
                .is_some_and(|extension| extension == "json")
                && path.is_file()
        })
    })
}

/// The directories the Vulkan loader searches ([`vulkan_driver_dirs`]), as text, when none
/// holds a manifest (`holds`, [`holds_a_manifest`] on the machine) and none of
/// [`VULKAN_DRIVER_VARIABLES`] names one; none when a driver is declared.
pub(crate) fn vulkan_searched(
    variable: &dyn Fn(&str) -> Option<OsString>,
    holds: &dyn Fn(&Path) -> bool,
) -> Option<Vec<String>> {
    let named = VULKAN_DRIVER_VARIABLES
        .iter()
        .any(|name| variable(name).is_some_and(|value| !value.is_empty()));
    let dirs = vulkan_driver_dirs(variable);
    (!named && !dirs.iter().any(|dir| holds(dir)))
        .then(|| dirs.iter().map(|dir| dir.display().to_string()).collect())
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

/// Every npm or pnpm configuration file of the user or the machine that npm or pnpm would
/// read for a command of the checks, sorted.
///
/// The repository's own settings are
/// `pnpm-workspace.yaml`, and a `.npmrc` in it is refused by the supply-chain gate.
///
/// The user's `.npmrc` in `home`; pnpm's `rc` below `xdg_config_home` (else `.config` in
/// `home`) and in `Library/Preferences` of `home` (pnpm's on macOS); and npm's global
/// `etc/npmrc` beside each directory of programs of `path` (npm's prefix is the directory
/// above the one holding Node.js).
#[must_use]
pub fn node_config_candidates(
    home: Option<OsString>,
    xdg_config_home: Option<OsString>,
    path: Option<&OsStr>,
) -> Vec<PathBuf> {
    let home = home.map(PathBuf::from);
    let pnpm_config = xdg_config_home
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|home| home.join(".config")));
    let user = home.iter().flat_map(|home| {
        [
            home.join(".npmrc"),
            home.join("Library/Preferences/pnpm/rc"),
        ]
    });
    let global = path
        .iter()
        .flat_map(std::env::split_paths)
        .filter_map(|dir| dir.parent().map(|prefix| prefix.join("etc/npmrc")))
        .collect::<Vec<_>>();
    user.chain(pnpm_config.map(|dir| dir.join("pnpm/rc")))
        .chain(global)
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
pub(crate) fn supported(release: Version) -> bool {
    (release.major, release.minor) >= NODE_FLOOR
}

/// The oldest Node.js release the packages support, major and minor: the `engines` of
/// every `package.json` is exactly `>=` this release (checked by [`release_gate`](crate::release::release_gate)).
pub const NODE_FLOOR: (u64, u64) = (22, 18);

/// Checks the platform and the tools the checks need.
///
/// `answers` holds what each of [`TOOLS`] answered: its version line, or why asking failed
/// (a missing program, a missing cargo subcommand or toolchain, a broken install). Every
/// tool must answer (`prlimit` on Linux only, [`Need::PresentOnLinux`]), the Cargo tools at
/// their pinned version and Node.js at exactly the release of `node_version` (the text of
/// [`NODE_VERSION_FILE`], which CI installs, so that a local check differs from CI only by
/// platform), and the Rust compiler at exactly the release of `toolchain` (the text of
/// [`RUST_TOOLCHAIN_FILE`]: an override, a `+toolchain` or `RUSTUP_TOOLCHAIN` would replace
/// it), on a platform the checks support (macOS or Linux; Windows through WSL2), in an
/// `environment` that sets none of the variables of [`refused_variable`] and has no Cargo
/// configuration outside the repository's own; and, on Linux when `driver` is
/// [`GpuDriver::Required`], with a Vulkan driver declared to the loader.
///
/// # Errors
///
/// Returns one failure listing every problem found, each with how to fix it: an
/// unsupported platform, a tool that did not answer (with its own reason), or a tool at a
/// version the checks do not accept (Node.js included, which must be `node_version`, and
/// the Rust compiler, which must be the channel of `toolchain`), a refused variable, a
/// Cargo configuration file outside the repository, or a Vulkan driver required and not
/// found.
pub fn preflight_gate(
    os: &str,
    answers: &[Result<String, CommandFailure>],
    node_version: &str,
    toolchain: &str,
    environment: &Environment,
    driver: GpuDriver,
) -> Result<(), Failure> {
    let mut problems = Vec::new();
    if os == "windows" {
        problems.push("the checks run on macOS and Linux: on Windows, run them in WSL2".to_owned());
    }
    for (tool, answer) in TOOLS.iter().zip(answers) {
        if tool.need == Need::PresentOnLinux && os != "linux" {
            continue;
        }
        let install = tool.install;
        let version = match answer {
            Err(failure) => {
                problems.push(format!("{failure}; to install: {install}"));
                continue;
            }
            Ok(version) => version.trim(),
        };
        match tool.need {
            Need::Present | Need::PresentOnLinux => {}
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
                refused_runtime(name).map_or_else(
                    || {
                        format!(
                            "{name} is set, which changes what cargo builds or how it lints: \
                             unset it"
                        )
                    },
                    |(runtime, what)| {
                        format!(
                            "{name} is set, which changes how {runtime} runs the TypeScript \
                             checks and the processes their tests start ({what}): unset it"
                        )
                    },
                )
            }),
    );
    if os == "linux"
        && driver == GpuDriver::Required
        && let Some(searched) = &environment.vulkan_searched
    {
        problems.push(format!(
            "no Vulkan driver declared to the loader (searched {}; {} unset); to install: \
             {VULKAN_INSTALL}",
            searched.join(", "),
            VULKAN_DRIVER_VARIABLES
                .join(", ")
                .replacen(", VK_ADD", " and VK_ADD", 1)
        ));
    }
    problems.extend(environment.node_configs.iter().map(|path| {
        format!(
            "{path} configures every npm or pnpm command here: move it away (the configuration \
             of the repository is pnpm-workspace.yaml)"
        )
    }));
    problems.extend(environment.cargo_configs.iter().map(|path| {
        format!(
            "{path} configures every cargo command here: move it away (the configuration of the repository is {CARGO_CONFIG_FILE})"
        )
    }));
    verdict("the checks cannot run here", problems)
}

#[cfg(test)]
#[path = "preflight_tests.rs"]
mod tests;
