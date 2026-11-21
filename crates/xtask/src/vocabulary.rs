//! The vocabulary gate: the words the glossary replaces, refused in the repository's prose.

use crate::{
    release::{CHANGELOG, unreleased},
    steps::{Failure, verdict},
};

/// The reviewed vocabulary: the synonyms `GLOSSARY.md` replaces, each with the word to use
/// instead, where it is refused and the contexts where it is allowed ([`vocabulary_gate`]).
pub const VOCABULARY: &str = "vocabulary.txt";

/// The tracked files whose prose [`vocabulary_gate`] reads (git pathspecs).
///
/// Markdown, the comments and string literals of Rust and TypeScript, and the string
/// values of the JSON files of the specifications and the conformance corpus (the
/// roadmap's board, written only through its command, apart).
pub const VOCABULARY_SOURCES: [&str; 9] = [
    "*.md",
    "*.rs",
    "*.ts",
    "*.tsx",
    "*.mts",
    "*.js",
    "*.mjs",
    "spec/*.json",
    "conformance/*.json",
];

/// The language of a file, which decides what of it is prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Language {
    /// Markdown: everything but code blocks, code spans and link targets.
    Markdown,
    /// Rust: comments and string literals, format placeholders apart.
    Rust,
    /// TypeScript or JavaScript: comments and string literals, template substitutions
    /// apart.
    Script,
    /// JSON: the string values, the keys apart.
    Json,
}

/// The language of `path`, by its extension; `None` for a file without prose to read.
pub(crate) fn language(path: &str) -> Option<Language> {
    match path.rsplit_once('.').map(|(_, extension)| extension) {
        Some("md") => Some(Language::Markdown),
        Some("rs") => Some(Language::Rust),
        Some("ts" | "tsx" | "mts" | "js" | "mjs") => Some(Language::Script),
        Some("json") => Some(Language::Json),
        _ => None,
    }
}

/// Collects prose while lexing a source: each line of it with its line number.
#[derive(Debug, Default)]
pub(crate) struct Prose {
    /// The lines of prose found so far: (line number from 1, text).
    lines: Vec<(usize, String)>,
    /// The line being lexed, from 1.
    line: usize,
    /// The prose of the current line, not pushed yet.
    current: String,
}

impl Prose {
    /// Ends the current piece of prose: pushes it unless it is blank.
    fn flush(&mut self) {
        let text = core::mem::take(&mut self.current);
        if !text.trim().is_empty() {
            self.lines.push((self.line, text));
        }
    }

    /// Adds a character of prose; a line break ends the line.
    fn push(&mut self, character: char) {
        if character == '\n' {
            self.flush();
            self.line = self.line.saturating_add(1);
        } else {
            self.current.push(character);
        }
    }

    /// Passes over a character of code: only its line break counts.
    fn skip(&mut self, character: char) {
        if character == '\n' {
            self.flush();
            self.line = self.line.saturating_add(1);
        }
    }

    /// Adds a string literal: one whose whole text is a single word is a name (a program,
    /// a key, an identifier), not prose; so are Rust's format placeholders (`{name}`).
    fn literal(&mut self, text: &str, rust: bool) {
        let word = !text.is_empty()
            && text
                .chars()
                .all(|character| character.is_alphanumeric() || "_-~.:/@".contains(character));
        if word {
            for character in text.chars() {
                self.skip(character);
            }
        } else {
            self.flush();
            let mut depth = 0_usize;
            for character in text.chars() {
                match character {
                    '{' if rust => depth = depth.saturating_add(1),
                    '}' if rust && depth > 0 => depth = depth.saturating_sub(1),
                    _ if depth > 0 && !character.is_whitespace() => {}
                    _ => self.push(character),
                }
            }
            self.flush();
        }
    }
}

/// Reads the prose of a Rust or TypeScript source, one character at a time.
#[derive(Debug)]
pub(crate) struct Lexer {
    /// The source.
    chars: Vec<char>,
    /// The next character to read.
    index: usize,
    /// Whether the source is Rust (else TypeScript or JavaScript).
    rust: bool,
    /// The prose read so far.
    prose: Prose,
}

