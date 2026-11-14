//! Rust sources read into the tokens of a real tokenizer, as every gate over them reads them.

use core::str::FromStr as _;

use crate::steps::Failure;

/// What a token of a Rust source is ([`RustSource`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TokenKind {
    /// An identifier or a keyword, its text without a raw identifier's `r#` (`r#fn` is
    /// `fn`), so that a rule naming a word sees it however it is written.
    Ident,
    /// A punctuation character, `joint` when the next token is a punctuation character
    /// written right after it (the first `=` of `==`, but not of `= =`).
    Punct {
        /// Whether the next punctuation character follows it with no space.
        joint: bool,
    },
    /// A literal, its text as written: a string, raw or not (quotes, prefix and escapes
    /// included), a byte or C string, a char, a number with its suffix (`1.0f32`).
    Literal,
    /// The opening delimiter of a group, `(`, `[` or `{`.
    Open,
    /// The closing delimiter of a group, `)`, `]` or `}`.
    Close,
    /// A doc comment (`///`, `/** */`, and `//!`, `/*! */` when `inner`), its text as
    /// written: prose, never code, whatever it holds.
    Doc {
        /// Whether it documents the item it is in (`//!`, `/*! */`).
        inner: bool,
    },
}

/// A token of a Rust source ([`RustSource`]): its kind, its text and the line it starts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Token {
    /// What it is.
    pub(crate) kind: TokenKind,
    /// Its text ([`TokenKind`] says which).
    pub(crate) text: String,
    /// The line it starts on, from 1.
    pub(crate) line: usize,
}

impl Token {
    /// Whether it is the identifier or keyword `name`.
    pub(crate) fn is_ident(&self, name: &str) -> bool {
        self.kind == TokenKind::Ident && self.text == name
    }

    /// Whether it is the punctuation character `character`.
    pub(crate) fn is_punct(&self, character: char) -> bool {
        matches!(self.kind, TokenKind::Punct { .. }) && self.text.chars().eq([character])
    }

    /// Whether it opens a group with `delimiter` (`(`, `[` or `{`).
    pub(crate) fn opens(&self, delimiter: char) -> bool {
        self.kind == TokenKind::Open && self.text.chars().eq([delimiter])
    }

    /// Whether it is written as `other` is, wherever each stands: the same kind and text.
    fn same(&self, other: &Self) -> bool {
        self.kind == other.kind && self.text == other.text
    }
}

/// A Rust source of the repository, read as rustc reads it: its path, and its tokens.
///
/// The one reader of Rust code for the gates that follow it ([`RustSource::parse`]): they
/// read tokens, never characters or lines, so that a comment (nested ones included), a
/// string (raw ones included) or a doc comment is never code to them, and code is never
/// hidden from them by a spelling rustc accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RustSource {
    /// Its path, relative to the root.
    pub(crate) path: String,
    /// Its tokens, in order: a group as its opening delimiter, its tokens, its closing
    /// delimiter.
    pub(crate) tokens: Vec<Token>,
}

impl RustSource {
    /// Reads the Rust file `path`, whose text is `text`, into its tokens.
    ///
    /// The tokenizer is proc-macro2's own lexer of Rust, which nests block comments as rustc
    /// does and gives each doc comment as one token. It lexes the whole text, so the start
    /// of a file rustc may strip before lexing is refused instead: `#!` not directly
    /// followed by `[` (a shebang), after an optional byte order mark (which rustc strips).
    /// Frontmatter (`---`), which rustc would strip too, the pinned rustc refuses to
    /// compile (E0658). Every token keeps the line it starts on.
    ///
    /// # Errors
    ///
    /// Returns, naming the file and the line, a text that starts with `#!` not directly
    /// followed by `[`, or that does not lex (an unclosed comment or string, an unbalanced
    /// delimiter, a char literal of two characters …), which rustc refuses too.
    pub fn parse(path: &str, text: &str) -> Result<Self, Failure> {
        let body = text.strip_prefix('\u{feff}').unwrap_or(text);
        if body.starts_with("#!") && !body.starts_with("#![") {
            return Err(Failure::from(format!(
                "{path}:1: starts with `#!` not directly followed by `[`, which rustc may strip \
                 as a shebang before it lexes: an inner attribute is written `#![`, and no Rust \
                 source of the repository holds a shebang"
            )));
        }
        let lexed = proc_macro2::TokenStream::from_str(text)
            .map(|stream| {
                let mut tokens = Vec::new();
                flatten(stream, &mut tokens);
                tokens
            })
            .map_err(|error| {
                Failure::from(format!(
                    "{path}:{}: does not lex as Rust ({error}): the gates read the tokens of \
                     every Rust source",
                    error.span().start().line
                ))
            });
        // Every span of this thread is now dropped: free their record of the text.
        proc_macro2::extra::invalidate_current_thread_spans();
        Ok(Self {
            path: path.to_owned(),
            tokens: lexed?,
        })
    }

    /// Its path, relative to the root.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The index of the closing delimiter of the group whose opening delimiter is at
    /// `open` (every group of a lexed text closes).
    fn close_of(&self, open: usize) -> usize {
        let mut depth = 0_usize;
        self.tokens
            .iter()
            .enumerate()
            .skip(open)
            .find_map(|(index, token)| {
                match token.kind {
                    TokenKind::Open => depth = depth.saturating_add(1),
                    TokenKind::Close => depth = depth.saturating_sub(1),
                    TokenKind::Ident
                    | TokenKind::Punct { .. }
                    | TokenKind::Literal
                    | TokenKind::Doc { .. } => {}
                }
                (depth == 0).then_some(index)
            })
            .unwrap_or(self.tokens.len())
    }

