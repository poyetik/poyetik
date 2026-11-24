//! Versions, the changelog and the release gates.

use alloc::collections::BTreeSet;

use crate::{
    actions::git_output,
    changes::main_base,
    cli::file_at,
    history::BREAKING_ENTRY,
    preflight::NODE_FLOOR,
    runner::Runner,
    steps::{Failure, all_of, verdict},
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
    let restore = format!("copy it back as {base} holds it (see: git show {base}:{CHANGELOG})");
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

/// The breaking entries of the `[Unreleased]` section of `changelog` (those of
/// [`unreleased_entries`] that open [`BREAKING_ENTRY`]), each with its line number and the
/// names it cites: of each code span, the last identifier before any `<` or `(`
/// (`audio_wasm::Engine` names `Engine`, `Block<S, N>` names `Block`, a span with no
/// identifier, a number, none).
pub(crate) fn breaking_entries(changelog: &str) -> Vec<(usize, Vec<String>)> {
    unreleased_entries(changelog)
        .into_iter()
        .filter(|(_, entry)| breaking(entry))
        .map(|(line, entry)| (line, cited_names(&entry)))
        .collect()
}

/// The names `entry` cites: of each code span, the last identifier before any `<` or `(`
/// (`audio_wasm::Engine` names `Engine`, `Block<S, N>` names `Block`, a span with no
/// identifier, a number, none).
fn cited_names(entry: &str) -> Vec<String> {
    names(spans(entry))
}

/// The names `spans`, code spans, cite ([`cited_names`]).
fn names<'text>(spans: impl Iterator<Item = &'text str>) -> Vec<String> {
    spans
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
        .collect()
}

/// The entries of `[Unreleased]` under its heading `### <kind>`, each with its line number.
fn kind_entries(changelog: &str, of: &str) -> Vec<(usize, String)> {
    let mut lines = BTreeSet::new();
    let mut kind = "";
    for (line, text) in unreleased_lines(changelog) {
        match shape(text) {
            Shape::Kind => kind = text.trim_start_matches("### "),
            Shape::Entry(_) if kind == of => {
                let _new = lines.insert(line);
            }
            Shape::Entry(_) | Shape::Blank | Shape::Continued(_) | Shape::Other => {}
        }
    }
    unreleased_entries(changelog)
        .into_iter()
        .filter(|(line, _)| lines.contains(line))
        .collect()
}

/// The entries of `[Unreleased]` under its heading `### Removed`, each with its line number
/// and the names it cites ([`cited_names`]).
pub(crate) fn removed_entries(changelog: &str) -> Vec<(usize, Vec<String>)> {
    kind_entries(changelog, "Removed")
        .into_iter()
        .map(|(line, entry)| (line, cited_names(&entry)))
        .collect()
}

/// How a code span names the check itself, whatever its flags: its words, which every version
/// holds, are no name of a changed or fixed entry ([`changed_entries`]), nor a start of them
/// (`` `cargo xtask` ``).
const THE_CHECK: &str = "cargo xtask check";

