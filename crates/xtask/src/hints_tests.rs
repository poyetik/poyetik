//! Tests of the runnable hints gate: every hint names a command a user can run.

use super::{HINT_OPENING, RUNNABLE_PREFIXES, runnable_hints_gate};
use crate::steps::Failure;

/// A source at `path` holding `text`, where `{see}` stands for a hint's opening.
fn source(path: &str, text: &str) -> (String, String) {
    (path.to_owned(), text.replace("{see}", HINT_OPENING))
}

/// The refusal of the hint naming `named`, at `at` (file and line).
fn refusal(at: &str, named: &str) -> String {
    format!(
        "{at}: the hint `{HINT_OPENING}{named})` names no command a user can run: write the \
         exact command, starting with one of cargo xtask, cargo run, cargo update, pnpm, git, \
         node"
    )
}

#[test]
fn accepts_every_runnable_hint() {
    let sources = [
        source("a.rs", "\"error: x {see}cargo xtask help)\"\n"),
        source(
            "b.rs",
            "\"x {see}cargo run -p visual-cli -- help)\" y {see}git checkout -- f)\n",
        ),
        source("c.ts", "`error: ${reason} {see}pnpm roadmap help)`\n"),
        source(
            "d.ts",
            "`{see}node scripts/benchmarks.ts help)` `{see}${benchmarks.command} help)`\n",
        ),
        source(
            "e.rs",
            "\"lists no cards {see}\\\n    pnpm roadmap validate)\"\n",
        ),
        source("f.ts", "/\\{see}(.+)\\)$/\n"),
        source("g.rs", "no hint at all\n"),
    ];
    assert_eq!(runnable_hints_gate(&sources), Ok(()));
}

#[test]
fn refuses_a_hint_that_names_no_command() {
    assert_eq!(
        runnable_hints_gate(&[source(
            "a.rs",
            "fn f() {}\n\"error: x {see}audio-cli help)\"\n"
        )]),
        Err(Failure::from(refusal("a.rs:2", "audio-cli help"))),
        "a program the repository runs through Cargo"
    );
    assert_eq!(
        runnable_hints_gate(&[
            source(
                "s.ts",
                "\"x {see}bindings/audio-wasm/README.md)\" and {see}roadmap help)\n"
            ),
            source("t.rs", "\"x {see}\\\n    audio-cli help)\"\n"),
            source("u.rs", "\"x {see}\\\n"),
        ]),
        Err(Failure {
            summary: "4 problems, listed above".to_owned(),
            details: vec![
                refusal("s.ts:1", "bindings/audio-wasm/README.md"),
                refusal("s.ts:1", "roadmap help"),
                refusal("t.rs:1", "audio-cli help"),
                refusal("u.rs:1", ""),
            ],
            usage: false,
        }),
        "a document, a program name, each continued line read on, and a continuation at the \
         end of the file"
    );
    // An `.astro` page is code: its hints are read too (CODE_SOURCES, F12-1).
    assert_eq!(
        runnable_hints_gate(&[source(
            "apps/roadmap/src/pages/index.astro",
            "<p>{see}roadmap help)</p>\n"
        )]),
        Err(Failure::from(refusal(
            "apps/roadmap/src/pages/index.astro:1",
            "roadmap help"
        ))),
    );
    // Near misses of each prefix: another case, no space after it, a prefix inside a word.
    for hint in [
        "Cargo xtask help",
        "cargo xtaskhelp",
        "pnpmroadmap",
        "gitk",
        "nodejs x",
        "cargo help",
    ] {
        assert_eq!(
            runnable_hints_gate(&[source("a.rs", &format!("{{see}}{hint})\n"))]),
            Err(Failure::from(refusal("a.rs:1", hint))),
            "{hint}"
        );
    }
}

#[test]
fn reads_the_code_of_rust_and_typescript_for_the_commands_the_repository_runs() {
    assert_eq!(HINT_OPENING, ["(see", ":", " "].concat());
    assert_eq!(
        RUNNABLE_PREFIXES,
        [
            "cargo xtask ",
            "cargo run ",
            "cargo update ",
            "pnpm ",
            "git ",
            "node "
        ]
    );
}

#[test]
fn refuses_the_hint_that_revealed_it() {
    // The usage hint of `visual-gpu`'s benchmark before the third deep review of visual
    // block 2 (C7-2): a program name no shell has, the binary being run through Cargo.
    let held = "        line: format!(\"{line} {see}visual-gpu help)\"),\n";
    assert_eq!(
        runnable_hints_gate(&[source("crates/visual-gpu/src/bench.rs", held)]),
        Err(Failure::from(refusal(
            "crates/visual-gpu/src/bench.rs:1",
            "visual-gpu help"
        )))
    );
}

#[test]
fn refuses_a_hint_opened_in_another_form() {
    // Finding F5-9 of the fourth deep review of visual block 2 (#34): the gate read the
    // opening as written only, so a hint in another case or spacing was never read. The
    // fixtures build each form from pieces, so that this file holds none.
    let opened = |at: &str, written: &str| {
        format!(
            "{at}: a hint opened `{written}`: write it `{HINT_OPENING}<command>)`, the one form \
             the hints gate reads"
        )
    };
    for (written, text) in [
        (
            concat!("(See", ": "),
            concat!("x (See", ": cargo xtask help)"),
        ),
        (
            concat!("(SEE", ": "),
            concat!("x (SEE", ": audio-cli help)"),
        ),
        (
            concat!("(see", ":c"),
            concat!("x (see", ":cargo xtask help)"),
        ),
        (
            concat!("(see ", ": "),
            concat!("x (see ", ": cargo xtask help)"),
        ),
        (concat!("(see", ":"), concat!("x (see", ":")),
        (concat!("(see", ":\u{e9}"), concat!("x (see", ":\u{e9})")),
    ] {
        assert_eq!(
            runnable_hints_gate(&[source("a.rs", text)]),
            Err(Failure::from(opened("a.rs:1", written))),
            "{text}"
        );
    }
    for text in [
        concat!("the (seen", ": x) case"),
        concat!("(se", ": x)"),
        "(see the docs)",
        concat!("a regex \\(See", ": (.+)\\)"),
        "(",
    ] {
        assert_eq!(
            runnable_hints_gate(&[source("a.rs", text)]),
            Ok(()),
            "{text}"
        );
    }
}
