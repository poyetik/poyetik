//! Tests of Rust sources read into the tokens of a real tokenizer, as every gate over them reads
//! them.

use crate::Failure;

#[test]
fn reads_a_raw_identifier_as_its_name() {
    let words = |text: &str| -> Vec<String> {
        crate::RustSource::parse("a.rs", text)
            .expect("lexes")
            .tokens
            .into_iter()
            .filter(|token| token.kind == crate::rust_source::TokenKind::Ident)
            .map(|token| token.text)
            .collect()
    };
    assert_eq!(
        words("let r#f64 = r#_x;"),
        ["let", "f64", "_x"],
        "raw identifiers"
    );
    assert_eq!(
        words("mod r#helper;"),
        ["mod", "helper"],
        "a raw module name"
    );
    assert_eq!(words("r #x"), ["r", "x"], "a space: no raw identifier");
    assert_eq!(words("r#\"s\"# a"), ["a"], "a raw string stays prose");
    assert_eq!(words("br#\"s\"# b"), ["b"], "a raw byte string too");
}

#[test]
fn reads_a_rust_source_into_its_tokens() {
    use crate::rust_source::{Token, TokenKind};
    let token = |kind: TokenKind, text: &str, line: usize| Token {
        kind,
        text: text.to_owned(),
        line,
    };
    let (ident, literal, open, close) = (
        TokenKind::Ident,
        TokenKind::Literal,
        TokenKind::Open,
        TokenKind::Close,
    );
    let alone = TokenKind::Punct { joint: false };
    let joint = TokenKind::Punct { joint: true };
    let text = "//! inner\n/*! block */\nx /* a /* b */ c */ y\n/// outer\n/** block */\n!r#fn \"multi\nline\" 1.0f32 == = ( [ {\n} ] )\n#[doc = \"w\"] '\\'' 'a\n";
    let source = crate::RustSource::parse("crates/a/src/x.rs", text).expect("lexes");
    assert_eq!(source.path(), "crates/a/src/x.rs", "its path");
    assert_eq!(
        source.tokens,
        [
            token(TokenKind::Doc { inner: true }, "//! inner", 1),
            token(TokenKind::Doc { inner: true }, "/*! block */", 2),
            token(ident, "x", 3),
            token(ident, "y", 3),
            token(TokenKind::Doc { inner: false }, "/// outer", 4),
            token(TokenKind::Doc { inner: false }, "/** block */", 5),
            token(alone, "!", 6),
            token(ident, "fn", 6),
            token(literal, "\"multi\nline\"", 6),
            token(literal, "1.0f32", 7),
            token(joint, "=", 7),
            token(alone, "=", 7),
            token(alone, "=", 7),
            token(open, "(", 7),
            token(open, "[", 7),
            token(open, "{", 7),
            token(close, "}", 8),
            token(close, "]", 8),
            token(close, ")", 8),
            token(alone, "#", 9),
            token(open, "[", 9),
            token(ident, "doc", 9),
            token(alone, "=", 9),
            token(literal, "\"w\"", 9),
            token(close, "]", 9),
            token(literal, "'\\''", 9),
            token(joint, "'", 9),
            token(ident, "a", 9),
        ],
        "nested comments dropped, doc comments as prose, lines where each token starts"
    );
    for lexed in [
        "\u{feff}fn f() {}",
        "#![forbid(x)]\n",
        "fn f() {}\n---\n",
        "",
        // Frontmatter, which the pinned rustc refuses to compile (E0658).
        "---\nfn f() {}",
        "\u{feff}---\n",
    ] {
        assert!(
            crate::RustSource::parse("a.rs", lexed).is_ok(),
            "what rustc lexes as it is, or refuses: {lexed:?}"
        );
    }
    let start = "a.rs:1: starts with `#!` not directly followed by `[`, which rustc may strip \
                 as a shebang before it lexes: an inner attribute is written `#![`, and no Rust \
                 source of the repository holds a shebang";
    for stripped in [
        "#!/usr/bin/env run\nfn f() {}",
        "\u{feff}#! \"\nfn f() {}\n// \"",
        "#! [forbid(x)]",
    ] {
        assert_eq!(
            crate::RustSource::parse("a.rs", stripped).err(),
            Some(Failure::from(start.to_owned())),
            "a start rustc strips: {stripped:?}"
        );
    }
    for (unlexed, line) in [
        ("fn f() {}\n/* unclosed /* */", 2_usize),
        ("let s = 1;\n\nlet t = \"unclosed", 3_usize),
        ("let s = r#\"unclosed\"", 1_usize),
        ("fn f() { (\n}", 2_usize),
        ("let c = 'ab';", 1_usize),
        ("let q = '\\", 1_usize),
    ] {
        assert_eq!(
            crate::RustSource::parse("a.rs", unlexed).err(),
            Some(Failure::from(format!(
                "a.rs:{line}: does not lex as Rust (cannot parse string into token stream): the \
                 gates read the tokens of every Rust source"
            ))),
            "{unlexed:?}"
        );
    }
}

#[test]
fn reads_the_leading_inner_attributes_of_a_file() {
    let leading = |text: &str| {
        crate::RustSource::parse("a.rs", text)
            .expect("lexes")
            .leading_attributes()
    };
    assert_eq!(
        leading("//! A.\n#![a(b, [c])]\n/*! B. */\n#![d]\nfn f() {}\n#![e]\n"),
        [1..13, 14..19],
        "inner doc comments between them, nested groups, none after an item"
    );
    assert_eq!(
        leading("#![a]\n#!b\n"),
        core::iter::once(0..5).collect::<Vec<_>>(),
        "a `#!` opening no attribute ends them"
    );
    assert_eq!(
        leading("/// A.\n#![a]\n"),
        [],
        "after an outer doc comment, none"
    );
    assert_eq!(leading(""), [], "an empty file");
}

#[test]
fn reads_what_a_literal_holds_between_its_quotes() {
    assert_eq!(crate::rust_source::quoted("\"a\""), Some("a"));
    assert_eq!(crate::rust_source::quoted("\"\""), Some(""));
    assert_eq!(
        crate::rust_source::quoted("\""),
        None,
        "one quote opens and closes nothing"
    );
    assert_eq!(
        crate::rust_source::quoted("r\"a\""),
        None,
        "no opening quote"
    );
    assert_eq!(
        crate::rust_source::quoted("\"a\"x"),
        None,
        "a suffix after the closing quote"
    );
}
