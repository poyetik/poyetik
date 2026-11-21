//! The one reader of logical text: every gate over text reads it as it is written,
//! wrapped, through this module, never one line at a time.
//!
//! A sentence is read whole across the lines of its paragraph or comment block
//! ([`paragraphs`]), a string across the lines a formatter splits it on ([`code_prose`]),
//! and code across the lines of its block ([`code_blocks`]); each text is named at the line
//! where what a gate finds in it starts ([`line_at`]). The vocabulary, retired patterns,
//! citation and list counts gates read through it, and the rewrap law
//! (`every_text_gate_refuses_its_fixtures_in_every_rewrap`) holds each of them to it.

use core::ops::Range;

use crate::{
    citations::leading_number,
    workspace::{Reading, reading_of},
};

/// The kinds of comment prose is written in: lines of one kind only are joined into a
/// paragraph ([`paragraphs`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ProseKind {
    /// A line of a text file (read as [`Reading::Markdown`] or [`Reading::Text`]): every line
    /// is prose.
    Text,
    /// An outer doc comment, `///`.
    Doc,
    /// An inner doc comment, `//!`.
    InnerDoc,
    /// A line comment, `//`.
    Comment,
    /// A line of a block comment: its opening `/*` or `/**`, then each line starting with `*`.
    Block,
    /// A comment of a file read as [`Reading::Hash`]: `#` and a space.
    Hash,
}

/// The markers that open a comment of code, each with its kind, the longest first where
/// one starts another: a comment of every file that is neither a text file nor read as
/// [`Reading::Hash`] ([`reading_of`]).
pub(crate) const COMMENT_MARKERS: [(&str, ProseKind); 5] = [
    ("//!", ProseKind::InnerDoc),
    ("///", ProseKind::Doc),
    ("//", ProseKind::Comment),
    ("/**", ProseKind::Block),
    ("/*", ProseKind::Block),
];

/// The prose of `line`, a line of the file at `path`, and its kind; `None` for a line that
/// holds none (a line of code, or a line that opens with code before its comment).
/// `in_block` says whether a block comment is open, which a line starting with `*` then
/// continues. The text is trimmed, without its marker, and without the `*/` closing a
/// block comment.
pub(crate) fn prose_line<'a>(
    path: &str,
    line: &'a str,
    in_block: bool,
) -> Option<(ProseKind, &'a str)> {
    let trimmed = line.trim();
    let reading = reading_of(path);
    if matches!(reading, Reading::Markdown | Reading::Text) {
        return Some((ProseKind::Text, trimmed));
    }
    if reading == Reading::Hash {
        return trimmed
            .strip_prefix('#')
            .filter(|rest| rest.is_empty() || rest.starts_with(' '))
            .map(|rest| (ProseKind::Hash, rest.trim()));
    }
    let body = trimmed.strip_suffix("*/").unwrap_or(trimmed);
    COMMENT_MARKERS
        .iter()
        .find_map(|(marker, kind)| Some((*kind, body.strip_prefix(marker)?.trim())))
        .or_else(|| {
            body.strip_prefix('*')
                .filter(|_| in_block)
                .map(|rest| (ProseKind::Block, rest.trim()))
        })
}

/// Whether the prose `text` stands alone, joined to no line around it: a heading (`#` then
/// a space or another `#`) or a row of a table (`|`).
pub(crate) fn stands_alone(text: &str) -> bool {
    text.starts_with('|')
        || text
            .strip_prefix('#')
            .is_some_and(|rest| rest.starts_with([' ', '#']))
}

/// Whether the prose `text` opens a paragraph: an item of a list (`- `, `* `, or a number
/// and `. `), which the lines after it continue.
pub(crate) fn opens_paragraph(text: &str) -> bool {
    text.starts_with("- ")
        || text.starts_with("* ")
        || leading_number(text).is_some_and(|(_, rest)| rest.starts_with(". "))
}

