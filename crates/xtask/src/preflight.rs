//! The pinned tools and the environment the check refuses, checked before any step.

use alloc::collections::BTreeSet;
use std::ffi::OsString;
use std::path::Path;
use std::path::PathBuf;

use crate::{
    changes::{LockEntry, LockKey, lock_dependency, lock_entries},
    dependencies::CARGO_CONFIG,
    release::Version,
    runner::CommandFailure,
    steps::{Failure, verdict},
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

/// The tables of the root manifest every workspace apart's manifest holds exactly.
///
/// Each from its header to the header that follows it in the root's (the package fields
/// its crates inherit, then every lint).
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
/// being any line whose text starts with `[`; from `[workspace.package]` to
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

/// Checks that the manifest of every workspace apart shares the root's settings.
///
/// Each of `manifests` (its path and its text: those of
/// [`ApartWorkspace::ALL`](crate::workspace::ApartWorkspace::ALL)) must
/// hold each of the [`SHARED_MANIFEST_TABLES`] of the root manifest (`root`) exactly, and
/// end it there (the end of the file or another table's header follows), so
/// that its crates share the root's version, edition, license and lints: a setting changed
/// at the root is changed there too, or this fails.
///
/// # Errors
///
/// Returns each table the root does not hold as expected, and each table a manifest does
/// not hold exactly.
pub fn apart_manifests_gate(root: &str, manifests: &[(&str, &str)]) -> Result<(), Failure> {
    let mut problems = Vec::new();
    for &(start, end) in &SHARED_MANIFEST_TABLES {
        let table = root
            .split_once(start)
            .and_then(|(_, rest)| rest.split_once(end))
            .map(|(body, _)| format!("{start}{body}"));
        let Some(table) = table else {
            problems.push(format!("Cargo.toml has no {start} table followed by {end}"));
            continue;
        };
        problems.extend(
            manifests
                .iter()
                .filter(|(_, text)| !holds_whole(text, &table))
                .map(|(path, _)| format!("{path} does not hold the root's {start} table exactly")),
        );
    }
    verdict(
        "every workspace apart shares the root's package fields and lints",
        problems,
    )
}

/// The manifest of the GPU workspace.
pub const GPU_MANIFEST: &str = concat!(crate::workspace::gpu_workspace!(), "/Cargo.toml");

/// Whether `text` holds `table` whole: its text, then the end of `text` or the header of
/// another table, so that no key added after it extends it.
fn holds_whole(text: &str, table: &str) -> bool {
    text.split_once(table)
        .is_some_and(|(_, after)| after.is_empty() || after.starts_with('['))
}

/// The configuration files that Cargo, rustfmt, Clippy and rustup look for from a
/// directory upward, as git pathspecs matching them at any depth.
pub const TOOL_CONFIGS: [&str; 7] = [
    "*clippy.toml",
    "*rustfmt.toml",
    "*rust-toolchain",
    "*rust-toolchain.toml",
    "*.cargo/config.toml",
    "*.cargo/config",
    "*.clippy.toml",
];

/// Checks that each of `tracked` (the tracked files [`TOOL_CONFIGS`] match) is at the root.
///
/// A tool reads the nearest of its files going up from the crate it works on, so one
/// below the root would replace the root's for every crate beneath it (a workspace apart's,
/// among them): Clippy's determinism bans, rustfmt's style, the pinned toolchain or a build
/// flag, unseen by the gates that read the root's.
///
/// # Errors
///
/// Returns each such file below the root.
pub fn nested_tool_configs_gate(tracked: &[String]) -> Result<(), Failure> {
    let problems = tracked
        .iter()
        .filter(|path| path.trim_start_matches(".cargo/").contains('/'))
        .map(|path| format!("{path} would replace the root's configuration below it"))
        .collect();
    verdict(
        "the tools read their configuration from the root only",
        problems,
    )
}

/// The packages the GPU renderer and the WGSL tools must resolve to the same versions.
///
/// The GPU layer and its shader compiler, so that the shaders the tools accept are those
/// the renderer's compiler accepts, and a crate using both never builds two of them.
pub const SHARED_GPU_PACKAGES: [&str; 2] = ["naga", "wgpu"];

/// Checks that the WGSL tools build the very naga and wgpu the GPU renderer builds, with
/// the very packages beneath them.
///
/// `gpu` and `wgsl` are the texts of their `Cargo.lock` files ([`GPU_LOCK`], [`WGSL_LOCK`]),
/// read as Cargo writes them (the lock parser of the default check's widening). The tools
/// must resolve naga, and every package their lock reaches from each of
/// [`SHARED_GPU_PACKAGES`] they resolve (the package and its dependencies, followed) must be
/// one the renderer's lock resolves, at the same version, from the same source, with the
/// same checksum: a patched or vendored copy, or a dependency of naga at another version
/// (a number parser, a float type), would let the tools accept a shader the renderer's
/// compiler reads otherwise. Only their features may differ: the renderer turns on more
/// of naga's (its backends) than the tools' validation needs.
///
/// # Errors
///
/// Returns each package reached that the renderer does not resolve as the tools do, a
/// naga missing from the tools' lock, and each lock file not in Cargo's shape.
pub fn shared_gpu_versions_gate(gpu: &str, wgsl: &str) -> Result<(), Failure> {
    let summary =
        "the WGSL tools build the GPU renderer's naga and wgpu, and the packages beneath them";
    let parsed = |path: &str, lock: &str| {
        lock_entries(lock).map_err(|why| format!("{path} is not as Cargo writes it: {why}"))
    };
    let (renderer, tools) = match (parsed(GPU_LOCK, gpu), parsed(WGSL_LOCK, wgsl)) {
        (Ok(renderer), Ok(tools)) => (renderer, tools),
        (renderer, tools) => {
            return verdict(
                summary,
                renderer.err().into_iter().chain(tools.err()).collect(),
            );
        }
    };
    let mut problems = Vec::new();
    let mut pending: Vec<(&LockKey, &LockEntry)> = tools
        .iter()
        .filter(|((name, _, _), _)| SHARED_GPU_PACKAGES.contains(&name.as_str()))
        .collect();
    if !pending.iter().any(|((name, _, _), _)| name == "naga") {
        problems.push(format!("{WGSL_LOCK} resolves no naga"));
    }
    let mut reached = BTreeSet::new();
    while let Some((key, entry)) = pending.pop() {
        if !reached.insert(key) {
            continue;
        }
        for dependency in entry.dependencies() {
            match lock_dependency(&tools, dependency) {
                Ok(found) => pending.push(found),
                Err(why) => problems.push(format!("{WGSL_LOCK}: {why}")),
            }
        }
        let (name, version, source) = key;
        let named = format!("{name} {version} ({source})");
        match renderer.get(key) {
            Some(same) if same.checksum() == entry.checksum() => {}
            Some(_) => problems.push(format!(
                "{WGSL_LOCK} resolves {named} with another checksum than {GPU_LOCK}"
            )),
            None => problems.push(format!(
                "{WGSL_LOCK} resolves {named}, which {GPU_LOCK} does not: {}",
                listed(
                    &renderer
                        .keys()
                        .filter(|(other, _, _)| other == name)
                        .map(|(_, other, from)| format!("{other} ({from})"))
                        .collect::<Vec<_>>()
                )
            )),
        }
    }
    verdict(summary, problems)
}

/// `versions` as a reader sees them: `none` when empty, else joined by `, `.
fn listed(versions: &[String]) -> String {
    if versions.is_empty() {
        "none".to_owned()
    } else {
        versions.join(", ")
    }
}

/// The lock file of the GPU workspace.
pub const GPU_LOCK: &str = "crates/visual-gpu/Cargo.lock";

/// The lock file of the WGSL tools workspace.
pub const WGSL_LOCK: &str = "tooling/wgsl/Cargo.lock";

/// The Cargo tools the checks run, at the exact version they need (as `cargo install`
/// names them): the preflight refuses another version, and the CI workflow installs these.
pub const PINNED_TOOLS: [(&str, &str); 4] = [
    ("cargo-llvm-cov", "0.9.1"),
    ("cargo-mutants", "27.1.0"),
    ("cargo-deny", "0.20.2"),
    ("cargo-nextest", "0.9.152"),
];

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
pub const TOOLS: [Tool; 13] = [
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
        program: "sh",
        args: &[
            "-c",
            "ls /usr/share/vulkan/icd.d/*.json || ls /etc/vulkan/icd.d/*.json",
        ],
        need: Need::PresentOnLinux,
        install: Install::Text(
            "install a Vulkan driver, which the GPU workspace's tests render on (on Ubuntu, mesa-vulkan-drivers: lavapipe renders on the processor)",
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

/// Whether the variable `name` is one the preflight refuses.
///
/// It refuses one of [`REFUSED_VARIABLES`], a variable a crate reads at build time
/// (`BUILD_VARIABLES`: the engine block a build names, where the checks build every engine
/// block), or a name starting with one of [`REFUSED_VARIABLE_PREFIXES`] other than
/// `CARGO_TARGET_DIR`.
#[must_use]
pub fn refused_variable(name: &str) -> bool {
    REFUSED_VARIABLES.contains(&name)
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
/// configuration outside the repository's own.
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

#[cfg(test)]
#[path = "preflight_tests.rs"]
mod tests;
