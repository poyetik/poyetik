//! Tests of the retired patterns gate: a pattern a fix retired never comes back in its
//! scope, and the list stays current.

use super::{RETIRED_EXEMPT, RETIRED_PATTERNS, retired_patterns_gate};
use crate::steps::Failure;

/// A source at `path` holding `text`.
fn source(path: &str, text: &str) -> (String, String) {
    (path.to_owned(), text.to_owned())
}

/// The gate's one problem `line`.
fn refused(line: &str) -> Result<(), Failure> {
    Err(Failure::from(line.to_owned()))
}

#[test]
fn refuses_a_retired_pattern_in_its_scope_only() {
    let list = "# A comment.\n\nretire `weak() ||` in crates/a/ b/c.rs -- a vacuous assert: assert the case itself\n";
    let sources = [
        source(
            "crates/a/src/lib_tests.rs",
            "fn t() {\n    assert!(weak() || x);\n}\n",
        ),
        source("b/c.rs", "ok\n"),
        source("crates/ab/src/x.rs", "weak() || y\n"),
        source("b/c.rs.bak.rs", "weak() || y\n"),
    ];
    assert_eq!(
        retired_patterns_gate(&sources, list),
        refused(
            "crates/a/src/lib_tests.rs:2: holds `weak() ||`, which a fix retired: a vacuous \
             assert: assert the case itself"
        ),
        "in a directory named, never in one sharing its prefix, nor in a file sharing a \
         file's prefix"
    );
    assert_eq!(
        retired_patterns_gate(
            &[source("b/c.rs", "weak() ||\n"), source("crates/a/x.rs", "")],
            list
        ),
        refused(
            "b/c.rs:1: holds `weak() ||`, which a fix retired: a vacuous assert: assert the case itself"
        ),
        "in a file named"
    );
    assert_eq!(
        retired_patterns_gate(&sources[1..], list),
        refused(
            "retired-patterns.txt: `weak() ||` names crates/a/, which holds no tracked source: \
             remove or correct it"
        ),
        "a scope gone is refused, so that the list stays current"
    );
    // No scope: every source; case sensitive, anywhere in a line, every line it is on.
    let everywhere = "retire `Old wording` -- reworded: the new wording\n";
    assert_eq!(
        retired_patterns_gate(
            &[source(
                "x.ts",
                "a Old wording b\nold wording\nOld wording\n"
            )],
            everywhere
        ),
        Err(Failure {
            summary: "2 problems, listed above".to_owned(),
            details: vec![
                "x.ts:1: holds `Old wording`, which a fix retired: reworded: the new wording"
                    .to_owned(),
                "x.ts:3: holds `Old wording`, which a fix retired: reworded: the new wording"
                    .to_owned(),
            ],
            usage: false,
        })
    );
    assert_eq!(
        retired_patterns_gate(&[source("x.ts", "fine\n")], everywhere),
        Ok(())
    );
}

#[test]
fn refuses_every_line_of_the_list_not_of_its_form() {
    let form = "not `retire `<pattern>`[ in <path>…] -- <reason>`";
    for (line, why) in [
        ("retire weak -- r", "no backticks"),
        ("retire `weak -- r", "no closing backtick"),
        ("retire `weak`", "no reason"),
        ("retire `weak` -- ", "an empty reason"),
        ("retire `` -- r", "an empty pattern"),
        ("retire ` ` -- r", "a blank pattern"),
        ("retire `weak` crates/ -- r", "a scope without `in`"),
        ("retire `weak` in  crates/ -- r", "an empty path"),
        ("retire `weak` in crates/  -- r", "a trailing space"),
        ("Retire `weak` -- r", "another case"),
        (" # a comment", "an indented comment"),
        (" ", "a line of spaces"),
    ] {
        assert_eq!(
            retired_patterns_gate(&[], &format!("{line}\n")),
            refused(&format!("{RETIRED_PATTERNS}:1: {form}")),
            "{why}: {line:?}"
        );
    }
}

#[test]
fn reads_the_repository_s_list_and_its_sources() {
    assert_eq!(RETIRED_PATTERNS, "retired-patterns.txt");
    assert_eq!(
        RETIRED_EXEMPT,
        ["retired-patterns.txt", "crates/xtask/src/retired_tests.rs"]
    );
    assert_eq!(retired_patterns_gate(&[], ""), Ok(()), "an empty list");
}

#[test]
fn refuses_the_assertion_that_revealed_it() {
    // The vacuous assertion the third deep review of visual block 2 found left behind
    // (C5-8), as `visual-gpu`'s tests held it, under the entry that retires it.
    let held = "    assert!(\n        took.is_none() || timed.period.is_some(),\n        \"a time only when the GPU has a clock\"\n    );\n";
    assert_eq!(
        retired_patterns_gate(
            &[source("crates/visual-gpu/src/lib_tests.rs", held)],
            "retire `is_none() ||` in crates/visual-gpu/ -- vacuous\n"
        ),
        refused(
            "crates/visual-gpu/src/lib_tests.rs:2: holds `is_none() ||`, which a fix retired: \
             vacuous"
        )
    );
}

