//! Tests of the list counts gate: no prose restates how many entries a typed list holds.

use alloc::collections::{BTreeMap, BTreeSet};

use super::{list_counts_gate, typed_lists};
use crate::tests::found;

/// `path` holding `text`, as a tracked source.
fn source(path: &str, text: &str) -> (String, String) {
    (path.to_owned(), text.to_owned())
}

/// The lists of the tests: a constant array of five paths, one of `&'static str` of two,
/// both declared twice with other lengths elsewhere apart, and a register of three entries.
fn lists() -> Vec<(String, String)> {
    vec![
        source(
            "crates/xtask/src/unsafety.rs",
            "/// Doc.\npub const UNSAFE_FILES: [&str; 5] = [\"a\", \"b\", \"c\", \"d\", \"e\"];\n\
             const PAIR: [&'static str; 2] = [\"x\", \"y\"];\nconst lower: [&str; 7] = [];\n\
             const NUMBERS: [u8; 4] = [1, 2, 3, 4];\nconst SPACED : [&str; 6] = [];\n\
             const BROKEN: [&str; x] = [];\nconst OPEN: [&str; 3\n",
        ),
        source("crates/other/src/lib.rs", "const PAIR: [&str; 8] = [];\n"),
        source(
            "register.txt",
            "# A comment.\n\nfirst -- why\n  second -- why\n\t# indented comment\nthird\n",
        ),
        source("tooling/nested.txt", "one\ntwo\nthree\nfour\n"),
        source("UPPER.TXT", "a\nb\nc\nd\n"),
        source(
            "crates/a/src/UPPER.RS",
            "const SHOUTED: [&str; 9] = [];\nconst V2_NAMES: [&str; 11] = [];\nconst : [&str; 4];\n",
        ),
        source("crates/a/src/kept.rsx", "const SKIPPED: [&str; 9] = [];\n"),
    ]
}

/// The problem the gate names for `list`, said `count` at `path:line`.
fn said(at: &str, count: usize, list: &str) -> String {
    format!(
        "{at}: says {count}, how many entries `{list}` holds: name the list, never its count, \
         which its next entry makes wrong"
    )
}

#[test]
fn reads_the_constant_arrays_of_strings_and_the_registers_at_the_root() {
    let expected: BTreeMap<String, BTreeSet<usize>> = [
        ("PAIR", BTreeSet::from([2, 8])),
        ("SHOUTED", BTreeSet::from([9])),
        ("V2_NAMES", BTreeSet::from([11])),
        ("UPPER.TXT", BTreeSet::from([4])),
        ("UNSAFE_FILES", BTreeSet::from([5])),
        ("register.txt", BTreeSet::from([3])),
    ]
    .into_iter()
    .map(|(name, counts)| (name.to_owned(), counts))
    .collect();
    assert_eq!(
        typed_lists(&lists()),
        expected,
        "arrays of `&str` and `&'static str` by their uppercase names, every length a name is \
         declared with; a register's lines but blank ones and comments; no array of another \
         type, no lowercase, spaced or empty name, digits allowed, no length unread, no `.txt` file in a folder; \
         extensions in any case, no other extension"
    );
}

#[test]
fn refuses_a_sentence_naming_a_list_and_writing_its_count() {
    let mut sources = lists();
    sources.push(source(
        "CLAUDE.md",
        "Intro.\n\nThe five audited files of\n`UNSAFE_FILES`. Fine: `UNSAFE_FILES` holds them.\n\
         \n- each reviewed in `register.txt`\n  (3 reviewed).\n- `PAIR` holds Two; `PAIR`, 8.\n\
         - A date, 2026-05-12, and 1.5.0 of `UNSAFE_FILES`. `UNSAFE_FILES_MORE` has 5, \
         5 of NOT_UNSAFE_FILES, path.UNSAFE_FILES 5, `register.txt.bak` 3, `my-register.txt` 3.\n\
         - The one `PAIR` here, 3 of `PAIR`.\n",
    ));
    sources.push(source(
        "crates/xtask/src/lib.rs",
        "/// Lists `PAIR`: the\n/// two entries.\nconst X: u8 = 2; // PAIR, 2\n",
    ));
    assert_eq!(
        list_counts_gate(&sources),
        found(
            "the counts of the typed lists",
            &[
                said("CLAUDE.md:3", 5, "UNSAFE_FILES"),
                said("CLAUDE.md:6", 3, "register.txt"),
                said("CLAUDE.md:8", 2, "PAIR"),
                said("CLAUDE.md:8", 8, "PAIR"),
                said("crates/xtask/src/lib.rs:1", 2, "PAIR"),
            ]
        ),
        "a count in digits or in words, any case, read across the lines of a paragraph or a \
         comment, sentence by sentence, at the line its sentence starts; never a date, a \
         version, another name holding the list's, a count of another value, \"one\", nor a \
         comment after code"
    );
}

#[test]
fn reads_no_released_section_of_the_changelog() {
    let mut sources = lists();
    sources.push(source(
        "CHANGELOG.md",
        "# Changelog\n\n## [Unreleased]\n\n- `register.txt`, 3 reviewed.\n\n\
         ## [0.1.0] - 2026-11-08\n\n- `register.txt`, 3 reviewed.\n",
    ));
    sources.push(source(
        "README.md",
        "## [0.1.0] - 2026-11-08\n\n- `register.txt`, 3 reviewed.\n",
    ));
    assert_eq!(
        list_counts_gate(&sources),
        found(
            "the counts of the typed lists",
            &[
                said("CHANGELOG.md:5", 3, "register.txt"),
                said("README.md:3", 3, "register.txt"),
            ]
        ),
        "[Unreleased] read, a released section not; only the changelog's are records"
    );
    assert_eq!(list_counts_gate(&lists()), Ok(()), "no prose, nothing said");
}

#[test]
fn refuses_each_count_in_every_rewrap_of_its_sentence() {
    for (path, text) in [
        (
            "CLAUDE.md",
            "The five audited files of `UNSAFE_FILES`, wrapped.",
        ),
        (
            "README.md",
            "- Each reviewed in `register.txt` (3 reviewed).",
        ),
        (
            "crates/a/src/lib.rs",
            "/// The `PAIR` of two entries, each a name.",
        ),
        ("tooling/a.ts", "// PAIR holds 8 names, a comment."),
    ] {
        let with = |written: &str| {
            let mut sources = lists();
            sources.push(source(path, written));
            list_counts_gate(&sources)
        };
        let refused = with(text);
        assert!(refused.is_err(), "{path}: {text}");
        let wraps = crate::tests::rewraps(path, text);
        assert!(!wraps.is_empty(), "{text}");
        for wrapped in wraps {
            assert_eq!(
                crate::tests::problems_anywhere(&with(&wrapped)),
                crate::tests::problems_anywhere(&refused),
                "{path}: {wrapped}"
            );
        }
    }
}