impl Lexer {
    /// The character `offset` after the next one to read.
    fn at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.index.saturating_add(offset)).copied()
    }

    /// The characters from `start` (an index) to `end`, or to the end of the source.
    fn span(&self, start: usize, end: usize) -> &[char] {
        let end = end.min(self.chars.len());
        self.chars.get(start.min(end)..end).unwrap_or_default()
    }

    /// The index of the first occurrence of `pattern` from `start` on, or the end of the
    /// source when there is none.
    fn find(&self, start: usize, pattern: &[char]) -> usize {
        self.span(start, usize::MAX)
            .windows(pattern.len())
            .position(|window| window == pattern)
            .map_or(self.chars.len(), |found| start.saturating_add(found))
    }

    /// Reads the characters up to `end` (an index) as code: only their line breaks count.
    fn skip_to(&mut self, end: usize) {
        let end = end.min(self.chars.len());
        for character in self.span(self.index, end).to_vec() {
            self.prose.skip(character);
        }
        self.index = end;
    }

    /// Reads the characters from `start` to `end` (indices) as prose, then goes past
    /// `after` more characters.
    fn read_prose(&mut self, start: usize, end: usize, after: usize) {
        for character in self.span(start, end).to_vec() {
            self.prose.push(character);
        }
        self.prose.flush();
        self.index = end.saturating_add(after).min(self.chars.len());
    }

    /// Reads a line comment, `//` included, as prose.
    fn line_comment(&mut self) {
        let end = self.find(self.index, &['\n']);
        self.read_prose(self.index, end, 0);
    }

    /// Reads a block comment, `/*` and `*/` apart, as prose.
    fn block_comment(&mut self) {
        let start = self.index.saturating_add(2);
        let end = self.find(start, &['*', '/']);
        self.read_prose(start, end, 2);
    }

    /// Reads a Rust raw string (`r"…"`, `r#"…"#`) if one starts here.
    fn raw_string(&mut self) -> bool {
        let hashes = self
            .span(self.index.saturating_add(1), usize::MAX)
            .iter()
            .take_while(|&&hash| hash == '#')
            .count();
        let start = self.index.saturating_add(hashes).saturating_add(2);
        if self.chars.get(start.saturating_sub(1)) != Some(&'"') {
            return false;
        }
        let closing: Vec<char> = core::iter::once('"')
            .chain(core::iter::repeat_n('#', hashes))
            .collect();
        let end = self.find(start, &closing);
        let literal: String = self.span(start, end).iter().collect();
        self.prose.literal(&literal, true);
        self.index = end.saturating_add(closing.len()).min(self.chars.len());
        true
    }

    /// Reads a Rust character literal (`'a'`, `'\n'`, `'\u{22}'`, `'"'`) or a lifetime
    /// (`'a`): code.
    fn quote_mark(&mut self) {
        let end = match (self.at(1), self.at(2)) {
            // An escape: up to the quote after the escaped character.
            (Some('\\'), _) => self
                .find(self.index.saturating_add(3), &['\''])
                .saturating_add(1),
            (Some(_), Some('\'')) => self.index.saturating_add(3),
            _ => self.index.saturating_add(1),
        };
        self.skip_to(end);
    }

    /// Reads a regular expression literal (`/…/flags`), from its opening `/`: code, its
    /// quotes included. It ends at its first `/` outside a character class (`[…]`) and not
    /// escaped, then its flags; or, unterminated, before the end of its line.
    fn regex(&mut self) {
        let mut end = self.index.saturating_add(1);
        let mut class = false;
        while let Some(character) = self.chars.get(end).copied() {
            match character {
                '\n' => break,
                '\\' => end = end.saturating_add(1),
                '[' => class = true,
                ']' => class = false,
                '/' if !class => {
                    end = end.saturating_add(1);
                    while self.chars.get(end).is_some_and(char::is_ascii_alphabetic) {
                        end = end.saturating_add(1);
                    }
                    break;
                }
                _ => {}
            }
            end = end.saturating_add(1);
        }
        self.skip_to(end);
    }

    /// Reads a template substitution (`${…}`), from its `$`, up to its matching brace:
    /// code, of which it returns the line breaks.
    fn substitution(&mut self) -> String {
        let mut depth = 0_usize;
        let mut breaks = String::new();
        let rest = self.span(self.index, usize::MAX).to_vec();
        let mut used = rest.len();
        for (offset, &character) in rest.iter().enumerate() {
            match character {
                '\n' => breaks.push('\n'),
                '{' => depth = depth.saturating_add(1),
                '}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        used = offset.saturating_add(1);
                        break;
                    }
                }
                _ => {}
            }
        }
        self.index = self.index.saturating_add(used);
        breaks
    }

    /// Reads a string literal delimited by `quote`, as prose.
    fn string(&mut self, quote: char) {
        let mut literal = String::new();
        self.index = self.index.saturating_add(1);
        while let Some(&character) = self.chars.get(self.index) {
            self.index = self.index.saturating_add(1);
            if character == quote {
                break;
            }
            if character == '\\' {
                // An escaped quote stays a quote, an escaped line break a line break; any
                // other escape is no prose.
                let escaped = self.chars.get(self.index).copied().unwrap_or(' ');
                literal.push(if matches!(escaped, '"' | '\'' | '\n') {
                    escaped
                } else {
                    ' '
                });
                self.index = self.index.saturating_add(1);
            } else if quote == '`' && character == '$' && self.chars.get(self.index) == Some(&'{') {
                literal.push(' ');
                literal.push_str(&self.substitution());
            } else {
                literal.push(character);
            }
        }
        self.prose.literal(&literal, self.rust);
    }
}

