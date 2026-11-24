//! The list counts gate: no prose restates how many entries a typed list holds, nor copies
//! them.
//!
//! A list of names the repository keeps typed, a constant array in Rust of strings, or of
//! tuples each opened by one, or a register at the root, grows as entries are added; a sentence saying how many it holds,
//! or a paragraph quoting its entries, goes stale at the next one, unseen. The prose names the list instead (CLAUDE.md, "Checked"
//! names its checker).

use alloc::collections::{BTreeMap, BTreeSet};

use crate::{
    citations::sentence_spans,
    licenses::LICENSE_FILES,
    logical_text::{joined, line_at, paragraphs},
    release::{CHANGELOG, released_heading},
    rust_source::{Token, TokenKind, close_of, tokens_of},
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

/// A typed list: its name, its count and its entries ([`declared_lists`]).
type TypedList = (String, usize, BTreeSet<String>);

/// The names of `declared` ([`declared_lists`]), each with the counts it holds.
pub(crate) fn typed_lists(declared: &[TypedList]) -> BTreeMap<&str, BTreeSet<usize>> {
    let mut lists: BTreeMap<&str, BTreeSet<usize>> = BTreeMap::new();
    for (name, count, _) in declared {
        let _new = lists.entry(name).or_default().insert(*count);
    }
    lists
}

/// Every typed list of `sources` (path and text): every constant array of strings of a Rust file
/// (`const NAME: [&str; N]`, `&'static str` too), or of tuples opened by one
/// (`[(&str, …); N]`), and every register at the root (a tracked `.txt` file outside every
/// folder), whose entries are its lines that are neither blank nor a comment (`#`). Each with
/// its name, its count (an array's declared length, a register's entries) and its entries
/// (an array's strings, or the first of each tuple; what each line of a register reviews, the text before its ` -- `,
/// since no prose quotes a whole review), each trimmed. Two lists of one name are two lists.
pub(crate) fn declared_lists(sources: &[(String, String)]) -> Vec<TypedList> {
    let mut lists = Vec::new();
    for (path, text) in sources {
        let extension = std::path::Path::new(path)
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or_default();
        if extension.eq_ignore_ascii_case("rs") {
            for (name, count, entries) in string_arrays(text) {
                let entries = entries
                    .iter()
                    .map(|entry| entry.trim().to_owned())
                    .collect();
                lists.push((name, count, entries));
            }
        } else if extension.eq_ignore_ascii_case("txt") && !path.contains('/') {
            let entries: Vec<&str> = text
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .collect();
            let reviewed = entries
                .iter()
                .map(|entry| {
                    entry
                        .split(" -- ")
                        .next()
                        .unwrap_or_default()
                        .trim()
                        .to_owned()
                })
                .collect();
            lists.push((path.clone(), entries.len(), reviewed));
        }
    }
    lists
}

/// The fewest entries of one typed list a paragraph may quote before it copies the list:
/// two entries are an example, three a copy, and every entry of a list of two is one too.
const COPIED_ENTRIES: usize = 3;

/// The folders whose folders are the project's parts, each named by its folder.
const PART_PARENTS: [&str; 4] = ["crates/", "bindings/", "apps/", "tooling/"];

/// The folders and the names of the project's parts among the paths of `sources`: every
/// folder holding one, as its path from the root and a `/` (`crates/`, `crates/a/`), and
/// every part's name, a part being a folder of one of [`PART_PARENTS`]. The prose that
/// describes the project names its folders and parts one by one, held to them by the
/// README gates, so neither is a copied entry.
fn part_names(sources: &[(String, String)]) -> BTreeSet<&str> {
    let mut names = BTreeSet::new();
    for (path, _) in sources {
        for (end, _) in path.match_indices('/') {
            let _new = names.insert(path.get(..=end).unwrap_or_default());
        }
        if let Some((part, _)) = PART_PARENTS
            .iter()
            .find_map(|parent| path.strip_prefix(parent))
            .and_then(|rest| rest.split_once('/'))
        {
            let _new = names.insert(part);
        }
    }
    names
}

/// Whether `path` is prose that describes the project rather than code: `CLAUDE.md`, a
/// README, the specification (`spec/`) and the changelog, where a copy of a typed list goes
/// stale unseen.
fn describes(path: &str) -> bool {
    path == "CLAUDE.md"
        || path == CHANGELOG
        || path.starts_with("spec/")
        || path.ends_with("README.md")
}

/// The code spans of `paragraph` (the text between pairs of code quotes), trimmed.
fn code_spans(paragraph: &str) -> BTreeSet<&str> {
    paragraph
        .split('`')
        .skip(1)
        .step_by(2)
        .map(str::trim)
        .collect()
}

/// The constant arrays of strings, and of tuples whose first member is a string, `text`
/// declares, each with its length and its entries ([`entries`]: their quotes removed, a raw
/// or escaped string kept as written), read from its tokens ([`tokens_of`], the one reader of
/// Rust code), so that a declaration a comment or a string holds declares nothing:
/// `const NAME: [&str; N]` or `const NAME: [(&str, …); N]`, `&'static str` too, the name in
/// capitals. A text that does not lex declares none (the gates of Rust code refuse it).
fn string_arrays(text: &str) -> Vec<(String, usize, Vec<String>)> {
    let tokens = tokens_of(text);
    tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| token.is_ident("const"))
        .filter_map(|(at, _)| {
            let mut rest = tokens.get(at.saturating_add(1)..)?.iter();
            let name = rest.next().filter(|name| {
                name.kind == TokenKind::Ident
                    && name.text.chars().all(|character| {
                        character.is_ascii_uppercase()
                            || character.is_ascii_digit()
                            || character == '_'
                    })
            })?;
            let mut expect =
                |wanted: &dyn Fn(&Token) -> bool| rest.next().filter(|token| wanted(token));
            let _typed = expect(&|token| token.is_punct(':'))?;
            let _open = expect(&|token| token.opens('['))?;
            let tuple = expect(&|token| token.is_punct('&') || token.opens('('))?.opens('(');
            if tuple {
                let _reference = expect(&|token| token.is_punct('&'))?;
            }
            // `str`, or the lifetime `'static` then `str`.
            if expect(&|token| token.is_ident("str") || token.is_punct('\''))?.is_punct('\'') {
                let _lifetime = expect(&|token| token.is_ident("static"))?;
                let _str = expect(&|token| token.is_ident("str"))?;
            }
            if tuple {
                // `:`, `[`, then the parenthesis that opens the tuple's type.
                let members = at.saturating_add(4);
                rest = tokens
                    .get(close_of(&tokens, members).saturating_add(1)..)
                    .unwrap_or_default()
                    .iter();
            }
            let mut expect_after =
                |wanted: &dyn Fn(&Token) -> bool| rest.next().filter(|token| wanted(token));
            let _length = expect_after(&|token| token.is_punct(';'))?;
            let count = expect_after(&|token| token.kind == TokenKind::Literal)?
                .text
                .parse()
                .ok()?;
            let _close = expect_after(&|token| token.kind == TokenKind::Close)?;
            let _assigned = expect_after(&|token| token.is_punct('='))?;
            let value = tokens.len().saturating_sub(rest.as_slice().len());
            let _array = tokens.get(value).filter(|token| token.opens('['))?;
            let array = tokens
                .get(value..close_of(&tokens, value))
                .unwrap_or_default();
            Some((name.text.clone(), count, entries(array, tuple)))
        })
        .collect()
}