/// The paragraphs of `text`, the file at `path`, as a sentence is wrapped over them: each
/// the lines it joins, with their index, as [`prose_line`] reads them.
///
/// Consecutive lines of prose of one kind are joined; a blank line (or a blank comment line,
/// or a line of code) ends a paragraph, a list item opens one ([`opens_paragraph`]), a
/// heading or a table row is one of its own ([`stands_alone`]), and so is the end of a
/// block comment.
pub(crate) fn paragraphs<'a>(path: &str, text: &'a str) -> Vec<Vec<(usize, &'a str)>> {
    let mut paragraphs = Paragraphs::default();
    let mut in_block = false;
    for (index, line) in text.lines().enumerate() {
        let prose = prose_line(path, line, in_block);
        let closes = line.trim_end().ends_with("*/");
        in_block = prose.is_some_and(|(found, _)| found == ProseKind::Block) && !closes;
        paragraphs.line(prose.map(|(kind, words)| (index, kind, words, closes)));
    }
    paragraphs.end()
}

/// The paragraphs being read ([`paragraphs`]), one line of prose (or none) at a time.
#[derive(Debug, Default)]
struct Paragraphs<'a> {
    /// The paragraphs read.
    read: Vec<Vec<(usize, &'a str)>>,
    /// The lines of the paragraph being read, with their index.
    current: Vec<(usize, &'a str)>,
    /// The kind of the paragraph being read; `None` when the next line joins nothing.
    kind: Option<ProseKind>,
}

impl<'a> Paragraphs<'a> {
    /// Reads one line: its index, the kind of its prose, its words and whether it closes a
    /// block comment; `None` for a line that holds no prose (a line of code).
    fn line(&mut self, prose: Option<(usize, ProseKind, &'a str, bool)>) {
        let joins = prose.is_some_and(|(_, found, words, _)| {
            Some(found) == self.kind && !stands_alone(words) && !opens_paragraph(words)
        });
        if !joins {
            self.flush();
        }
        if let Some((index, found, words, closes)) =
            prose.filter(|(_, _, words, _)| !words.is_empty())
        {
            self.current.push((index, words));
            self.kind = Some(found);
            if stands_alone(words) || (found == ProseKind::Block && closes) {
                self.flush();
                self.kind = None;
            }
        } else {
            // A blank line, or one that holds no prose: the next line joins nothing, so it
            // flushes the paragraph before it.
            self.kind = None;
        }
    }

    /// Ends the paragraph being read, if any.
    fn flush(&mut self) {
        if !self.current.is_empty() {
            self.read.push(core::mem::take(&mut self.current));
        }
    }

    /// The paragraphs read, the last one ended.
    fn end(mut self) -> Vec<Vec<(usize, &'a str)>> {
        self.flush();
        self.read
    }
}

/// A text joined from lines ([`joined`]): the text, and the index of each line with the
/// range it takes in it.
pub(crate) type Logical = (String, Vec<(usize, Range<usize>)>);

/// The lines of `paragraph` ([`paragraphs`]) joined by a space, with the range each takes
/// in the joined text and its index.
pub(crate) fn joined(paragraph: &[(usize, &str)]) -> Logical {
    let mut joined = String::new();
    let mut spans = Vec::new();
    for (index, words) in paragraph {
        if !joined.is_empty() {
            joined.push(' ');
        }
        let start = joined.len();
        joined.push_str(words);
        spans.push((*index, start..joined.len()));
    }
    (joined, spans)
}

/// The number, from 1, of the line of `spans` ([`joined`]) that holds the byte at `offset`
/// of their joined text: the last that starts at or before it.
pub(crate) fn line_at(spans: &[(usize, Range<usize>)], offset: usize) -> usize {
    spans
        .iter()
        .take_while(|(_, span)| span.start <= offset)
        .last()
        .map_or(0, |(index, _)| *index)
        .saturating_add(1)
}

/// `text` with every run of white space one space, and none at its ends.
pub(crate) fn collapsed(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The lines of `lines`, each with its index, joined ([`joined`]) with every run of white
/// space one space ([`collapsed`]), the lines left blank dropped.
pub(crate) fn logical(lines: &[(usize, &str)]) -> Logical {
    let owned: Vec<(usize, String)> = lines
        .iter()
        .map(|(index, line)| (*index, collapsed(line)))
        .filter(|(_, line)| !line.is_empty())
        .collect();
    let borrowed: Vec<(usize, &str)> = owned
        .iter()
        .map(|(index, line)| (*index, line.as_str()))
        .collect();
    joined(&borrowed)
}

/// The blocks of `text`, each the lines between two blank lines, with their index: code a
/// formatter wraps over lines reads as one line, never across a blank line.
pub(crate) fn code_blocks(text: &str) -> Vec<Vec<(usize, &str)>> {
    let mut blocks = Vec::new();
    let mut block = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            blocks.push(core::mem::take(&mut block));
        } else {
            block.push((index, line));
        }
    }
    blocks.push(block);
    blocks.retain(|lines| !lines.is_empty());
    blocks
}

/// One logical text of prose ([`prose_texts`]), and whether it is a run of string literals
/// (or a JSON string), whose whole text of a single word is a name, not prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProseText {
    /// The text, joined from its lines, with the range each takes.
    pub(crate) text: Logical,
    /// Whether it is a run of string literals or a JSON string.
    pub(crate) literal: bool,
}

