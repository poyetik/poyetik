//! Tests of the hermetic git gate: the places that start a process, and the copies of git's
//! variables.

use super::{PROCESS_STARTS, SCRIPT_STARTS, STARTS_FILE, hermetic_git_gate};
use crate::steps::Failure;
use crate::tests::{failed, source};

/// A copy of `HERMETIC_GIT` as a Rust test declares it, as literal facts: a copy whose
/// entries follow xtask's would agree with any change of them.
const HERMETIC_COPY: &str = "const HERMETIC_GIT: [(&str, &str); 4] = [(\"GIT_CONFIG_NOSYSTEM\", \"1\"), (\"GIT_CONFIG_GLOBAL\", \"/dev/null\"), (\"GIT_ATTR_NOSYSTEM\", \"1\"), (\"XDG_CONFIG_HOME\", \"/dev/null\")];\n";

/// A copy of `LOCAL_GIT_VARIABLES` as a Rust test declares it.
const LOCAL_COPY: &str = "const LOCAL_GIT_VARIABLES: [&str; 14] = [\"GIT_ALTERNATE_OBJECT_DIRECTORIES\", \"GIT_CONFIG\", \"GIT_CONFIG_PARAMETERS\", \"GIT_CONFIG_COUNT\", \"GIT_OBJECT_DIRECTORY\", \"GIT_DIR\", \"GIT_WORK_TREE\", \"GIT_IMPLICIT_WORK_TREE\", \"GIT_GRAFT_FILE\", \"GIT_INDEX_FILE\", \"GIT_REPLACE_REF_BASE\", \"GIT_PREFIX\", \"GIT_SHALLOW_FILE\", \"GIT_COMMON_DIR\"];\n";

/// A copy of `HERMETIC_GIT` as a script declares it.
const HERMETIC_SCRIPT: &str = "const HERMETIC_GIT: Readonly<Record<string, string>> = {\n  \"GIT_CONFIG_NOSYSTEM\": \"1\",\n  \"GIT_CONFIG_GLOBAL\": devNull,\n  \"GIT_ATTR_NOSYSTEM\": \"1\",\n  \"XDG_CONFIG_HOME\": devNull,\n};\n";

/// A copy of `LOCAL_GIT_VARIABLES` as a script declares it.
const LOCAL_SCRIPT: &str = "const LOCAL_GIT_VARIABLES: readonly string[] = [\n  \"GIT_ALTERNATE_OBJECT_DIRECTORIES\",\n  \"GIT_CONFIG\",\n  \"GIT_CONFIG_PARAMETERS\",\n  \"GIT_CONFIG_COUNT\",\n  \"GIT_OBJECT_DIRECTORY\",\n  \"GIT_DIR\",\n  \"GIT_WORK_TREE\",\n  \"GIT_IMPLICIT_WORK_TREE\",\n  \"GIT_GRAFT_FILE\",\n  \"GIT_INDEX_FILE\",\n  \"GIT_REPLACE_REF_BASE\",\n  \"GIT_PREFIX\",\n  \"GIT_SHALLOW_FILE\",\n  \"GIT_COMMON_DIR\",\n];\n";

/// The import that starts processes in a script.
const IMPORT: &str = "import { execFileSync } from \"node:child_process\";\n";

/// The test file of a row of [`PROCESS_STARTS`] that starts git hermetically.
const SHELL_FILE: &str = "crates/audio-cli/tests/binary.rs";

/// A `shell` starting `sh`, whose body holds `body` after it.
fn shell(body: &str) -> String {
    format!("fn shell(dir: &Path) {{ let mut sh = Command::new(\"sh\"); {body} }}\n")
}

/// The body that sets and removes git's variables.
const HERMETIC_BODY: &str =
    "let _ = sh.envs(HERMETIC_GIT); for v in LOCAL_GIT_VARIABLES { let _ = sh.env_remove(v); }";

/// The gate on one file.
fn gate(path: &str, text: &str) -> Result<(), Failure> {
    hermetic_git_gate(&[source(path, text)])
}

/// What the gate says a Rust or script start lacks.
fn lacks(place: &str, what: &str) -> Result<(), Failure> {
    failed(format!(
        "{place} {what}: name both lists, the file's own copy or xtask's in xtask \
         (crates/xtask/src/runner.rs), so that the git it starts reads neither the machine's \
         configuration nor an enclosing git process's"
    ))
}

