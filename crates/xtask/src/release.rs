//! Versions, the changelog and the release gates.

use alloc::collections::BTreeSet;

use crate::{
    actions::git_output,
    changes::main_base,
    cli::file_at,
    history::BREAKING_ENTRY,
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

/// The word every changelog holds, in its title (`# Changelog`, which [`release_gate`]
/// requires): searched with the names of [`breaking_names`] so that git finds at least one
/// word at the merge base, a search finding nothing being an exit code git shares with its
/// failures.
const CHANGELOG_TITLE: &str = "Changelog";

/// The breaking entries of the `[Unreleased]` section of `changelog`, each with its line
/// number and the names it cites: of each code span, the last identifier before any `<` or
/// `(` (`audio_wasm::Engine` names `Engine`, `Block<S, N>` names `Block`, a span with no
/// identifier, a number, none).
pub(crate) fn breaking_entries(changelog: &str) -> Vec<(usize, Vec<String>)> {
    unreleased(changelog)
        .lines()
        .enumerate()
        .filter(|(_, line)| line.starts_with(BREAKING_ENTRY))
        .map(|(index, line)| {
            let names = line
                .split('`')
                .skip(1)
                .step_by(2)
                .filter_map(|span| {
                    span.split(['<', '('])
                        .next()
                        .unwrap_or_default()
                        .split(|character: char| !(character.is_alphanumeric() || character == '_'))
                        .rfind(|word| {
                            word.chars()
                                .next()
                                .is_some_and(|first| first.is_alphabetic() || first == '_')
                        })
                        .map(str::to_owned)
                })
                .collect();
            (index.saturating_add(1), names)
        })
        .collect()
}

/// The first words two entries of `[Unreleased]` share for [`duplicate_entries_gate`] to
/// take them for one change.
///
/// Entries of two changes of one function have opened alike for fewer, the copies a rebase
/// kept for many more (the cases of its tests, taken from the history of the changelog).
pub const SAME_OPENING: usize = 8;

/// The code spans two entries of `[Unreleased]` cite alike for [`duplicate_entries_gate`]
/// to take them for one change.
///
/// They must also make two thirds of the spans either cites: entries of two changes of one
/// function have cited as many alike, at a smaller share (the cases of its tests, taken
/// from the history of the changelog).
pub const SAME_NAMES: usize = 3;

/// The entries of the `[Unreleased]` section of `changelog`, each with its line number and
/// without its bullet: the lines starting `- `, a line indented below one being part of it.
fn unreleased_entries(changelog: &str) -> Vec<(usize, &str)> {
    unreleased(changelog)
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            line.strip_prefix("- ")
                .map(|entry| (index.saturating_add(1), entry))
        })
        .collect()
}

/// The code spans `entry` cites, each once.
fn code_spans(entry: &str) -> BTreeSet<&str> {
    entry.split('`').skip(1).step_by(2).collect()
}

/// Why two entries of `[Unreleased]` read as one change, if they do: they open with the
/// same [`SAME_OPENING`] words, or cite at least [`SAME_NAMES`] code spans alike, two
/// thirds or more of the spans either cites.
fn same_change(first: &str, second: &str) -> Option<String> {
    let opening = first
        .split_whitespace()
        .zip(second.split_whitespace())
        .take_while(|(word, other)| word == other)
        .count();
    if opening >= SAME_OPENING {
        return Some(format!("both open with the same {SAME_OPENING} words"));
    }
    let (names, others) = (code_spans(first), code_spans(second));
    let shared = names.intersection(&others).count();
    let either = names.union(&others).count();
    (shared >= SAME_NAMES && shared.saturating_mul(3) >= either.saturating_mul(2))
        .then(|| format!("both cite {shared} of the {either} code spans either cites"))
}