/// The prose of a Rust or TypeScript source as logical texts: each paragraph of its
/// comments, consecutive lines of one kind opening their lines, joined as [`paragraphs`]
/// joins them (a comment after code alone); then each run of its string literals
/// ([`Lexeme::Literal`]): a literal and those `+` alone joins to it, each line of a
/// literal wrapped over lines joined.
pub(crate) fn code_prose(text: &str, language: Language) -> Vec<ProseText> {
    let lexed = lexemes(text, language);
    let mut paragraphs = Paragraphs::default();
    let mut previous: Option<usize> = None;
    for lexeme in &lexed {
        if let Lexeme::Comment {
            line,
            kind,
            words,
            alone,
            closes,
        } = lexeme
        {
            if !alone || previous.is_none_or(|before| before.saturating_add(1) != *line) {
                paragraphs.line(None);
            }
            paragraphs.line(Some((*line, *kind, words.as_str(), *closes)));
            if !alone {
                paragraphs.line(None);
            }
            previous = Some(*line);
        }
    }
    let mut texts: Vec<ProseText> = paragraphs
        .end()
        .iter()
        .map(|paragraph| ProseText {
            text: logical(paragraph),
            literal: false,
        })
        .collect();
    let mut runs: Vec<Vec<(usize, &str)>> = Vec::new();
    for lexeme in &lexed {
        if let Lexeme::Literal {
            line,
            text: value,
            joins,
        } = lexeme
        {
            let pieces = value
                .split('\n')
                .enumerate()
                .map(|(offset, piece)| (line.saturating_add(offset), piece));
            match runs.last_mut() {
                Some(run) if *joins => run.extend(pieces),
                _ => runs.push(pieces.collect()),
            }
        }
    }
    texts.extend(
        runs.iter()
            .map(|run| ProseText {
                text: logical(run),
                literal: true,
            })
            .filter(|run| !run.text.0.is_empty()),
    );
    texts
}

/// The prose of a Markdown file: its paragraphs ([`paragraphs`]), fenced code blocks
/// apart.
pub(crate) fn markdown_prose(path: &str, text: &str) -> Vec<Logical> {
    let mut fenced = false;
    let kept: Vec<&str> = text
        .lines()
        .map(|line| {
            let fence = line.trim_start().starts_with("```");
            let code = fenced || fence;
            fenced ^= fence;
            if code { "" } else { line }
        })
        .collect();
    paragraphs(path, &kept.join("\n"))
        .iter()
        .map(|paragraph| logical(paragraph))
        .collect()
}

