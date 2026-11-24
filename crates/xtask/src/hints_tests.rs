//! Tests of the runnable hints gate: every hint names a command a user can run.

use super::{HINT_OPENING, README_HINT_OPENING, RUNNABLE_PREFIXES, runnable_hints_gate};
use crate::{
    Cmd, Dir, Runner as _,
    steps::Failure,
    tests::{hermetic_git, scratch, system},
};

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
            "`{see}node --version)` `{see}${benchmarks.command} help)`\n",
        ),
        source(
            "e.rs",
            "\"lists no cards {see}\\\n    pnpm roadmap validate)\"\n",
        ),
        source(
            "f.ts",
            "/\\{see}(.+)\\)$/ // as matched by \\{see}(.+)\\)\n",
        ),
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
        (
            concat!("(run", ": "),
            concat!("x (run", ": cargo xtask help)"),
        ),
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
            "\"{see}git log a<b) {see}git log <1 >out) {see}git log <a b>) {see}git log <)\""
        )]),
        Ok(()),
        "a comparison, redirections, a bracket closed after a space or never"
    );
    // The word in the innermost brackets is the placeholder named.
    assert_eq!(
        runnable_hints_gate(&[source("a.rs", "\"{see}git log <a<b>)\"")]),
        Err(Failure::from(held("<b>", "git log <a<b>")))
    );
    // A parenthesis the command opens is closed by its own: the command goes on past it.
    assert_eq!(
        runnable_hints_gate(&[source("a.rs", "// {see}git log (a) <b>)\n")]),
        Err(Failure::from(held("<b>", "git log (a) <b>")))
    );
    // A parenthesis inside an interpolation neither opens nor closes the hint's: the
    // command ends at `x`, then past `b}`.
    assert_eq!(
        runnable_hints_gate(&[source("a.rs", "// {see}git log {a(b} x) <y>)\n")]),
        Ok(())
    );
    assert_eq!(
        runnable_hints_gate(&[source("a.rs", "// {see}git log {a)b} <y>)\n")]),
        Err(Failure::from(held("<y>", "git log {a)b} <y>")))
    );
}

/// The refusal of the hint `command`, at `at` (file and line), for `problem`.
fn refused(at: &str, command: &str, problem: &str) -> String {
    format!("{at}: the hint `{HINT_OPENING}{command})` {problem}")
}

/// Why a path a hint names is refused.
fn relative(word: &str) -> String {
    format!(
        "names `{word}` from where it runs, which fails from another directory: write the path \
         from the repository's root (`:/` for git, `\"$(git rev-parse --show-toplevel)/…\"` for \
         another command) or absolute"
    )
}

#[test]
fn accepts_only_the_git_commands_a_hint_may_name() {
    // Findings H1-1 and H14-3 of the fifth deep review of visual block 2 (#34): every tool's
    // restore ran `git checkout` without `HEAD`, which restores from the index (a damaged file
    // once staged stays damaged), by a path git refuses from a subdirectory. K7-4, K7-13 and
    // K10-4 of the sixth: the gate read `git checkout` alone, by a list of bad forms, said of a
    // switch that it restores from the index, and accepted a restore of nothing.
    let no_form = "is no git command a hint may name: write one of git checkout HEAD --, git \
                   restore --worktree --, git add --, git add --force --, git add --renormalize \
                   --, then its paths, or one of git -C \"$(git rev-parse --show-toplevel)\" \
                   update-index, git -C \"$(git rev-parse --show-toplevel)\" check-attr, git \
                   diff, git log, git rev-parse, git show";
    for command in [
        concat!("git checkout", " -- :/a"),
        "git checkout HEAD --:/a",
        "git checkout b",
        "git restore :/a",
        "git restore --staged -- :/a",
        "git reset --hard",
        "git stash",
        "git clean -fd",
        "git add :/a",
        "git update-index --no-skip-worktree -- 'a'",
        "git -C \"$(git rev-parse --show-toplevel)\" checkout HEAD -- :/a",
        "git -C sub update-index --no-skip-worktree -- 'a'",
        "git logs",
    ] {
        assert_eq!(
            runnable_hints_gate(&[source(
                "a.rs",
                &format!("\"{{see}}{})\"", command.replace('"', "\\\""))
            )]),
            Err(Failure::from(refused("a.rs:1", command, no_form))),
            "{command}"
        );
    }
    for (command, problem) in [
        ("git checkout HEAD --", "names no path".to_owned()),
        ("git checkout HEAD -- a.txt", relative("a.txt")),
        ("git checkout HEAD -- :/a '/b c' x", relative("x")),
        ("git restore --worktree -- a", relative("a")),
        ("git add --force -- 'a b'", relative("'a b'")),
    ] {
        assert_eq!(
            runnable_hints_gate(&[source("a.rs", &format!("\"{{see}}{command})\""))]),
            Err(Failure::from(refused("a.rs:1", command, &problem))),
            "{command}"
        );
    }
    assert_eq!(
        runnable_hints_gate(&[
            source(
                "a.rs",
                "\"{see}git checkout HEAD -- :/a ':/b c' /d '/e f')\""
            ),
            source("b.rs", "\"{see}git checkout HEAD -- {})\""),
            source("c.ts", "`{see}git checkout HEAD -- :/${file} ${other})`"),
            source(
                "d.rs",
                "\"{see}git restore --worktree -- ':/a') {see}git add -- ':/a') {see}git add \
                 --force -- ':/{a}') {see}git add --renormalize -- ':/a')\""
            ),
            source("e.rs", "\"{see}git checkout HEAD --\""),
            source(
                "f.rs",
                "\"{see}git -C \\\"$(git rev-parse --show-toplevel)\\\" update-index {flag} -- 'a/b') \
                 {see}git -C \\\"$(git rev-parse --show-toplevel)\\\" check-attr -a -- 'a/b')\""
            ),
            source(
                "g.rs",
                "\"{see}git show {base}:{CHANGELOG}) {see}git diff --stat {base}) {see}git \
                 rev-parse --git-common-dir) {see}git log)\""
            ),
        ]),
        Ok(()),
        "each form of the list, a path quoted or interpolated, a literal that ends before its \
         paths"
    );
}

