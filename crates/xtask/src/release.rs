//! Versions, the changelog and the release gates.

use crate::{
    changes::main_base,
    cli::file_at,
    preflight::NODE_FLOOR,
    runner::Runner,
    steps::{Failure, verdict},
};

/// The project manifest holding the workspace version.
pub const CARGO_TOML: &str = "Cargo.toml";

/// The changelog.
pub const CHANGELOG: &str = "CHANGELOG.md";

/// A semantic version `MAJOR.MINOR.PATCH` (no pre-release or build metadata).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    /// Incremented for incompatible changes.
    pub major: u64,
    /// Incremented for compatible additions.
    pub minor: u64,
    /// Incremented for compatible fixes.
    pub patch: u64,
}

impl Version {
    /// Parses `MAJOR.MINOR.PATCH`: digits only, no leading zero, as semantic versioning requires.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let number = |part: &str| {
            let digits = !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
            let leading_zero = part.len() > 1 && part.starts_with('0');
            if digits && !leading_zero {
                part.parse::<u64>().ok()
            } else {
                None
            }
        };
        let mut parts = text.split('.');
        let (Some(major), Some(minor), Some(patch), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return None;
        };
        Some(Self {
            major: number(major)?,
            minor: number(minor)?,
            patch: number(patch)?,
        })
    }
}