/// The entries of `[Unreleased]` under its headings `### Changed` and `### Fixed`, the
/// breaking ones apart ([`breaking_entries`], which a gate of their own reads), each with its
/// line number and the names it cites ([`cited_names`]), the check's own ([`THE_CHECK`]) apart.
pub(crate) fn changed_entries(changelog: &str) -> Vec<(usize, Vec<String>)> {
    let breaking: BTreeSet<usize> = breaking_entries(changelog)
        .into_iter()
        .map(|(line, _)| line)
        .collect();
    let mut entries = kind_entries(changelog, "Changed");
    entries.extend(kind_entries(changelog, "Fixed"));
    entries
        .into_iter()
        .filter(|(line, _)| !breaking.contains(line))
        .map(|(line, entry)| {
            let cited = spans(&entry)
                .filter(|span| !(span.starts_with(THE_CHECK) || THE_CHECK.starts_with(span)));
            (line, names(cited))
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

/// The run of consecutive words two entries of `[Unreleased]` share for
/// [`duplicate_entries_gate`] to take them for one change, wherever it stands in each.
///
/// Measured on 2026-11-20 over every pair of entries of each section of the changelog: the
/// pair a block kept (one sentence in an added entry and in its breaking rewrite) shares
/// 31 words, while the closest distinct pairs of `[Unreleased]` share 13, 12 and 10, and
/// those of 0.7.0 and 0.6.0 at most 8 and 7: 16 refuses the first with a margin above all
/// the others.
pub const SAME_RUN: usize = 16;

/// The kinds of change of Keep a Changelog, the headings `### <kind>` of `[Unreleased]`.
pub const CHANGE_KINDS: [&str; 6] = [
    "Added",
    "Changed",
    "Deprecated",
    "Removed",
    "Fixed",
    "Security",
];

/// The heading of the section of changes not released yet.
const UNRELEASED_HEADING: &str = "## [Unreleased]";

/// The lines of the `[Unreleased]` section of `changelog`, each with its line number: from
/// the line after its heading to the next heading of its level (the first released
/// section, which [`release_gate`] requires to follow it), none without that heading.
fn unreleased_lines(changelog: &str) -> impl Iterator<Item = (usize, &str)> {
    changelog
        .lines()
        .enumerate()
        .map(|(index, line)| (index.saturating_add(1), line))
        .skip_while(|(_, line)| *line != UNRELEASED_HEADING)
        .skip(1)
        .take_while(|(_, line)| !line.starts_with("## "))
}

/// What a line of `[Unreleased]` is, read by [`unreleased_shape_gate`] and
/// [`unreleased_entries`] alike.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape<'line> {
    /// Nothing but white space.
    Blank,
    /// A heading `### <kind>`, a kind of [`CHANGE_KINDS`].
    Kind,
    /// The first line of an entry, `- ` then its text, which this holds.
    Entry(&'line str),
    /// A line indented below an entry, part of it (its text wrapped, a list inside it):
    /// this holds it without its indentation.
    Continued(&'line str),
    /// Anything else, which the gate refuses.
    Other,
}

/// The shape of `line` ([`Shape`]).
fn shape(line: &str) -> Shape<'_> {
    if line.trim().is_empty() {
        return Shape::Blank;
    }
    if line
        .strip_prefix("### ")
        .is_some_and(|kind| CHANGE_KINDS.contains(&kind))
    {
        return Shape::Kind;
    }
    if let Some(entry) = line
        .strip_prefix("- ")
        .filter(|entry| entry.starts_with(|first: char| !first.is_whitespace()))
    {
        return Shape::Entry(entry);
    }
    if line.starts_with([' ', '\t']) {
        return Shape::Continued(line.trim_start());
    }
    Shape::Other
}

/// The entries of the `[Unreleased]` section of `changelog`, each with its line number and
/// its text: its first line without its bullet, then each line indented below it, without
/// its indentation, joined by a space (a code span wrapped across lines stays one span).
///
/// It is the one reader of the entries: the gates of `[Unreleased]` read them through it,
/// and [`unreleased_shape_gate`] refuses every line it does not read (another bullet, a
/// line of no entry), from the same pass ([`read_unreleased`]).
pub(crate) fn unreleased_entries(changelog: &str) -> Vec<(usize, String)> {
    read_unreleased(changelog).0
}

/// One pass over the lines of `[Unreleased]` ([`Shape`]): its entries, as
/// [`unreleased_entries`] gives them, and the lines no entry holds, as
/// [`unreleased_shape_gate`] refuses them, so that the reader and the gate cannot disagree
/// on what continues an entry.
fn read_unreleased(changelog: &str) -> (Vec<(usize, String)>, Vec<String>) {
    let mut entries: Vec<(usize, String)> = Vec::new();
    let mut problems = Vec::new();
    let mut open: Option<(usize, String)> = None;
    // The first blank line since the open entry's last line, if any: an entry after it is
    // the same list's, split in two.
    let mut blank: Option<usize> = None;
    for (line, text) in unreleased_lines(changelog) {
        let shape = shape(text);
        if let (Shape::Entry(_), Some(at), Some(_)) = (shape, blank, open.as_ref()) {
            problems.push(format!(
                "{CHANGELOG}:{at}: a blank line between two entries of one section: the \
                 entries of a section follow each other, one list"
            ));
        }
        blank = match shape {
            Shape::Blank => blank.or(Some(line)),
            Shape::Entry(_) | Shape::Continued(_) | Shape::Kind | Shape::Other => None,
        };
        match (shape, open.as_mut()) {
            (Shape::Entry(entry), _) => entries.extend(open.replace((line, entry.to_owned()))),
            (Shape::Continued(more), Some((_, entry))) => {
                entry.push(' ');
                entry.push_str(more);
            }
            (Shape::Kind, _) => entries.extend(open.take()),
            (Shape::Blank, _) => {}
            (Shape::Continued(_), None) | (Shape::Other, _) => {
                entries.extend(open.take());
                problems.push(format!(
                    "{CHANGELOG}:{line}: a line of [Unreleased] is blank, a heading `### <kind>` \
                     ({}), an entry `- <text>`, or a line indented below an entry",
                    CHANGE_KINDS.join(", ")
                ));
            }
        }
    }
    entries.extend(open);
    (entries, problems)
}

/// Whether `entry`, an entry's text without its bullet ([`unreleased_entries`]), opens
/// [`BREAKING_ENTRY`], the one form the gates of breaking changes read.
fn breaking(entry: &str) -> bool {
    BREAKING_ENTRY
        .strip_prefix("- ")
        .is_some_and(|form| entry.starts_with(form))
}

/// Checks that every line of `[Unreleased]` has one of the shapes its gates read.
///
/// A line is blank, a heading `### <kind>` (one of [`CHANGE_KINDS`]), the first line of an
/// entry, `- ` then its text, or a line indented below an entry, part of it; and no blank line
/// stands between two entries of one section, which would make two lists of it that a reader
/// takes for two sections. So [`BREAKING_ENTRY`],
/// the one form the gates of breaking changes read, is the form of every entry: an entry under
/// another bullet (`* `, `+ `, `1. `), which an editor's list style writes, would otherwise pass
/// every gate of `[Unreleased]` unread.
///
/// # Errors
///
/// Returns every other line, and every blank line between two entries of one section, with
/// its number.
pub fn unreleased_shape_gate(changelog: &str) -> Result<(), Failure> {
    verdict("the shape of [Unreleased]", read_unreleased(changelog).1)
}

/// The code spans of `entry`, in their order: the text between each pair of backquotes.
fn spans(entry: &str) -> impl Iterator<Item = &str> {
    entry.split('`').skip(1).step_by(2)
}

/// The code spans `entry` cites, each once.
fn code_spans(entry: &str) -> BTreeSet<&str> {
    spans(entry).collect()
}

/// Why two entries of `[Unreleased]` read as one change, if they do: they open with the
/// same [`SAME_OPENING`] words, share a run of [`SAME_RUN`] consecutive words anywhere, or
/// cite at least [`SAME_NAMES`] code spans alike, two thirds or more of the spans either
/// cites.
fn same_change(first: &str, second: &str) -> Option<String> {
    let opening = first
        .split_whitespace()
        .zip(second.split_whitespace())
        .take_while(|(word, other)| word == other)
        .count();
    if opening >= SAME_OPENING {
        return Some(format!("both open with the same {SAME_OPENING} words"));
    }
    let words: Vec<&str> = first.split_whitespace().collect();
    let runs: BTreeSet<&[&str]> = words.windows(SAME_RUN).collect();
    let second_words: Vec<&str> = second.split_whitespace().collect();
    if second_words.windows(SAME_RUN).any(|run| runs.contains(run)) {
        return Some(format!("both hold the same run of {SAME_RUN} words"));
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
/// change when they open alike, share a long run of words or cite mostly the same items
/// (`same_change`), across
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

/// Checks that every entry of `[Unreleased]` that says it breaks is written
/// [`BREAKING_ENTRY`].
///
/// That is the form the gates of breaking changes read ([`breaking_names_gate`],
/// `breaking_bang_gate`): an entry whose first word, past any bold or italics (`*`, `_`), is
/// "breaking" in another case or form (`- **BREAKING:**`, `- Breaking:`,
/// `- __Breaking:__`), or that marks itself breaking at its end (`(breaking)`,
/// `(breaking change)`, a last word "breaking"), would pass both unread. An entry under another bullet, or with no
/// space after its bullet, is refused by [`unreleased_shape_gate`].
///
/// # Errors
///
/// Returns every such entry, with its line.
pub fn breaking_spellings_gate(changelog: &str) -> Result<(), Failure> {
    let problems = unreleased_entries(changelog)
        .into_iter()
        .filter(|(_, entry)| {
            let lower = entry.to_ascii_lowercase();
            // Its first word, past any bold or italics, or a mark of it at its end, as an
            // honest edit writes it: "(breaking)", "(breaking change)", ", breaking.".
            let first = lower.trim_start_matches(['*', '_']).starts_with("breaking");
            let marked = lower.contains("(breaking")
                || lower
                    .trim_end()
                    .trim_end_matches(['.', ')'])
                    .ends_with("breaking");
            (first || marked) && !breaking(entry)
        })
        .map(|(line, _)| {
            format!(
                "{CHANGELOG}:{line}: a breaking entry starts `{BREAKING_ENTRY}`, which the gates \
                 of breaking changes read"
            )
        })
        .collect();
    verdict("the breaking entries of the changelog", problems)
}

/// Checks that every breaking entry of `[Unreleased]` names something the last version has,
/// and says each name it cites that the last version lacks in an entry under `### Added`.
///
/// `[Unreleased]` describes the change from the last version, whose version commit is the
/// merge base with `main` (`base`, found with `against`): a breaking entry changes what that
/// version holds, so at least one name it cites (of each code span, the last identifier before any
/// `<` or `(`) is in `found`, the names git finds as words in the tree of `base`. An entry citing
/// only names that version never had breaks nothing released. Every other name it cites is new
/// in the release (what replaces the old, a variant gained), so an entry under `### Added` cites
/// it too: what the branch adds is said where a reader of the release looks for it, never only
/// among what breaks. The test is by word, anywhere in the tree: a name the version held in
/// another sense passes, which only review sees; a code span with no identifier (a number, an
/// address) names nothing.
///
/// # Errors
///
/// Returns every breaking entry none of whose names is in `found`, and every one citing a name
/// neither in `found` nor cited by an added entry, with its line and those names.
pub fn breaking_names_gate(
    changelog: &str,
    found: &BTreeSet<String>,
    (base, against): (&str, &str),
) -> Result<(), Failure> {
    let added: BTreeSet<String> = kind_entries(changelog, "Added")
        .into_iter()
        .flat_map(|(_, entry)| cited_names(&entry))
        .collect();
    let problems = breaking_entries(changelog)
        .into_iter()
        .filter_map(|(line, names)| {
            if !names.iter().any(|name| found.contains(name)) {
                return Some(format!(
                    "{CHANGELOG}:{line}: a breaking entry cites no name that {base} (merge base \
                     with {against}) holds ({}): name what it changes as the last version has \
                     it (see: git diff --stat {base}), or it breaks nothing released",
                    if names.is_empty() {
                        "it cites none".to_owned()
                    } else {
                        names.join(", ")
                    }
                ));
            }
            let mut new: Vec<String> = Vec::new();
            for name in names {
                if !found.contains(&name) && !added.contains(&name) && !new.contains(&name) {
                    new.push(name);
                }
            }
            (!new.is_empty()).then(|| {
                format!(
                    "{CHANGELOG}:{line}: a breaking entry cites {}, which {base} (merge base with \
                     {against}) does not hold and no entry under `### Added` cites: say what the \
                     release adds in its added entry, and in the breaking entry what changes of \
                     the last version",
                    new.join(", ")
                )
            })
        })
        .collect();
    verdict("the breaking entries of the changelog", problems)
}

/// Checks that every removed entry of `[Unreleased]` names only what the last version has.
///
/// `[Unreleased]` describes the change from the last version, whose version commit is the
/// merge base with `main` (`base`, found with `against`): an entry under `### Removed`
/// removes what that version held, so every name it cites (`removed_entries`) is in
/// `found`, the names git finds as words in the tree of `base`. A name only the branch
/// held, added then removed before the release, is no change of the release: its entry
/// belongs to another kind, or nowhere. The test is by word, anywhere in the tree: a name
/// the version held in another sense passes, which only review sees.
///
/// # Errors
///
/// Returns every removed entry citing a name not in `found`, with its line and those names.
pub fn removed_names_gate(
    changelog: &str,
    found: &BTreeSet<String>,
    (base, against): (&str, &str),
) -> Result<(), Failure> {
    let problems = removed_entries(changelog)
        .into_iter()
        .filter_map(|(line, names)| {
            let mut missing: Vec<String> = Vec::new();
            for name in names {
                if !found.contains(&name) && !missing.contains(&name) {
                    missing.push(name);
                }
            }
            (!missing.is_empty()).then(|| {
                format!(
                    "{CHANGELOG}:{line}: a removed entry cites {}, which {base} (merge base with \
                     {against}) does not hold: [Unreleased] removes only what the last version \
                     had (see: git diff --stat {base})",
                    missing.join(", ")
                )
            })
        })
        .collect();
    verdict("the removed entries of the changelog", problems)
}

/// Checks that every changed or fixed entry of `[Unreleased]` names something the last
/// version has.
///
/// `[Unreleased]` describes the change from the last version, whose version commit is the
/// merge base with `main` (`base`, found with `against`): an entry under `### Changed` or
/// `### Fixed` (the breaking ones apart, [`breaking_names_gate`]) says by name what it
/// changes as that version has it, so at least one name it cites is in `found`, the names
/// git finds as words in the tree of `base`, those of a span naming `cargo xtask check` apart.
/// An entry citing none, or only names that version never had, describes what the release
/// adds: it belongs to an entry under `### Added`. Its other names may be new (what the change adds to what it changes), and
/// the test is by word, anywhere in the tree: an entry that changes one thing the version
/// held and one it did not passes, which only review sees.
///
/// # Errors
///
/// Returns every such entry none of whose names is in `found`, with its line and its names.
pub fn changed_names_gate(
    changelog: &str,
    found: &BTreeSet<String>,
    (base, against): (&str, &str),
) -> Result<(), Failure> {
    let problems = changed_entries(changelog)
        .into_iter()
        .filter(|(_, names)| !names.iter().any(|name| found.contains(name)))
        .map(|(line, names)| {
            format!(
                "{CHANGELOG}:{line}: a changed or fixed entry cites no name that {base} (merge \
                 base with {against}) holds ({}): name what it changes as the last version has \
                 it (see: git diff --stat {base}), or say it in the entry under `### Added` of \
                 what the release adds",
                if names.is_empty() {
                    "it cites none, the check itself apart".to_owned()
                } else {
                    names.join(", ")
                }
            )
        })
        .collect();
    verdict("the changed and fixed entries of the changelog", problems)
}

/// Searches the tree of the merge base with `main` ([`main_base`]) for the names the
/// breaking, the removed, the changed and the fixed entries of `changelog` cite, then checks
/// each entry ([`breaking_names_gate`], [`removed_names_gate`], [`changed_names_gate`]);
/// nothing to search when there is no such entry (entries citing no name are judged too). The
/// search prints the names alone, whatever the user's configuration
/// (`grep.lineNumber` and `grep.column` turned off on its command line).
///
/// # Errors
///
/// Returns why there is no merge base, why git cannot search its tree, a breaking entry
/// citing nothing it holds, or a name neither it holds nor an entry under `### Added` cites, a
/// removed entry citing a name it does not hold, or a changed or
/// fixed entry citing nothing it holds.
pub(crate) fn breaking_names(runner: &mut dyn Runner, changelog: &str) -> Result<(), Failure> {
    let mut entries = breaking_entries(changelog);
    entries.extend(removed_entries(changelog));
    entries.extend(changed_entries(changelog));
    if entries.is_empty() {
        return Ok(());
    }
    let names: BTreeSet<String> = entries.into_iter().flat_map(|(_, names)| names).collect();
    let (base, against) = main_base(runner, "the names the changelog's entries cite")?;
    // One alternation, which git searches many times faster than as many patterns: a name holds
    // letters, digits and `_` alone ([`cited_names`]), none of them an operator of the pattern.
    let searched: Vec<&str> = core::iter::once(CHANGELOG_TITLE)
        .chain(names.iter().map(String::as_str))
        .collect();
    let pattern = format!("({})", searched.join("|"));
    let args = [
        "grep",
        "--color=never",
        "--no-line-number",
        "--no-column",
        "--only-matching",
        "-h",
        "--text",
        "--word-regexp",
        "--extended-regexp",
        "-e",
        pattern.as_str(),
        base.as_str(),
        "--",
    ];
    let words = git_output(
        runner,
        &args,
        &format!("search the names the changelog's entries cite at the merge base {base}"),
    )?;
    let found: BTreeSet<String> = words.lines().map(str::to_owned).collect();
    if !found.contains(CHANGELOG_TITLE) {
        return Err(Failure::from(format!(
            "git grep found no `{CHANGELOG_TITLE}`, the word of the changelog's title, at the \
             merge base {base}: its output is not the words it matched, so no name it cites can \
             be judged"
        )));
    }
    all_of([
        breaking_names_gate(changelog, &found, (&base, against)),
        removed_names_gate(changelog, &found, (&base, against)),
        changed_names_gate(changelog, &found, (&base, against)),
    ])
}

#[cfg(test)]
#[path = "release_tests.rs"]
mod tests;