/// What the gate says a start of git on the repository itself lacks, which keeps the user's
/// configuration.
fn lacks_local(place: &str) -> Result<(), Failure> {
    failed(format!(
        "{place} removes no variable of LOCAL_GIT_VARIABLES: remove them, the file's own copy \
         or xtask's in xtask (crates/xtask/src/runner.rs), so that the git it starts reads no \
         enclosing git process's, keeping the user's configuration (`safe.directory`)"
    ))
}

/// What the gate says of a script's copy of `list` at `place` that is not `copy`, xtask's.
fn unlike_script(place: &str, list: &str, copy: &str) -> Result<(), Failure> {
    failed(format!(
        "{place}: `{list}` is not xtask's (crates/xtask/src/runner.rs): declare it once, in code, \
         as `{}` (white space aside)",
        copy.split_whitespace().collect::<Vec<_>>().join(" ")
    ))
}

/// What the gate says of a copy of `list` at `place` that is not xtask's.
fn unlike(place: &str, list: &str) -> Result<(), Failure> {
    failed(format!(
        "{place}: `{list}` is not xtask's (crates/xtask/src/runner.rs): declare the same \
         entries, in the same order, as literals"
    ))
}

#[test]
fn accepts_a_listed_start_with_its_copies() {
    let whole = format!("{HERMETIC_COPY}{LOCAL_COPY}{}", shell(HERMETIC_BODY));
    assert_eq!(gate(SHELL_FILE, &whole), Ok(()), "a copy of both lists");
    assert_eq!(
        gate(
            "crates/xtask/tests/binary.rs",
            "use xtask::{HERMETIC_GIT, LOCAL_GIT_VARIABLES};\nfn git_command() { let c = Command::new(\"git\"); c.envs(HERMETIC_GIT); for v in LOCAL_GIT_VARIABLES {} }\n",
        ),
        Ok(()),
        "xtask's own lists, in xtask"
    );
    assert_eq!(
        gate(
            "crates/xtask/src/runner.rs",
            "fn command(program: &str) { let c = Command::new(program); for v in LOCAL_GIT_VARIABLES {} }\n",
        ),
        Ok(()),
        "xtask's runner keeps the user's configuration"
    );
    assert_eq!(
        gate(
            "crates/visual-gpu/src/bench.rs",
            "impl A { fn system(&self) { std::process::Command::new(self.program); } }\n",
        ),
        Ok(()),
        "no git"
    );
    for itself in [
        "Command::new(env!(\"CARGO_BIN_EXE_audio-cli\"))",
        "Command::new(std::env::current_exe().expect(\"the test binary\"))",
    ] {
        assert_eq!(
            gate("crates/a/tests/t.rs", &format!("fn f() {{ {itself}; }}\n")),
            Ok(()),
            "{itself}: the crate's own program"
        );
    }
    let script = format!("{IMPORT}{HERMETIC_SCRIPT}{LOCAL_SCRIPT}");
    assert_eq!(
        gate("tooling/wasm-build/test/bench.test.ts", &script),
        Ok(()),
        "a script's copy of both lists"
    );
    assert_eq!(
        gate(
            "tooling/config/test/linted.test.ts",
            &format!("{IMPORT}{LOCAL_SCRIPT}")
        ),
        Ok(()),
        "the repository itself, its user's configuration kept"
    );
    assert_eq!(
        gate("tooling/oracle/test/bin.test.ts", IMPORT),
        Ok(()),
        "a script that starts no git"
    );
    assert_eq!(
        gate("a.ts", "// The HERMETIC_GIT of xtask.\nconst other = 1;\n"),
        Ok(()),
        "a list named, none declared"
    );
    assert_eq!(
        gate(
            "a.rs",
            "//! Command::new(\"git\")\nfn f() { g(\"Command::new(\"); }\n"
        ),
        Ok(()),
        "a comment and a string start nothing"
    );
    assert_eq!(
        gate(
            "a.rs",
            "enum Command { A }\nfn f() { Command::A; let Command = 1; }\nfn g(a: u8, Command: u8) {}\nfn h() -> Command"
        ),
        Ok(()),
        "another `Command`, never made by `new`"
    );
}