#[test]
fn refuses_a_path_read_from_where_a_hint_runs() {
    // Finding K7-3 of the sixth deep review of visual block 2 (#34): a relative path runs from one
    // directory alone.
    for (command, word) in [
        (
            "cargo run --manifest-path crates/a/Cargo.toml -- help",
            "crates/a/Cargo.toml",
        ),
        ("node scripts/benchmarks.ts help", "scripts/benchmarks.ts"),
        ("pnpm exec node a/b.ts", "a/b.ts"),
        ("pnpm --filter x node a/b.ts", "a/b.ts"),
    ] {
        assert_eq!(
            runnable_hints_gate(&[source("a.rs", &format!("\"{{see}}{command})\""))]),
            Err(Failure::from(refused("a.rs:1", command, &relative(word)))),
            "{command}"
        );
    }
    assert_eq!(
        runnable_hints_gate(&[
            source(
                "a.rs",
                "\"{see}cargo run --manifest-path \\\"$(git rev-parse --show-toplevel)/a/Cargo.toml\\\" -- help)\""
            ),
            source(
                "b.ts",
                "`{see}pnpm --filter @a/b exec node scripts/c.ts help) {see}pnpm --filter ${a} exec node c/d.ts) {see}pnpm --filter @a/b check)`"
            ),
            source(
                "c.rs",
                "\"{see}cargo update --manifest-path {} -p x) {see}cargo xtask a '/b/c d')\""
            ),
        ]),
        Ok(()),
        "from the root, a package's name, a path from the package `pnpm --filter … exec` runs \
         in, interpolated, absolute"
    );
}

