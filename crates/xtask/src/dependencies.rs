//! The dependency register, the pins of bit-defining dependencies and the audio library's isolation.

use alloc::collections::BTreeMap;
use alloc::collections::BTreeSet;

use crate::{
    changes::LOCK_FILE,
    math_layer::{cargo_packages, dependencies_of, workspace_dependency, workspace_root},
    reviews::{compare_reviews, count},
    steps::{Cmd, Dir, Failure, step_verdict, verdict},
    workspace::{CargoWorkspace, CrateRole, Extent, LIBRARIES, Library, crates_in},
};

/// The exact Cargo configuration of the repository: an alias, nothing else.
///
/// Build flags (`rustflags`) could lift any lint of the workspace in every crate, and an
/// unlocked alias would rewrite a stale `Cargo.lock` before the checks.
pub const CARGO_CONFIG: &str = "[alias]
# The single entry point for project automation: cargo xtask check runs every check.
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

/// The scope of the workspace's own packages: a package of the workspace is named under it
/// and linked with a `workspace:` version, both or neither (checked by [`dependency_gate`]).
pub(crate) const WORKSPACE_SCOPE: &str = "@poyetik/";

/// The register of the third-party code that runs at build time: every build script and
/// procedural macro of a crate a Cargo workspace builds, at its exact version, with what it
/// does.
pub const BUILD_TIME_REVIEWED: &str = "build-time-code.txt";

/// The command listing every crate `workspace` builds, its dependencies' included, with
/// their targets.
#[must_use]
pub fn resolved_metadata_command(workspace: CargoWorkspace) -> Cmd {
    Cmd::new(
        "cargo",
        &[
            &["metadata", "--format-version", "1", "--locked"],
            workspace.manifest_args(),
        ]
        .concat(),
        Dir::Root,
    )
}

/// Checks that every third-party build script and procedural macro is reviewed.
///
/// Each crate a workspace builds that runs code at build time, a build script or a
/// procedural macro, is reviewed in `reviewed` ([`BUILD_TIME_REVIEWED`]) at its exact
/// version, and every review is current: `metadata` holds each
/// workspace's resolved metadata ([`resolved_metadata_command`]). A crate both workspaces
/// build is reviewed once; the workspaces' own crates are reviewed as code.
///
/// # Errors
///
/// Returns every unreviewed one, every stale review and every review without a reason, or
/// why a workspace's metadata cannot be read.
pub fn build_time_gate(metadata: &[String], reviewed: &str) -> Result<(), Failure> {
    let mut found = BTreeSet::new();
    for listed in metadata {
        let json: serde_json::Value = serde_json::from_str(listed)
            .map_err(|error| Failure::from(format!("invalid cargo metadata: {error}")))?;
        let packages = json
            .get("packages")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter(|package| {
                package
                    .get("source")
                    .is_some_and(|source| !source.is_null())
            });
        for package in packages {
            let field = |key: &str| {
                package
                    .get(key)
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
            };
            let kinds = package
                .get("targets")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|target| target.get("kind").and_then(serde_json::Value::as_array))
                .flatten()
                .filter_map(serde_json::Value::as_str);
            for kind in kinds {
                let what = match kind {
                    "custom-build" => "build script",
                    "proc-macro" => "proc macro",
                    _ => continue,
                };
                let _new = found.insert(format!("{}@{} ({what})", field("name"), field("version")));
            }
        }
    }
    let mut problems = Vec::new();
    compare_reviews(&count(found), Extent::Complete, reviewed, &mut problems);
    verdict(
        &format!(
            "code third-party crates run at build time must be reviewed in {BUILD_TIME_REVIEWED}"
        ),
        problems,
    )
}

/// The release line of a crate's `version`, as Cargo reads compatibility: its major
/// version, its minor one below 1.0, its patch one below 0.1.
fn release_line(version: &str) -> &str {
    let mut ends = version.match_indices('.').map(|(at, _)| at);
    let (first, second) = (ends.next(), ends.next());
    let end = if !version.starts_with("0.") {
        first
    } else if !version.starts_with("0.0.") {
        second
    } else {
        None
    };
    end.and_then(|end| version.get(..end)).unwrap_or(version)
}

