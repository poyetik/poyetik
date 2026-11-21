//! Tests of the vocabulary gate: the words the glossary replaces, refused in the repository's
//! prose.

use crate::tests::{FAKE_WORDS, failed, passing};
use crate::{Action, Failure, Step, run_steps};

#[test]
fn reads_the_prose_of_each_language() {
    let rust = "/// A doc.\nlet a = \"text {name} here\"; // trailing\nlet b = 'x'; let c = '\\''; fn f<'a>() {}\n/* block\n   two */ let d = r#\"raw \"quoted\" text\"#;\nlet e = \"word\"; let f = r\"plain raw\"; let g = br\"two bytes\";\nlet h = \"a\\\nb\";\n";
    assert_eq!(
        crate::vocabulary::code_prose(rust, crate::vocabulary::Language::Rust),
        [
            (1, "/// A doc."),
            (2, "text  here"),
            (2, "// trailing"),
            (4, " block"),
            (5, "   two "),
            (5, "raw \"quoted\" text"),
            (6, "plain raw"),
            (6, "two bytes"),
            (7, "a"),
            (8, "b"),
        ]
        .map(|(line, text)| (line, text.to_owned()))
        .to_vec(),
        "comments and literals, placeholders, single words, characters and lifetimes apart"
    );
    let script = "const a = 'single \\'quoted\\''; // note\nconst b = `tem ${node.x} plate ${{ a: 1 }.a}`;\nconst c = `two\n${\n0} lines`;\nconst d = \"it's\"; /* unclosed";
    assert_eq!(
        crate::vocabulary::code_prose(script, crate::vocabulary::Language::Script),
        [
            (1, "single 'quoted'"),
            (1, "// note"),
            (2, "tem   plate  "),
            (3, "two"),
            (5, " lines"),
            (6, "it's"),
            (6, " unclosed"),
        ]
        .map(|(line, text)| (line, text.to_owned()))
        .to_vec(),
        "every quote, substitutions apart, line numbers kept"
    );
    assert_eq!(
        crate::vocabulary::markdown_prose("# Title\n\n```sh\nnode x\n```\ntext\n"),
        [(1, "# Title"), (2, ""), (6, "text")]
            .map(|(line, text)| (line, text.to_owned()))
            .to_vec(),
        "fenced code apart"
    );
    assert_eq!(
        crate::vocabulary::without_code("a `node` [link](node.md) (node) b"),
        "a        [link]          (node) b",
        "code spans and link targets apart"
    );
    let json = "{\n  \"description\": \"a long render\", \"key\": \"word\",\n  \"say \\\"x\\\" here\" : [\"two words\", \"esc\\\\aped \\\"q\\\"\"],\n  \"open\": \"to the end\n}\n";
    assert_eq!(
        crate::vocabulary::json_prose(json),
        [
            (2, "a long render"),
            (3, "two words"),
            (3, "esc\\aped \"q\""),
            (4, "to the end"),
        ]
        .map(|(line, text)| (line, text.to_owned()))
        .to_vec(),
        "string values, keys and single words apart, escapes read, an unclosed string to the \
         end of its line"
    );
    assert_eq!(
        crate::vocabulary::language("spec/b.json"),
        Some(crate::vocabulary::Language::Json)
    );
    assert_eq!(
        crate::vocabulary::language("a/b.tsx"),
        Some(crate::vocabulary::Language::Script)
    );
    assert_eq!(crate::vocabulary::language("a/b.txt"), None);
    assert_eq!(crate::vocabulary::language("Makefile"), None);
}

#[test]
fn finds_only_words_of_prose() {
    for (text, expected) in [
        ("the node runs", vec![4]),
        ("node.", vec![0]),
        ("a node's id", vec![2]),
        ("nodes", vec![]),
        ("anode", vec![]),
        ("node_id", vec![]),
        ("engine-node", vec![]),
        (".node-version", vec![]),
        ("/engines/node", vec![]),
        ("Node.js node:fs", vec![]),
        ("\"node\" 'node'", vec![]),
        ("\"node", vec![1]),
        ("node: a box", vec![0]),
        ("@node #node $node ~node \\node", vec![]),
    ] {
        let term = if text.starts_with("Node") {
            "Node"
        } else {
            "node"
        };
        assert_eq!(
            crate::vocabulary::occurrences(text, term),
            expected,
            "{text}"
        );
    }
}

/// The vocabulary of the gate's tests.
const WORDS: &str = "# words\n\n\
    forbid node -> box -- the glossary\n\
    forbid object -> box in crates/ -- the glossary\n\
    forbid tap block -> render quantum -- the glossary\n\
    allow node as \"engine node\" -- Web Audio\n\
    allow node in web/ -- Web Audio\n\
    allow object as \"JSON object\" -- JSON\n";

