//! The dependency register, the pins of bit-defining dependencies and the audio library's isolation.

use alloc::collections::BTreeSet;

use crate::{
    math_layer::{cargo_packages, dependencies_of, workspace_dependency, workspace_root},
    reviews::{compare_reviews, count},
    steps::{Cmd, Dir, Failure, step_verdict, verdict},
    workspace::{CrateRole, Extent, crates_in},
};

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
/// dependency. No crate renames a dependency, a crate of the workspace included (`package =`
/// in its manifest, `rename` in the metadata): a crate named `core` or `std` that is
/// another would change what `::core` names, and the reviews name packages.
///
/// # Errors
///
/// Returns every renamed dependency, unreviewed dependency, stale review and review without
/// a reason, or why the metadata or a manifest cannot be read.
pub fn dependency_gate(
    metadata: &str,
    packages: &[(String, String)],
    reviewed: &str,
) -> Result<(), Failure> {
    let mut keys = Vec::new();
    let mut problems = Vec::new();
    for (name, krate) in cargo_packages(metadata)? {
        for dependency in dependencies_of(&krate) {
            let text = |key: &str| dependency.get(key).and_then(serde_json::Value::as_str);
            let package = text("name").unwrap_or_default();
            if let Some(rename) = text("rename") {
                problems.push(format!(
                    "{name} depends on {package} renamed `{rename}`: a dependency keeps its \
                     package's name, so that no name (`core`, `std` …) can stand for another \
                     crate"
                ));
            }
            if dependency.get("path").is_none() {
                let kind = text("kind").unwrap_or("normal");
                keys.push(format!("{name}: {package} ({kind})"));
            }
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
    compare_reviews(&count(keys), Extent::Complete, reviewed, &mut problems);
    verdict(
        &format!(
            "direct dependencies must be reviewed in {DEPENDENCIES_REVIEWED} and keep their names"
        ),
        problems,
    )
}

/// The start of the reason of a bit-defining dependency's review ([`DEPENDENCIES_REVIEWED`]).
///
/// Its code computes output samples or hashes, or the values they are computed from
/// (`spec/engine-design.md`, decision 29).
pub const BIT_DEFINING: &str = "bit-defining: ";

/// Whether `req`, a version requirement as cargo metadata writes it, names exactly one
/// version: `=X.Y.Z`.
pub(crate) fn exact_version(req: &str) -> bool {
    req.strip_prefix('=').is_some_and(|version| {
        let parts: Vec<&str> = version.split('.').collect();
        parts.len() == 3
            && parts
                .iter()
                .all(|part| !part.is_empty() && part.chars().all(|digit| digit.is_ascii_digit()))
    })
}

/// Checks that every bit-defining dependency is pinned to one exact version.
///
/// A dependency is bit-defining when its review in `reviewed` ([`DEPENDENCIES_REVIEWED`])
/// gives a reason starting with [`BIT_DEFINING`]; every normal or build dependency of the
/// audio library ([`CrateRole::Library`]) must be reviewed so, since its code runs in the
/// library's. Each must be required as `=X.Y.Z` in its crate's manifest (as `metadata`, the
/// output of [`metadata_command`], gives it), so that an update, which can move bits, is
/// a decision of its own and never a side effect of another.
///
/// # Errors
///
/// Returns every dependency of the library not reviewed as bit-defining and every
/// bit-defining dependency not pinned, or why the metadata cannot be read.
pub fn pin_gate(metadata: &str, reviewed: &str) -> Result<(), Failure> {
    let marked: BTreeSet<&str> = reviewed
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.split_once(" -- "))
        .filter(|(_, reason)| reason.starts_with(BIT_DEFINING))
        .map(|(key, _)| key.trim())
        .collect();
    let mut problems = Vec::new();
    for (name, krate) in cargo_packages(metadata)? {
        let dependencies = krate
            .get("dependencies")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter(|dependency| dependency.get("path").is_none());
        for dependency in dependencies {
            let text = |key: &str| dependency.get(key).and_then(serde_json::Value::as_str);
            let kind = text("kind").unwrap_or("normal");
            let key = format!("{name}: {} ({kind})", text("name").unwrap_or_default());
            let library = crates_in(CrateRole::Library).contains(&name.as_str()) && kind != "dev";
            let bits = marked.contains(key.as_str());
            if library && !bits {
                problems.push(format!(
                    "{key}: its code runs in the audio library's: review it in \
                     {DEPENDENCIES_REVIEWED} with a reason starting `{BIT_DEFINING}`"
                ));
            }
            let req = text("req").unwrap_or_default();
            if (library || bits) && !exact_version(req) {
                problems.push(format!(
                    "{key} is required as `{req}`: pin it to one version, `=X.Y.Z`, in its \
                     Cargo.toml, so that an update, which can move bits, is decided alone \
                     (spec/engine-design.md, decision 29)"
                ));
            }
        }
    }
    step_verdict(problems)
}

/// Checks that the audio library ([`CrateRole::Library`]) stands alone: each of its crates
/// depends, among the crates of the project, only on its own, and its tests also on
/// [`CrateRole::LibraryTests`].
///
/// `metadata` is the output of [`metadata_command`]; a dependency of the project is one
/// with a `path`, and it is allowed only as a crate of [`RUST_CRATES`](crate::workspace::RUST_CRATES) it names and places
/// (`workspace_dependency`). A third-party dependency
/// is reviewed by [`dependency_gate`], and pinned by [`pin_gate`] when it is the library's.
///
/// # Errors
///
/// Returns every other dependency of a crate of the library on the project, every crate
/// of the library the metadata does not list, or why the metadata cannot be read.
pub fn library_gate(metadata: &str) -> Result<(), Failure> {
    let packages = cargo_packages(metadata)?;
    let root = workspace_root(metadata)?;
    let mut problems = Vec::new();
    let (library_crates, test_crates) = (
        crates_in(CrateRole::Library),
        crates_in(CrateRole::LibraryTests),
    );
    for library in library_crates.iter().copied() {
        let Some((_, krate)) = packages.iter().find(|(name, _)| name == library) else {
            problems.push(format!(
                "{library}: not in the cargo metadata, so its dependencies cannot be checked"
            ));
            continue;
        };
        for dependency in dependencies_of(krate) {
            let known = workspace_dependency(dependency, &root)?;
            let text = |key: &str| dependency.get(key).and_then(serde_json::Value::as_str);
            let Some(path) = text("path") else {
                continue;
            };
            let name = text("name").unwrap_or_default();
            let kind = text("kind").unwrap_or("normal");
            let allowed = |crates: &[&str]| known.is_some_and(|known| crates.contains(&known.name));
            if !(allowed(&library_crates) || (kind == "dev" && allowed(&test_crates))) {
                problems.push(format!(
                    "{library} depends on {name} ({kind}, {path}): the audio library depends \
                     only on its own crates ({}), its tests also on {}; move what it needs \
                     into the library, or the dependency out of it",
                    library_crates.join(", "),
                    test_crates.join(", ")
                ));
            }
        }
    }
    step_verdict(problems)
}

/// Checks that every crate only tests use holds [`CrateRole::SharedTests`].
///
/// Such a crate is one of the workspace that some package of `metadata` (the output of
/// [`metadata_command`]) depends on, only as a development dependency or from such a crate
/// (the largest such set): a change to it can weaken the tests of every crate using it, so
/// the default check widens to them, and a crate left without the role would be left out
/// of that widening. A crate may hold the role though code depends on it too (the
/// fingerprint battery, whose test matrix the tests read and which `audio-cli` runs): it
/// then widens more, never less.
///
/// # Errors
///
/// Returns every crate only tests use that does not hold the role, or why the metadata
/// cannot be read.
pub fn shared_tests_gate(metadata: &str) -> Result<(), Failure> {
    let packages = cargo_packages(metadata)?;
    let root = workspace_root(metadata)?;
    // Each dependency on a crate of the workspace: (dependent, dependency, kind).
    let mut edges: Vec<(String, &'static str, String)> = Vec::new();
    for (name, package) in &packages {
        for dependency in dependencies_of(package) {
            let Some(krate) = workspace_dependency(dependency, &root)? else {
                continue;
            };
            let kind = dependency.get("kind").and_then(serde_json::Value::as_str);
            edges.push((
                name.clone(),
                krate.name,
                kind.unwrap_or("normal").to_owned(),
            ));
        }
    }
    let mut shared: BTreeSet<&'static str> = edges.iter().map(|(_, krate, _)| *krate).collect();
    // Each round drops the crates code outside the set depends on, until none is left to
    // drop: at most one round per crate.
    loop {
        let leaving: Vec<&'static str> = shared
            .iter()
            .copied()
            .filter(|krate| {
                edges.iter().any(|(user, used, kind)| {
                    used == krate && kind != "dev" && !shared.contains(user.as_str())
                })
            })
            .collect();
        if leaving.is_empty() {
            break;
        }
        for krate in leaving {
            let _removed = shared.remove(krate);
        }
    }
    let flagged = crates_in(CrateRole::SharedTests);
    let problems = shared
        .into_iter()
        .filter(|name| !flagged.contains(name))
        .map(|name| {
            format!(
                "{name}: only tests reach it, so a change to it can weaken the tests of every \
                 crate using it: give it CrateRole::SharedTests"
            )
        })
        .collect();
    step_verdict(problems)
}

#[cfg(test)]
#[path = "dependencies_tests.rs"]
mod tests;
