//! The license gates: the JavaScript dependencies' licenses, and each part's license against its
//! folder's.

use crate::{
    dependencies::NPM_FIELDS,
    math_layer::{cargo_packages, dependencies_of, workspace_root},
    steps::{Cmd, Dir, Failure, step_verdict, verdict},
    supply_chain::{PNPM_WORKSPACE, ignored_packages},
    workspace::PACKAGES,
};

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
pub(crate) fn license_commands() -> [Cmd; 2] {
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
pub(crate) fn licensed(
    report: &str,
    what: &str,
    expected: &[&str],
) -> Result<Vec<(String, String)>, String> {
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

/// Checks that no advisory `pnpm audit` ignores is in a package an app ships.
///
/// `shipped` is the license report of what the apps ship (the first of
/// [`license_commands`], read as [`license_gate`] reads it), `workspace` the text of
/// [`PNPM_WORKSPACE`], whose register of exceptions names each advisory's package
/// ([`AUDIT_CONFIG`](crate::supply_chain::AUDIT_CONFIG)): `pnpm audit` ignores an advisory
/// in the whole graph, so this holds an exception's premise, that it covers development
/// tools only. It reads the reasons in their one form, which the supply-chain gate requires
/// of every advisory ignored (a precondition: that gate refuses any other). The report names
/// packages, not versions, so a shipped package of that name is refused even at a fixed
/// version.
///
/// # Errors
///
/// Returns every advisory ignored in a shipped package, or why the report is not of the
/// expected shape.
pub(crate) fn shipped_exceptions_gate(shipped: &str, workspace: &str) -> Result<(), Failure> {
    let [shipped_controls, _] = LICENSE_CONTROLS;
    let packages = licensed(shipped, "shipped", shipped_controls)?;
    let problems = ignored_packages(workspace)
        .into_iter()
        .filter(|(_, _, package)| packages.iter().any(|(name, _)| name == package))
        .map(|(at, id, package)| {
            format!(
                "`{id}` ({package}) is ignored by `pnpm audit` in a package an app ships \
                 ({PNPM_WORKSPACE}:{at}): an exception covers development tools only"
            )
        })
        .collect();
    verdict("pnpm audit's exceptions", problems)
}

/// The license of every part outside `apps/`: the libraries, the bindings, the
/// specifications and the tooling, open source.
pub const OPEN_LICENSE: &str = "MIT OR Apache-2.0";

/// The license of the parts under [`APPS`]: source-available, with no end date.
///
/// Any use is permitted but providing others a product that competes with the software.
/// The license is not on the SPDX list, so it is named by an SPDX `LicenseRef-`.
pub const APPS_LICENSE: &str = "LicenseRef-PolyForm-Perimeter-1.0.1";

/// The folder of parts under [`APPS_LICENSE`].
pub const APPS: &str = "apps/";

/// The files holding the text of each license, relative to the root.
///
/// Each with the line it starts with (spaces around it apart): the open license's two
/// alternatives at the root, which covers every part but the apps, and the apps' in their
/// folder.
pub const LICENSE_FILES: [(&str, &str); 3] = [
    ("LICENSE-MIT", "MIT License"),
    ("LICENSE-APACHE", "Apache License"),
    ("apps/LICENSE", "# PolyForm Perimeter License 1.0.1"),
];

/// The license of the part in `dir`, relative to the root (`.` for the root itself):
/// [`APPS_LICENSE`] under [`APPS`], [`OPEN_LICENSE`] everywhere else.
#[must_use]
pub fn license_of(dir: &str) -> &'static str {
    if dir.starts_with(APPS) {
        APPS_LICENSE
    } else {
        OPEN_LICENSE
    }
}

/// Checks that every part is under its folder's license and no open part depends on an app.
///
/// `metadata` is the output of [`metadata_command`](crate::dependencies::metadata_command):
/// every crate declares the license of its directory ([`license_of`]) in its manifest, and
/// none outside [`APPS`] depends on a crate of the workspace under [`APPS`] (a `path`
/// dependency of any kind). `packages` are the `package.json` files (path and text): each
/// declares the license of its directory in its `license` field, and none outside [`APPS`]
/// depends on a package of the workspace under it (a `workspace:` dependency, in any of
/// [`NPM_FIELDS`]), so that the open parts stay open and
/// usable without the apps. `texts` are the [`LICENSE_FILES`] (path and text), each starting
/// with its license's first line.
///
/// # Errors
///
/// Returns every crate or package without its folder's license, every open part that
/// depends on a part of the apps, every license file that does not start with its
/// license's first line, and invalid metadata or manifests.
pub fn part_license_gate(
    metadata: &str,
    packages: &[(String, String)],
    texts: &[(String, String)],
) -> Result<(), Failure> {
    let mut problems = Vec::new();
    let root = workspace_root(metadata)?;
    let relative = |path: &str| -> String {
        let dir = path.rsplit_once('/').map_or("", |(dir, _)| dir);
        dir.strip_prefix(root.as_str())
            .map_or(dir, |rest| rest.trim_start_matches('/'))
            .to_owned()
    };
    for (name, krate) in cargo_packages(metadata)? {
        let manifest = krate
            .get("manifest_path")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| Failure::from(format!("cargo metadata gives {name} no manifest")))?;
        let dir = relative(manifest);
        let wanted = license_of(&dir);
        match krate.get("license").and_then(serde_json::Value::as_str) {
            Some(license) if license == wanted => {}
            other => problems.push(format!(
                "{dir}: the crate {name} is under {}, not {wanted}",
                other.unwrap_or("no license")
            )),
        }
        for dependency in dependencies_of(&krate) {
            let Some(path) = dependency.get("path").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let target = path
                .strip_prefix(root.as_str())
                .map_or(path, |rest| rest.trim_start_matches('/'));
            if wanted == OPEN_LICENSE && license_of(target) == APPS_LICENSE {
                problems.push(format!(
                    "{dir}: the crate {name}, under {OPEN_LICENSE}, depends on {target}, \
                     under {APPS_LICENSE}"
                ));
            }
        }
    }
    let mut parts: Vec<(String, String, serde_json::Value)> = Vec::new();
    for (path, text) in packages {
        let fields: serde_json::Value = serde_json::from_str(text)
            .map_err(|error| Failure::from(format!("{path}: invalid package.json: {error}")))?;
        let dir = path.strip_suffix("/package.json").unwrap_or(".").to_owned();
        let name = fields
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        parts.push((dir, name, fields));
    }
    for (dir, name, fields) in &parts {
        let wanted = license_of(dir);
        match fields.get("license").and_then(serde_json::Value::as_str) {
            Some(license) if license == wanted => {}
            other => problems.push(format!(
                "{dir}/package.json: {name} is under {}, not {wanted}",
                other.unwrap_or("no license")
            )),
        }
        for field in NPM_FIELDS {
            let declared = fields
                .get(field)
                .and_then(serde_json::Value::as_object)
                .into_iter()
                .flatten();
            for (dependency, version) in declared {
                let local = version
                    .as_str()
                    .is_some_and(|version| version.starts_with("workspace:"));
                let target = parts.iter().find(|(_, other, _)| other == dependency);
                if let (true, Some((target, _, _))) = (local, target)
                    && wanted == OPEN_LICENSE
                    && license_of(target) == APPS_LICENSE
                {
                    problems.push(format!(
                        "{dir}/package.json: {name}, under {OPEN_LICENSE}, depends on \
                         {dependency} ({field}), under {APPS_LICENSE}"
                    ));
                }
            }
        }
    }
    for (file, first) in LICENSE_FILES {
        let starts = texts
            .iter()
            .find(|(path, _)| path == file)
            .and_then(|(_, text)| text.lines().find(|line| !line.trim().is_empty()))
            .is_some_and(|line| line.trim() == first);
        if !starts {
            problems.push(format!("{file} does not start with \"{first}\""));
        }
    }
    step_verdict(problems)
}

#[cfg(test)]
#[path = "licenses_tests.rs"]
mod tests;