    /// The leading inner attributes of the file, each the range of its tokens: every
    /// `#![…]` and inner doc comment before the first other token, which apply to the
    /// whole file (a `#![…]` in an inline module is that module's).
    pub(crate) fn leading_attributes(&self) -> Vec<core::ops::Range<usize>> {
        let mut attributes = Vec::new();
        let mut at = 0_usize;
        loop {
            match self.tokens.get(at..).unwrap_or_default() {
                [doc, ..] if doc.kind == (TokenKind::Doc { inner: true }) => {
                    at = at.saturating_add(1);
                }
                [hash, bang, open, ..]
                    if hash.is_punct('#') && bang.is_punct('!') && open.opens('[') =>
                {
                    // Its closing `]` is past its `#![`, so each attribute moves past the last.
                    let end = self
                        .close_of(at.saturating_add(2))
                        .saturating_add(1)
                        .max(at.saturating_add(3));
                    attributes.push(at..end);
                    at = end;
                }
                _ => return attributes,
            }
        }
    }

    /// The range of the tokens of `attribute` (an inner attribute written as a constant,
    /// `#![forbid(…)]`) among the leading inner attributes of the file
    /// ([`leading_attributes`](Self::leading_attributes)), if it holds it: the same tokens,
    /// spaces and comments apart, so that the attribute in a comment, a string or an inline
    /// module, or after the file's first item, is not it.
    pub(crate) fn leading_attribute(&self, attribute: &str) -> Option<core::ops::Range<usize>> {
        let wanted = tokens_of(attribute);
        self.leading_attributes()
            .into_iter()
            .find(|range| same_tokens(self.tokens.get(range.clone()).unwrap_or_default(), &wanted))
    }

    /// The ranges of every occurrence of the tokens of `attribute` (written as a constant)
    /// in the file, wherever it stands.
    pub(crate) fn occurrences(&self, attribute: &str) -> Vec<core::ops::Range<usize>> {
        let wanted = tokens_of(attribute);
        self.tokens
            .windows(wanted.len())
            .enumerate()
            .filter(|(_, window)| same_tokens(window, &wanted))
            .map(|(at, _)| at..at.saturating_add(wanted.len()))
            .collect()
    }
}

/// Whether `held` and `wanted` are the same tokens, written alike ([`Token::same`]).
pub(crate) fn same_tokens(held: &[Token], wanted: &[Token]) -> bool {
    held.len() == wanted.len() && held.iter().zip(wanted).all(|(one, other)| one.same(other))
}

/// The tokens of `text`, a constant of xtask written as Rust (none when it does not lex).
pub(crate) fn tokens_of(text: &str) -> Vec<Token> {
    RustSource::parse("", text)
        .map(|source| source.tokens)
        .unwrap_or_default()
}

/// Appends the tokens of `stream` to `tokens` ([`RustSource`]).
pub(crate) fn flatten(stream: proc_macro2::TokenStream, tokens: &mut Vec<Token>) {
    use proc_macro2::{Delimiter, Spacing, TokenTree};
    // The lexer gives a doc comment as `#`, `!` when inner, and a bracketed group, each
    // spanning the comment's own text, which starts with `/`.
    let documents = |tree: &TokenTree| {
        tree.span()
            .source_text()
            .is_some_and(|text| text.starts_with('/'))
    };
    let mut trees = stream.into_iter().peekable();
    while let Some(tree) = trees.next() {
        let line = tree.span().start().line;
        let (kind, text) = match tree {
            // Its own text read only for a `#`: reading every token's is most of the cost.
            TokenTree::Punct(ref hash) if hash.as_char() == '#' && documents(&tree) => {
                let inner = trees
                    // A doc comment's `#` is followed by its `!` when inner, else its group.
                    .next_if(|bang| matches!(bang, TokenTree::Punct(_)))
                    .is_some();
                let _group = trees.next();
                let text = tree.span().source_text().unwrap_or_default();
                (TokenKind::Doc { inner }, text)
            }
            TokenTree::Group(group) => {
                // `Delimiter::None` comes only from a procedural macro's input, never from text.
                let (open, close) = match group.delimiter() {
                    Delimiter::Brace => ("{", "}"),
                    Delimiter::Bracket => ("[", "]"),
                    Delimiter::Parenthesis | Delimiter::None => ("(", ")"),
                };
                tokens.push(Token {
                    kind: TokenKind::Open,
                    text: open.to_owned(),
                    line,
                });
                flatten(group.stream(), tokens);
                tokens.push(Token {
                    kind: TokenKind::Close,
                    text: close.to_owned(),
                    line: group.span_close().start().line,
                });
                continue;
            }
            TokenTree::Ident(ident) => {
                let text = ident.to_string();
                let name = text
                    .strip_prefix("r#")
                    .map_or_else(|| text.clone(), str::to_owned);
                (TokenKind::Ident, name)
            }
            TokenTree::Punct(punct) => (
                TokenKind::Punct {
                    joint: punct.spacing() == Spacing::Joint,
                },
                punct.as_char().to_string(),
            ),
            TokenTree::Literal(literal) => (TokenKind::Literal, literal.to_string()),
        };
        tokens.push(Token { kind, text, line });
    }
}
