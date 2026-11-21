//! The vocabulary gate: the words the glossary replaces, refused in the repository's prose.

use crate::{
    logical_text::{Logical, ProseText, line_at, prose_texts},
    release::{CHANGELOG, unreleased},
    steps::{Failure, verdict},
    workspace::{Reading, reading_of, sources_where},
};

/// The reviewed vocabulary: the synonyms `GLOSSARY.md` replaces, each with the word to use
/// instead, where it is refused and the contexts where it is allowed ([`vocabulary_gate`]).
pub const VOCABULARY: &str = "vocabulary.txt";

/// The tracked files whose prose [`vocabulary_gate`] reads (git pathspecs).
///
/// Markdown, the comments and string literals of Rust, TypeScript and JavaScript (every
/// kind of [`SOURCE_KINDS`](crate::workspace::SOURCE_KINDS) read so, the Astro templates
/// apart until reading them is roadmap #24 step 35), and the string values of the JSON
/// files of the specifications and the conformance corpus (the roadmap's board, written
/// only through its command, apart).
#[must_use]
pub fn vocabulary_sources() -> Vec<&'static str> {
    sources_where(|kind| {
        matches!(
            kind.reading,
            Reading::Markdown | Reading::Rust | Reading::Script
        ) && kind.pattern != "*.astro"
    })
    .into_iter()
    .chain(["spec/*.json", "conformance/*.json"])
    .collect()
}

/// `text` without its code spans (between backquotes) and link targets (`](…)`): code and
/// addresses, not prose, each character a space of its own length, so that every offset of
/// `text` names the same byte of what is kept.
pub(crate) fn without_code(text: &str) -> String {
    let mut kept = String::new();
    let mut code = false;
    let mut target = false;
    let mut previous = ' ';
    for character in text.chars() {
        match character {
            '`' if !target => {
                code = !code;
                kept.push(' ');
            }
            '(' if !code && previous == ']' => {
                target = true;
                kept.push(' ');
            }
            ')' if target => {
                target = false;
                kept.push(' ');
            }
            _ if code || target => kept.extend(core::iter::repeat_n(' ', character.len_utf8())),
            _ => kept.push(character),
        }
        previous = character;
    }
    kept
}

/// `text`, a run of Rust string literals, without its format placeholders (`{name}`), each
/// character they hold, but a blank, a space of its own length: a placeholder is code.
pub(crate) fn without_placeholders(text: &str) -> String {
    let mut depth = 0_usize;
    text.chars()
        .flat_map(|character| {
            let placeholder = match character {
                '{' => {
                    depth = depth.saturating_add(1);
                    true
                }
                '}' if depth > 0 => {
                    depth = depth.saturating_sub(1);
                    true
                }
                _ => depth > 0 && !character.is_whitespace(),
            };
            let kept = if placeholder { ' ' } else { character };
            core::iter::repeat_n(kept, if placeholder { character.len_utf8() } else { 1 })
        })
        .collect()
}

/// Whether `text` is a single word, a name (a program, a key, an identifier), not prose: a
/// literal of it alone is not read.
fn single_word(text: &str) -> bool {
    text.chars()
        .all(|character| character.is_alphanumeric() || "_-~.:/@".contains(character))
}

/// Whether `character` continues a word or a name around an occurrence: then the
/// occurrence is part of a name (`audio-boxes`, `.node-version`, `nodes_by_id`), not a
/// word of prose.
pub(crate) fn joins(character: char) -> bool {
    character.is_alphanumeric() || "_-/~@#$\\".contains(character)
}

/// The byte offsets in `text` of every occurrence of `term` as a word of prose: not inside
/// a longer word or name, not quoted alone (`"nodes"`, a key), not followed by `.` or `:`
/// and a letter (`Node.js`, `node:fs`).
pub(crate) fn occurrences(text: &str, term: &str) -> Vec<usize> {
    text.match_indices(term)
        .map(|(start, _)| start)
        .filter(|&start| {
            let before = text.get(..start).and_then(|head| head.chars().next_back());
            let mut after = text
                .get(start.saturating_add(term.len())..)
                .unwrap_or_default()
                .chars();
            let (next, then) = (after.next(), after.next());
            let quoted = matches!(
                (before, next),
                (Some('"'), Some('"')) | (Some('\''), Some('\''))
            );
            let dotted = matches!(next, Some('.' | ':')) && then.is_some_and(char::is_alphanumeric);
            !before.is_some_and(|character| joins(character) || character == '.')
                && !next.is_some_and(joins)
                && !quoted
                && !dotted
        })
        .collect()
}