/// The prose of a JSON document: its string values, each a logical text of its line.
///
/// A JSON string holds no line break. A key (a string followed by `:`) is no prose. A
/// string runs to its closing quote, an escaped quote (`\"`) included, or to the end of
/// its line: nothing is skipped, even in a document that is not valid JSON, which its own
/// readers refuse.
pub(crate) fn json_prose(text: &str) -> Vec<Logical> {
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
                prose.push(logical(&[(index, value.as_str())]));
            }
        }
    }
    prose.retain(|(value, _)| !value.is_empty());
    prose
}

/// The prose of the file at `path` as logical texts, read as its kind says
/// ([`reading_of`]): Markdown's paragraphs, fenced code apart; the paragraphs of a text
/// file or of the comments of a configuration file, a shader, a page or a style sheet; the
/// comments and literal runs of Rust and TypeScript ([`code_prose`]); the strings of JSON;
/// nothing of a file of no prose.
pub(crate) fn prose_texts(path: &str, text: &str) -> Vec<ProseText> {
    let prose = |texts: Vec<Logical>, literal| {
        texts
            .into_iter()
            .map(|logical_text| ProseText {
                text: logical_text,
                literal,
            })
            .collect()
    };
    match reading_of(path) {
        Reading::Markdown => prose(markdown_prose(path, text), false),
        Reading::Text | Reading::Hash | Reading::Comments => prose(
            paragraphs(path, text)
                .iter()
                .map(|paragraph| logical(paragraph))
                .collect(),
            false,
        ),
        Reading::Rust => code_prose(text, Language::Rust),
        Reading::Script => code_prose(text, Language::Script),
        Reading::Json => prose(json_prose(text), true),
        Reading::Nothing => Vec::new(),
    }
}

/// The logical texts of `text`, the file at `path`, as a gate searching it for a phrase
/// reads them, each joined with every run of white space one space ([`logical`]): each
/// block of it between blank lines ([`code_blocks`]), so that code wrapped over lines (a
/// formatter's split before an operator) reads as one line; then each logical text of its
/// prose ([`prose_texts`]), without its comment markers, so that a sentence wrapped in a
/// comment, or in a string a formatter splits (a Rust `\` and the next line's indentation,
/// a TypeScript `+`), reads whole.
pub(crate) fn logical_texts(path: &str, text: &str) -> Vec<Logical> {
    code_blocks(text)
        .iter()
        .map(|block| logical(block))
        .chain(prose_texts(path, text).into_iter().map(|prose| prose.text))
        .collect()
}

/// The language of a source whose prose a lexer reads ([`code_prose`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Language {
    /// Rust: comments and string literals, raw strings included.
    Rust,
    /// TypeScript or JavaScript: comments and string literals (template literals, their
    /// substitutions apart); a regular expression literal is code.
    Script,
}

/// A piece of prose a lexer reads in a source ([`lexemes`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Lexeme {
    /// One line of a comment.
    Comment {
        /// The index of its line, from 0.
        line: usize,
        /// Its kind: a line comment's marker, or a line of a block comment.
        kind: ProseKind,
        /// Its words, trimmed, without its marker (and a block's `*` or `*/`).
        words: String,
        /// Whether it opens its line, no code before it: a comment after code stands alone.
        alone: bool,
        /// Whether it ends a block comment.
        closes: bool,
    },
    /// A string literal.
    Literal {
        /// The index of the line it opens on, from 0.
        line: usize,
        /// Its value: an escaped quote a quote, a line break (written, or escaped with what
        /// indents the next line) a line break, any other escape and a template
        /// substitution a space.
        text: String,
        /// Whether it continues the literal before it, joined by `+` alone (TypeScript and
        /// JavaScript): one string, as a formatter splits a long one.
        joins: bool,
    },
}

/// Reads the comments and string literals of a Rust or TypeScript source, one character at
/// a time.
#[derive(Debug)]
struct Lexer {
    /// The source.
    chars: Vec<char>,
    /// The next character to read.
    index: usize,
    /// Whether the source is Rust (else TypeScript or JavaScript).
    rust: bool,
    /// The index in [`Lexer::chars`] of each line break.
    breaks: Vec<usize>,
    /// The prose read so far.
    lexemes: Vec<Lexeme>,
    /// The index after the last literal read, if any: a literal joined to it by `+` alone
    /// continues it.
    after_literal: Option<usize>,
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