#[test]
fn refuses_the_synonyms_the_glossary_replaces() {
    let sources: Vec<(String, String)> = [
        (
            "README.md",
            "A node, an engine node.\n\n```\nnode\n```\nA `node`, a tap block.\nAn engine node, a node.\n",
        ),
        (
            "crates/a/src/lib.rs",
            "/// An object, a JSON object.\nfn f() { let node = \"the node\"; }\n",
        ),
        (
            "apps/a/b.ts",
            "// an object here is fine\nconst node = 'node';\n",
        ),
        ("web/c.ts", "/** A node. */\n"),
        ("notes.txt", "node\n"),
    ]
    .map(|(path, text)| (path.to_owned(), text.to_owned()))
    .to_vec();
    let context = "words the glossary replaces (reviewed in vocabulary.txt)";
    assert_eq!(
        crate::vocabulary_gate(&sources, WORDS),
        failed(format!(
            "{context}: 5 problems, listed above\n\
             README.md:1: \"node\": say \"box\" (the glossary)\n\
             README.md:6: \"tap block\": say \"render quantum\" (the glossary)\n\
             README.md:7: \"node\": say \"box\" (the glossary)\n\
             crates/a/src/lib.rs:1: \"object\": say \"box\" (the glossary)\n\
             crates/a/src/lib.rs:2: \"node\": say \"box\" (the glossary)"
        )),
        "prose only, each rule where it applies, allowed contexts apart"
    );
    let clean: Vec<(String, String)> = [(
        "web/c.ts",
        "/** A node: an engine node, a JSON object. */\n",
    )]
    .map(|(path, text)| (path.to_owned(), text.to_owned()))
    .to_vec();
    assert_eq!(
        crate::vocabulary_gate(
            &clean,
            &format!("{WORDS}allow box in x/ -- r\nallow node in other/ -- r\nforbid broken\n")
        ),
        failed(format!(
            "{context}: 4 problems, listed above\n\
             vocabulary.txt:11: write `forbid <term> -> <instead>[ in <path>…] -- <reason>`, `allow <term> in <path> -- <reason>` or `allow <term> as \"<phrase>\" -- <reason>`\n\
             stale: allow object as \"JSON object\": it allows nothing\n\
             allow box in x/: no rule refuses \"box\"\n\
             stale: allow node in other/: it allows nothing"
        )),
        "a malformed line, an allowance of nothing refused, allowances that allow nothing"
    );
    for malformed in [
        "forbid a -- r",
        "allow a -- r",
        "allow a as b -- r",
        "allow a as \"b -- r",
        "permit a in b -- r",
        "forbid a -> b",
    ] {
        assert!(
            crate::vocabulary_gate(&[], malformed)
                .is_err_and(|failure| failure.summary.contains("vocabulary.txt:1: write")),
            "{malformed}"
        );
    }
    assert!(crate::vocabulary::within("web/c.ts", "web/"), "a directory");
    assert!(crate::vocabulary::within("web", "web"), "a file");
    assert!(
        !crate::vocabulary::within("web/c.ts", "web"),
        "a file is not a directory"
    );
}

#[test]
fn reads_a_star_as_any_part_of_one_name() {
    // A list of crates copied into each rule missed the crate born after it (finding C7-5 of
    // the third deep review of visual block 2, #34: crates/visual-gpu/): a star names them
    // all, those to come included.
    let within = crate::vocabulary::within;
    for (path, scope) in [
        ("crates/visual-gpu/src/a.rs", "crates/visual-*/"),
        ("crates/visual-/a.rs", "crates/visual-*/"),
        ("crates/visual-core/src/b/c.rs", "crates/visual-*/"),
        ("spec/image-hash.md", "spec/image-*.md"),
        ("spec/x.md", "spec/*"),
    ] {
        assert!(within(path, scope), "{path} within {scope}");
    }
    for (path, scope) in [
        ("crates/audio-core/src/a.rs", "crates/visual-*/"),
        ("crates/visual-gpu", "crates/visual-*/"),
        ("crates/visual/gpu/a.rs", "crates/visual-*/"),
        ("spec/image-hash.json", "spec/image-*.md"),
        ("spec/a/x.md", "spec/*"),
    ] {
        assert!(!within(path, scope), "{path} not within {scope}");
    }
    assert_eq!(
        crate::vocabulary_gate(
            &[(
                "crates/visual-gpu/src/a.rs".to_owned(),
                "// A frame.\n".to_owned()
            )],
            "forbid frame -> image in crates/visual-*/ -- the visual library's word\n"
        ),
        failed(
            "words the glossary replaces (reviewed in vocabulary.txt): \
             crates/visual-gpu/src/a.rs:1: \"frame\": say \"image\" (the visual library's word)"
        )
    );
}

