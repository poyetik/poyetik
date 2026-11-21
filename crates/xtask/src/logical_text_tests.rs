//! Tests of the one reader of logical text: paragraphs, string runs, code blocks and the
//! lexer of Rust and TypeScript prose.

use crate::logical_text::{
    Language, Logical, code_blocks, code_prose, json_prose, logical_texts, markdown_prose,
    prose_texts,
};

/// Each logical text of prose `code_prose` reads in `text`, as `N:text` (`N` the number of
/// its first line), a run of literals marked `"`.
fn read(text: &str, language: Language) -> Vec<String> {
    code_prose(text, language)
        .into_iter()
        .map(|prose| {
            let mark = if prose.literal { "\"" } else { "" };
            numbered(vec![prose.text])
                .concat()
                .replacen(':', &format!(":{mark}"), 1)
        })
        .collect()
}

/// Each logical text of `texts` as `N:text` (`N` the number of its first line).
fn numbered(texts: Vec<Logical>) -> Vec<String> {
    texts
        .into_iter()
        .map(|(words, spans)| {
            let line = spans
                .first()
                .map_or(0, |(index, _)| index.saturating_add(1));
            format!("{line}:{words}")
        })
        .collect()
}

#[test]
fn reads_the_prose_of_each_language() {
    let rust = "/// A doc.\nlet a = \"text {name} here\"; // trailing\nlet b = 'x'; let c = '\\''; fn f<'a>() {}\n/* block\n   two */ let d = r#\"raw \"quoted\" text\"#;\nlet e = \"word\"; let f = r\"plain raw\"; let g = br\"two bytes\";\nlet h = \"a\\\n    b\";\n";
    assert_eq!(
        read(rust, Language::Rust),
        [
            "1:A doc.",
            "2:trailing",
            "4:block two",
            "2:\"text {name} here",
            "5:\"raw \"quoted\" text",
            "6:\"word",
            "6:\"plain raw",
            "6:\"two bytes",
            "7:\"a b",
        ],
        "comment paragraphs without their markers, then literal runs; characters and \
         lifetimes apart; an escaped line break and the indentation after it join a literal"
    );
    let script = "const a = 'single \\'quoted\\''; // note\nconst b = `tem ${node.x} plate ${{ a: 1 }.a}`;\nconst c = `two\n${\n0} lines`;\nconst d = \"it's\"; /* unclosed";
    assert_eq!(
        read(script, Language::Script),
        [
            "1:note",
            "6:unclosed",
            "1:\"single 'quoted'",
            "2:\"tem plate",
            "3:\"two lines",
            "6:\"it's",
        ],
        "every quote, substitutions apart, line numbers kept"
    );
    assert_eq!(
        numbered(markdown_prose(
            "x.md",
            "# Title\n\n```sh\nnode x\n```\ntext\nwrapped\n"
        )),
        ["1:# Title", "6:text wrapped"],
        "fenced code apart, a paragraph joined"
    );
    let json = "{\n  \"description\": \"a long render\", \"key\": \"word\",\n  \"say \\\"x\\\" here\" : [\"two words\", \"esc\\\\aped \\\"q\\\"\"],\n  \"open\": \"to the end\n}\n";
    assert_eq!(
        numbered(json_prose(json)),
        [
            "2:a long render",
            "2:word",
            "3:two words",
            "3:esc\\aped \"q\"",
            "4:to the end",
        ],
        "string values, keys apart, escapes read, an unclosed string to the end of its line"
    );
    let literal = |path: &str, text: &str| -> Vec<bool> {
        prose_texts(path, text)
            .into_iter()
            .map(|prose| prose.literal)
            .collect()
    };
    assert_eq!(literal("spec/b.json", "[\"a b\"]"), [true], "JSON");
    assert_eq!(
        literal("a/b.tsx", "// a b\n'c d';"),
        [false, true],
        "a script"
    );
    assert_eq!(literal("a/b.txt", "a b\n"), [false], "a text file");
    assert_eq!(
        literal("x.toml", "# a b\nk = 1\n"),
        [false],
        "a hash comment"
    );
    assert_eq!(
        literal("x.wgsl", "// a b\nfn f() {}\n"),
        [false],
        "a shader"
    );
    assert_eq!(
        literal("Cargo.lock", "a b\n"),
        Vec::<bool>::new(),
        "no prose"
    );
}