    /// The index, from 0, of the line holding the character at `index`.
    fn line_of(&self, index: usize) -> usize {
        self.breaks.partition_point(|&at| at < index)
    }

    /// Whether only blanks precede the character at `index` on its line.
    fn opens_line(&self, index: usize) -> bool {
        let start = self
            .line_of(index)
            .checked_sub(1)
            .and_then(|line| self.breaks.get(line))
            .map_or(0, |at| at.saturating_add(1));
        self.span(start, index)
            .iter()
            .all(|character| character.is_whitespace())
    }

    /// Reads a line comment, from its `//` to the end of its line.
    fn line_comment(&mut self) {
        let start = self.index;
        let end = self.find(start, &['\n']);
        let text: String = self.span(start, end).iter().collect();
        let (kind, words) = COMMENT_MARKERS
            .iter()
            .find_map(|(marker, kind)| Some((*kind, text.strip_prefix(marker)?)))
            .unwrap_or((ProseKind::Comment, text.as_str()));
        self.lexemes.push(Lexeme::Comment {
            line: self.line_of(start),
            kind,
            words: words.trim().to_owned(),
            alone: self.opens_line(start),
            closes: false,
        });
        self.index = end;
    }

    /// Reads a block comment, `/*` (or `/**`) and `*/` apart, one lexeme a line, each
    /// without the `*` that opens it.
    fn block_comment(&mut self) {
        let start = self.index;
        let end = self.find(start.saturating_add(2), &['*', '/']);
        let body: String = self.span(start.saturating_add(2), end).iter().collect();
        let first = self.line_of(start);
        let alone = self.opens_line(start);
        let lines: Vec<&str> = body.split('\n').collect();
        let last = lines.len().saturating_sub(1);
        for (offset, line) in lines.into_iter().enumerate() {
            let trimmed = line.trim();
            let words = trimmed.strip_prefix('*').unwrap_or(trimmed);
            self.lexemes.push(Lexeme::Comment {
                line: first.saturating_add(offset),
                kind: ProseKind::Block,
                words: words.trim().to_owned(),
                alone: alone || offset > 0,
                closes: offset == last,
            });
        }
        self.index = end.saturating_add(2).min(self.chars.len());
    }

