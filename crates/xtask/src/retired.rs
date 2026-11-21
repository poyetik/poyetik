//! The retired patterns gate: a pattern a fix retired (a weakened assertion, an old wording)
//! never comes back, at the sites the fix listed or anywhere else in its scope.
//!
//! A fix that lands at the sites a finding names leaves the same pattern elsewhere: the
//! commit that retires a pattern adds it to [`RETIRED_PATTERNS`], so that every tracked
//! text file is searched, new ones included, at every check, but the list itself, the
//! gate's tests and the released sections of the changelog ([`RETIRED_EXEMPT`]).

use alloc::collections::BTreeSet;

use crate::{
    logical_text::{collapsed, line_at, logical_texts},
    release::{CHANGELOG, unreleased},
    steps::{Failure, step_verdict},
};

/// The list of retired patterns, at the repository root.
pub const RETIRED_PATTERNS: &str = "retired-patterns.txt";

/// The gate's own tests, which hold retired patterns as fixtures.
pub const RETIRED_TESTS: &str = "crates/xtask/src/retired_tests.rs";

/// The tracked text files the gate never searches, of those it is given.
///
/// Its list, which names every pattern, and its own tests ([`RETIRED_TESTS`]). Of the changelog
/// ([`CHANGELOG`]), only the part before its first released section is searched: a released
/// entry says what a release did, in that release's words.
pub const RETIRED_EXEMPT: [&str; 2] = [RETIRED_PATTERNS, RETIRED_TESTS];

/// One entry of [`RETIRED_PATTERNS`].
#[derive(Debug, Clone, PartialEq, Eq)]
struct Retired {
    /// The text refused, as written (no pattern syntax).
    pattern: String,
    /// The files (a path) and directories (a path ending with `/`) it is refused in;
    /// every source when there are none.
    paths: Vec<String>,
    /// Why it was retired, and what replaced it.
    reason: String,
}

impl Retired {
    /// Whether the file at `path` is in the entry's scope.
    fn covers(&self, path: &str) -> bool {
        self.paths.is_empty()
            || self.paths.iter().any(|scope| {
                scope
                    .strip_suffix('/')
                    .map_or(path == scope, |dir| path.starts_with(&format!("{dir}/")))
            })
    }
}

/// The form of an entry, named in every refusal of a line.
const FORM: &str = "retire `<pattern>`[ in <path>…] -- <reason>";

/// Reads one line of [`RETIRED_PATTERNS`]: `None` for a comment or a blank line.
fn entry(line: &str) -> Option<Result<Retired, String>> {
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    Some(parse_entry(line).ok_or_else(|| format!("not `{FORM}`")))
}

/// The entry `line` writes, if it is of the form [`FORM`] with a pattern, every path and a
/// reason not empty.
fn parse_entry(line: &str) -> Option<Retired> {
    let rest = line.strip_prefix("retire `")?;
    let (pattern, rest) = rest.split_once('`')?;
    let (scope, reason) = rest.split_once(" -- ")?;
    let paths: Vec<String> = if scope.is_empty() {
        Vec::new()
    } else {
        scope
            .strip_prefix(" in ")?
            .split(' ')
            .map(str::to_owned)
            .collect()
    };
    let well_formed = !pattern.trim().is_empty()
        && !reason.trim().is_empty()
        && paths.iter().all(|path| !path.is_empty());
    well_formed.then(|| Retired {
        pattern: pattern.to_owned(),
        paths,
        reason: reason.to_owned(),
    })
}

/// Checks that no file of `sources` holds a pattern `list` retires in its scope.
///
/// `sources` are the tracked text files, each with its path (those of [`RETIRED_EXEMPT`]
/// skipped, and of the changelog its part before its first release only), and `list` is
/// [`RETIRED_PATTERNS`]. Every path an entry names must still hold a source, so that the
/// list stays current.
///
/// The list holds comments (`#`), blank lines and entries, one a line:
/// `` retire `<pattern>`[ in <path>…] -- <reason> ``, the pattern refused as written, case
/// sensitive, in the files and directories (ending with `/`) named, or in every source when
/// none is. A source is read as its logical texts: every run of white
/// space is one space, in the pattern too, and a line break one space, so that code a
/// formatter wraps, and a sentence wrapped in a comment, its markers apart, are read
/// whole; each holder is named at the line where the pattern starts.
///
/// # Errors
///
/// Returns every line of the list not of that form, every path an entry names that holds no
/// source, and every line of a source holding a retired pattern, with its file, its line,
/// the pattern and the reason it was retired.
pub fn retired_patterns_gate(sources: &[(String, String)], list: &str) -> Result<(), Failure> {
    let mut problems = Vec::new();
    let mut retired = Vec::new();
    for (index, line) in list.lines().enumerate() {
        match entry(line) {
            None => {}
            Some(Ok(parsed)) => retired.push(parsed),
            Some(Err(why)) => {
                problems.push(format!(
                    "{RETIRED_PATTERNS}:{}: {why}",
                    index.saturating_add(1)
                ));
            }
        }
    }
    for entry in &retired {
        for scope in &entry.paths {
            let single = Retired {
                paths: vec![scope.clone()],
                ..entry.clone()
            };
            if !sources.iter().any(|(path, _)| single.covers(path)) {
                problems.push(format!(
                    "{RETIRED_PATTERNS}: `{}` names {scope}, which holds no tracked source: \
                     remove or correct it",
                    entry.pattern
                ));
            }
        }
    }
    for (path, text) in sources
        .iter()
        .filter(|(path, _)| !RETIRED_EXEMPT.contains(&path.as_str()))
    {
        let text = if path == CHANGELOG {
            unreleased(text)
        } else {
            text
        };
        let texts = logical_texts(path, text);
        for entry in retired.iter().filter(|entry| entry.covers(path)) {
            let pattern = collapsed(&entry.pattern);
            let lines: BTreeSet<usize> = texts
                .iter()
                .flat_map(|(joined, spans)| {
                    joined
                        .match_indices(pattern.as_str())
                        .map(|(offset, _)| line_at(spans, offset))
                })
                .collect();
            problems.extend(lines.into_iter().map(|line| {
                format!(
                    "{path}:{line}: holds `{}`, which a fix retired: {}",
                    entry.pattern, entry.reason
                )
            }));
        }
    }
    step_verdict(problems)
}

#[cfg(test)]
#[path = "retired_tests.rs"]
mod tests;