#[test]
fn joins_a_comment_paragraph_and_a_string_run() {
    // Findings H17-1, H5-11 and H17-3 of the fifth deep review of visual block 2 (#34): a
    // string continued by a Rust `\` or a TypeScript `+`, and a term wrapped over two lines
    // of a comment, were read one line at a time.
    for (text, language, expected) in [
        (
            "let a = \"quite \\\n         still\";",
            Language::Rust,
            vec!["1:\"quite still"],
        ),
        (
            "const a = \"quite \" +\n  \"still\";",
            Language::Script,
            vec!["1:\"quite still"],
        ),
        (
            "const a = \"quite \" + b + \"still\";",
            Language::Script,
            vec!["1:\"quite", "1:\"still"],
        ),
        (
            "const a = \"quite \" ++ \"still\";",
            Language::Script,
            vec!["1:\"quite", "1:\"still"],
        ),
        (
            "let a = (\"quite \", \"still\");",
            Language::Rust,
            vec!["1:\"quite", "1:\"still"],
        ),
        (
            "// trait\n// object\n//\n// one\nlet a = 1; // two\n// three\n\n// four",
            Language::Rust,
            vec!["1:trait object", "4:one", "5:two", "6:three", "8:four"],
        ),
        (
            "/// a\n// b\n//! c\n/**\n * d\n * e\n *\n * - f\n * g\n */",
            Language::Rust,
            vec!["1:a", "2:b", "3:c", "5:d e", "8:- f g"],
        ),
    ] {
        assert_eq!(read(text, language), expected, "{text}");
    }
}

#[test]
fn ends_a_code_block_at_a_blank_line() {
    // Finding H17-10 of the fifth deep review of visual block 2 (#34): the whole text was
    // joined across blank lines, so a paragraph's last word met the next one's first.
    assert_eq!(
        code_blocks("a\nb\n\n  \nc\n"),
        [vec![(0, "a"), (1, "b")], vec![(4, "c")]],
        "blocks of the lines between blank lines"
    );
    assert_eq!(
        numbered(logical_texts(
            "x.rs",
            "let a =\n    b;\n\nlet c = d; // e\n"
        )),
        ["1:let a = b;", "4:let c = d; // e", "4:e"],
        "each block, then each text of prose"
    );
    assert_eq!(
        numbered(logical_texts(
            ".githooks/pre-push",
            "#!/bin/sh\n# quite\n# still\nexec x\n"
        )),
        ["1:#!/bin/sh # quite # still exec x", "2:quite still"],
        "a hook's comments joined without their marks"
    );
}

#[test]
fn reads_a_regular_expression_literal_as_code() {
    for (text, expected) in [
        ("const r = /it's \"x\"/g; // after", "1:after"),
        ("if (/['\"]/.test(s)) { f('a b'); }", "1:\"a b"),
        ("const r = /[/'\"]/; const s = 'c d';", "1:\"c d"),
        ("const r = /a\\/'/; const s = 'e f';", "1:\"e f"),
        ("return /'/.test(x) ? 'g h' : '';", "1:\"g h"),
        ("x = [/'/, !/\"/, a && /'/];\nconst s = 'i j';", "2:\"i j"),
        ("const r = /'\nconst s = 'k l';", "2:\"k l"),
        ("const n = (a) / b; const t = 'm n' / 2;", "1:\"m n"),
        ("const n = a / b / c; const t = x[0] / 'o p';", "1:\"o p"),
        ("const n = a /* c d */ / b;", "1:c d"),
        ("typeof /'/; const t = 'q r';", "1:\"q r"),
        ("const t = 's t'; const r = /'", "1:\"s t"),
        ("n = a / b; t = 'u v'; m = c / d;", "1:\"u v"),
        ("n = a_ / b; t = 'u v'; m = c_ / d;", "1:\"u v"),
        ("n = $ / b; t = 'u v'; m = $ / d;", "1:\"u v"),
        // A JSX tag's `/` opens no literal: the string or the comment after it is prose.
        ("<p>a</p>{\"w x\"}", "1:\"w x"),
        ("<X a={b} />{\"w x\"}", "1:\"w x"),
        ("<p>a</p> // w x", "1:w x"),
        // An increment or a decrement ends an operand: a `/` after it is a division.
        ("n = a++ / b; t = 'y z'; m = c / d;", "1:\"y z"),
        ("n = a-- / b; t = 'y z'; m = c / d;", "1:\"y z"),
        ("n = a + /'/.x; t = 'y z';", "1:\"y z"),
        ("n = a - /'/.x; t = 'y z';", "1:\"y z"),
        ("s = r.replace(/>/g, 'y z');", "1:\"y z"),
        ("if (a) {} /'/.test(b); t = 'y z';", "1:\"y z"),
    ] {
        assert_eq!(
            read(text, Language::Script).join("|"),
            expected,
            "a regular expression literal is code, its quotes none; a division is no \
             literal: {text}"
        );
    }
    assert_eq!(
        read("let a = b / c; let s = \"it's / here\";", Language::Rust),
        ["1:\"it's / here"],
        "Rust has no regular expression literal"
    );
}