#[test]
fn refuses_a_bless_no_row_of_the_specification_writes_the_file_by() {
    // Findings K7-1, K1-2 and K7-7 of the sixth deep review of visual block 2 (#34): a hint named
    // a tool's own bless, which the project's rule replaces by one command.
    let problem = "names a bless that is none of the commands writing a file only a tool writes \
                   (spec/README.md): cargo xtask bless, cargo xtask bless-benchmarks, cargo run \
                   -p visual-cli -- bless";
    for (path, text, command) in [
        (
            "a.rs",
            "\"{see}cargo run -p visual-gpu -- bless)\"",
            "cargo run -p visual-gpu -- bless",
        ),
        (
            "a.rs",
            "\"{see}cargo xtask bless-all)\"",
            "cargo xtask bless-all",
        ),
        ("a.ts", "`{see}${command.a} bless)`", "${…} bless"),
    ] {
        assert_eq!(
            runnable_hints_gate(&[source(path, text)]),
            Err(Failure::from(refused(
                &format!("{path}:1"),
                command,
                problem
            ))),
            "{text}"
        );
    }
    assert_eq!(
        runnable_hints_gate(&[source(
            "a.rs",
            "\"{see}cargo xtask bless) {see}cargo xtask bless --case-changed {case}) {see}cargo \
             xtask bless-benchmarks) {see}cargo run -p visual-cli -- bless {}) {see}{})\""
        )]),
        Ok(()),
        "each whole or with its options, and a command wholly interpolated, its tool's tests'"
    );
    // The commands of the table's column, each without its directory, and no other.
    let index = include_str!("../../../spec/README.md");
    let written: alloc::collections::BTreeSet<String> = index
        .split_once("| File | Written by |")
        .expect("the table")
        .1
        .lines()
        .skip(2)
        .take_while(|line| line.starts_with('|'))
        .map(|line| {
            let cell = line.split('|').nth(2).expect("its second column");
            let command = cell.trim().trim_matches('`');
            command
                .split(' ')
                .take_while(|word| !word.contains('/'))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    assert_eq!(
        written,
        super::BLESS_COMMANDS
            .map(str::to_owned)
            .into_iter()
            .collect()
    );
}

#[test]
fn refuses_a_command_bare_in_parentheses_in_a_message() {
    // Finding K7-5 of the sixth deep review of visual block 2 (#34): `(pnpm build:wasm)` opens no
    // hint the gate reads.
    let bare = |at: &str, command: &str| {
        format!(
            "{at}: a message names the command `{command}` bare in parentheses: write it as a \
             hint, `{HINT_OPENING}{command})`, which the hints gate reads"
        )
    };
    for (path, text, command) in [
        ("a.rs", "\"build it (pnpm build:wasm)\"", "pnpm build:wasm"),
        (
            "a.ts",
            "const a = \"x (cargo xtask list (a)), y\";",
            "cargo xtask list (a)",
        ),
        ("b.test.ts", "`x (node a ${b})`", "node a ${…}"),
    ] {
        assert_eq!(
            runnable_hints_gate(&[source(path, text)]),
            Err(Failure::from(bare(&format!("{path}:1"), command))),
            "{text}"
        );
    }
    for (path, text) in [
        ("a_tests.rs", "\"build it (pnpm build:wasm)\""),
        ("a.rs", "// build it (pnpm build:wasm)\n"),
        ("a.rs", "\"a gitk (gitk), (x pnpm a)\""),
        ("README.md", "build it (pnpm build:wasm)\n"),
    ] {
        assert_eq!(runnable_hints_gate(&[source(path, text)]), Ok(()), "{text}");
    }
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

/// Runs the hint of `refusal` as a user's shell would, in `directory`, and returns what it
/// writes.
fn followed(directory: &std::path::Path, refusal: &Failure) -> String {
    let (_, rest) = refusal.summary.split_once(HINT_OPENING).expect("a hint");
    let command = super::hint_command(rest);
    let mut runner = system(directory.to_path_buf());
    let ran = runner.run_for_stdout(&Cmd::new("sh", &["-c", command], Dir::Root));
    assert!(ran.ended.is_ok(), "{command}: {}", ran.stderr);
    ran.stdout
}

#[test]
fn follows_the_git_commands_xtasks_own_hints_name() {
    // Finding K5-5 of the sixth deep review of visual block 2 (#34): they were read as text
    // alone. Each followed through a shell, in real git, from every directory of the shared test
    // matrix, what was staged kept.
    let matrix: serde_json::Value =
        serde_json::from_str(include_str!("../../../conformance/test-matrix.json")).expect("JSON");
    let root = scratch(matrix["hint_repository"].as_str().expect("a name"));
    let changelog = "# Changelog\n\n## [0.1.0] - 2026-01-01\n";
    let (name, file) = ("it's a.rs", root.join("it's a.rs"));
    std::fs::write(&file, "a\n").expect("a file");
    std::fs::write(root.join("CHANGELOG.md"), changelog).expect("the changelog");
    let _init = hermetic_git(&root, &["init", "--quiet"]);
    let _add = hermetic_git(&root, &["add", "."]);
    let _commit = hermetic_git(&root, &["commit", "--quiet", "-m", "first"]);
    let base = hermetic_git(&root, &["rev-parse", "HEAD"]);
    let at = (base.trim(), "main");
    for directory in matrix["hint_directories"].as_array().expect("directories") {
        let below = root.join(directory.as_str().expect("a directory"));
        std::fs::create_dir_all(&below).expect("a directory");
        for staged in matrix["hint_staged"].as_array().expect("states") {
            let kept = if staged == true { "staged\n" } else { "a\n" };
            std::fs::write(&file, kept).expect("the content kept");
            let _staged = hermetic_git(&root, &["add", name]);
            std::fs::write(&file, "mutated\n").expect("a mutant left");
            let left = crate::preflight::mutant_left_gate(&format!("{name}:1:mutated"));
            let _said = followed(&below, &left.expect_err("refused"));
            assert_eq!(std::fs::read_to_string(&file).expect("read"), kept);
            assert_eq!(hermetic_git(&root, &["show", &format!(":{name}")]), kept);
        }
        let _hide = hermetic_git(&root, &["update-index", "--skip-worktree", name]);
        let hidden = crate::hidden(&format!("S {name}\0")).expect("hidden");
        let _said = followed(&below, &Failure::from(hidden));
        let listed = hermetic_git(&root, &["ls-files", "-v", name]);
        assert!(listed.starts_with("H "), "{listed}");
        let sections = crate::released_sections_gate(Some(changelog), "# Changelog\n", at);
        assert_eq!(followed(&below, &sections.expect_err("refused")), changelog);
        let removed = "# Changelog\n\n## [Unreleased]\n\n### Removed\n\n- `gone`.\n";
        let names = crate::removed_names_gate(removed, &alloc::collections::BTreeSet::new(), at);
        assert_eq!(
            followed(&below, &names.expect_err("refused")),
            " it's a.rs | 2 +-\n 1 file changed, 1 insertion(+), 1 deletion(-)\n"
        );
    }
}