/// Checks that `[Unreleased]` holds one entry per change.
///
/// `[Unreleased]` describes each change from the last version once: an entry for a state
/// changed again before the release is rewritten, not followed by another, and a rebase
/// that keeps an older wording beside its rewrite leaves two. Two entries are taken for one
/// change when they open alike or cite mostly the same items (`same_change`), across
/// the kinds of change (a change added, then changed, is one entry). The test is a
/// heuristic on the words and names of the entries: two entries of one change worded and
/// named apart pass, which only review sees.
///
/// # Errors
///
/// Returns every later entry that repeats an earlier one, with both lines and why.
pub fn duplicate_entries_gate(changelog: &str) -> Result<(), Failure> {
    let entries = unreleased_entries(changelog);
    let mut problems = Vec::new();
    for (at, (line, entry)) in entries.iter().enumerate() {
        let earlier = entries.get(..at).unwrap_or_default();
        if let Some((first, why)) = earlier
            .iter()
            .find_map(|(first, other)| same_change(other, entry).map(|why| (first, why)))
        {
            problems.push(format!(
                "{CHANGELOG}:{line}: this entry and the one at line {first} describe one \
                 change ({why}): keep one entry per change, saying it from the last version"
            ));
        }
    }
    verdict("the entries of [Unreleased]", problems)
}

/// Checks that every breaking entry of `[Unreleased]` names something the last version has.
///
/// `[Unreleased]` describes the change from the last version, whose version commit is the
/// merge base with `main` (`base`, found with `against`): a breaking entry changes what that
/// version holds, so at least one name it cites (of each code span, the last identifier before any `<` or `(`) is in `found`, the
/// names git finds as words in the tree of `base`. An entry citing only names that version
/// never had breaks nothing released. The test is by word, anywhere in the tree: a name
/// the version held in another sense passes, which only review sees.
///
/// # Errors
///
/// Returns every breaking entry none of whose names is in `found`, with its line.
pub fn breaking_names_gate(
    changelog: &str,
    found: &BTreeSet<String>,
    (base, against): (&str, &str),
) -> Result<(), Failure> {
    let problems = breaking_entries(changelog)
        .into_iter()
        .filter(|(_, names)| !names.iter().any(|name| found.contains(name)))
        .map(|(line, names)| {
            format!(
                "{CHANGELOG}:{line}: a breaking entry cites no name that {base} (merge base with \
                 {against}) holds ({}): name what it changes as the last version has it \
                 (`git show {base}:<file>`), or it breaks nothing released",
                if names.is_empty() {
                    "it cites none".to_owned()
                } else {
                    names.join(", ")
                }
            )
        })
        .collect();
    verdict("the breaking entries of the changelog", problems)
}

/// Searches the tree of the merge base with `main` ([`main_base`]) for the names the
/// breaking entries of `changelog` cite, then checks each entry ([`breaking_names_gate`]);
/// nothing to search when there is no breaking entry.
///
/// # Errors
///
/// Returns why there is no merge base, why git cannot search its tree, or an entry citing
/// nothing it holds.
pub(crate) fn breaking_names(runner: &mut dyn Runner, changelog: &str) -> Result<(), Failure> {
    let names: BTreeSet<String> = breaking_entries(changelog)
        .into_iter()
        .flat_map(|(_, names)| names)
        .collect();
    if names.is_empty() {
        return Ok(());
    }
    let (base, against) = main_base(runner, "the names the breaking entries cite")?;
    let mut args = vec![
        "grep",
        "--color=never",
        "--only-matching",
        "-h",
        "--word-regexp",
        "--fixed-strings",
        "-e",
        CHANGELOG_TITLE,
    ];
    for name in &names {
        args.extend(["-e", name.as_str()]);
    }
    args.extend([base.as_str(), "--"]);
    let words = git_output(
        runner,
        &args,
        &format!("search the names the breaking entries cite at the merge base {base}"),
    )?;
    let found = words.lines().map(str::to_owned).collect();
    breaking_names_gate(changelog, &found, (&base, against))
}

#[cfg(test)]
#[path = "release_tests.rs"]
mod tests;
