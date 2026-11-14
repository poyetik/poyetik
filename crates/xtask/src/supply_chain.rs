//! The supply-chain configuration xtask owns, and its gate.

use crate::steps::{Failure, verdict};

/// The Dependabot configuration.
pub const DEPENDABOT: &str = ".github/dependabot.yml";

/// The pnpm workspace configuration.
pub const PNPM_WORKSPACE: &str = "pnpm-workspace.yaml";

/// The cargo-deny configuration.
pub const DENY_TOML: &str = "deny.toml";

/// The root manifest of the pnpm workspace.
pub const ROOT_PACKAGE_JSON: &str = "package.json";

/// The files [`supply_chain_gate`] reads, in the order it takes their texts.
pub const SUPPLY_CHAIN_FILES: [&str; 4] =
    [DENY_TOML, DEPENDABOT, PNPM_WORKSPACE, ROOT_PACKAGE_JSON];

/// The cargo-deny configuration, exactly.
///
/// Licenses, advisories (no advisory ignored), bans (no duplicate crate skipped, `libm`
/// with no feature, as the engine uses it) and sources. Only developers edit [`DENY_TOML`],
/// so its whole text is xtask's: an exception (an advisory ignored, a crate skipped, a
/// license allowed) is a change of xtask.
pub const DENY_CONFIG: &str = r#"# Supply-chain policy (cargo-deny). Exactly xtask's `DENY_CONFIG`: a change, an exception
# included, is made in both.
[graph]
all-features = true

[licenses]
allow = [
  "MIT",
  "Apache-2.0",
  # unicode-ident (a build-time dependency of serde's derive macros): the permissive
  # license of the Unicode data tables.
  "Unicode-3.0",
]
confidence-threshold = 0.93
private = { ignore = true }

[advisories]
yanked = "deny"

[bans]
multiple-versions = "deny"
wildcards = "deny"
allow-wildcard-paths = true

# libm without any feature, as the engine depends on it: a dependency enabling one (num-traits'
# `libm`, through proptest's `no_std`, turns on its `arch` paths) would make the tests run
# other code than the shipped engine.
[[bans.features]]
crate = "libm"
exact = true
allow = []

[sources]
unknown-registry = "deny"
unknown-git = "deny"
allow-registry = ["https://github.com/rust-lang/crates.io-index"]
"#;

/// The Dependabot configuration, exactly.
///
/// Each ecosystem waits a week before it proposes a release (`cooldown`), so that a
/// hijacked one is likely found and yanked first. Only developers edit [`DEPENDABOT`], so
/// its whole text is xtask's.
pub const DEPENDABOT_CONFIG: &str = r"# Proposes dependency updates; every proposal goes through the full Quality workflow.
# Exactly xtask's `DEPENDABOT_CONFIG`: a change is made in both.
version: 2
updates:
  - package-ecosystem: cargo
    directory: /
    schedule:
      interval: weekly
    # A release is proposed only once it is a week old, so that a hijacked one is likely
    # found and yanked first.
    cooldown:
      default-days: 7
    commit-message:
      prefix: build
  - package-ecosystem: npm
    directory: /
    schedule:
      interval: weekly
    # A release is proposed only once it is a week old, so that a hijacked one is likely
    # found and yanked first.
    cooldown:
      default-days: 7
    commit-message:
      prefix: build
  - package-ecosystem: github-actions
    directory: /
    schedule:
      interval: weekly
    # A release is proposed only once it is a week old, so that a hijacked one is likely
    # found and yanked first.
    cooldown:
      default-days: 7
    commit-message:
      prefix: ci
";

/// The top-level keys of [`PNPM_WORKSPACE`], each exactly once.
///
/// Dependabot edits the values of `overrides` and `catalog`, so the file is read by its
/// keys, not compared whole; any other key (`minimumReleaseAgeExclude`, an audit setting …)
/// is a setting xtask has not reviewed.
pub const PNPM_WORKSPACE_KEYS: [&str; 4] =
    ["packages", "minimumReleaseAge", "overrides", "catalog"];

/// The line of [`PNPM_WORKSPACE`] that sets how old a release must be before pnpm installs
/// it: a week, in minutes.
pub const MINIMUM_RELEASE_AGE: &str = "minimumReleaseAge: 10080";

/// The first line where `found` and `expected` differ, numbered from 1, with both lines
/// (`None` past the end of a text), or `None` when they are equal.
pub(crate) fn first_difference<'text>(
    found: &'text str,
    expected: &'text str,
) -> Option<(usize, Option<&'text str>, Option<&'text str>)> {
    let found: Vec<&str> = found.split('\n').collect();
    let expected: Vec<&str> = expected.split('\n').collect();
    (0..found.len().max(expected.len()))
        .map(|at| {
            (
                at.saturating_add(1),
                found.get(at).copied(),
                expected.get(at).copied(),
            )
        })
        .find(|(_, found, expected)| found != expected)
}