#[test]
fn reads_odd_sources_without_losing_prose() {
    let rust = "let a = \"\"; let r#type = ar\"one two\"; x ` y\nlet b = \"{a b} } c\";";
    assert_eq!(
        read(rust, Language::Rust),
        ["1:\"one two", "2:\"{a b} } c"],
        "an empty literal (no text), a raw identifier, an `r` ending a name, a backquote"
    );
    let script = "const a = r\"one two\"; const b = 'a {b} c'; const c = `cost $5 and '$x'`; const d = 'tab\\there';";
    assert_eq!(
        read(script, Language::Script),
        [
            "1:\"one two",
            "1:\"a {b} c",
            "1:\"cost $5 and '$x'",
            "1:\"tab here"
        ],
        "no raw strings, braces kept, a dollar sign that starts no substitution, an escape"
    );
    let quotes = "let q = '\\\"'; // one\nlet d = '\"'; // two\n'\\x41'\"'tail word\"\n_r\"a\\tb c\" ar\"d\\te f\" br\"g\\th i\"";
    assert_eq!(
        read(quotes, Language::Rust),
        [
            "1:one",
            "2:two",
            "3:\"'tail word",
            "4:\"a b c",
            "4:\"d e f",
            "4:\"g\\th i"
        ],
        "character literals holding a quote, an escape of several characters, an `r` ending \
         a name (after `_` or a letter), a raw byte string"
    );
    for (text, language, expected) in [
        ("r\"open to the end", Language::Rust, "1:\"open to the end"),
        ("\"open to the end", Language::Rust, "1:\"open to the end"),
        ("`open ${to the end", Language::Script, "1:\"open"),
        ("'\\", Language::Rust, ""),
    ] {
        assert_eq!(
            read(text, language).join("|"),
            expected,
            "an unterminated literal: {text}"
        );
    }
}

/// A gate over text, run on one file (its path and text), as the rewrap law runs it.
type TextGate = fn(&str, &str) -> Result<(), crate::Failure>;

