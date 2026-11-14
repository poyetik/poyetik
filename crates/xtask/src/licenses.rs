//! The license gate of the JavaScript dependencies.

use crate::{
    steps::{Cmd, Dir, Failure, verdict},
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