/// The problems of `text`, the pnpm workspace configuration: a top-level line (one starting
/// at column 0, comments apart) that is not a key of [`PNPM_WORKSPACE_KEYS`], a key twice
/// or missing, and a release age other than [`MINIMUM_RELEASE_AGE`].
pub(crate) fn pnpm_workspace_problems(text: &str) -> Vec<String> {
    let mut problems = Vec::new();
    let mut seen = Vec::new();
    for (at, line) in (1_usize..).zip(text.lines()) {
        if line.is_empty() || line.starts_with(|first: char| first.is_whitespace() || first == '#')
        {
            continue;
        }
        let key = line.split_once(':').map_or(line, |(key, _)| key);
        if !PNPM_WORKSPACE_KEYS.contains(&key) {
            problems.push(format!(
                "{PNPM_WORKSPACE}:{at}: `{line}`, not a top-level key xtask accepts ({}): a \
                 setting of pnpm is a change of xtask",
                PNPM_WORKSPACE_KEYS.join(", ")
            ));
        } else if seen.contains(&key) {
            problems.push(format!("{PNPM_WORKSPACE}:{at}: `{key}` a second time"));
        } else if key == "minimumReleaseAge" && line != MINIMUM_RELEASE_AGE {
            problems.push(format!(
                "{PNPM_WORKSPACE}:{at}: `{line}`, where xtask requires `{MINIMUM_RELEASE_AGE}`"
            ));
        }
        seen.push(key);
    }
    problems.extend(
        PNPM_WORKSPACE_KEYS
            .iter()
            .filter(|key| !seen.contains(key))
            .map(|key| format!("{PNPM_WORKSPACE} lacks the top-level key `{key}`")),
    );
    problems
}

/// Checks the supply-chain configuration.
///
/// `texts` are those of [`SUPPLY_CHAIN_FILES`], in its order: [`DENY_TOML`] is exactly
/// [`DENY_CONFIG`] and [`DEPENDABOT`] exactly [`DEPENDABOT_CONFIG`]; [`PNPM_WORKSPACE`]
/// holds the top-level keys of [`PNPM_WORKSPACE_KEYS`] alone, each once, its release age
/// [`MINIMUM_RELEASE_AGE`]; and [`ROOT_PACKAGE_JSON`] holds no `pnpm` key, from which pnpm
/// reads settings too. `npm_configs` are the tracked npm configuration files (`.npmrc`),
/// each refused: pnpm reads settings there too (`minimum-release-age-exclude` …). So an
/// advisory ignored, a crate skipped, a release age shortened or a package excluded from it
/// is a change of xtask, reviewed as such.
///
/// # Errors
///
/// Returns every difference, unaccepted key, `pnpm` key and npm configuration file, or why
/// the root manifest is not a JSON object.
pub fn supply_chain_gate(texts: &[String; 4], npm_configs: &[String]) -> Result<(), Failure> {
    let [deny, dependabot, pnpm, package] = texts;
    let mut problems = Vec::new();
    for (path, text, name, expected) in [
        (DENY_TOML, deny, "DENY_CONFIG", DENY_CONFIG),
        (
            DEPENDABOT,
            dependabot,
            "DEPENDABOT_CONFIG",
            DEPENDABOT_CONFIG,
        ),
    ] {
        if let Some((line, found, wanted)) = first_difference(text, expected) {
            let shown = |side: Option<&str>| match side {
                None => "the end".to_owned(),
                Some("") => "an empty line".to_owned(),
                Some(written) => format!("`{written}`"),
            };
            problems.push(format!(
                "{path}:{line}: {}, where xtask's `{name}` has {}: the file is xtask's, so a \
                 change of its policy (an exception included) is a change of xtask",
                shown(found),
                shown(wanted)
            ));
        }
    }
    problems.extend(pnpm_workspace_problems(pnpm));
    match serde_json::from_str::<serde_json::Value>(package)
        .ok()
        .as_ref()
        .and_then(serde_json::Value::as_object)
    {
        None => problems.push(format!("{ROOT_PACKAGE_JSON} is not a JSON object")),
        Some(manifest) if manifest.contains_key("pnpm") => problems.push(format!(
            "{ROOT_PACKAGE_JSON} holds a `pnpm` key, from which pnpm reads settings: they go in \
             {PNPM_WORKSPACE}, whose keys xtask checks"
        )),
        Some(_) => {}
    }
    problems.extend(npm_configs.iter().map(|path| {
        format!(
            "{path}: an npm configuration file, from which pnpm reads settings \
             (`minimum-release-age-exclude` …): they go in {PNPM_WORKSPACE}, whose keys xtask \
             checks"
        )
    }));
    verdict("the supply-chain configuration", problems)
}

#[cfg(test)]
#[path = "supply_chain_tests.rs"]
mod tests;
