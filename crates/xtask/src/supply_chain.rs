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
/// keys, not compared whole; any other key (`minimumReleaseAgeExclude` …) but
/// [`AUDIT_CONFIG`] is a setting xtask has not reviewed.
pub const PNPM_WORKSPACE_KEYS: [&str; 4] =
    ["packages", "minimumReleaseAge", "overrides", "catalog"];

/// The one optional top-level key of [`PNPM_WORKSPACE`]: the register of the advisories
/// `pnpm audit` ignores, at most once.
///
/// Accepted only as `auditConfig:` alone on its line, then `  ignoreGhsas:` and one
/// `    - <id>` line per advisory, each id a GitHub advisory id as GitHub writes it, with its
/// reason on a comment line `# <id>: <reason>` at column 0, continued on the comment lines
/// right under it that open with `#  `, citing the roadmap step that removes it (`#N step
/// M`); absent when the register is empty. Any other audit setting (`ignoreCves`, an
/// advisory without its reason) is a setting xtask has not reviewed.
pub const AUDIT_CONFIG: &str = "auditConfig";

/// The line under [`AUDIT_CONFIG`] that opens its list of advisories.
const IGNORED_ADVISORIES: &str = "  ignoreGhsas:";

/// The opening of a line of the list of [`IGNORED_ADVISORIES`], before the advisory's id.
const IGNORED_ADVISORY: &str = "    - ";

/// The opening of a comment line that gives the reason of an ignored advisory.
const REASON_OPENING: &str = "# GHSA-";

/// The opening of a comment line that continues the reason above it (a hanging indent).
const REASON_CONTINUED: &str = "#  ";

/// The characters of the groups of a GitHub advisory id.
const ADVISORY_ALPHABET: &str = "23456789cfghjmpqrvwx";

/// Whether `text` is a GitHub advisory id as GitHub writes it: `GHSA-`, then three groups of
/// four characters of [`ADVISORY_ALPHABET`], joined by dashes.
fn advisory_id(text: &str) -> bool {
    text.strip_prefix("GHSA-").is_some_and(|groups| {
        let groups: Vec<&str> = groups.split('-').collect();
        groups.len() == 3
            && groups.iter().all(|group| {
                group.len() == 4 && group.chars().all(|found| ADVISORY_ALPHABET.contains(found))
            })
    })
}

/// A reason of an ignored advisory ([`written_reasons`]).
struct Reason<'text> {
    /// The line it opens on, from 1.
    at: usize,
    /// The advisory's id.
    id: &'text str,
    /// Its text, continuation lines joined.
    text: String,
    /// The comment line right under it that is not read as its continuation, with its
    /// number, if any: one that opens neither with [`REASON_CONTINUED`] nor another reason.
    unread: Option<(usize, &'text str)>,
}

/// The reasons of the ignored advisories `text` gives ([`Reason`]), and the problems of the
/// lines that open one ([`REASON_OPENING`]) and are not one.
///
/// A reason is the comment line `# <id>: <reason>`, at column 0 anywhere in the file, the
/// reason not empty, continued on the comment lines right under it that open with
/// [`REASON_CONTINUED`]; each advisory has one.
fn written_reasons(text: &str) -> (Vec<Reason<'_>>, Vec<String>) {
    let lines: Vec<&str> = text.lines().collect();
    let mut reasons: Vec<Reason<'_>> = Vec::new();
    let mut problems = Vec::new();
    for (at, line) in (1_usize..).zip(&lines) {
        if !line.starts_with(REASON_OPENING) {
            continue;
        }
        let entry = line
            .split_once(": ")
            .and_then(|(head, reason)| Some((head.strip_prefix("# ")?, reason.trim())))
            .filter(|(id, reason)| advisory_id(id) && !reason.is_empty());
        match entry {
            None => problems.push(format!(
                "{PNPM_WORKSPACE}:{at}: `{line}`, where xtask reads a reason as `# GHSA-…: \
                 <reason>`, continued on lines opening with `{REASON_CONTINUED}`"
            )),
            Some((id, _)) if reasons.iter().any(|reason| reason.id == id) => problems.push(
                format!("{PNPM_WORKSPACE}:{at}: the reason of `{id}` a second time"),
            ),
            Some((id, first)) => {
                let continued: Vec<&str> = lines
                    .iter()
                    .skip(at)
                    .map_while(|next| next.strip_prefix(REASON_CONTINUED))
                    .map(str::trim)
                    .collect();
                let after = at.saturating_add(continued.len());
                let unread = lines
                    .get(after)
                    .filter(|next| next.starts_with('#') && !next.starts_with(REASON_OPENING))
                    .map(|next| (after.saturating_add(1), *next));
                let joined: Vec<&str> = core::iter::once(first).chain(continued).collect();
                reasons.push(Reason {
                    at,
                    id,
                    text: joined.join(" "),
                    unread,
                });
            }
        }
    }
    (reasons, problems)
}