#[test]
fn refuses_a_start_that_is_no_row() {
    let unlisted = |place: &str, function: &str| {
        failed(format!(
            "{place}: `{function}` starts a process and is no row of PROCESS_STARTS: add it, \
             with what it may start and why (crates/xtask/src/hermetic.rs)"
        ))
    };
    let path = "crates/a/tests/t.rs";
    for (text, function) in [
        ("fn f() { Command::new(\"git\").output(); }\n", "f"),
        ("fn f() { Command::new(\"sh\").output(); }\n", "f"),
        ("fn f() { Command::new(GIT).output(); }\n", "f"),
        ("fn f() { Command::new(\"/usr/bin/git\").output(); }\n", "f"),
        ("fn f() { Command::new(env!(\"GIT\")).output(); }\n", "f"),
        ("fn f() { Command :: new (program).output(); }\n", "f"),
        ("fn f() { let start = Command::new; start(GIT); }\n", "f"),
        (
            "fn f() { fn g() {} std::process::Command::new(\"git\"); }\n",
            "f",
        ),
        (
            "fn f() { fn g(a: [u8; { 1 }]) { Command::new(\"git\"); } }\n",
            "g",
        ),
        (
            "trait T { fn declared(a: [u8; 2]); fn f() { Command::new(\"git\"); } }\n",
            "f",
        ),
        ("static C: L = L::new(|| Command::new(\"git\"));\n", ""),
        (
            "fn before() {} static C: L = L::new(|| Command::new(\"git\"));\n",
            "",
        ),
    ] {
        assert_eq!(
            gate(path, text),
            unlisted(&format!("{path}:1"), function),
            "{text}"
        );
    }
    assert_eq!(
        gate(
            SHELL_FILE,
            &format!(
                "{HERMETIC_COPY}{LOCAL_COPY}fn other() {{ Command::new(\"sh\"); {HERMETIC_BODY} }}\n"
            )
        ),
        unlisted(&format!("{SHELL_FILE}:3"), "other"),
        "another function of a listed file"
    );
    assert_eq!(
        gate(
            "crates/a/tests/t.rs",
            &format!("{HERMETIC_COPY}{LOCAL_COPY}{}", shell(HERMETIC_BODY))
        ),
        unlisted("crates/a/tests/t.rs:3", "shell"),
        "a listed function's name in another file, whatever its file declares"
    );
    let script = |file: &str| {
        failed(format!(
            "{file}: holds `child_process` and is no row of SCRIPT_STARTS: add it, with what \
             it may start and why (crates/xtask/src/hermetic.rs)"
        ))
    };
    for text in [
        "import { execFileSync } from \"node:child_process\";\nexecFileSync(\n  \"git\",\n  [],\n);\n",
        "import * as run from \"child_process\";\nrun.execSync(\"git status\");\n",
        "const { spawn } = await import(\"node:child_process\");\nspawn(GIT);\n",
    ] {
        assert_eq!(gate("a/b.test.ts", text), script("a/b.test.ts"), "{text}");
    }
}

#[test]
fn refuses_a_command_under_another_name() {
    assert_eq!(
        gate(
            "crates/a/tests/t.rs",
            "use std::process::Command as C;\nfn f() { C::new(\"git\"); }\n"
        ),
        failed(
            "crates/a/tests/t.rs:1: renames `Command`, so that the processes it starts are seen \
             by no gate: name it as it is"
        ),
        "an alias"
    );
}

