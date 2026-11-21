//! Tests of the runnable hints gate: every hint names a command a user can run.

use super::{HINT_OPENING, README_HINT_OPENING, RUNNABLE_PREFIXES, runnable_hints_gate};
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
            "\"x {see}cargo run -p visual-cli -- help)\" y {see}git checkout HEAD -- :/f)\n",
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
                "\"x {see}bindings/audio-wasm/README.md)\" // and {see}roadmap help)\n"
            ),
            source("t.rs", "\"x {see}\\\n    audio-cli help)\"\n"),
            source("u.rs", "\"x {see}\\\n    \""),
        ]),
        Err(Failure {
            summary: "4 problems, listed above".to_owned(),
            details: vec![
                refusal("s.ts:1", "roadmap help"),
                refusal("s.ts:1", "bindings/audio-wasm/README.md"),
                refusal("t.rs:1", "audio-cli help"),
                format!(
                    "u.rs:1: a hint opened `{}`: write it `{HINT_OPENING}<command>)`, the one \
                     form the hints gate reads here",
                    HINT_OPENING.trim_end()
                ),
            ],
            usage: false,
        }),
        "a comment's hint, then the literals': a document, a program name, a continued line \
         read on, and a hint left open at the end of its literal"
    );
    // An `.astro` page is code: its hints are read too (`code_sources`, F12-1).
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
            runnable_hints_gate(&[source("a.rs", &format!("\"{{see}}{hint})\"\n"))]),
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
             the hints gate reads here"
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
            runnable_hints_gate(&[source("a.rs", &format!("// {text}"))]),
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
            runnable_hints_gate(&[source("a.rs", &format!("// {text}"))]),
            Ok(()),
            "{text}"
        );
    }
}

/// A README at `path` holding `text`, where `{commands}` stands for a README hint's opening
/// and `{see}` for a hint's.
fn readme(path: &str, text: &str) -> (String, String) {
    source(path, &text.replace("{commands}", README_HINT_OPENING))
}

/// The refusal of the command `command` of a README's hint, at `at` (file and line).
fn unrunnable(at: &str, command: &str) -> String {
    format!(
        "{at}: the command `{command}` of a README's hint is no command a user can run: write \
         it as it is run, starting with one of cargo xtask, cargo run, cargo update, pnpm, git, \
         node"
    )
}

#[test]
fn reads_the_commands_a_readme_names() {
    // Finding F6-11 of the fourth deep review of visual block 2 (#34): crates/README.md
    // named `audio-cli help` and `visual-cli help`, programs no shell has, and the gate read
    // only code.
    assert_eq!(README_HINT_OPENING, ["(commands", ":", " "].concat());
    assert_eq!(
        runnable_hints_gate(&[
            readme(
                "README.md",
                "| `x` | A tool {commands}`cargo run -p x -- help`, `pnpm x` and \
                 `git log (a)`) and `audio-cli help` |\n"
            ),
            readme("crates/NOT-README.md", "{commands}`audio-cli help`)\n"),
        ]),
        Ok(()),
        "every command runnable, a parenthesis in code quotes kept inside, a span after the \
         hint not its; only a README is read so"
    );
    assert_eq!(
        runnable_hints_gate(&[readme("crates/README.md", "Also {see}audio-cli help).\n")]),
        Err(Failure::from(format!(
            "crates/README.md:1: a hint opened `{HINT_OPENING}`: write it \
             `{README_HINT_OPENING}<command>)`, the one form the hints gate reads here"
        ))),
        "a README's hints open in its own form only"
    );
    assert_eq!(
        runnable_hints_gate(&[readme(
            "crates/README.md",
            "first\n| `audio-cli` | x {commands}`audio-cli help`, `cargo run -p a -- help`, \
             `Cargo xtask`) and {commands}none) |\n{commands}`visual-cli help`\n"
        )]),
        Err(Failure {
            summary: "4 problems, listed above".to_owned(),
            details: vec![
                unrunnable("crates/README.md:2", "audio-cli help"),
                unrunnable("crates/README.md:2", "Cargo xtask"),
                format!(
                    "crates/README.md:2: a README's hint `{README_HINT_OPENING}none)` names no \
                     command: name each in code quotes"
                ),
                unrunnable("crates/README.md:3", "visual-cli help"),
            ],
            usage: false,
        }),
        "each command a program name or another case, a hint naming none, a hint never closed \
         read to the end of its line"
    );
    assert_eq!(
        runnable_hints_gate(&[readme(
            "README.md",
            "{commands}`git log (a)`, `audio-cli help`) then `visual-cli help`\n"
        )]),
        Err(Failure::from(unrunnable("README.md:1", "audio-cli help"))),
        "a parenthesis inside code quotes closes no hint: the commands after it are read"
    );
}