/// The entries of `array`, the tokens of an array's value from its opening bracket: its
/// strings, or, when its entries are `tuple`s, the first member of each that is a string.
fn entries(array: &[Token], tuple: bool) -> Vec<String> {
    let mut depth = 0_usize;
    array
        .iter()
        .zip(array.iter().skip(1))
        .filter_map(|(token, next)| {
            let first = depth == 1 && token.opens('(');
            match token.kind {
                TokenKind::Open => depth = depth.saturating_add(1),
                TokenKind::Close => depth = depth.saturating_sub(1),
                TokenKind::Ident
                | TokenKind::Punct { .. }
                | TokenKind::Literal
                | TokenKind::Doc { .. } => {}
            }
            (!tuple || first).then_some(next)
        })
        .filter_map(|token| {
            token
                .text
                .strip_prefix('"')
                .and_then(|quoted| quoted.strip_suffix('"'))
        })
        .map(str::to_owned)
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
/// A paragraph of the prose that describes the project (`describes`: `CLAUDE.md`, a README,
/// `spec/`, the changelog's `[Unreleased]` section) that quotes, each in code quotes, three
/// entries of one typed list (`COPIED_ENTRIES`), or both of a list of two, copies the list: the
/// next entry leaves the copy stale, unseen. It names the list instead. A folder or a part's
/// name (`part_names`), or a license file ([`LICENSE_FILES`]), is no copied entry: that prose
/// names them, and gates hold it to them.
///
/// # Errors
///
/// Returns every such sentence, named at the line where it starts, with the list and its
/// count, and every such paragraph, named at its first line, with the list and how many of
/// its entries it quotes.
pub fn list_counts_gate(sources: &[(String, String)]) -> Result<(), Failure> {
    let entries_of = declared_lists(sources);
    let lists = typed_lists(&entries_of);
    let mut named = part_names(sources);
    // The license files too, which the license statements name one by one, as they name the
    // folders each covers (the license gates hold them).
    named.extend(LICENSE_FILES.map(|(path, _)| path));
    let mut problems = Vec::new();
    for (path, text) in sources {
        let mut released = false;
        for paragraph in paragraphs(path, text) {
            let (words, spans) = joined(&paragraph);
            released |= path == CHANGELOG && released_heading(&words).is_some();
            if released {
                continue;
            }
            if describes(path) {
                let quoted = code_spans(&words);
                for (list, _, entries) in &entries_of {
                    let copied = entries
                        .iter()
                        .filter(|entry| {
                            quoted.contains(entry.as_str()) && !named.contains(entry.as_str())
                        })
                        .count();
                    if entries.len() > 1 && copied >= COPIED_ENTRIES.min(entries.len()) {
                        problems.push(format!(
                            "{path}:{}: quotes {copied} entries of `{list}`: name the list, \
                             never a copy of its entries, which its next entry makes wrong",
                            line_at(&spans, 0)
                        ));
                    }
                }
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