#[test]
fn refuses_a_listed_start_without_git_s_variables() {
    let both = "sets no variable of HERMETIC_GIT and removes no variable of LOCAL_GIT_VARIABLES";
    let declared = format!("{HERMETIC_COPY}{LOCAL_COPY}");
    let at = |line: usize| format!("{SHELL_FILE}:{line}: `shell`");
    for (text, line, what) in [
        (format!("{declared}{}", shell("")), 3, both),
        (
            format!(
                "{declared}{}",
                shell(
                    "/* HERMETIC_GIT LOCAL_GIT_VARIABLES */ g(\"HERMETIC_GIT LOCAL_GIT_VARIABLES\");"
                )
            ),
            3,
            both,
        ),
        (
            format!("{declared}{}fn g() {{ {HERMETIC_BODY} }}\n", shell("")),
            3,
            both,
        ),
        (shell(HERMETIC_BODY), 1, both),
        (
            format!("{LOCAL_COPY}{}", shell(HERMETIC_BODY)),
            2,
            "sets no variable of HERMETIC_GIT",
        ),
        (
            format!("{HERMETIC_COPY}{}", shell(HERMETIC_BODY)),
            2,
            "removes no variable of LOCAL_GIT_VARIABLES",
        ),
        (
            format!("{declared}{}", shell("sh.envs(HERMETIC_GIT);")),
            3,
            "removes no variable of LOCAL_GIT_VARIABLES",
        ),
        (
            format!("{declared}{}", shell("for v in LOCAL_GIT_VARIABLES {}")),
            3,
            "sets no variable of HERMETIC_GIT",
        ),
    ] {
        assert_eq!(gate(SHELL_FILE, &text), lacks(&at(line), what), "{text}");
    }
    assert_eq!(
        gate(
            "crates/xtask/src/runner.rs",
            "fn command(program: &str) { Command::new(program).envs(HERMETIC_GIT); }\n"
        ),
        lacks_local("crates/xtask/src/runner.rs:1: `command`"),
        "the repository itself, an enclosing git process's variables kept"
    );
    let bench = "tooling/wasm-build/test/bench.test.ts";
    for (text, path, what) in [
        (IMPORT.to_owned(), bench, both),
        (
            format!("{IMPORT}{LOCAL_SCRIPT}"),
            bench,
            "sets no variable of HERMETIC_GIT",
        ),
        (
            format!("{IMPORT}{HERMETIC_SCRIPT}"),
            bench,
            "removes no variable of LOCAL_GIT_VARIABLES",
        ),
    ] {
        assert_eq!(
            gate(path, &text),
            lacks(&format!("{path}:"), what),
            "{text}"
        );
    }
    // Finding MA-4 of the seventh deep review of visual block 2 (#34): a start on the
    // repository itself was told to name both lists, which loses `safe.directory`.
    let linted = "tooling/config/test/linted.test.ts";
    assert_eq!(gate(linted, IMPORT), lacks_local(&format!("{linted}:")));
}

#[test]
fn refuses_a_copy_that_is_not_xtask_s() {
    for (list, copy, others) in [
        (
            "HERMETIC_GIT",
            HERMETIC_COPY,
            [
                ("(\"GIT_ATTR_NOSYSTEM\", \"1\"), ", ""),
                (
                    "\"GIT_ATTR_NOSYSTEM\", \"1\"",
                    "\"GIT_ATTR_NOSYSTEM\", \"0\"",
                ),
                (
                    "(\"GIT_CONFIG_GLOBAL\", \"/dev/null\"), (\"GIT_ATTR_NOSYSTEM\", \"1\")",
                    "(\"GIT_ATTR_NOSYSTEM\", \"1\"), (\"GIT_CONFIG_GLOBAL\", \"/dev/null\")",
                ),
                ("\"XDG_CONFIG_HOME\"", "XDG"),
                ("= [", "= &["),
                ("const", "static"),
            ],
        ),
        (
            "LOCAL_GIT_VARIABLES",
            LOCAL_COPY,
            [
                (", \"GIT_COMMON_DIR\"", ""),
                ("\"GIT_DIR\"", "\"GIT_DIRS\""),
                (
                    "\"GIT_DIR\", \"GIT_WORK_TREE\"",
                    "\"GIT_WORK_TREE\", \"GIT_DIR\"",
                ),
                ("\"GIT_PREFIX\"", "concat!(\"GIT_\", \"PREFIX\")"),
                ("= [", "= other::ALL; const O: [u8; 0] = ["),
                ("const", "static"),
            ],
        ),
    ] {
        assert_eq!(gate("crates/a/src/b.rs", copy), Ok(()), "xtask's entries");
        for (from, to) in others {
            let text = copy.replacen(from, to, 1);
            assert_ne!(text, copy, "{from}: replaced");
            let found = gate("crates/a/src/b.rs", &text);
            if to == "static" {
                assert_eq!(found, Ok(()), "{list}: a static of the same entries");
            } else {
                assert_eq!(found, unlike("crates/a/src/b.rs:1", list), "{text}");
            }
        }
    }
    assert_eq!(
        gate(
            "crates/a/src/b.rs",
            "fn f() {}\n\nstatic HERMETIC_GIT: [(&str, &str); 0] = [];\n"
        ),
        unlike("crates/a/src/b.rs:3", "HERMETIC_GIT"),
        "a static of other entries, in a file that starts nothing, named at its line"
    );
    for (list, copy, others) in [
        (
            "HERMETIC_GIT",
            HERMETIC_SCRIPT,
            vec![
                ("  \"GIT_ATTR_NOSYSTEM\": \"1\",\n", ""),
                (
                    "\"GIT_ATTR_NOSYSTEM\": \"1\"",
                    "\"GIT_ATTR_NOSYSTEM\": \"0\"",
                ),
                (
                    "\"XDG_CONFIG_HOME\": devNull",
                    "\"XDG_CONFIG_HOME\": \"/dev/null\"",
                ),
                (": Readonly<Record<string, string>>", ""),
                ("const ", "let "),
                ("const ", "var "),
                ("const ", "// const "),
                (
                    "};\n",
                    "};\nconst HERMETIC_GIT_2 = 1; const HERMETIC_GIT = {};\n",
                ),
            ],
        ),
        (
            "LOCAL_GIT_VARIABLES",
            LOCAL_SCRIPT,
            vec![
                ("  \"GIT_COMMON_DIR\",\n", ""),
                (
                    "\"GIT_DIR\",\n  \"GIT_WORK_TREE\"",
                    "\"GIT_WORK_TREE\",\n  \"GIT_DIR\"",
                ),
                ("readonly string[]", "string[]"),
                ("const ", "/* const "),
            ],
        ),
    ] {
        assert_eq!(gate("a/b.test.ts", copy), Ok(()), "xtask's entries");
        for (from, to) in others {
            let text = copy.replacen(from, to, 1);
            assert_ne!(text, copy, "{from}: replaced");
            assert_eq!(
                gate("a/b.test.ts", &text),
                unlike_script("a/b.test.ts", list, copy),
                "{text}"
            );
        }
    }
    let line = format!(
        "// {}",
        LOCAL_SCRIPT
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    );
    assert_eq!(
        gate("a/b.test.ts", &line),
        unlike_script("a/b.test.ts", "LOCAL_GIT_VARIABLES", LOCAL_SCRIPT),
        "the whole list in a comment"
    );
}

