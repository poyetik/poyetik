//! Tests of the conflict marker gate: no tracked text file holds a conflict marker git
//! left.

use super::{MARKER_SIZE, binary_suffixes, conflict_marker_gate};
use crate::steps::Failure;

/// A file of `lines`, each ended by a line feed.
fn file(path: &str, lines: &[&str]) -> (String, String) {
    (
        path.to_owned(),
        lines.iter().flat_map(|line| [*line, "\n"]).collect(),
    )
}

#[test]
fn the_marker_size_is_git_s() {
    // git's default conflict-marker-size.
    assert_eq!(MARKER_SIZE, 7);
}

#[test]
fn refuses_every_marker_of_a_content_conflict_in_each_style() {
    // The merge style, then diff3 and zdiff3, which add the common ancestor.
    let merge = file(
        "a.rs",
        &[
            "fn a() {}",
            "<<<<<<< HEAD",
            "x",
            "=======",
            "y",
            ">>>>>>> feature",
            "end",
        ],
    );
    let diff3 = file(
        "b.ts",
        &[
            "<<<<<<< ours",
            "x",
            "||||||| base",
            "z",
            "=======",
            "y",
            ">>>>>>> theirs",
        ],
    );
    assert_eq!(
        conflict_marker_gate(&[merge, diff3]),
        Err(Failure {
            summary: "7 problems, listed above".to_owned(),
            details: [
                "a.rs:2", "a.rs:4", "a.rs:6", "b.ts:1", "b.ts:3", "b.ts:5", "b.ts:7",
            ]
            .iter()
            .map(|at| format!("{at}: a conflict marker git left, unresolved"))
            .collect(),
            usage: false,
        })
    );
}

#[test]
fn refuses_the_wider_markers_of_a_rename_conflict_and_bare_ones() {
    // A rename conflict writes eight characters and a path; a marker may also end its line.
    for line in [
        "<<<<<<<< HEAD:bindings/audio-wasm/scripts/cli.ts",
        ">>>>>>>> 0123abc (a commit):bindings/audio-wasm/scripts/cli.ts",
        "<<<<<<<",
        ">>>>>>>",
        "|||||||",
        "<<<<<<< HEAD\r",
    ] {
        assert_eq!(
            conflict_marker_gate(&[file("c.md", &["text", line])]),
            Err(Failure::from(
                "c.md:2: a conflict marker git left, unresolved".to_owned()
            )),
            "{line:?}"
        );
    }
    // The separator, eight wide, between two markers, and a CRLF line.
    assert_eq!(
        conflict_marker_gate(&[file("d.md", &["<<<<<<<< a", "========\r", ">>>>>>>> b"])])
            .map_err(|failure| failure.details.join("\n")),
        Err("d.md:1: a conflict marker git left, unresolved\nd.md:2: a conflict marker git left, unresolved\nd.md:3: a conflict marker git left, unresolved".to_owned()),
        "the separator of a wider conflict"
    );
}

#[test]
fn accepts_what_only_looks_like_a_marker() {
    let lines = [
        // Fewer than seven characters, or not at the start of the line.
        "<<<<<< six",
        ">>>>>> six",
        " <<<<<<< indented",
        "\"<<<<<<< quoted\"",
        // Followed by something else than a space.
        "<<<<<<<x",
        ">>>>>>>>=",
        "|||||||-",
        // A Markdown underline or rule: a separator outside a conflict.
        "=======",
        "==========",
        // A separator followed by text.
        "======= text",
        "",
    ];
    assert_eq!(conflict_marker_gate(&[file("e.md", &lines)]), Ok(()));
    // After the closing marker, a separator is an underline again.
    assert_eq!(
        conflict_marker_gate(&[file(
            "f.md",
            &["<<<<<<< a", ">>>>>>> b", "=======", "Title"]
        )])
        .map_err(|failure| failure.details.join("\n")),
        Err("f.md:1: a conflict marker git left, unresolved\nf.md:2: a conflict marker git left, unresolved".to_owned()),
        "the two markers, not the underline after them"
    );
    assert_eq!(conflict_marker_gate(&[]), Ok(()), "no file");
}

#[test]
fn reads_the_binary_suffixes_of_the_attributes() {
    let attributes =
        "# A comment\n\n* text=auto eol=lf\n*.wasm binary\n*.wav   binary\n*.md diff=markdown\n";
    assert_eq!(
        binary_suffixes(attributes),
        Ok(vec![".wasm".to_owned(), ".wav".to_owned()])
    );
    assert_eq!(
        binary_suffixes(""),
        Ok(Vec::new()),
        "nothing declared binary"
    );
}

#[test]
fn refuses_a_binary_declaration_it_cannot_read() {
    for line in [
        "assets/** binary",
        "*.wasm binary -diff",
        "*.png -text binary",
        "*. binary",
        "*.tar.gz binary",
        "logo.png binary",
    ] {
        assert_eq!(
            binary_suffixes(&format!("* text=auto\n{line}\n")),
            Err(Failure::from(
                ".gitattributes:2: declares files binary in another form than `*.<extension> \
                 binary`, which the conflict marker gate reads"
                    .to_owned()
            )),
            "{line}"
        );
    }
}