#[test]
fn reads_each_hint_whole_through_the_logical_text_reader() {
    // Findings H5-3, H12-5 and H17-6 of the fifth deep review of visual block 2 (#34): the
    // gate read one physical line, so a Rust string continued by `\` was vouched for by
    // its first word, a README's hint wrapped over lines was read up to its line's end, and
    // `( see`, `(Commands` and `(command` escaped it.
    assert_eq!(
        runnable_hints_gate(&[
            source("a.rs", "\"x {see}cargo \\\n    xtask help)\"\n"),
            source(
                "b.ts",
                "const a = \"x {see}pnpm \" +\n  \"roadmap help)\";\n"
            ),
            source("c.ts", "`{see}${benchmarks.command} help (a)`;\n"),
            source(
                "d.rs",
                "/// A doc,\n/// wrapped {see}cargo\n/// xtask help).\n"
            ),
            readme(
                "README.md",
                "A part\n{commands}`cargo run -p x\n-- help`)\n"
            ),
        ]),
        Ok(()),
        "a Rust continuation, a TypeScript `+` join, a substitution and a parenthesis kept \
         inside, a comment and a README paragraph wrapped"
    );
    let opened = |at: &str, written: &str, accepted: &str| {
        format!(
            "{at}: a hint opened `{written}`: write it `{accepted}<command>)`, the one form the \
             hints gate reads here"
        )
    };
    assert_eq!(
        runnable_hints_gate(&[
            source("a.rs", "\"x {see}\\\n    audio-cli help)\"\n"),
            source("b.ts", "`{see}a${x} b`;\n"),
            source("c.rs", concat!("// ( see", ": cargo xtask help)\n")),
            source("d.rs", concat!("\"(command", ": cargo xtask help)\"\n")),
            readme("e.rs", "\"x {commands}`x`)\"\n"),
            readme(
                "README.md",
                "A part {commands}`cargo run -p x -- help`,\n`audio-cli help`)\n"
            ),
            readme(
                "x/README.md",
                concat!("A (Commands", ": `x`)\n{see}cargo xtask help)\n")
            ),
        ]),
        Err(Failure {
            summary: "8 problems, listed above".to_owned(),
            details: vec![
                refusal("a.rs:1", "audio-cli help"),
                refusal("b.ts:1", "a${…} b"),
                opened("c.rs:1", concat!("( see", ": "), HINT_OPENING),
                opened("d.rs:1", concat!("(command", ": "), HINT_OPENING),
                opened("e.rs:1", README_HINT_OPENING, HINT_OPENING),
                unrunnable("README.md:1", "audio-cli help"),
                opened(
                    "x/README.md:1",
                    concat!("(Commands", ": "),
                    README_HINT_OPENING
                ),
                opened("x/README.md:2", HINT_OPENING, README_HINT_OPENING),
            ],
            usage: false,
        }),
        "a continued command read whole, a substitution shown, every other opening, the \
         other kind's included"
    );
}

#[test]
fn refuses_a_hint_holding_a_placeholder() {
    // Finding H13-10 of the fifth deep review of visual block 2 (#34): the shared versions
    // gate's hint named `--precise <version>`, which a shell reads as a redirection from a
    // file `version`.
    let held = |written: &str, command: &str| {
        format!(
            "a.rs:1: the hint `{HINT_OPENING}{command})` holds the placeholder `{written}`: \
             name the exact value, one hint per value"
        )
    };
    assert_eq!(
        runnable_hints_gate(&[
            source("a.rs", "\"{see}cargo update -p x --precise <version>)\""),
            source("b.rs", "\"{see}git checkout HEAD -- :/\u{2026})\""),
        ]),
        Err(Failure {
            summary: "2 problems, listed above".to_owned(),
            details: vec![
                held("<version>", "cargo update -p x --precise <version>"),
                held("\u{2026}", "git checkout HEAD -- :/\u{2026}").replacen("a.rs", "b.rs", 1),
            ],
            usage: false,
        }),
    );
    assert_eq!(
        runnable_hints_gate(&[source(
            "a.rs",
            "\"{see}git log a<b) {see}git x <1 >out) {see}git x <a b>) {see}git x <)\""
        )]),
        Ok(()),
        "a comparison, redirections, a bracket closed after a space or never"
    );
}

