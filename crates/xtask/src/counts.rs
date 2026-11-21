//! The list counts gate: no prose restates how many entries a typed list holds.
//!
//! A list of names the repository keeps typed, a constant array of strings in Rust or a
//! register at the root, grows as entries are added; a sentence saying how many it holds
//! goes stale at the next one, unseen. The prose names the list instead (CLAUDE.md, "Checked"
//! names its checker).

use alloc::collections::{BTreeMap, BTreeSet};

use crate::{
    citations::{joined, line_at, paragraphs, sentence_spans},
    release::{CHANGELOG, released_heading},
    steps::{Failure, verdict},
};

/// The words a count is written with, each with its value: from two, since "one" is far more
/// often an article than a count (`the one commit`, `one of`), up to twenty, the largest the
/// project's prose spells out.
const NUMBER_WORDS: [(&str, usize); 19] = [
    ("two", 2),
    ("three", 3),
    ("four", 4),
    ("five", 5),
    ("six", 6),
    ("seven", 7),
    ("eight", 8),
    ("nine", 9),
    ("ten", 10),
    ("eleven", 11),
    ("twelve", 12),
    ("thirteen", 13),
    ("fourteen", 14),
    ("fifteen", 15),
    ("sixteen", 16),
    ("seventeen", 17),
    ("eighteen", 18),
    ("nineteen", 19),
    ("twenty", 20),
];

/// The typed lists of `sources` (path and text), each name with the counts it holds: every
/// constant array of strings of a Rust file (`const NAME: [&str; N]`, `&'static str` too),
/// and every register at the root (a tracked `.txt` file outside every folder), whose entries
/// are its lines that are neither blank nor a comment (`#`).
pub(crate) fn typed_lists(sources: &[(String, String)]) -> BTreeMap<String, BTreeSet<usize>> {
    let mut lists: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
    for (path, text) in sources {
        let extension = std::path::Path::new(path)
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or_default();
        if extension.eq_ignore_ascii_case("rs") {
            for (name, count) in string_arrays(text) {
                let _new = lists.entry(name).or_default().insert(count);
            }
        } else if extension.eq_ignore_ascii_case("txt") && !path.contains('/') {
            let entries = text
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .count();
            let _new = lists.entry(path.clone()).or_default().insert(entries);
        }
    }
    lists
}

/// The constant arrays of strings `text` declares, each with its length:
/// `const NAME: [&str; N]` or `const NAME: [&'static str; N]`.
fn string_arrays(text: &str) -> Vec<(String, usize)> {
    text.match_indices("const ")
        .filter_map(|(at, opening)| {
            let rest = text.get(at.saturating_add(opening.len())..)?;
            let (name, typed) = rest.split_once(':')?;
            let named = !name.is_empty()
                && name.chars().all(|character| {
                    character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
                });
            let length = ["[&str; ", "[&'static str; "]
                .into_iter()
                .find_map(|form| typed.trim_start().strip_prefix(form))?;
            let (digits, _) = length.split_once(']')?;
            let count = digits.parse().ok()?;
            named.then(|| (name.to_owned(), count))
        })
        .collect()
}

/// Whether `sentence` names `list`: as a whole name, no letter, digit, `_` or `-` joined to
/// it on either side, nor a `.` before it or a `.` and a letter or digit after it (a path,
/// `path.NAME`, or a longer file name, `register.txt.bak`), so that a sentence ending on
/// the name, or `xtask's NAME`, names it.
fn names(sentence: &str, list: &str) -> bool {
    let joins = |character: char| character.is_alphanumeric() || ['_', '-'].contains(&character);
    sentence.match_indices(list).any(|(at, _)| {
        let before = sentence.get(..at).and_then(|text| text.chars().next_back());
        let mut after = sentence
            .get(at.saturating_add(list.len())..)
            .unwrap_or_default()
            .chars();
        let next = after.next();
        let extended = next == Some('.') && after.next().is_some_and(joins);
        before.is_none_or(|character| !joins(character) && character != '.')
            && next.is_none_or(|character| !joins(character))
            && !extended
    })
}

/// The counts `sentence` writes: each word of digits alone, or a number word
/// ([`NUMBER_WORDS`]), a word being what white space, parentheses, code quotes, `,`, `;`,
/// `:` and a closing `.` delimit, so that a date (`2026-11-12`) or a version (`1.99.1`) is
/// no count.
fn counts(sentence: &str) -> BTreeSet<usize> {
    sentence
        .split(|character: char| {
            character.is_whitespace() || ['(', ')', '`', ',', ';', ':', '"'].contains(&character)
        })
        .map(|word| word.trim_end_matches('.'))
        .filter_map(|word| {
            if !word.is_empty() && word.chars().all(|character| character.is_ascii_digit()) {
                word.parse().ok()
            } else {
                NUMBER_WORDS
                    .iter()
                    .find(|(number, _)| word.eq_ignore_ascii_case(number))
                    .map(|(_, value)| *value)
            }
        })
        .collect()
}

/// Checks that no sentence of prose says how many entries a typed list holds.
///
/// `sources` are the tracked files a sentence can be written in
/// ([`citation_sources`](crate::citations::citation_sources), path and text); the lists are
/// those they declare
/// (`typed_lists`). A sentence of a paragraph of prose (`paragraphs`: a text file's, a
/// comment's, read whole across the lines it is wrapped on) that names a list and writes its
/// count, in digits or in words, restates it: the next entry makes it wrong, and nothing
/// says so. It names the list instead. The released sections of the changelog are dated
/// records, never read. The test is by value: a count that already differs from the list's
/// passes, which the gate refused when it was written; a number of the same value counting
/// something else in that sentence is refused too, and is reworded.
///
/// # Errors
///
/// Returns every such sentence, named at the line where it starts, with the list and its
/// count.
pub fn list_counts_gate(sources: &[(String, String)]) -> Result<(), Failure> {
    let lists = typed_lists(sources);
    let mut problems = Vec::new();
    for (path, text) in sources {
        let mut released = false;
        for paragraph in paragraphs(path, text) {
            let (words, spans) = joined(&paragraph);
            released |= path == CHANGELOG && released_heading(&words).is_some();
            if released {
                continue;
            }
            for sentence in sentence_spans(&words) {
                let said = words.get(sentence.clone()).unwrap_or_default();
                let written = counts(said);
                for (list, held) in &lists {
                    if let Some(count) = held.intersection(&written).next()
                        && names(said, list)
                    {
                        problems.push(format!(
                            "{path}:{}: says {count}, how many entries `{list}` holds: name the \
                             list, never its count, which its next entry makes wrong",
                            line_at(&spans, sentence.start)
                        ));
                    }
                }
            }
        }
    }
    verdict("the counts of the typed lists", problems)
}

#[cfg(test)]
#[path = "counts_tests.rs"]
mod tests;
