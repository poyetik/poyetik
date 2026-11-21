//! Tests of the vocabulary gate: the words the glossary replaces, refused in the repository's
//! prose.

use crate::tests::{FAKE_WORDS, failed, passing};
use crate::{Action, Failure, Step, run_steps};

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
    unlisted.broken = Some(format!("list {}", crate::vocabulary_sources().join(" ")));
    assert!(
        run_steps(&step, &mut unlisted, &mut Vec::new()).is_err_and(|failure| failure
            .summary
            .starts_with("Words: cannot list the prose files")),
        "the files cannot be listed"
    );
}

#[test]
fn reads_code_spans_and_link_targets_as_code() {
    assert_eq!(
        crate::vocabulary::without_code("a `node` [link](node.md) (node) b"),
        "a        [link]          (node) b",
        "code spans and link targets apart"
    );
    assert_eq!(
        crate::vocabulary::without_code("[a](x`y) `a](b` c"),
        format!("[a]{}c", " ".repeat(13)),
        "a backquote in a link target, a bracket in a code span"
    );
    assert_eq!(
        crate::vocabulary::without_code("`é` é"),
        "     é",
        "each character a space of its own length, so that offsets hold"
    );
    assert_eq!(
        crate::vocabulary::without_placeholders("a {b é} {{c}} é } d"),
        format!("a{}é }} d", " ".repeat(14)),
        "a placeholder's characters each a space of their length, its blanks kept"
    );
}

#[test]
fn blanks_format_placeholders_in_rust_literals_only() {
    // A Rust literal's `{…}` is a format placeholder, code; a TypeScript literal's braces and
    // a Rust comment's are prose.
    let words = "forbid node -> box -- the glossary\n";
    let gate = |path: &str, text: &str| {
        crate::vocabulary_gate(&[(path.to_owned(), text.to_owned())], words)
    };
    assert_eq!(
        gate("a.rs", "let a = \"the {node} runs\";\n"),
        Ok(()),
        "a placeholder"
    );
    for (path, text) in [
        ("a.ts", "const a = \"the {node} runs\";\n"),
        ("a.rs", "/// The {node} runs.\n"),
    ] {
        assert_eq!(
            gate(path, text),
            failed(format!(
                "words the glossary replaces (reviewed in vocabulary.txt): {path}:1: \"node\": \
                 say \"box\" (the glossary)"
            )),
            "{path}: {text}"
        );
    }
}