#[test]
fn refuses_a_restore_from_the_index_or_from_where_it_ran() {
    // Findings H1-1 and H14-3 of the fifth deep review of visual block 2 (#34): every
    // tool's restore ran `git checkout` without `HEAD`, which restores from the index (a damaged
    // file once staged stays damaged), by a path git refuses from a subdirectory.
    let refused = |at: &str, command: &str, problem: &str| {
        format!("{at}: the hint `{HINT_OPENING}{command})` {problem}")
    };
    let index = "restores from the index, which may hold the damage staged: write \
                 `git checkout HEAD -- `";
    let relative = |word: &str| {
        format!(
            "names `{word}` from where it runs, which fails from another directory: write the \
             path from the repository's root (`:/`) or absolute"
        )
    };
    assert_eq!(
        runnable_hints_gate(&[
            source("a.rs", concat!("\"{see}git checkout", " -- spec/a.txt)\"")),
            source("b.rs", "\"{see}git checkout HEAD -- spec/a.txt)\""),
            source("c.rs", "\"{see}git checkout HEAD -- :/a '/b c' x)\""),
        ]),
        Err(Failure {
            summary: "3 problems, listed above".to_owned(),
            details: vec![
                refused("a.rs:1", concat!("git checkout", " -- spec/a.txt"), index),
                refused(
                    "b.rs:1",
                    "git checkout HEAD -- spec/a.txt",
                    &relative("spec/a.txt")
                ),
                refused(
                    "c.rs:1",
                    "git checkout HEAD -- :/a '/b c' x",
                    &relative("x")
                ),
            ],
            usage: false,
        }),
        "from the index; a path from where it ran, alone or after anchored ones"
    );
    assert_eq!(
        runnable_hints_gate(&[
            source(
                "a.rs",
                "\"{see}git checkout HEAD -- :/a ':/b c' /d '/e f')\""
            ),
            source("b.rs", "\"{see}git checkout HEAD -- {})\""),
            source("c.ts", "`{see}git checkout HEAD -- :/${file} ${other})`"),
            source("d.rs", "\"{see}git restore --worktree -- ':/a')\""),
        ]),
        Ok(()),
        "from the root, absolute, quoted, interpolated; another restore is no checkout"
    );
}

#[test]
fn refuses_a_command_in_code_quotes_in_a_message_of_rust() {
    // Findings H7-1, H13-2 and J13-n2 of the fifth deep review of visual block 2 (#34):
    // remedies named their command in code quotes, which the gate never read, while the
    // rule said it holds every remedy.
    let quoted = |at: &str, span: &str| {
        format!(
            "{at}: a message names the command `{span}` in code quotes: write it as a hint, \
             `{HINT_OPENING}{span})`, which the hints gate reads"
        )
    };
    let tick = '`';
    assert_eq!(
        runnable_hints_gate(&[
            source(
                "crates/a/src/x.rs",
                &format!("\"run {tick}cargo xtask bless{tick}\"")
            ),
            source(
                "crates/a/src/y.rs",
                &format!("\"first {tick}x{tick}, then\\\n    {tick}pnpm \\\n    roadmap{tick}\""),
            ),
        ]),
        Err(Failure {
            summary: "2 problems, listed above".to_owned(),
            details: vec![
                quoted("crates/a/src/x.rs:1", "cargo xtask bless"),
                quoted("crates/a/src/y.rs:2", "pnpm roadmap"),
            ],
            usage: false,
        }),
        "a span naming a command, wrapped over a continued string too; one naming none kept"
    );
    for (path, text) in [
        (
            "crates/a/src/x_tests.rs",
            format!("\"run {tick}cargo xtask bless{tick}\""),
        ),
        (
            "crates/a/tests/binary.rs",
            format!("\"run {tick}cargo xtask bless{tick}\""),
        ),
        (
            "tests/x.rs",
            format!("\"run {tick}cargo xtask bless{tick}\""),
        ),
        (
            "crates/a/src/x.rs",
            format!("/// Run {tick}cargo xtask bless{tick}."),
        ),
        (
            "x.ts",
            format!("const a = \"run {tick}cargo xtask bless{tick}\";"),
        ),
        (
            "crates/a/src/x.rs",
            format!("\"an open {tick}cargo xtask\""),
        ),
    ] {
        assert_eq!(
            runnable_hints_gate(&[source(path, &text)]),
            Ok(()),
            "{path}: {text}"
        );
    }
}
