//! Tests of the list counts gate: no prose restates how many entries a typed list holds.

use alloc::collections::{BTreeMap, BTreeSet};

use super::{declared_lists, list_counts_gate, typed_lists};
use crate::tests::{found, source};

/// The lists of the tests: a constant array of five paths, one of `&'static str` of two,
/// both declared twice with other lengths elsewhere apart, and a register of three entries.
fn lists() -> Vec<(String, String)> {
    vec![
        source(
            "crates/xtask/src/unsafety.rs",
            "/// Doc.\npub const UNSAFE_FILES: [&str; 5] = [\"a\", \"b\", \"c\", \"d\", \"e\"];\n\
             const PAIR: [&'static str; 2] = [\"x\", \"y\"];\nconst lower: [&str; 7] = [];\n\
             const NUMBERS: [u8; 4] = [1, 2, 3, 4];\nconst SPACED : [&str; 6] = [];\n\
             const BROKEN: [&str; x] = [];\nconst CLOSED: [&str; 3, 4] = [];\n\
             const WORDED: [&str; \"3\"] = [];\n",
        ),
        // Finding H12-6 of the fifth deep review of visual block 2 (#34): the lists were
        // found by a search of the raw text, so a declaration a string or a comment held,
        // as the fixtures of these tests, registered a list.
        source(
            "crates/a/src/held.rs",
            "// const COMMENTED: [&str; 3] = [];\nconst S: &str = \"const QUOTED: [&str; 4] \
             = [];\";\n",
        ),
        source(
            "crates/a/src/open.rs",
            "const UNLEXED: [&str; 3] = [];\nconst OPEN: [&str; 3\n",
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
        ("SPACED", BTreeSet::from([6])),
        ("V2_NAMES", BTreeSet::from([11])),
        ("UPPER.TXT", BTreeSet::from([4])),
        ("UNSAFE_FILES", BTreeSet::from([5])),
        ("register.txt", BTreeSet::from([3])),
    ]
    .into_iter()
    .map(|(name, counts)| (name.to_owned(), counts))
    .collect();
    assert_eq!(
        typed_lists(&declared_lists(&lists())),
        expected,
        "arrays of `&str` and `&'static str` by their uppercase names, every length a name is \
         declared with; a register's lines but blank ones and comments; no array of another \
         type, no lowercase or empty name, digits allowed, spacing as rustc reads it, no length \
         unread or unclosed, no declaration in a comment or a string, nothing of a source \
         that does not lex, no `.txt` file in a folder; extensions in any case, no other \
         extension"
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

/// The problem the gate names for a paragraph at `at` quoting `copied` entries of `list`.
fn copied(at: &str, copied: usize, list: &str) -> String {
    format!(
        "{at}: quotes {copied} entries of `{list}`: name the list, never a copy of its \
         entries, which its next entry makes wrong"
    )
}

#[test]
fn refuses_a_paragraph_of_prose_quoting_the_entries_of_a_list() {
    // Findings H10-1, H6-3 and H17-7 of the fifth deep review of visual block 2 (#34):
    // CLAUDE.md and the changelog spelled out `RUNNABLE_PREFIXES` entry by entry, and a
    // prefix added to it left both copies stale; only counts were refused.
    let mut sources = lists();
    sources.push(source(
        "crates/a/src/hints.rs",
        "const PREFIXES: [&str; 4] = [\"cargo xtask \", \"pnpm \", \"git \", \"cargo run \"];\n\
         const CRATES: [&str; 3] = [\"a\", \"b-c\", \"d\"];\n\
         const FOLDERS: [&str; 3] = [\"crates/\", \"bindings/d/\", \"e/\"];\n\
         const LONE: [&str; 1] = [\"lone\"];\n",
    ));
    for part in [
        "crates/b-c/Cargo.toml",
        "bindings/d/package.json",
        "tooling/x",
    ] {
        sources.push(source(part, ""));
    }
    let copy = "Intro.\n\nA hint starts with `cargo xtask`, `pnpm`\nor `git`, as written.\n\n\
                Examples: `pnpm` and `git`; `lone`.\n\n- `x` or `y`; parts `a`, `b-c` and `d`.\n\
                In `crates/`, `bindings/d/`, `e/`.\n- \
                `first`, `second`, `third`.\n";
    for path in ["CLAUDE.md", "README.md", "apps/a/README.md", "spec/a.md"] {
        sources.push(source(path, copy));
    }
    sources.push(source(
        "crates/a/src/lib.rs",
        "/// `cargo xtask`, `pnpm`, `git`.\n",
    ));
    sources.push(source("tooling/a.md", copy));
    sources.push(source(
        "CHANGELOG.md",
        "# Changelog\n\n## [Unreleased]\n\n- `pnpm`, `git` and `cargo run`.\n\n\
         ## [0.1.0] - 2026-11-08\n\n- `pnpm`, `git` and `cargo run`.\n",
    ));
    let mut expected = Vec::new();
    for path in ["CLAUDE.md", "README.md", "apps/a/README.md", "spec/a.md"] {
        expected.extend([
            copied(&format!("{path}:3"), 3, "PREFIXES"),
            copied(&format!("{path}:8"), 2, "PAIR"),
            copied(&format!("{path}:10"), 3, "register.txt"),
        ]);
    }
    expected.push(copied("CHANGELOG.md:5", 3, "PREFIXES"));
    assert_eq!(
        list_counts_gate(&sources),
        found("the counts of the typed lists", &expected),
        "three entries of a list in code quotes across a paragraph's lines, or both of a list of two, a string's \
         trailing space trimmed, at the paragraph's first line, in CLAUDE.md, a README, spec/ \
         and the changelog's [Unreleased]; two entries of a longer list, the one of a list of one, the names of parts (a folder of \
         crates/, bindings/, apps/ or tooling/, not a file there) and folders, a released \
         section, code
         and other files pass"
    );
}

#[test]
fn refuses_a_copy_and_a_count_of_a_list_of_tuples() {
    // Finding K17-7 of the sixth deep review of visual block 2 (#34): the changelog spelled
    // out two lists whose entries are tuples, which the gate read as no list.
    let declared = |list: &str| {
        vec![
            source("crates/a/src/lists.rs", list),
            source(
                "spec/a.md",
                "Set `set.a`, `set.b` and\n`set.c`.\n\nOf `SETTINGS`, the 3 settings.\n\n\
                 Values `on`, `off`, `low`.\n",
            ),
        ]
    };
    for list in [
        "const SETTINGS: [(&str, &str); 3] = [(\"set.a\", \"on\"), (\"set.b\", \"off\"), (\"set.c\", \"low\")];\n",
        "pub const SETTINGS: [(&'static str, &str, Kind); 3] = [\n    (\"set.a\", \"on\", Kind::A),\n    (\"set.b\", \"off\", Kind::B(\"x\")),\n    (\"set.c\", \"low\", Kind::A),\n];\n",
        "const SETTINGS: [(&str,); 3] = [(\"set.a\",), (\"set.b\",), (\"set.c\",)];\n",
    ] {
        assert_eq!(
            list_counts_gate(&declared(list)),
            found(
                "the counts of the typed lists",
                &[
                    copied("spec/a.md:1", 3, "SETTINGS"),
                    said("spec/a.md:4", 3, "SETTINGS"),
                ]
            ),
            "each tuple by its first string: {list}"
        );
    }
    for other in [
        "const SETTINGS: [(Kind, &str); 3] = [(Kind::A, \"set.a\"), (Kind::A, \"set.b\"), (Kind::A, \"set.c\")];\n",
        "const SETTINGS: [(u8, &str); 3] = [(1, \"set.a\"), (2, \"set.b\"), (3, \"set.c\")];\n",
        "const SETTINGS: [(&[u8], &str); 3] = [(b\"a\", \"set.a\"), (b\"b\", \"set.b\"), (b\"c\", \"set.c\")];\n",
        "const SETTINGS: (&str, &str, &str) = (\"set.a\", \"set.b\", \"set.c\");\n",
    ] {
        assert_eq!(
            list_counts_gate(&declared(other)),
            Ok(()),
            "no tuple of it starts with a string: {other}"
        );
    }
    for inside in [
        "const SETTINGS: [(&str, &str); 3] = [(\"set.a\", \"set.b\"), (NAMED, \"set.c\"), (\"d\", \"e\")];\n",
        "const SETTINGS: [(&str, Kind); 3] = [(\"d\", B(\"set.a\")), (\"e\", B(\"set.b\")), (\"f\", B(\"set.c\"))];\n",
    ] {
        assert_eq!(
            list_counts_gate(&declared(inside)),
            found(
                "the counts of the typed lists",
                &[said("spec/a.md:4", 3, "SETTINGS")]
            ),
            "the first member of a tuple alone is an entry: {inside}"
        );
    }
}

#[test]
fn lets_the_prose_name_the_license_files_it_grants() {
    // A copy of them in another list too, which is the same paths.
    let sources = [
        source(
            "crates/a/src/lists.rs",
            "const FILES: [(&str, &str); 4] = [(\"LICENSE-MIT\", \"a\"), (\"LICENSE-APACHE\", \"b\"), (\"apps/LICENSE\", \"c\"), (\"LICENSE-X\", \"d\")];\n",
        ),
        source(
            "README.md",
            "Under `LICENSE-MIT`, `LICENSE-APACHE` and `apps/LICENSE`.\n",
        ),
    ];
    assert_eq!(list_counts_gate(&sources), Ok(()), "the license files");
    let others = sources.map(|(path, text)| (path, text.replace("LICENSE", "OTHER")));
    assert_eq!(
        list_counts_gate(&others),
        found(
            "the counts of the typed lists",
            &[copied("README.md:1", 3, "FILES")]
        ),
        "any other list's"
    );
}