#[test]
fn reads_a_pattern_wrapped_over_lines() {
    // Finding F5-1 of the fourth deep review of visual block 2 (#34): the gate matched one
    // physical line at a time, so rustfmt's split of a long condition before its operator,
    // or a wording wrapped in a doc comment, held a retired pattern unseen.
    let list =
        "retire `is_none() ||` -- vacuous\nretire `did not give the image back` -- reworded\n";
    for (path, text, line, pattern, reason) in [
        (
            "x_tests.rs",
            "    assert!(\n        took.is_none()\n            || timed.period.is_some(),\n    );\n",
            2_usize,
            "is_none() ||",
            "vacuous",
        ),
        (
            "x.rs",
            "/// The GPU\n/// did not give the\n///   image back.\nfn f() {}\n",
            2_usize,
            "did not give the image back",
            "reworded",
        ),
        (
            "x.ts",
            "/**\n * It did not give\n * the image back.\n */\n",
            2_usize,
            "did not give the image back",
            "reworded",
        ),
        (
            "x.ts",
            "const a = \"did not  give the image back\";\n",
            1_usize,
            "did not give the image back",
            "reworded",
        ),
    ] {
        assert_eq!(
            retired_patterns_gate(&[source(path, text)], list),
            refused(&format!(
                "{path}:{line}: holds `{pattern}`, which a fix retired: {reason}"
            )),
            "{text}"
        );
    }
    assert_eq!(
        retired_patterns_gate(
            &[source(
                "x.rs",
                "/// It did not give the\n// image back.\nlet a = b.is_none(); // ||\n"
            )],
            "retire `did not give the image back` -- reworded\n"
        ),
        Ok(()),
        "comments of two kinds are not one sentence"
    );
}

#[test]
fn searches_every_text_file_but_its_list_its_tests_and_released_entries() {
    // Finding F10-5 of the fourth deep review of visual block 2 (#34): the gate searched
    // six code globs while its doc and CLAUDE.md said the whole tree, so an old wording
    // could not be retired from a README. Every tracked text file is searched now.
    let list = "retire `never widened` -- reworded: the root's lines never widened\n";
    let held = "It is never widened.\n";
    for path in [
        "README.md",
        "x.toml",
        "x.yml",
        "x.json",
        "x.txt",
        "x.astro",
        "x.css",
    ] {
        assert_eq!(
            retired_patterns_gate(&[source(path, held)], list),
            refused(&format!(
                "{path}:1: holds `never widened`, which a fix retired: reworded: the root's \
                 lines never widened"
            )),
            "{path}"
        );
    }
    let changelog = "# Changelog\n\n## [Unreleased]\n\n- It is never widened.\n\n## [0.1.0] - 2026-01-01\n\n- It is never widened.\n";
    assert_eq!(
        retired_patterns_gate(&[source("CHANGELOG.md", changelog)], list),
        refused(
            "CHANGELOG.md:5: holds `never widened`, which a fix retired: reworded: the root's \
             lines never widened"
        ),
        "the unreleased part of the changelog only"
    );
    for path in RETIRED_EXEMPT {
        assert_eq!(
            retired_patterns_gate(&[source(path, held)], list),
            Ok(()),
            "{path}"
        );
    }
}

#[test]
fn reads_a_string_a_formatter_splits_and_a_hook_s_comments_whole() {
    // Findings H17-1, H5-11 and H17-10 of the fifth deep review of visual block 2 (#34):
    // a Rust string continued by `\`, a TypeScript string joined by `+` and the `#` comments
    // of a git hook were read one line at a time, and the whole text was joined across
    // blank lines, so a paragraph's last word met the next one's first.
    let list = "retire `only idle` -- reworded\n";
    for (path, text, line) in [
        (
            "x.rs",
            "fn f() {\n    g(\"only \\\n       idle\");\n}\n",
            2_usize,
        ),
        ("x.ts", "f(\"only \" +\n  \"idle\");\n", 1_usize),
        (
            ".githooks/pre-push",
            "#!/bin/sh\n# Run only\n# idle checks.\n",
            2_usize,
        ),
        (".gitignore", "# only\n# idle\nx/\n", 1_usize),
    ] {
        assert_eq!(
            retired_patterns_gate(&[source(path, text)], list),
            refused(&format!(
                "{path}:{line}: holds `only idle`, which a fix retired: reworded"
            )),
            "{text}"
        );
    }
    assert_eq!(
        retired_patterns_gate(&[source("x.txt", "Not only\n\nidle.\n")], list),
        Ok(()),
        "a blank line ends a text"
    );
}