/// The problems of the advisories [`AUDIT_CONFIG`] ignores against their `reasons`
/// ([`written_reasons`]): a reason that cites no roadmap step, an advisory without its
/// reason, a reason of no advisory ignored, and, when the key is `present`, an empty list.
fn audit_problems(ignored: &[(usize, &str)], reasons: &[Reason<'_>], present: bool) -> Vec<String> {
    let mut problems: Vec<String> = reasons
        .iter()
        .filter(|reason| {
            crate::citations::line_citations(&reason.text, false)
                .0
                .is_empty()
        })
        .map(|reason| {
            let unread = reason.unread.map_or_else(String::new, |(line, text)| {
                format!("; line {line}, `{text}`, is not read as its continuation")
            });
            format!(
                "{PNPM_WORKSPACE}:{}: the reason of `{}` cites no roadmap step (`#N step M`) \
                 that removes the exception (a reason is continued on the lines right under \
                 it opening with `{REASON_CONTINUED}`){unread}",
                reason.at, reason.id
            )
        })
        .collect();
    problems.extend(
        ignored
            .iter()
            .filter(|(_, id)| !reasons.iter().any(|reason| reason.id == *id))
            .map(|(at, id)| {
                format!(
                    "{PNPM_WORKSPACE}:{at}: `{id}` ignored by `pnpm audit` without its reason: \
                     write `# {id}: <reason>, removed by #N step M` at column 0"
                )
            }),
    );
    problems.extend(
        reasons
            .iter()
            .filter(|reason| !ignored.iter().any(|(_, known)| *known == reason.id))
            .map(|reason| {
                format!(
                    "{PNPM_WORKSPACE}:{}: the reason of `{}`, which `{AUDIT_CONFIG}` does not \
                     ignore: an exception's reason goes with it",
                    reason.at, reason.id
                )
            }),
    );
    if present && ignored.is_empty() {
        problems.push(format!(
            "{PNPM_WORKSPACE}: `{AUDIT_CONFIG}` ignores no advisory: remove the key, absent \
             while the register of exceptions is empty"
        ));
    }
    problems
}

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

/// The problem of `line`, line `at` under [`AUDIT_CONFIG`], if any: it is accepted as
/// [`IGNORED_ADVISORIES`] once, which sets `listing`, then as an [`IGNORED_ADVISORY`] of an
/// id not yet in `ignored`, which it joins. Any other line is refused, its kind said: a
/// comment (indented, or after an advisory: a reason is written at column 0), a line of
/// spaces, an advisory whose id is not as GitHub writes it, or a setting.
fn audit_line_problem<'text>(
    at: usize,
    line: &'text str,
    listing: &mut bool,
    ignored: &mut Vec<(usize, &'text str)>,
) -> Option<String> {
    if line == IGNORED_ADVISORIES && !*listing {
        *listing = true;
        None
    } else if let Some(id) = line
        .strip_prefix(IGNORED_ADVISORY)
        .filter(|id| *listing && advisory_id(id))
    {
        if ignored.iter().any(|(_, known)| *known == id) {
            Some(format!(
                "{PNPM_WORKSPACE}:{at}: `{id}` ignored a second time"
            ))
        } else {
            ignored.push((at, id));
            None
        }
    } else if line.trim_start().starts_with('#')
        || line.starts_with(IGNORED_ADVISORY) && line.contains(" #")
    {
        Some(format!(
            "{PNPM_WORKSPACE}:{at}: `{line}`, a comment under `{AUDIT_CONFIG}`: a reason is \
             written at column 0, `# <id>: <reason>`, continued on lines opening with \
             `{REASON_CONTINUED}`"
        ))
    } else if line.trim().is_empty() {
        Some(format!(
            "{PNPM_WORKSPACE}:{at}: a line of spaces under `{AUDIT_CONFIG}`: leave it empty"
        ))
    } else if *listing && line.starts_with(IGNORED_ADVISORY) {
        Some(format!(
            "{PNPM_WORKSPACE}:{at}: `{line}`, an advisory id not as GitHub writes it: `GHSA-` \
             then three groups of four characters of `{ADVISORY_ALPHABET}`, lower case, \
             joined by `-`"
        ))
    } else {
        Some(format!(
            "{PNPM_WORKSPACE}:{at}: `{line}`, not a line xtask accepts under `{AUDIT_CONFIG}` \
             (`{IGNORED_ADVISORIES}`, then one `{IGNORED_ADVISORY}GHSA-…` per advisory): a \
             setting of pnpm is a change of xtask"
        ))
    }
}

/// The problems of `text`, the pnpm workspace configuration: a top-level line (one starting
/// at column 0, comments apart) that is not a key of [`PNPM_WORKSPACE_KEYS`] or
/// [`AUDIT_CONFIG`], a key twice, a key of [`PNPM_WORKSPACE_KEYS`] missing, a release age
/// other than [`MINIMUM_RELEASE_AGE`], and an audit exception not in its one form or without
/// its reason ([`AUDIT_CONFIG`]).
pub(crate) fn pnpm_workspace_problems(text: &str) -> Vec<String> {
    let mut problems = Vec::new();
    let mut seen = Vec::new();
    let mut current = "";
    let mut listing = false;
    let mut ignored: Vec<(usize, &str)> = Vec::new();
    for (at, line) in (1_usize..).zip(text.lines()) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with(char::is_whitespace) {
            if current == AUDIT_CONFIG {
                problems.extend(audit_line_problem(at, line, &mut listing, &mut ignored));
            }
            continue;
        }
        let key = line.split_once(':').map_or(line, |(key, _)| key);
        current = key;
        if !PNPM_WORKSPACE_KEYS.contains(&key) && key != AUDIT_CONFIG {
            problems.push(format!(
                "{PNPM_WORKSPACE}:{at}: `{line}`, not a top-level key xtask accepts ({}, \
                 {AUDIT_CONFIG}): a setting of pnpm is a change of xtask",
                PNPM_WORKSPACE_KEYS.join(", ")
            ));
        } else if seen.contains(&key) {
            problems.push(format!("{PNPM_WORKSPACE}:{at}: `{key}` a second time"));
        } else if key == "minimumReleaseAge" && line != MINIMUM_RELEASE_AGE {
            problems.push(format!(
                "{PNPM_WORKSPACE}:{at}: `{line}`, where xtask requires `{MINIMUM_RELEASE_AGE}`"
            ));
        } else if key == AUDIT_CONFIG && line.strip_prefix(AUDIT_CONFIG) != Some(":") {
            problems.push(format!(
                "{PNPM_WORKSPACE}:{at}: `{line}`, where xtask accepts `{AUDIT_CONFIG}:` alone, \
                 its advisories on the lines under it"
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
    let (reasons, reason_problems) = written_reasons(text);
    problems.extend(reason_problems);
    problems.extend(audit_problems(
        &ignored,
        &reasons,
        seen.contains(&AUDIT_CONFIG),
    ));
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