/// One rule of [`VOCABULARY`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Wording {
    /// `forbid <term> -> <instead>[ in <path>…] -- <reason>`: `term` is refused (in the
    /// files under `paths` only, when given), `instead` said in its place.
    Forbid {
        /// The word or phrase refused, matched case-sensitively as a word of prose.
        term: String,
        /// What to say instead.
        instead: String,
        /// Where the rule applies (a file, or a directory ending with `/`); everywhere
        /// when empty.
        paths: Vec<String>,
        /// Why.
        reason: String,
    },
    /// `allow <term> in <path> -- <reason>`: `term` is allowed in that file or directory.
    InPath {
        /// The refused word or phrase allowed here.
        term: String,
        /// A file, or a directory ending with `/`.
        path: String,
    },
    /// `allow <term> as "<phrase>" -- <reason>`: `term` is allowed inside `phrase`.
    InPhrase {
        /// The refused word or phrase allowed here.
        term: String,
        /// A phrase holding `term`.
        phrase: String,
    },
}

/// Whether `path` is `scope`, or under it when `scope` is a directory (ending with `/`).
///
/// A `*` in `scope` stands for any part of one name, never a `/`, so that a rule names a
/// family of directories at once (`crates/visual-*/`), those to come included.
pub(crate) fn within(path: &str, scope: &str) -> bool {
    match scope.split_once('*') {
        Some((before, after)) => path.strip_prefix(before).is_some_and(|rest| {
            let name = rest.find('/').unwrap_or(rest.len());
            (0..=name).any(|end| within(rest.get(end..).unwrap_or_default(), after))
        }),
        None if scope.ends_with('/') => path.starts_with(scope),
        None => path == scope,
    }
}

/// The prose of `text`, the file at `path`, as the gate searches it: each logical text of it
/// ([`prose_texts`]) with the range each of its lines takes, a file of a kind whose prose the
/// gate does not read (a text or configuration file) apart, and of the changelog only the
/// part before its first release. A literal run of a single word is no prose; Rust format
/// placeholders and code spans and link targets are blanked, each offset kept.
fn file_prose(path: &str, text: &str) -> Vec<Logical> {
    let reading = reading_of(path);
    if !matches!(
        reading,
        Reading::Markdown | Reading::Rust | Reading::Script | Reading::Json
    ) {
        return Vec::new();
    }
    // Released history keeps the words it was released with: the changelog's prose stops
    // at its first released section ([`released_sections_gate`] keeps them).
    let text = if path == CHANGELOG {
        unreleased(text)
    } else {
        text
    };
    prose_texts(path, text)
        .into_iter()
        .filter(|prose| !(prose.literal && single_word(&prose.text.0)))
        .map(
            |ProseText {
                 text: (words, spans),
                 literal,
             }| {
                let words = if literal && reading == Reading::Rust {
                    without_placeholders(&words)
                } else {
                    words
                };
                (without_code(&words), spans)
            },
        )
        .collect()
}

/// Reads the rules of [`VOCABULARY`], adding a problem for every line it cannot read.
pub(crate) fn wordings(vocabulary: &str, problems: &mut Vec<String>) -> Vec<Wording> {
    let mut rules = Vec::new();
    for (index, line) in vocabulary.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let number = index.saturating_add(1);
        let rule = line.split_once(" -- ").and_then(|(rule, reason)| {
            let reason = reason.trim().to_owned();
            if let Some(forbid) = rule.strip_prefix("forbid ") {
                let (term, rest) = forbid.split_once(" -> ")?;
                let (instead, paths) = rest.split_once(" in ").unwrap_or((rest, ""));
                Some(Wording::Forbid {
                    term: term.trim().to_owned(),
                    instead: instead.trim().to_owned(),
                    paths: paths.split_whitespace().map(str::to_owned).collect(),
                    reason,
                })
            } else {
                let allow = rule.strip_prefix("allow ")?;
                if let Some((term, phrase)) = allow.split_once(" as ") {
                    let phrase = phrase.trim().strip_prefix('"')?.strip_suffix('"')?;
                    Some(Wording::InPhrase {
                        term: term.trim().to_owned(),
                        phrase: phrase.to_owned(),
                    })
                } else {
                    let (term, path) = allow.split_once(" in ")?;
                    Some(Wording::InPath {
                        term: term.trim().to_owned(),
                        path: path.trim().to_owned(),
                    })
                }
            }
        });
        match rule {
            Some(rule) => rules.push(rule),
            None => problems.push(format!(
                "{VOCABULARY}:{number}: write `forbid <term> -> <instead>[ in <path>…] -- <reason>`, \
                 `allow <term> in <path> -- <reason>` or `allow <term> as \"<phrase>\" -- <reason>`"
            )),
        }
    }
    rules
}

