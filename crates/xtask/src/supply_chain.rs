//! The supply-chain configuration xtask owns, and its gate.

use crate::{
    steps::{Cmd, Dir, Failure, verdict},
    workspace::{CargoWorkspace, package_args},
};

/// The Dependabot configuration.
pub const DEPENDABOT: &str = ".github/dependabot.yml";

/// The pnpm workspace configuration.
pub const PNPM_WORKSPACE: &str = "pnpm-workspace.yaml";

/// The cargo-deny configuration.
pub const DENY_TOML: &str = "deny.toml";

/// The root manifest of the pnpm workspace.
pub const ROOT_PACKAGE_JSON: &str = "package.json";

/// The files outside [`PNPM_WORKSPACE`] from which pnpm reads settings or runs code.
///
/// As git pathspecs: npm configuration files, and pnpm's hooks files in either module form,
/// each refused wherever it is tracked ([`supply_chain_gate`]).
pub const PNPM_SETTINGS_FILES: [&str; 3] = ["*.npmrc", "*.pnpmfile.cjs", "*.pnpmfile.mjs"];

/// The files [`supply_chain_gate`] reads, in the order it takes their texts.
pub const SUPPLY_CHAIN_FILES: [&str; 6] = [
    DENY_TOML,
    DEPENDABOT,
    PNPM_WORKSPACE,
    ROOT_PACKAGE_JSON,
    GPU_DENY_TOML,
    WGSL_DENY_TOML,
];

/// The cargo-deny configuration of the GPU workspace
/// ([`GPU_WORKSPACE`](crate::workspace::GPU_WORKSPACE)), exactly [`gpu_deny_config`].
pub const GPU_DENY_TOML: &str = concat!(crate::workspace::gpu_workspace!(), "/deny.toml");

/// The first lines of [`gpu_deny_config`], in place of [`DENY_CONFIG`]'s two.
pub const GPU_DENY_HEADER: &str = r"# Supply-chain policy of the GPU workspace (cargo-deny): the root's (deny.toml), without its
# libm feature ban (wgpu's tree turns them on; xtask proves none reaches the CPU reference this
# workspace builds, `cpu_reference_gate`), with the exceptions of wgpu's tree.
# Exactly what xtask builds from the root's (`gpu_deny_config`).
";

/// What the GPU workspace's policy adds to the root's.
///
/// The licenses and duplicates of wgpu's tree, each allowed for its crate only, each duplicate at its exact version (the
/// maintainer's decision, 2026-11-16).
pub const GPU_DENY_EXCEPTIONS: &str = r#"# wgpu's tree: libloading (loads the Vulkan and DirectX libraries) is ISC, foldhash
# (hashbrown's hasher) Zlib, both permissive; allowed for those crates only.
[[licenses.exceptions]]
crate = "libloading"
allow = ["ISC"]

[[licenses.exceptions]]
crate = "foldhash"
allow = ["Zlib"]

# wgpu's tree holds two versions of each, which this workspace cannot choose: gpu-allocator's
# hashbrown beside naga's, zerocopy-derive's syn beside bytemuck_derive's. The older of each
# pair is skipped, at its exact version.
[[bans.skip]]
crate = "hashbrown@0.16.1"

[[bans.skip]]
crate = "syn@2.0.119"
"#;

/// The cargo-deny configuration of the WGSL proof suite's workspace
/// ([`WGSL_WORKSPACE`](crate::workspace::WGSL_WORKSPACE)), exactly [`wgsl_deny_config`].
pub const WGSL_DENY_TOML: &str = "tooling/wgsl/deny.toml";

/// The first lines of [`wgsl_deny_config`], in place of [`DENY_CONFIG`]'s two.
pub const WGSL_DENY_HEADER: &str = r"# Supply-chain policy of the WGSL tools workspace (cargo-deny): the root's (deny.toml), without
# its libm feature ban (no engine crate is built here), with the exceptions of wgpu's tree.
# Exactly what xtask builds from the root's (`wgsl_deny_config`).
";

/// What the WGSL tools workspace's policy adds to the root's.
///
/// Exactly the GPU workspace's ([`GPU_DENY_EXCEPTIONS`]): the WGSL tools build the GPU
/// renderer's wgpu and the packages beneath it, at the same versions
/// (`shared_gpu_versions_gate`).
pub const WGSL_DENY_EXCEPTIONS: &str = GPU_DENY_EXCEPTIONS;

/// The ban of `libm`'s features in [`DENY_CONFIG`], from its comment to the section after
/// it: the one part of the root's policy the GPU workspace does without, building no crate
/// of the engine.
const LIBM_BAN: (&str, &str) = ("# libm without any feature", "[sources]");

/// The cargo-deny configuration of the GPU workspace: `apart_deny_config` of
/// [`GPU_DENY_HEADER`] and [`GPU_DENY_EXCEPTIONS`].
#[must_use]
pub fn gpu_deny_config() -> String {
    apart_deny_config(GPU_DENY_HEADER, GPU_DENY_EXCEPTIONS)
}