    /// Adds a literal that opens at `start` and whose value is `text`, ending at the next
    /// character to read.
    fn literal(&mut self, start: usize, text: String) {
        let joins = !self.rust
            && self.after_literal.is_some_and(|after| {
                let between = self.span(after, start);
                between
                    .iter()
                    .filter(|&&character| character == '+')
                    .count()
                    == 1
                    && between
                        .iter()
                        .all(|&character| character == '+' || character.is_whitespace())
            });
        self.lexemes.push(Lexeme::Literal {
            line: self.line_of(start),
            text,
            joins,
        });
        self.after_literal = Some(self.index);
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
        let text: String = self.span(start, end).iter().collect();
        let opening = self.index;
        self.index = end.saturating_add(closing.len()).min(self.chars.len());
        self.literal(opening, text);
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
        self.index = end.min(self.chars.len());
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
        self.index = end.min(self.chars.len());
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

    /// Reads a string literal delimited by `quote`.
    fn string(&mut self, quote: char) {
        let opening = self.index;
        let mut text = String::new();
        self.index = self.index.saturating_add(1);
        while let Some(&character) = self.chars.get(self.index) {
            self.index = self.index.saturating_add(1);
            if character == quote {
                break;
            }
            if character == '\\' {
                // An escaped quote stays a quote; an escaped line break ends the line, the
                // blanks indenting the next one with it; any other escape is no prose.
                let escaped = self.chars.get(self.index).copied().unwrap_or(' ');
                self.index = self.index.saturating_add(1);
                if escaped == '\n' {
                    text.push('\n');
                    while self
                        .chars
                        .get(self.index)
                        .is_some_and(|&blank| blank == ' ' || blank == '\t')
                    {
                        self.index = self.index.saturating_add(1);
                    }
                } else {
                    text.push(if matches!(escaped, '"' | '\'') {
                        escaped
                    } else {
                        ' '
                    });
                }
            } else if quote == '`' && character == '$' && self.chars.get(self.index) == Some(&'{') {
                text.push(' ');
                text.push_str(&self.substitution());
            } else {
                text.push(character);
            }
        }
        self.literal(opening, text);
    }
}

/// The words after which a `/` of TypeScript or JavaScript opens a regular expression
/// literal: an operand may follow them, never a division.
const REGEX_KEYWORDS: [&str; 14] = [
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
fn continues_name(character: char) -> bool {
    character.is_alphanumeric() || character == '_' || character == '$'
}

/// Whether an operand may follow the code of `chars` read up to the character at `at`,
/// one of code, neither blank nor a comment's: then a `/` opens a regular expression
/// literal, else it is a division. An operand ends at a name that is not one of
/// [`REGEX_KEYWORDS`], a number, `)`, `]`, or an increment or a decrement (`a++ / n`).
fn operand_may_follow(chars: &[char], at: usize) -> bool {
    let preceding = at.checked_sub(1).and_then(|back| chars.get(back)).copied();
    match chars.get(at).copied() {
        Some(character) if continues_name(character) => {
            let start = chars
                .get(..at)
                .unwrap_or_default()
                .iter()
                .rposition(|&before| !continues_name(before))
                .map_or(0, |before| before.saturating_add(1));
            let word: String = chars.get(start..=at).unwrap_or_default().iter().collect();
            REGEX_KEYWORDS.contains(&word.as_str())
        }
        Some(')' | ']') => false,
        Some(sign @ ('+' | '-')) => preceding != Some(sign),
        _ => true,
    }
}

/// The comments and string literals of a Rust or TypeScript source, in order
/// ([`Lexeme`]). A regular expression literal of TypeScript or JavaScript is code: its
/// quotes open no string. So is the `/` of a JSX tag, which opens none: right after `<`
/// (`</p>`), or before `>` after a `}` (`<X a={b} />`; after a name or a literal a `/` is a
/// division already, and a literal opening with `>` elsewhere, `/>/g`, stays one).
pub(crate) fn lexemes(text: &str, language: Language) -> Vec<Lexeme> {
    let rust = language == Language::Rust;
    let chars: Vec<char> = text.chars().collect();
    let breaks = chars
        .iter()
        .enumerate()
        .filter(|(_, character)| **character == '\n')
        .map(|(at, _)| at)
        .collect();
    let mut lexer = Lexer {
        chars,
        index: 0,
        rust,
        breaks,
        lexemes: Vec::new(),
        after_literal: None,
    };
    // Whether an operand may follow the code read so far, which tells a regular
    // expression literal from a division: blanks and comments leave it, a literal ends an
    // operand.
    let mut operand_may_come = true;
    // The last character of code read that is not blank, which tells the `/` of a JSX tag.
    let mut previous = None;
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
            ('/', next)
                if !rust
                    && operand_may_come
                    && previous != Some('<')
                    && !(previous == Some('}') && next == Some('>')) =>
            {
                lexer.regex();
                operand_may_come = false;
            }
            // Only Rust gets here with a quote mark.
            ('\'', _) => lexer.quote_mark(),
            _ => {
                if !character.is_whitespace() {
                    operand_may_come = operand_may_follow(&lexer.chars, start);
                    previous = Some(character);
                }
            }
        }
        // Every step reads at least one character, so that lexing always ends.
        lexer.index = lexer.index.max(start.saturating_add(1));
    }
    lexer.lexemes
}

#[cfg(test)]
#[path = "logical_text_tests.rs"]
mod tests;