#[test]
fn leaves_the_released_sections_of_the_changelog_as_they_were_released() {
    let changelog = "# Changelog\n\n## [Unreleased]\n\n- a node.\n\n## [0.2.0] - 2026-1-01\n\n\
                     - a node.\n\n## [0.1.0] - 2026-11-08\n\n- a node.\n\n## [0.0.1] - 2026-11-01\n\
                     \n- a node.\n";
    let sources: Vec<(String, String)> = [
        ("CHANGELOG.md", changelog),
        ("docs/CHANGELOG.md", changelog),
    ]
    .map(|(path, text)| (path.to_owned(), text.to_owned()))
    .to_vec();
    assert_eq!(
        crate::vocabulary_gate(&sources, FAKE_WORDS),
        failed(
            "words the glossary replaces (reviewed in vocabulary.txt): 6 problems, listed above\n\
             CHANGELOG.md:5: \"node\": say \"box\" (the glossary)\n\
             CHANGELOG.md:9: \"node\": say \"box\" (the glossary)\n\
             docs/CHANGELOG.md:5: \"node\": say \"box\" (the glossary)\n\
             docs/CHANGELOG.md:9: \"node\": say \"box\" (the glossary)\n\
             docs/CHANGELOG.md:13: \"node\": say \"box\" (the glossary)\n\
             docs/CHANGELOG.md:17: \"node\": say \"box\" (the glossary)"
        ),
        "the root changelog's prose read up to its first released heading, a malformed one \
         opening nothing; any other file read whole"
    );
}

#[test]
fn reads_the_prose_files_and_the_vocabulary() {
    let step = [Step {
        name: "Words".to_owned(),
        action: Action::VocabularyGate,
    }];
    let mut fake = passing();
    let replaced = fake
        .files
        .insert("README.md".to_owned(), "A node.\n".to_owned());
    assert!(replaced.is_some(), "README replaced");
    for (path, text) in [
        ("spec/v.json", "{\"description\": \"a node, then\"}\n"),
        ("conformance/cases/c.json", "{\"note\": \"one node\"}\n"),
    ] {
        let added = fake.files.insert(path.to_owned(), text.to_owned());
        assert!(added.is_none(), "fresh fake");
    }
    let board = fake.files.insert(
        crate::ROADMAP_BOARD.to_owned(),
        "{\"title\": \"a node\"}\n".to_owned(),
    );
    assert!(board.is_some(), "the board replaced");
    assert_eq!(
        run_steps(&step, &mut fake, &mut Vec::new()),
        Err(Failure {
            summary: "Words: words the glossary replaces (reviewed in vocabulary.txt): 3 \
                      problems, listed above"
                .to_owned(),
            details: [
                "README.md:1: \"node\": say \"box\" (the glossary)",
                "conformance/cases/c.json:1: \"node\": say \"box\" (the glossary)",
                "spec/v.json:1: \"node\": say \"box\" (the glossary)",
            ]
            .map(str::to_owned)
            .to_vec(),
            usage: false,
        }),
        "the tracked prose against the vocabulary, the JSON of the specifications and the \
         corpus included, the board's apart"
    );
    let mut lacking = passing();
    let removed = lacking.files.remove(crate::VOCABULARY);
    assert!(removed.is_some(), "vocabulary removed");
    assert_eq!(
        run_steps(&step, &mut lacking, &mut Vec::new()),
        failed("Words: cannot read vocabulary.txt: missing"),
        "no vocabulary"
    );
    let mut unlisted = passing();
    unlisted.broken = Some(
        "list *.md *.rs *.ts *.tsx *.mts *.js *.mjs spec/*.json conformance/*.json".to_owned(),
    );
    assert!(
        run_steps(&step, &mut unlisted, &mut Vec::new()).is_err_and(|failure| failure
            .summary
            .starts_with("Words: cannot list the prose files")),
        "the files cannot be listed"
    );
}

