//! The conflict marker gate: no tracked text file holds what git writes when a merge or a
//! rebase leaves a conflict for a person to resolve.

use crate::steps::{Failure, step_verdict};

/// The file that tells git which tracked files are binary: the gate reads every other
/// tracked file as text.
pub const GIT_ATTRIBUTES: &str = ".gitattributes";

/// The fewest marker characters git writes at the start of a conflict marker line.
///
/// It is git's default `conflict-marker-size`. A conflict inside a conflict (a nested
/// merge) writes wider markers, and the attribute can set any size, so the gate reads this
/// many or more.
pub const MARKER_SIZE: usize = 7;

/// The suffixes of the tracked files `attributes` ([`GIT_ATTRIBUTES`]) declares binary, each
/// from a line `*.<extension> binary`: the gate reads every other tracked file as text.
///
/// Comments, blank lines and lines without `binary` are not the gate's: they declare text
/// handling, which a conflict marker reaches like any other text.
///
/// # Errors
///
/// Returns every line that declares files binary in another form than `*.<extension>
/// binary`, which the gate cannot read and so refuses rather than skip.
pub fn binary_suffixes(attributes: &str) -> Result<Vec<String>, Failure> {
    let mut suffixes = Vec::new();
    let mut problems = Vec::new();
    for (index, line) in attributes.lines().enumerate() {
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.first().is_none_or(|first| first.starts_with('#')) || !words.contains(&"binary") {
            continue;
        }
        let extension = match words.as_slice() {
            [pattern, "binary"] => pattern.strip_prefix("*.").filter(|extension| {
                !extension.is_empty()
                    && extension
                        .chars()
                        .all(|character| character.is_ascii_alphanumeric())
            }),
            _ => None,
        };
        match extension {
            Some(extension) => suffixes.push(format!(".{extension}")),
            None => problems.push(format!(
                "{GIT_ATTRIBUTES}:{}: declares files binary in another form than `*.<extension> \
                 binary`, which the conflict marker gate reads",
                index.saturating_add(1)
            )),
        }
    }
    step_verdict(problems).map(|()| suffixes)
}

/// What a line of a text file is to the gate: a marker git writes at the start of a line,
/// or nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Marker {
    /// `<<<<<<<`: the start of a conflict, our side.
    Ours,
    /// `|||||||`: the common ancestor, in the diff3 and zdiff3 styles.
    Base,
    /// `=======`: between the two sides.
    Separator,
    /// `>>>>>>>`: the end of a conflict, their side.
    Theirs,
}

/// The marker `line` starts with: at least [`MARKER_SIZE`] of one of git's marker
/// characters, then the end of the line or a space (git writes the side's name after a
/// space); the separator alone on its line.
fn marker(line: &str) -> Option<Marker> {
    let line = line.strip_suffix('\r').unwrap_or(line);
    [
        ('<', Marker::Ours),
        ('|', Marker::Base),
        ('=', Marker::Separator),
        ('>', Marker::Theirs),
    ]
    .into_iter()
    .find_map(|(character, kind)| {
        let rest = line.trim_start_matches(character);
        let size = line.len().saturating_sub(rest.len());
        let ends = if kind == Marker::Separator {
            rest.is_empty()
        } else {
            rest.is_empty() || rest.starts_with(' ')
        };
        (size >= MARKER_SIZE && ends).then_some(kind)
    })
}

/// Checks that no file of `sources` (each with its path) holds a conflict marker.
///
/// A marker is a line starting with [`MARKER_SIZE`] or more `<`, `|` or `>`, then the end
/// of the line or a space; or `=` alone on its line, [`MARKER_SIZE`] or more of them,
/// between a `<` marker and the `>` marker that closes it (anywhere else such a line is a
/// Markdown underline or a rule, never git's).
///
/// A file quoting a marker on purpose quotes it inside a line (a string, an indented
/// example): a marker at the start of a line in a file a person meant is outside the gate's
/// scope, which is the conflict a merge or a rebase left unresolved.
///
/// # Errors
///
/// Returns every marker found, with its file and its line.
pub fn conflict_marker_gate(sources: &[(String, String)]) -> Result<(), Failure> {
    let mut problems = Vec::new();
    for (path, text) in sources {
        let mut open = false;
        for (index, line) in text.lines().enumerate() {
            let found = match marker(line) {
                Some(Marker::Ours) => {
                    open = true;
                    true
                }
                Some(Marker::Theirs) => {
                    open = false;
                    true
                }
                Some(Marker::Base) => true,
                Some(Marker::Separator) => open,
                None => false,
            };
            if found {
                problems.push(format!(
                    "{path}:{}: a conflict marker git left, unresolved",
                    index.saturating_add(1)
                ));
            }
        }
    }
    step_verdict(problems)
}

#[cfg(test)]
#[path = "conflicts_tests.rs"]
mod tests;