#[test]
fn refuses_a_stale_row_in_the_tree_that_holds_the_tables() {
    let mut tree = vec![source(STARTS_FILE, "")];
    for (path, function, _, _) in PROCESS_STARTS {
        tree.push(source(
            path,
            &format!("{HERMETIC_COPY}{LOCAL_COPY}fn {function}() {{ let mut sh = Command::new(p); {HERMETIC_BODY} }}\n"),
        ));
    }
    for (path, _, _) in SCRIPT_STARTS {
        tree.push(source(
            path,
            &format!("{IMPORT}{HERMETIC_SCRIPT}{LOCAL_SCRIPT}"),
        ));
    }
    assert_eq!(hermetic_git_gate(&tree), Ok(()), "every row current");
    for (path, place) in [
        (SHELL_FILE, format!("{SHELL_FILE}: `shell`")),
        (
            "tooling/oracle/test/bin.test.ts",
            "tooling/oracle/test/bin.test.ts".to_owned(),
        ),
    ] {
        let stale: Vec<_> = tree
            .iter()
            .map(|(file, text)| {
                let text = if file == path { "" } else { text.as_str() };
                source(file, text)
            })
            .collect();
        let refusal = failed(format!(
            "{place}: starts no process any more: remove its row (crates/xtask/src/hermetic.rs)"
        ));
        assert_eq!(hermetic_git_gate(&stale), refusal, "{path}: starts nothing");
        let gone: Vec<_> = tree
            .iter()
            .filter(|(file, _)| file != path)
            .cloned()
            .collect();
        assert_eq!(hermetic_git_gate(&gone), refusal, "{path}: gone");
        let other: Vec<_> = gone
            .iter()
            .filter(|(file, _)| file != STARTS_FILE)
            .cloned()
            .collect();
        assert_eq!(hermetic_git_gate(&other), Ok(()), "{path}: another tree");
    }
}

#[test]
fn refuses_a_source_that_does_not_lex() {
    assert_eq!(
        gate("a.rs", "Command \"open"),
        failed(
            "a.rs:1: does not lex as Rust (cannot parse string into token stream): the gates read the tokens of every Rust source"
        ),
        "never read as starting nothing"
    );
}