#[test]
fn reads_a_regular_expression_literal_as_code() {
    let lines = |text: &str| -> Vec<String> {
        crate::vocabulary::code_prose(text, crate::vocabulary::Language::Script)
            .into_iter()
            .map(|(line, prose)| format!("{line}:{prose}"))
            .collect()
    };
    for (text, expected) in [
        ("const r = /it's \"x\"/g; // after", "1:// after"),
        ("if (/['\"]/.test(s)) { f('a b'); }", "1:a b"),
        ("const r = /[/'\"]/; const s = 'c d';", "1:c d"),
        ("const r = /a\\/'/; const s = 'e f';", "1:e f"),
        ("return /'/.test(x) ? 'g h' : '';", "1:g h"),
        ("x = [/'/, !/\"/, a && /'/];\nconst s = 'i j';", "2:i j"),
        ("const r = /'\nconst s = 'k l';", "2:k l"),
        ("const n = (a) / b; const t = 'm n' / 2;", "1:m n"),
        ("const n = a / b / c; const t = x[0] / 'o p';", "1:o p"),
        ("const n = a /* c d */ / b;", "1: c d "),
        ("typeof /'/; const t = 'q r';", "1:q r"),
        ("const t = 's t'; const r = /'", "1:s t"),
        ("n = a / b; t = 'u v'; m = c / d;", "1:u v"),
        ("n = a_ / b; t = 'u v'; m = c_ / d;", "1:u v"),
        ("n = $ / b; t = 'u v'; m = $ / d;", "1:u v"),
        // A JSX tag's `/` opens no literal: the string or the comment after it is prose.
        ("<p>a</p>{\"w x\"}", "1:w x"),
        ("<X a={b} />{\"w x\"}", "1:w x"),
        ("<p>a</p> // w x", "1:// w x"),
        // An increment or a decrement ends an operand: a `/` after it is a division.
        ("n = a++ / b; t = 'y z'; m = c / d;", "1:y z"),
        ("n = a-- / b; t = 'y z'; m = c / d;", "1:y z"),
        ("n = a + /'/.x; t = 'y z';", "1:y z"),
        ("n = a - /'/.x; t = 'y z';", "1:y z"),
        ("s = r.replace(/>/g, 'y z');", "1:y z"),
    ] {
        assert_eq!(
            lines(text).join("|"),
            expected,
            "a regular expression literal is code, its quotes none; a division is no \
             literal: {text}"
        );
    }
    assert_eq!(
        crate::vocabulary::code_prose(
            "let a = b / c; let s = \"it's / here\";",
            crate::vocabulary::Language::Rust
        ),
        [(1, "it's / here".to_owned())],
        "Rust has no regular expression literal"
    );
}

#[test]
fn reads_odd_sources_without_losing_prose() {
    let lines = |text: &str, language| -> Vec<String> {
        crate::vocabulary::code_prose(text, language)
            .into_iter()
            .map(|(line, prose)| format!("{line}:{prose}"))
            .collect()
    };
    let rust = "let a = \"\"; let r#type = ar\"one two\"; x ` y\nlet b = \"{a b} } c\";";
    assert_eq!(
        lines(rust, crate::vocabulary::Language::Rust),
        ["1:one two", "2:  } c"],
        "an empty literal, a raw identifier, an `r` ending a name, a backquote; in a Rust \
         literal, what braces hold is a placeholder or code (JSON), never prose"
    );
    let script = "const a = r\"one two\"; const b = 'a {b} c'; const c = `cost $5 and '$x'`; const d = 'tab\\there';";
    assert_eq!(
        lines(script, crate::vocabulary::Language::Script),
        ["1:one two", "1:a {b} c", "1:cost $5 and '$x'", "1:tab here"],
        "no raw strings, braces kept, a dollar sign that starts no substitution, an escape"
    );
    let quotes = "let q = '\\\"'; // one\nlet d = '\"'; // two\n'\\x41'\"'tail word\"\n_r\"a\\tb c\" ar\"d\\te f\" br\"g\\th i\"";
    assert_eq!(
        lines(quotes, crate::vocabulary::Language::Rust),
        [
            "1:// one",
            "2:// two",
            "3:'tail word",
            "4:a b c",
            "4:d e f",
            "4:g\\th i"
        ],
        "character literals holding a quote, an escape of several characters, an `r` ending \
         a name (after `_` or a letter), a raw byte string"
    );
    for (text, language, expected) in [
        (
            "r\"open to the end",
            crate::vocabulary::Language::Rust,
            "1:open to the end",
        ),
        (
            "\"open to the end",
            crate::vocabulary::Language::Rust,
            "1:open to the end",
        ),
        (
            "`open ${to the end",
            crate::vocabulary::Language::Script,
            "1:open  ",
        ),
        ("'\\", crate::vocabulary::Language::Rust, ""),
    ] {
        assert_eq!(
            lines(text, language).join("|"),
            expected,
            "an unterminated literal: {text}"
        );
    }
    assert_eq!(
        crate::vocabulary::without_code("[a](x`y) `a](b` c"),
        format!("[a]{}c", " ".repeat(13)),
        "a backquote in a link target, a bracket in a code span"
    );
}
