//! Tests of the retired patterns gate: a pattern a fix retired never comes back in its
//! scope, and the list stays current.

use super::{RETIRED_PATTERNS, RETIRED_SOURCES, retired_patterns_gate};
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
        RETIRED_SOURCES,
        ["*.rs", "*.ts", "*.tsx", "*.mts", "*.js", "*.mjs"]
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