/// The cargo-deny configuration of the WGSL tools workspace: `apart_deny_config` of
/// [`WGSL_DENY_HEADER`] and [`WGSL_DENY_EXCEPTIONS`].
#[must_use]
pub fn wgsl_deny_config() -> String {
    apart_deny_config(WGSL_DENY_HEADER, WGSL_DENY_EXCEPTIONS)
}

/// The cargo-deny configuration of a workspace apart.
///
/// [`DENY_CONFIG`] with `header` for its first two lines and without its ban of `libm`'s
/// features (`LIBM_BAN`), then `exceptions` after an empty line. Derived, so that a change
/// of the root's policy is every workspace's too.
fn apart_deny_config(header: &str, exceptions: &str) -> String {
    let body = DENY_CONFIG.splitn(3, '\n').nth(2).unwrap_or_default();
    let (before, rest) = body.split_once(LIBM_BAN.0).unwrap_or((body, ""));
    let after = rest
        .find(LIBM_BAN.1)
        .and_then(|at| rest.get(at..))
        .unwrap_or_default();
    format!("{header}{before}{after}\n{exceptions}")
}

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
pub const DEPENDABOT_CONFIG: &str = concat!(
    r"# Proposes dependency updates; every proposal goes through the full Quality workflow.
# Exactly xtask's `DEPENDABOT_CONFIG`: a change is made in both.
version: 2
updates:
  - package-ecosystem: cargo
    directories:
      - /
      - /",
    crate::workspace::gpu_workspace!(),
    r"
      - /",
    crate::workspace::wgsl_workspace!(),
    r"
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
",
);

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
/// reason on a comment line `# <id> (<package>): <reason>` at column 0, naming the package
/// the advisory is about, continued on the comment lines right under it that open with `#  `,
/// citing the roadmap step that removes it (`#N step M`); absent when the register is empty.
/// The license step refuses an advisory ignored in a package an app ships
/// (`shipped_exceptions_gate`, the license step's): an exception
/// covers development tools only. Any other audit setting (`ignoreCves`, an
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
    /// The package the advisory is about, as npm names it.
    package: &'text str,
    /// Its text, continuation lines joined.
    text: String,
    /// The comment line right under it that is not read as its continuation, with its
    /// number, if any: one that opens neither with [`REASON_CONTINUED`] nor another reason.
    unread: Option<(usize, &'text str)>,
}

/// The packages of the advisories `text`, the pnpm workspace configuration, gives a reason
/// for, each (line of its reason, advisory id, package), in the order of the file: those of
/// the reasons in their one form, which the supply-chain gate requires of every advisory
/// `pnpm audit` ignores ([`supply_chain_gate`]); a reason in another form, which that gate
/// refuses, names none.
pub(crate) fn ignored_packages(text: &str) -> Vec<(usize, &str, &str)> {
    written_reasons(text)
        .0
        .into_iter()
        .map(|reason| (reason.at, reason.id, reason.package))
        .collect()
}

/// Whether `text` is a package's name as npm writes it: lower-case letters, digits and
/// `-._~`, scoped or not (`@scope/name`).
fn package_name(text: &str) -> bool {
    let name = |part: &str| {
        !part.is_empty()
            && part.chars().all(|character| {
                character.is_ascii_lowercase()
                    || character.is_ascii_digit()
                    || "-._~".contains(character)
            })
    };
    text.strip_prefix('@').map_or_else(
        || name(text),
        |scoped| {
            scoped
                .split_once('/')
                .is_some_and(|(scope, rest)| name(scope) && name(rest))
        },
    )
}