/// The prose of a JSON document: its string values, each with the number of its line.
///
/// A JSON string holds no line break. A key (a string followed by `:`) and a value of a
/// single word ([`Prose::literal`]) are no prose. A string runs to its closing quote, an escaped
/// quote (`\"`) included, or to the end of its line: nothing is skipped, even in a
/// document that is not valid JSON, which its own readers refuse.
pub(crate) fn json_prose(text: &str) -> Vec<(usize, String)> {
    let mut prose = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let mut rest = line;
        while let Some(start) = rest.find('"') {
            let after = rest.get(start.saturating_add(1)..).unwrap_or_default();
            let mut value = String::new();
            let mut chars = after.char_indices();
            let mut end = after.len();
            while let Some((at, character)) = chars.next() {
                match character {
                    '"' => {
                        end = at.saturating_add(1);
                        break;
                    }
                    '\\' => value.extend(chars.next().map(|(_, escaped)| escaped)),
                    _ => value.push(character),
                }
            }
            rest = after.get(end..).unwrap_or_default();
            if !rest.trim_start().starts_with(':') {
                let mut piece = Prose {
                    line: index.saturating_add(1),
                    ..Prose::default()
                };
                piece.literal(&value, false);
                prose.append(&mut piece.lines);
            }
        }
    }
    prose
}

/// The words after which a `/` of TypeScript or JavaScript opens a regular expression
/// literal: an operand may follow them, never a division.
pub(crate) const REGEX_KEYWORDS: [&str; 14] = [
    "return",
    "typeof",
    "case",
    "do",
    "else",
    "in",
    "of",
    "void",
    "throw",
    "new",
    "delete",
    "yield",
    "await",
    "instanceof",
];

/// Whether `character` continues a name of TypeScript or JavaScript (or a number).
pub(crate) fn names(character: char) -> bool {
    character.is_alphanumeric() || character == '_' || character == '$'
}

/// Whether an operand may follow the code of `chars` read up to the character at `at`,
/// one of code, neither blank nor a comment's: then a `/` opens a regular expression
/// literal, else it is a division. An operand ends at a name that is not one of
/// [`REGEX_KEYWORDS`], a number, `)` or `]`.
pub(crate) fn operand_may_follow(chars: &[char], at: usize) -> bool {
    match chars.get(at).copied() {
        Some(character) if names(character) => {
            let start = chars
                .get(..at)
                .unwrap_or_default()
                .iter()
                .rposition(|&before| !names(before))
                .map_or(0, |before| before.saturating_add(1));
            let word: String = chars.get(start..=at).unwrap_or_default().iter().collect();
            REGEX_KEYWORDS.contains(&word.as_str())
        }
        Some(')' | ']') => false,
        _ => true,
    }
}