/// A gate over text, its name, how it runs on one file, and its refusal fixtures.
type TextGateEntry = (&'static str, TextGate, Vec<(&'static str, &'static str)>);

/// The refusal fixtures of the citation gate, a card written `\u{23}N` and the visual
/// design's name `VD`, so that the gate, reading this file, sees no citation here.
fn citation_fixtures() -> Vec<(&'static str, &'static str)> {
    vec![
        ("x.md", "As `spec/VD`, decision 1, says."),
        ("x.rs", "/// As [`VD`](VD), decision 2, says."),
        ("x.toml", "# As spec/VD, decision 9."),
        ("spec/visual-design.md", "Linear light, as decision 3 says."),
        ("x.md", "Measured at \u{23}12 step 4 and later."),
        (
            "x.rs",
            "//! Measured at \u{23}12 steps 2 to 5, then \u{23}7 step 1.",
        ),
        (
            "x.md",
            "**Step.** \u{23}3 step 1, then \u{23}12: at step 4.",
        ),
        (
            "x.ts",
            "// Kept as the roadmap \u{23}12 says, at \u{23}12 item 2.",
        ),
        ("x.md", "It is not yet done (\u{23}12 step 1)."),
        (
            ".githooks/pre-push",
            "# It is not yet done (\u{23}12 step 1).",
        ),
    ]
}

/// The citation gate on one file, with a board of card 12 of three steps, the first done.
fn citation_text_gate(path: &str, text: &str) -> Result<(), crate::Failure> {
    crate::citation_gate(&[
        (
            crate::ROADMAP_BOARD.to_owned(),
            r#"{"cards":[{"id":12,"tasks":[{"done":true},{"done":false},{}]}]}"#.to_owned(),
        ),
        (path.to_owned(), text.to_owned()),
    ])
}

/// The list counts gate on one file, beside a source declaring `PAIR` and a register.
fn list_counts_text_gate(path: &str, text: &str) -> Result<(), crate::Failure> {
    crate::list_counts_gate(&[
        (
            "crates/a/src/lists.rs".to_owned(),
            "const PAIR: [&str; 2] = [\"x\", \"y\"];\n".to_owned(),
        ),
        (
            "register.txt".to_owned(),
            "a -- why\nb -- why\nc -- why\n".to_owned(),
        ),
        (path.to_owned(), text.to_owned()),
    ])
}

/// The retired patterns gate on one file, two patterns retired.
fn retired_text_gate(path: &str, text: &str) -> Result<(), crate::Failure> {
    crate::retired_patterns_gate(
        &[(path.to_owned(), text.to_owned())],
        "retire `weak() ||` -- vacuous\nretire `gave nothing of it back` -- reworded\n",
    )
}

/// The vocabulary gate on one file, a word and a phrase refused.
fn vocabulary_text_gate(path: &str, text: &str) -> Result<(), crate::Failure> {
    crate::vocabulary_gate(
        &[(path.to_owned(), text.to_owned())],
        "forbid node -> box -- the glossary\nforbid tap block -> render quantum -- the glossary\n",
    )
}

/// Every gate over text that reads it through the one reader of logical text
/// ([`crate::logical_text`]), each with its refusal fixtures, one line of a file each (never
/// a JSON string, which holds no line break, so that no editor wraps it): the
/// rewrap law (`every_text_gate_refuses_its_fixtures_in_every_rewrap`) refuses each in every
/// rewrap of it ([`crate::tests::rewraps`]). A gate that comes to read text through the reader joins it.
fn text_gates() -> Vec<TextGateEntry> {
    vec![
        ("citation_gate", citation_text_gate, citation_fixtures()),
        (
            "list_counts_gate",
            list_counts_text_gate,
            vec![
                ("CLAUDE.md", "The two names of `PAIR`, wrapped."),
                (
                    "README.md",
                    "- Each reviewed in `register.txt` (3 reviewed).",
                ),
                (
                    "crates/a/src/lib.rs",
                    "/// The `PAIR` of two entries, each a name.",
                ),
                ("tooling/a.ts", "// PAIR holds 2 names, a comment."),
            ],
        ),
        (
            "retired_patterns_gate",
            retired_text_gate,
            vec![
                (
                    "x_tests.rs",
                    "        assert!(took.weak() || timed.period.is_some(), \"why\");",
                ),
                ("x.rs", "/// The GPU gave nothing of it back to the caller."),
                (
                    "x.ts",
                    "    // The GPU gave nothing of it back to the caller.",
                ),
                ("x.toml", "# The GPU gave nothing of it back to the caller."),
                (
                    ".githooks/pre-push",
                    "# The GPU gave nothing of it back to the caller.",
                ),
                ("x.md", "The GPU gave nothing of it back to the caller."),
                ("x.rs", "    let a = \"the GPU gave nothing of it back\";"),
                ("x.ts", "    const a = \"the GPU gave nothing of it back\";"),
            ],
        ),
        (
            "vocabulary_gate",
            vocabulary_text_gate,
            vec![
                (
                    "README.md",
                    "The engine's tap block runs here, each node too.",
                ),
                (
                    "crates/a/src/lib.rs",
                    "/// The engine's tap block runs here.",
                ),
                ("x.ts", "    // The engine's tap block runs here."),
                ("x.rs", "    let a = \"the engine's tap block runs here\";"),
                (
                    "x.ts",
                    "    const a = 'the engine\\'s tap block runs here';",
                ),
            ],
        ),
    ]
}

#[test]
fn every_text_gate_refuses_its_fixtures_in_every_rewrap() {
    // The law of a gate reading text whole across lines (lessons of the fourth deep review
    // of visual block 2 (#34), F5-1 and F7-2, and #24 step 170): each refusal fixture of every
    // gate over text is refused, for the same problems, however an editor or a formatter
    // wraps it.
    for (gate, run, fixtures) in text_gates() {
        for (path, text) in fixtures {
            // VD for the visual design's name, so that the citation gate, reading this
            // file, does not take the fixtures for citations.
            let text = &text.replace("VD", "visual-design.md");
            let refused = run(path, text);
            assert!(refused.is_err(), "{gate}: {path}: {text}");
            let wraps = crate::tests::rewraps(path, text);
            assert!(!wraps.is_empty(), "{gate}: {text}");
            for wrapped in wraps {
                assert_eq!(
                    crate::tests::problems_anywhere(&run(path, &wrapped)),
                    crate::tests::problems_anywhere(&refused),
                    "{gate}: {path}: {wrapped}"
                );
            }
        }
    }
}