/// The third-party crates of a workspace's resolved metadata
/// ([`resolved_metadata_command`]), each name with every version its lock file resolves.
///
/// # Errors
///
/// Returns why the metadata cannot be read.
fn resolved_versions(metadata: &str) -> Result<BTreeMap<String, BTreeSet<String>>, Failure> {
    let json: serde_json::Value = serde_json::from_str(metadata)
        .map_err(|error| Failure::from(format!("invalid cargo metadata: {error}")))?;
    let mut versions: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for package in json
        .get("packages")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter(|package| {
            package
                .get("source")
                .is_some_and(|source| !source.is_null())
        })
    {
        let field = |key: &str| {
            package
                .get(key)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let _new = versions
            .entry(field("name"))
            .or_default()
            .insert(field("version"));
    }
    Ok(versions)
}

/// Checks that the crates both Cargo workspaces resolve are at one version in both lock
/// files.
///
/// `root` and `gpu` are each workspace's resolved metadata ([`resolved_metadata_command`]).
/// Every third-party crate both resolve has a version in common, and the same version in
/// each release line both resolve (`release_line`: `syn` 3 in both and 2 in the GPU
/// workspace alone passes), so that a dependency updated in one lock file alone (a
/// proposal of Dependabot's for one directory) is refused: the two workspaces' tests then
/// explore the same cases (`proptest`, `rand`), and a dependency of a bit-defining crate
/// the GPU workspace shares stays what the root's checks proved.
///
/// # Errors
///
/// Returns every crate whose versions differ, with both, or why the metadata cannot be
/// read.
pub fn shared_versions_gate(root: &str, gpu: &str) -> Result<(), Failure> {
    let (root, gpu) = (resolved_versions(root)?, resolved_versions(gpu)?);
    let mut problems = Vec::new();
    for (name, held) in &root {
        let Some(other) = gpu.get(name) else {
            continue;
        };
        let lines = |versions: &BTreeSet<String>| -> BTreeMap<String, String> {
            versions
                .iter()
                .map(|version| (release_line(version).to_owned(), version.clone()))
                .collect()
        };
        let (held_lines, other_lines) = (lines(held), lines(other));
        let apart = held.is_disjoint(other)
            || held_lines
                .iter()
                .any(|(line, version)| other_lines.get(line).is_some_and(|of| of != version));
        if apart {
            let joined = |versions: &BTreeSet<String>| {
                versions.iter().cloned().collect::<Vec<_>>().join(" and ")
            };
            problems.push(format!(
                "{name}: {LOCK_FILE} resolves {}, {GPU_LOCK} {}: bring them to one version {}",
                joined(held),
                joined(other),
                alignment_hints(name, held, &held_lines, other).join(" and ")
            ));
        }
    }
    verdict(
        "the crates both Cargo workspaces resolve must be at one version in both lock files",
        problems,
    )
}

/// The hints bringing the versions `other` of the crate `name` the GPU workspace resolves
/// to those of the root's, `held` (`held_lines` by release line): one exact command per
/// version apart, the root's first. A version of a release line the root resolves meets the
/// root's in it, the newer of the two: the GPU workspace's updated when the root's is newer,
/// the root's when it is older. With no line in common, one command alone, the older of the
/// two workspaces' newest to the newer, after which they have one. Each runs `cargo update`
/// with the version it changes and the one it sets, never a placeholder.
fn alignment_hints(
    name: &str,
    held: &BTreeSet<String>,
    held_lines: &BTreeMap<String, String>,
    other: &BTreeSet<String>,
) -> Vec<String> {
    let mut apart: Vec<(&String, &String)> = other
        .difference(held)
        .filter_map(|version| Some((version, held_lines.get(release_line(version))?)))
        .collect();
    if apart.is_empty() {
        apart.extend(newest(other).zip(newest(held)));
    }
    let (mut root, mut gpu) = (Vec::new(), Vec::new());
    for (version, target) in apart {
        if version_order(version, target).is_gt() {
            root.push(format!(
                "(see: cargo update -p {name}@{target} --precise {version})"
            ));
        } else {
            gpu.push(format!(
                "(see: cargo update --manifest-path \"$(git rev-parse --show-toplevel)/{}\" -p {name}@{version} --precise {target})",
                crate::preflight::GPU_MANIFEST
            ));
        }
    }
    root.extend(gpu);
    root
}

/// The newest of `versions`.
fn newest(versions: &BTreeSet<String>) -> Option<&String> {
    versions
        .iter()
        .max_by(|first, second| version_order(first, second))
}

/// The order of two crate versions, Semantic Versioning's precedence: a pre-release is below its
/// release, its parts compared as numbers below words, and build metadata orders nothing.
fn version_order(one: &str, other: &str) -> core::cmp::Ordering {
    /// The parts of `text` between its dots, in an order.
    fn parts(text: &str) -> Vec<(bool, u64, &str)> {
        text.split('.')
            .map(|part| {
                part.parse()
                    .map_or((true, 0, part), |number| (false, number, ""))
            })
            .collect()
    }
    let precedence = |version| {
        let (version, _) = str::split_once(version, '+').unwrap_or((version, ""));
        let (numbers, pre) = version.split_once('-').unwrap_or((version, ""));
        (parts(numbers), pre.is_empty(), parts(pre))
    };
    precedence(one).cmp(&precedence(other))
}

/// The GPU workspace's lock file.
const GPU_LOCK: &str = concat!(crate::workspace::gpu_workspace!(), "/Cargo.lock");

/// The command listing the crates of `workspace` and their direct dependencies.
#[must_use]
pub fn metadata_command(workspace: CargoWorkspace) -> Cmd {
    Cmd::new(
        "cargo",
        &[
            &["metadata", "--format-version", "1", "--no-deps", "--locked"],
            workspace.manifest_args(),
        ]
        .concat(),
        Dir::Root,
    )
}

/// The metadata of every workspace as one.
///
/// The root's ([`metadata_command`] of [`CargoWorkspace::Root`]), its packages followed by
/// those of each other workspace, so that every gate reading the crates sees them all,
/// paths relative to the root's. The
/// root's text unchanged when the others add no package, or when it holds no list of
/// packages, which the gates reading it then refuse in their own words.
///
/// # Errors
///
/// Returns why a text is not the metadata cargo writes: not JSON, or another workspace's
/// without its list of packages.
pub fn merged_metadata(root: &str, others: &[String]) -> Result<String, Failure> {
    let parse = |text: &str| -> Result<serde_json::Value, Failure> {
        serde_json::from_str(text)
            .map_err(|error| Failure::from(format!("invalid cargo metadata: {error}")))
    };
    let mut added = Vec::new();
    for other in others {
        added.extend(
            parse(other)?
                .get("packages")
                .and_then(serde_json::Value::as_array)
                .cloned()
                .ok_or_else(|| Failure::from("cargo metadata without its packages".to_owned()))?,
        );
    }
    if added.is_empty() {
        return Ok(root.to_owned());
    }
    let mut merged = parse(root)?;
    match merged
        .get_mut("packages")
        .and_then(serde_json::Value::as_array_mut)
    {
        Some(packages) => packages.extend(added),
        None => return Ok(root.to_owned()),
    }
    Ok(merged.to_string())
}

/// Checks that every direct third-party dependency is reviewed, and every review current.
///
/// `metadata` is the output of [`metadata_command`]: each crate's dependencies, those on
/// another crate of the workspace (a `path`) apart; `packages` the `package.json` files
/// (path and text), each declaring dependencies in its [`NPM_FIELDS`], those on another
/// package of the workspace apart, named under `@poyetik/` and linked with a
/// `workspace:` version, both or neither. `reviewed` ([`DEPENDENCIES_REVIEWED`])
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
                .flatten();
            for (dependency, version) in declared {
                let linked = version
                    .as_str()
                    .is_some_and(|version| version.starts_with("workspace:"));
                if linked != dependency.starts_with(WORKSPACE_SCOPE) {
                    problems.push(format!(
                        "{part}: {dependency} ({field}) is named under {WORKSPACE_SCOPE} or \
                         linked with workspace:, not both: a package of the workspace is both"
                    ));
                } else if !linked {
                    keys.push(format!("{part}: {dependency} ({field})"));
                }
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
/// Its code computes output samples, images or hashes, or the values they are computed from
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
/// gives a reason starting with [`BIT_DEFINING`]; every normal or build dependency of one
/// of the [`LIBRARIES`] must be reviewed so, since its code runs in the library's, and so
/// must every one of a conformance case reader ([`CrateRole::CaseReader`]), since it reads
/// the values rendered and hashed. Each
/// must be required as `=X.Y.Z` in its crate's manifest (as `metadata`, the output of
/// [`metadata_command`], gives it), so that an update, which can move bits, is a decision
/// of its own and never a side effect of another.
///
/// # Errors
///
/// Returns every dependency of a library or of a case reader not reviewed as bit-defining
/// and every bit-defining dependency not pinned, or why the metadata cannot be read.
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
            let library = LIBRARIES
                .iter()
                .find(|library| crates_in(library.crates).contains(&name.as_str()))
                .map(|library| library.name)
                .or_else(|| {
                    crates_in(CrateRole::CaseReader)
                        .contains(&name.as_str())
                        .then_some("conformance case reader")
                })
                .filter(|_| kind != "dev");
            let bits = marked.contains(key.as_str());
            if let Some(what) = library.filter(|_| !bits) {
                problems.push(format!(
                    "{key}: its code runs in the {what}'s: review it in \
                     {DEPENDENCIES_REVIEWED} with a reason starting `{BIT_DEFINING}`"
                ));
            }
            let req = text("req").unwrap_or_default();
            if (library.is_some() || bits) && !exact_version(req) {
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

/// Checks that each of the [`LIBRARIES`] stands alone, with its tests and its tools.
///
/// Among the crates of the project, each crate of a library depends only on the library's
/// own crates, and its tests also on the crates of its tests' role ([`CrateRole::LibraryTests`]
/// for the audio library, [`CrateRole::VisualLibraryTests`] for the visual one); a tool of
/// the library ([`CrateRole::VisualTool`]) depends on the same; and a crate of its tests'
/// role depends only on the library's crates and its other test crates, of any kind. Since
/// every crate it reaches is held to the same rule, nothing the library or its tests reach,
/// however indirectly, is outside them: a crate held by both libraries' tests (the counting
/// allocator) can then depend on nothing of either.
///
/// `metadata` is the output of [`metadata_command`]; a dependency of the project is one
/// with a `path`, and it is allowed only as a crate of [`RUST_CRATES`](crate::workspace::RUST_CRATES) it names and places
/// (`workspace_dependency`). A third-party dependency
/// is reviewed by [`dependency_gate`], and pinned by [`pin_gate`] when it is the library's.
///
/// # Errors
///
/// Returns every other dependency on the project of a crate of a library, of its tests or
/// of its tools, every such crate the metadata does not list, or why the metadata cannot be
/// read.
pub fn library_gate(metadata: &str) -> Result<(), Failure> {
    let packages = cargo_packages(metadata)?;
    let root = workspace_root(metadata)?;
    let mut problems = Vec::new();
    for Library {
        crates,
        tests,
        tools,
        name: what,
    } in LIBRARIES
    {
        let (library_crates, test_crates) = (crates_in(crates), crates_in(tests));
        let tool_crates = tools.map(crates_in).unwrap_or_default();
        let checked = library_crates
            .iter()
            .chain(&tool_crates)
            .chain(&test_crates);
        for library in checked.copied() {
            let Some((_, krate)) = packages.iter().find(|(name, _)| name == library) else {
                problems.push(format!(
                    "{library}: not in the cargo metadata, so its dependencies cannot be checked"
                ));
                continue;
            };
            let a_test = test_crates.contains(&library);
            for dependency in dependencies_of(krate) {
                let known = workspace_dependency(dependency, &root)?;
                let text = |key: &str| dependency.get(key).and_then(serde_json::Value::as_str);
                let Some(path) = text("path") else {
                    continue;
                };
                let name = text("name").unwrap_or_default();
                let kind = text("kind").unwrap_or("normal");
                let allowed =
                    |among: &[&str]| known.is_some_and(|known| among.contains(&known.name));
                let tests_allowed = kind == "dev" || a_test;
                if allowed(&library_crates) || (tests_allowed && allowed(&test_crates)) {
                    continue;
                }
                let (own, tests_too) = (library_crates.join(", "), test_crates.join(", "));
                problems.push(if a_test {
                    format!(
                        "{library}, a crate of the {what}'s tests, depends on {name} ({kind}, \
                         {path}): it depends, within the project, only on the {what}'s crates \
                         ({own}) and its other test crates ({tests_too}); move what it needs \
                         into them, or the dependency out of it"
                    )
                } else if tool_crates.contains(&library) {
                    format!(
                        "{library}, a tool of the {what}, depends on {name} ({kind}, {path}): it \
                         depends, within the project, only on the {what}'s crates ({own}), its \
                         tests also on {tests_too}; move what it needs into the library, or the \
                         dependency out of it"
                    )
                } else {
                    format!(
                        "{library} depends on {name} ({kind}, {path}): the {what} depends \
                         only on its own crates ({own}), its tests also on {tests_too}; move \
                         what it needs into the library, or the dependency out of it"
                    )
                });
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