/// Checks that the prose of the repository uses the words of the glossary.
///
/// `sources` are the tracked files of [`vocabulary_sources`] (path and text); their prose
/// is read as logical texts (`logical_text::prose_texts`), a term wrapped over two lines of one
/// paragraph, comment block or string read whole: the paragraphs of Markdown outside code
/// blocks, the comment paragraphs and string literal runs of Rust and TypeScript (a Rust `\`
/// and the next line's indentation, a TypeScript `+`, joining a string), and the string
/// values of JSON; code spans (between backquotes), link targets, Rust format placeholders,
/// template substitutions and literal runs of a single word (a key, a program name) apart.
/// Each term found is named at the line where it starts. `vocabulary` ([`VOCABULARY`]) lists the refused synonyms, each with
/// what to say instead and where it applies, and the contexts where one is allowed: a file or a
/// directory, or a phrase holding it. A term is matched case-sensitively, as a whole word,
/// never inside a name (`audio-boxes`, `.node-version`, `Node.js`). Every allowance must
/// still allow something, so that the list stays current. The released sections of
/// [`CHANGELOG`] (from its first heading `## [X.Y.Z] - YYYY-MM-DD` on) are not read: they keep
/// the words they were released with, byte for byte ([`released_sections_gate`](crate::release::released_sections_gate)).
///
/// # Errors
///
/// Returns every refused term found, as `<file>:<line>: "<term>": say "<instead>"
/// (<reason>)`, every malformed line, every allowance of a term no rule refuses, and every
/// allowance that allows nothing.
pub fn vocabulary_gate(sources: &[(String, String)], vocabulary: &str) -> Result<(), Failure> {
    let mut problems = Vec::new();
    let rules = wordings(vocabulary, &mut problems);
    let forbidden: Vec<(&str, &str, &[String], &str)> = rules
        .iter()
        .filter_map(|rule| match rule {
            Wording::Forbid {
                term,
                instead,
                paths,
                reason,
            } => Some((
                term.as_str(),
                instead.as_str(),
                paths.as_slice(),
                reason.as_str(),
            )),
            Wording::InPath { .. } | Wording::InPhrase { .. } => None,
        })
        .collect();
    let mut used = vec![false; rules.len()];
    for (path, text) in sources {
        let mut found = Vec::new();
        for (line, spans) in file_prose(path, text) {
            for &(term, instead, paths, reason) in &forbidden {
                if !paths.is_empty() && !paths.iter().any(|scope| within(path, scope)) {
                    continue;
                }
                for start in occurrences(&line, term) {
                    let number = line_at(&spans, start);
                    let end = start.saturating_add(term.len());
                    let allowed = rules.iter().position(|rule| match rule {
                        Wording::InPath {
                            term: allowed,
                            path: scope,
                        } => allowed == term && within(path, scope),
                        Wording::InPhrase {
                            term: allowed,
                            phrase,
                        } => {
                            allowed == term
                                && line.match_indices(phrase.as_str()).any(|(from, _)| {
                                    from <= start && end <= from.saturating_add(phrase.len())
                                })
                        }
                        Wording::Forbid { .. } => false,
                    });
                    match allowed.and_then(|index| used.get_mut(index)) {
                        Some(flag) => *flag = true,
                        None => found.push((
                            (number, start),
                            format!("{path}:{number}: \"{term}\": say \"{instead}\" ({reason})"),
                        )),
                    }
                }
            }
        }
        // In the order of the text: by line, then by place in the logical text.
        found.sort_by_key(|(at, _)| *at);
        problems.extend(found.into_iter().map(|(_, problem)| problem));
    }
    for (rule, &allowed) in rules.iter().zip(&used) {
        let (term, context) = match rule {
            Wording::InPath { term, path } => (term, format!("in {path}")),
            Wording::InPhrase { term, phrase } => (term, format!("as \"{phrase}\"")),
            Wording::Forbid { .. } => continue,
        };
        if !forbidden.iter().any(|(refused, ..)| refused == term) {
            problems.push(format!(
                "allow {term} {context}: no rule refuses \"{term}\""
            ));
        } else if !allowed {
            problems.push(format!("stale: allow {term} {context}: it allows nothing"));
        }
    }
    verdict(
        &format!("words the glossary replaces (reviewed in {VOCABULARY})"),
        problems,
    )
}

#[cfg(test)]
#[path = "vocabulary_tests.rs"]
mod tests;