/// The prose of a Rust or TypeScript source: its comments and string literals, each line
/// with its number. A regular expression literal of TypeScript or JavaScript is code: its
/// quotes open no string.
pub(crate) fn code_prose(text: &str, language: Language) -> Vec<(usize, String)> {
    let rust = language == Language::Rust;
    let mut lexer = Lexer {
        chars: text.chars().collect(),
        index: 0,
        rust,
        prose: Prose {
            line: 1,
            ..Prose::default()
        },
    };
    // Whether an operand may follow the code read so far, which tells a regular
    // expression literal from a division: blanks and comments leave it, a literal ends an
    // operand.
    let mut operand_may_come = true;
    while let Some(&character) = lexer.chars.get(lexer.index) {
        let start = lexer.index;
        // Inside a name, `r` starts no raw string; after a lone `b`, a raw byte string.
        let before = |back: usize| {
            start
                .checked_sub(back)
                .and_then(|at| lexer.chars.get(at))
                .is_some_and(|&named| named.is_alphanumeric() || named == '_')
        };
        let byte = start.checked_sub(1).and_then(|at| lexer.chars.get(at)) == Some(&'b');
        let identifier = before(1) && (!byte || before(2));
        match (character, lexer.at(1)) {
            ('/', Some('/')) => lexer.line_comment(),
            ('/', Some('*')) => lexer.block_comment(),
            ('r', Some(after))
                if matches!(after, '"' | '#') && rust && !identifier && lexer.raw_string() =>
            {
                operand_may_come = false;
            }
            ('"', _) => {
                lexer.string('"');
                operand_may_come = false;
            }
            ('\'' | '`', _) if !rust => {
                lexer.string(character);
                operand_may_come = false;
            }
            ('/', _) if !rust && operand_may_come => {
                lexer.regex();
                operand_may_come = false;
            }
            // Only Rust gets here with a quote mark.
            ('\'', _) => lexer.quote_mark(),
            _ => {
                lexer.skip_to(start.saturating_add(1));
                if !character.is_whitespace() {
                    operand_may_come = operand_may_follow(&lexer.chars, start);
                }
            }
        }
        // Every step reads at least one character, so that lexing always ends.
        lexer.index = lexer.index.max(start.saturating_add(1));
    }
    lexer.prose.flush();
    lexer.prose.lines
}

/// The prose of a Markdown file: every line outside fenced code blocks.
pub(crate) fn markdown_prose(text: &str) -> Vec<(usize, String)> {
    let mut fenced = false;
    let mut lines = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        } else if !fenced {
            lines.push((index.saturating_add(1), line.to_owned()));
        }
    }
    lines
}

/// `text` without its code spans (between backquotes) and link targets (`](…)`): code and
/// addresses, not prose.
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
            _ if code || target => kept.push(' '),
            _ => kept.push(character),
        }
        previous = character;
    }
    kept
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
/// `sources` are the tracked files of [`VOCABULARY_SOURCES`] (path and text); their prose
/// is the text of Markdown outside code blocks, the comments and string literals of Rust
/// and TypeScript, and the string values of JSON (`json_prose`), code spans (between backquotes),
/// link targets, Rust format placeholders, template substitutions and literals of a single word (a
/// key, a program name) apart. `vocabulary` ([`VOCABULARY`]) lists the refused synonyms, each with
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
        let Some(language) = language(path) else {
            continue;
        };
        // Released history keeps the words it was released with: the changelog's prose
        // stops at its first released section ([`released_sections_gate`] keeps them).
        let text = if path == CHANGELOG {
            unreleased(text)
        } else {
            text
        };
        let prose = match language {
            Language::Markdown => markdown_prose(text),
            Language::Rust | Language::Script => code_prose(text, language),
            Language::Json => json_prose(text),
        };
        for (number, line) in &prose {
            let line = without_code(line);
            for &(term, instead, paths, reason) in &forbidden {
                if !paths.is_empty() && !paths.iter().any(|scope| within(path, scope)) {
                    continue;
                }
                for start in occurrences(&line, term) {
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
                        None => problems.push(format!(
                            "{path}:{number}: \"{term}\": say \"{instead}\" ({reason})"
                        )),
                    }
                }
            }
        }
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