impl core::fmt::Display for Version {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Reads `version = "…"` in the `[workspace.package]` table of a `Cargo.toml`.
pub(crate) fn workspace_version(cargo_toml: &str) -> Option<&str> {
    cargo_toml
        .lines()
        .skip_while(|line| line.trim() != "[workspace.package]")
        .skip(1)
        .take_while(|line| !line.trim_start().starts_with('['))
        .find_map(|line| {
            let (key, value) = line.split_once('=')?;
            (key.trim() == "version").then(|| value.trim().trim_matches('"'))
        })
}

/// Parses a released changelog heading: `## [X.Y.Z] - YYYY-MM-DD`.
pub(crate) fn released_heading(heading: &str) -> Option<Version> {
    let rest = heading.strip_prefix("## [")?;
    let (version, date) = rest.split_once("] - ")?;
    let mut parts = date.split('-');
    let well_formed = [4, 2, 2].iter().all(|&len| {
        parts
            .next()
            .is_some_and(|part| part.len() == len && part.bytes().all(|byte| byte.is_ascii_digit()))
    }) && parts.next().is_none();
    if well_formed {
        Version::parse(version)
    } else {
        None
    }
}

/// Checks that the project has one version everywhere and a well-formed changelog.
///
/// The workspace version and the version of every package manifest must be equal, valid semantic
/// versions, and not older than the latest release in the changelog; every package manifest
/// supports Node.js from [`NODE_FLOOR`] on (`"engines": { "node": ">=22.18" }`). The changelog
/// starts with `# Changelog`, has `## [Unreleased]` first, then releases `## [X.Y.Z] - YYYY-MM-DD`
/// in strictly decreasing order.
///
/// # Errors
///
/// Returns a message listing every problem found.
pub fn release_gate(
    cargo_toml: &str,
    packages: &[(String, String)],
    changelog: &str,
) -> Result<(), Failure> {
    let mut problems = Vec::new();
    let workspace = workspace_version(cargo_toml);
    let version = workspace.and_then(Version::parse);
    if version.is_none() {
        problems.push(format!(
            "{CARGO_TOML}: [workspace.package] version must be X.Y.Z"
        ));
    }
    let floor = format!(">={}.{}", NODE_FLOOR.0, NODE_FLOOR.1);
    for (path, manifest) in packages {
        let json = serde_json::from_str::<serde_json::Value>(manifest).ok();
        let field = |pointer: &str| {
            json.as_ref()
                .and_then(|value| value.pointer(pointer))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        };
        let engine = field("/engines/node");
        if engine.as_deref() != Some(floor.as_str()) {
            problems.push(format!(
                "{path}: engines.node is {}, the packages support Node.js {floor} (NODE_FLOOR)",
                engine.as_deref().unwrap_or("(missing)")
            ));
        }
        let package = field("/version");
        if package.as_deref() != workspace {
            problems.push(format!(
                "{path}: version {} differs from the workspace version {}",
                package.as_deref().unwrap_or("(missing)"),
                workspace.unwrap_or("(missing)")
            ));
        }
    }
    if changelog.lines().next() != Some("# Changelog") {
        problems.push(format!("{CHANGELOG}: must start with \"# Changelog\""));
    }
    let headings: Vec<&str> = changelog
        .lines()
        .filter(|line| line.starts_with("## "))
        .collect();
    if headings.first() != Some(&"## [Unreleased]") {
        problems.push(format!(
            "{CHANGELOG}: the first section must be \"## [Unreleased]\""
        ));
    }
    let mut releases = Vec::new();
    for heading in headings.iter().skip(1) {
        match released_heading(heading) {
            Some(release) => releases.push(release),
            None => problems.push(format!("{CHANGELOG}: malformed heading \"{heading}\"")),
        }
    }
    if releases.windows(2).any(|pair| pair.first() <= pair.get(1)) {
        problems.push(format!(
            "{CHANGELOG}: releases must be in strictly decreasing order"
        ));
    }
    if let (Some(current), Some(latest)) = (version, releases.first())
        && current < *latest
    {
        problems.push(format!(
            "version {current} is older than the latest release {latest}"
        ));
    }
    verdict("the versions and the changelog", problems)
}

/// The released sections of a changelog: from each heading [`released_heading`] reads to
/// the next heading of its level, each with the number of its first line and its bytes, line
/// endings included.
pub(crate) fn released_section_texts(changelog: &str) -> Vec<(Version, usize, String)> {
    let mut sections = Vec::new();
    let mut open: Option<(Version, usize, String)> = None;
    for (index, line) in changelog.split_inclusive('\n').enumerate() {
        if line.starts_with("## ") {
            sections.extend(open.take());
            open = released_heading(line.trim_end_matches(['\n', '\r']))
                .map(|version| (version, index.saturating_add(1), String::new()));
        }
        if let Some((_, _, text)) = open.as_mut() {
            text.push_str(line);
        }
    }
    sections.extend(open);
    sections
}

/// The part of a changelog before its first released section (the first heading
/// [`released_heading`] reads): the whole changelog when it has none.
pub(crate) fn unreleased(changelog: &str) -> &str {
    let mut end = 0_usize;
    for line in changelog.split_inclusive('\n') {
        if released_heading(line.trim_end_matches(['\n', '\r'])).is_some() {
            break;
        }
        end = end.saturating_add(line.len());
    }
    changelog.get(..end).unwrap_or(changelog)
}

/// Checks that every section released in `at_base` is in `changelog` byte for byte.
///
/// `at_base` is the changelog at the merge base with `main`: a released section is never
/// edited, so that the release notes of a version are what reached `main` (a section a
/// version commit adds is new, and accepted). `base` is the merge base's commit and the
/// branch it was found with, named in the messages with the command that shows the
/// sections as released.
///
/// A section crafted below the oldest release with no blank line before its heading reads as
/// part of that release and passes; it is out of scope, since no honest edit writes one. The
/// changelog takes no link-reference footer: the oldest section runs to the end of the file,
/// so a footer added later is refused as an edit of it.
///
/// # Errors
///
/// Returns every released section of `at_base` missing from `changelog`, and every one
/// that differs, with the first line where it does.
pub fn released_sections_gate(
    at_base: Option<&str>,
    changelog: &str,
    (base, against): (&str, &str),
) -> Result<(), Failure> {
    let now = released_section_texts(changelog);
    let restore = format!("restore it from `git show {base}:{CHANGELOG}`");
    let mut problems = Vec::new();
    for (version, _, released) in released_section_texts(at_base.unwrap_or_default()) {
        match now.iter().find(|(held, ..)| *held == version) {
            None => problems.push(format!(
                "{CHANGELOG}: the released section [{version}] of {base} (merge base with \
                 {against}) is missing: {restore}"
            )),
            Some((_, first, text)) if *text != released => {
                let same = text
                    .split_inclusive('\n')
                    .zip(released.split_inclusive('\n'))
                    .take_while(|(line, kept)| line == kept)
                    .count();
                problems.push(format!(
                    "{CHANGELOG}:{}: the released section [{version}] differs from the one at \
                     {base} (merge base with {against}): a released section is never edited: \
                     {restore}",
                    first.saturating_add(same)
                ));
            }
            Some(_) => {}
        }
    }
    verdict("the released changelog sections", problems)
}

/// Reads the changelog at the merge base with `main` ([`main_base`]), then checks that
/// `changelog` keeps its released sections ([`released_sections_gate`]).
///
/// # Errors
///
/// Returns why there is no merge base, why git cannot give the changelog there, or a
/// released section edited.
pub(crate) fn released_sections(runner: &mut dyn Runner, changelog: &str) -> Result<(), Failure> {
    let (base, against) = main_base(runner, "the released changelog sections")?;
    let at_base = file_at(runner, &base, CHANGELOG)?;
    released_sections_gate(at_base.as_deref(), changelog, (&base, against))
}