/// The reasons of the ignored advisories `text` gives ([`Reason`]), and the problems of the
/// lines that open one ([`REASON_OPENING`]) and are not one.
///
/// A reason is the comment line `# <id> (<package>): <reason>`, at column 0 anywhere in the
/// file, the package as npm names it ([`package_name`]), the reason not empty, continued on
/// the comment lines right under it that open with [`REASON_CONTINUED`]; each advisory has
/// one.
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
            .and_then(|(head, reason)| {
                let (id, package) = head.strip_prefix("# ")?.split_once(" (")?;
                Some((id, package.strip_suffix(')')?, reason.trim()))
            })
            .filter(|(id, package, reason)| {
                advisory_id(id) && package_name(package) && !reason.is_empty()
            });
        match entry {
            None => problems.push(format!(
                "{PNPM_WORKSPACE}:{at}: `{line}`, where xtask reads a reason as `# GHSA-… \
                 (<package>): <reason>`, continued on lines opening with `{REASON_CONTINUED}`"
            )),
            Some((id, ..)) if reasons.iter().any(|reason| reason.id == id) => problems.push(
                format!("{PNPM_WORKSPACE}:{at}: the reason of `{id}` a second time"),
            ),
            Some((id, package, first)) => {
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
                    package,
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
                     write `# {id} (<package>): <reason>, removed by #N step M` at column 0"
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
             written at column 0, `# <id> (<package>): <reason>`, continued on lines opening \
             with `{REASON_CONTINUED}`"
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
/// [`MINIMUM_RELEASE_AGE`], and at most the register of [`AUDIT_CONFIG`] beside them, in its
/// one form; and [`ROOT_PACKAGE_JSON`] holds no `pnpm` key, from which pnpm
/// reads settings too. `settings_files` are the tracked files of [`PNPM_SETTINGS_FILES`],
/// each refused: pnpm reads settings in an npm configuration file (`.npmrc`:
/// `minimum-release-age-exclude` …), and runs the code of a hooks file (`.pnpmfile.cjs`,
/// `.pnpmfile.mjs`) at every install, which rewrites the manifests it resolves
/// (`readPackage`) and its settings (`updateConfig`). So a crate's advisory ignored (in
/// [`DENY_CONFIG`]), a crate skipped, a release age shortened or a package excluded from it
/// is a change of xtask, reviewed as such; an npm advisory ignored is a traced exception of
/// the maintainer, a line of [`PNPM_WORKSPACE`]'s register with its package, its reason and
/// the roadmap step that removes it, in the one form [`AUDIT_CONFIG`] documents.
///
/// # Errors
///
/// Returns every difference, unaccepted key, `pnpm` key, npm configuration file and pnpm
/// hooks file, or why the root manifest is not a JSON object.
pub fn supply_chain_gate(texts: &[String; 6], settings_files: &[String]) -> Result<(), Failure> {
    let [deny, dependabot, pnpm, package, gpu_deny, wgsl_deny] = texts;
    let mut problems = Vec::new();
    let gpu = gpu_deny_config();
    let wgsl = wgsl_deny_config();
    for (path, text, name, expected) in [
        (DENY_TOML, deny, "DENY_CONFIG", DENY_CONFIG),
        (
            DEPENDABOT,
            dependabot,
            "DEPENDABOT_CONFIG",
            DEPENDABOT_CONFIG,
        ),
        (GPU_DENY_TOML, gpu_deny, "gpu_deny_config", gpu.as_str()),
        (WGSL_DENY_TOML, wgsl_deny, "wgsl_deny_config", wgsl.as_str()),
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
    problems.extend(settings_files.iter().map(|path| {
        if path.contains(".pnpmfile.") {
            format!(
                "{path}: a pnpm hooks file, whose code pnpm runs at every install, rewriting \
                 the manifests it resolves (`readPackage`) and its settings (`updateConfig`): \
                 a setting goes in {PNPM_WORKSPACE}, whose keys xtask checks"
            )
        } else {
            format!(
                "{path}: an npm configuration file, from which pnpm reads settings \
                 (`minimum-release-age-exclude` …): they go in {PNPM_WORKSPACE}, whose keys \
                 xtask checks"
            )
        }
    }));
    verdict("the supply-chain configuration", problems)
}

#[cfg(test)]
#[path = "supply_chain_tests.rs"]
mod tests;

/// The crates of the CPU reference: what the GPU workspace builds of the visual library, and
/// what its tests compare the GPU's images with.
pub const CPU_REFERENCE: [&str; 2] = ["visual-core", "visual-boxes"];

/// The command printing the tree of the [`CPU_REFERENCE`] in `workspace`: its shipped
/// dependencies with their versions and features, one a line.
#[must_use]
pub fn cpu_reference_command(workspace: CargoWorkspace) -> Cmd {
    let packages = package_args(&CPU_REFERENCE);
    Cmd::new(
        "cargo",
        &[
            &["tree"],
            workspace.manifest_args(),
            &["-e", "normal,features", "--locked", "--prefix", "none"],
            &packages,
        ]
        .concat(),
        Dir::Root,
    )
}

/// Checks that the GPU workspace builds the CPU reference as the root does.
///
/// The trees of [`cpu_reference_command`] are the same in both (`root`, `gpu`), but the
/// mark of a feature the root's command line names, which the GPU workspace takes from its
/// dependency.
///
/// The GPU workspace drops the root's ban of libm's features (`gpu_deny_config`), which
/// wgpu's tree turns on: this proves, in its place, that none reaches the reference, nor
/// any other version or feature, so that the GPU's images are compared with the shipped
/// reference's.
///
/// # Errors
///
/// Returns both trees when they differ.
pub fn cpu_reference_gate(root: &str, gpu: &str) -> Result<(), Failure> {
    let read = |tree: &str| -> Vec<String> {
        tree.lines()
            .map(|line| line.trim_end_matches(" (command-line)").to_owned())
            .collect()
    };
    if read(root) == read(gpu) {
        return Ok(());
    }
    Err(Failure {
        summary: format!(
            "the GPU workspace builds the CPU reference ({}) otherwise than the root, so that \
             its tests would compare the GPU's images with another reference than the shipped \
             one",
            CPU_REFERENCE.join(", ")
        ),
        details: vec![
            "the root's tree:".to_owned(),
            root.to_owned(),
            "the GPU workspace's tree:".to_owned(),
            gpu.to_owned(),
        ],
        usage: false,
    })
}
