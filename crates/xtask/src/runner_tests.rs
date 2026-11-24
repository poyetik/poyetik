//! Tests of how the check runs commands and reads files: the runner, its failures and its real
//! implementation.

use std::ffi::OsString;
use std::io;
use std::path::Path;

use crate::tests::{fixture, hermetic_git, hermetic_git_command, scratch, system};
use crate::{Cmd, CommandFailure, Dir, Exit, Failed, Ran, Runner, SystemRunner};

#[test]
fn keeps_the_user_s_git_files_from_a_test_s_git() {
    let dir = scratch("user-git-files");
    let repository = dir.join("repository");
    let home = dir.join("home");
    std::fs::create_dir_all(repository.join("x.d")).expect("a directory");
    std::fs::create_dir_all(home.join(".config/git")).expect("the user's git directory");
    std::fs::write(repository.join("x.d/f"), "f\n").expect("a file");
    std::fs::write(home.join(".config/git/ignore"), "*.d\n").expect("the user's ignore file");
    let _init = hermetic_git(&repository, &["init", "--quiet", "--initial-branch=main"]);
    let status = |hermetic: bool| {
        let mut command = hermetic_git_command(
            &repository,
            &["status", "--porcelain", "--untracked-files=all"],
        );
        let _home = command.env("HOME", &home);
        if !hermetic {
            let _user = command.env_remove("XDG_CONFIG_HOME");
        }
        let output = command.output().expect("git runs");
        String::from_utf8(output.stdout).expect("UTF-8")
    };
    assert_eq!(
        status(false),
        "",
        "the user's ignore file, read below HOME without XDG_CONFIG_HOME, hides the file"
    );
    assert_eq!(
        status(true),
        "?? x.d/f\n",
        "a test's git reads no file of the user's"
    );
}

#[test]
fn starts_a_test_runner_s_commands_without_the_machine_s_git_configuration() {
    let runner = system(Path::new(".").to_path_buf());
    let set = |env: &[(&str, &str)]| -> Vec<(OsString, Option<OsString>)> {
        crate::runner::command("cargo", &runner.root, env)
            .get_envs()
            .filter(|(name, _)| {
                [
                    "GIT_CONFIG_NOSYSTEM",
                    "GIT_CONFIG_GLOBAL",
                    "GIT_ATTR_NOSYSTEM",
                    "XDG_CONFIG_HOME",
                ]
                .iter()
                .any(|hermetic| name == hermetic)
            })
            .map(|(name, value)| (name.to_owned(), value.map(OsString::from)))
            .collect()
    };
    let hermetic = set(runner.env);
    for (name, value) in [
        ("GIT_CONFIG_NOSYSTEM", "1"),
        ("GIT_CONFIG_GLOBAL", "/dev/null"),
        ("GIT_ATTR_NOSYSTEM", "1"),
        ("XDG_CONFIG_HOME", "/dev/null"),
    ] {
        assert!(
            hermetic.contains(&(OsString::from(name), Some(OsString::from(value)))),
            "{name}={value}: {hermetic:?}"
        );
    }
    assert_eq!(
        set(&[]),
        Vec::new(),
        "the checks' runner keeps the user's configuration"
    );
}

/// A `PATH` of a program directory below the runner's root, before the system's shell.
static PROGRAMS_BELOW: [(&str, &str); 1] = [("PATH", "programs:/usr/bin:/bin")];

#[test]
fn finds_npm_s_global_configuration_by_the_node_that_runs() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = scratch("node-program");
    for directory in ["programs", "installed/bin", "installed/etc"] {
        std::fs::create_dir_all(dir.join(directory)).expect("a directory");
    }
    let global = dir.join("installed/etc/npmrc");
    std::fs::write(&global, "audit=false\n").expect("a global npmrc");
    let shim = dir.join("programs/node");
    let mut runner = SystemRunner {
        root: dir.path(),
        env: &PROGRAMS_BELOW,
    };
    for (said, found) in [
        (
            format!(
                "[ \"$*\" = '-p process.execPath' ] && echo '{}'",
                dir.join("installed/bin/node").display()
            ),
            true,
        ),
        ("exit 1".to_owned(), false),
    ] {
        std::fs::write(&shim, format!("#!/bin/sh\n{said}\n")).expect("a shim of Node.js");
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).expect("a program");
        assert_eq!(
            runner
                .environment()
                .node_configs
                .contains(&global.display().to_string()),
            found,
            "{said}"
        );
    }
}

#[test]
fn starts_git_without_the_variables_that_change_its_pathspecs_and_its_diffs() {
    let git = crate::runner::command("git", Path::new("."), &[]);
    for variable in [
        "GIT_LITERAL_PATHSPECS",
        "GIT_GLOB_PATHSPECS",
        "GIT_NOGLOB_PATHSPECS",
        "GIT_ICASE_PATHSPECS",
        "GIT_DIFF_OPTS",
    ] {
        assert!(
            git.get_envs()
                .any(|(name, value)| name == variable && value.is_none()),
            "{variable}"
        );
    }
}

#[test]
fn the_system_clock_advances_across_a_command() {
    let mut runner = system(fixture("coverage-locality"));
    let before = runner.clock();
    let _version = runner
        .capture(&Cmd::new("git", &["--version"], Dir::Root))
        .expect("git answers");
    let after = runner.clock();
    assert!(
        after > before,
        "starting a process takes time: {before:?}, then {after:?}"
    );
}

#[test]
fn system_runner_runs_commands_and_reads_files() {
    let mut runner = system(fixture("coverage-locality"));
    let missing = Cmd {
        program: "no-such-program-xyz",
        args: vec![],
        dir: Dir::Root,
        env: Vec::new(),
    };
    assert_eq!(
        runner.run(&missing).map_err(|failure| format!(
            "{} {}",
            matches!(failure.failed, Failed::NotStarted(_)),
            failure
        )),
        Err(
            "true cannot run `no-such-program-xyz`: No such file or directory (os error 2)"
                .to_owned()
        ),
        "a program that cannot start"
    );
    // Commands that print nothing, so that the checks' output holds no line of a test.
    #[cfg(unix)]
    {
        let shell = |script: &str| Cmd {
            program: "sh",
            args: vec!["-c".to_owned(), script.to_owned()],
            dir: Dir::Root,
            env: Vec::new(),
        };
        assert_eq!(runner.run(&shell("true")), Ok(()), "a successful command");
        assert_eq!(
            runner
                .run(&shell("exit 101"))
                .map_err(|failure| failure.failed),
            Err(Failed::Ended {
                exit: Exit::Code(101_i32),
                reason: None
            }),
            "a failing command and its code, its error output left on the terminal"
        );
        let killed = Cmd {
            program: "sh",
            args: vec!["-c".to_owned(), "kill -9 $$".to_owned()],
            dir: Dir::Root,
            env: Vec::new(),
        };
        assert_eq!(
            runner.run(&killed).map_err(|failure| failure.to_string()),
            Err("`sh -c kill -9 $$` was stopped by signal 9 (SIGKILL)".to_owned()),
            "a command a signal stopped, named"
        );
    }
    assert!(
        runner
            .read("Cargo.toml")
            .is_ok_and(|text| text.contains("[workspace]")),
        "reads below the root"
    );
    assert_eq!(
        runner
            .read("no-such-file")
            .map_err(|failure| failure.to_string()),
        Err("No such file or directory (os error 2)".to_owned()),
        "missing file"
    );
}

#[test]
fn system_runner_writes_files_and_counts_threads() {
    let root = scratch("write");
    let mut runner = system(root.path());
    assert!(runner.write("a/b/c.diff", "text\n").is_ok(), "written");
    assert_eq!(
        std::fs::read_to_string(root.join("a/b/c.diff")).expect("readable"),
        "text\n",
        "its directory created"
    );
    assert!(runner.write("a/b/c.diff", "new\n").is_ok(), "rewritten");
    assert_eq!(
        std::fs::read_to_string(root.join("a/b/c.diff")).expect("readable"),
        "new\n",
        "replaced"
    );
    assert_eq!(
        runner
            .write("a/b/c.diff/d", "")
            .map_err(|failure| failure.to_string()),
        Err("File exists (os error 17)".to_owned()),
        "cannot create a directory where a file is"
    );
    assert_eq!(
        runner
            .write("a/b", "")
            .map_err(|failure| failure.to_string()),
        Err("Is a directory (os error 21)".to_owned()),
        "cannot write a directory"
    );
    assert_eq!(
        runner.parallelism(),
        std::thread::available_parallelism().expect("known here"),
        "the machine's"
    );
}

#[test]
fn system_runner_empties_fresh_directories() {
    let root = scratch("fresh");
    let mut runner = system(root.path());
    assert!(runner.fresh_dir("out/wav").is_ok(), "created when missing");
    std::fs::write(root.join("out/wav/stale.wav"), b"old").expect("stale file");
    assert!(runner.fresh_dir("out/wav").is_ok(), "emptied when present");
    assert_eq!(
        std::fs::read_dir(root.join("out/wav"))
            .expect("still a directory")
            .count(),
        0,
        "the stale file is gone"
    );
    std::fs::write(root.join("file"), b"").expect("a plain file");
    assert_eq!(
        runner
            .fresh_dir("file/below")
            .map_err(|failure| failure.to_string()),
        Err("Not a directory (os error 20)".to_owned()),
        "cannot create under a file"
    );
    // A path that exists but cannot be emptied as a directory (a file): emptying it fails
    // with the reason of the removal, not of the creation that would follow (the file is
    // already there), and with no permission bit involved, so that it fails for root too.
    std::fs::write(root.join("plain"), b"kept").expect("a plain file");
    assert_eq!(
        runner.fresh_dir("plain").map_err(|error| error.kind()),
        Err(io::ErrorKind::NotADirectory),
        "a path that cannot be emptied is an error, said by its removal"
    );
    assert_eq!(
        std::fs::read(root.join("plain")).expect("still there"),
        b"kept",
        "and is left as it was"
    );
}

#[test]
fn system_runner_lists_the_tracked_typescript_files() {
    // A repository of its own: the tests also run from copies without git history.
    let root = scratch("git");
    std::fs::create_dir_all(root.join("src")).expect("temporary repository");
    for (file, text) in [
        ("src/a.ts", "a"),
        ("src/b.tsx", "b"),
        ("src/c.js", "c"),
        ("d.ts", "d"),
        ("src/naïve.ts", "n"),
        ("src/gone e.ts", "g"),
    ] {
        std::fs::write(root.join(file), text).expect("source file");
    }
    std::fs::write(root.join("untracked.ts"), "u").expect("untracked file");
    let _init = hermetic_git(&root, &["init", "--quiet"]);
    let _add = hermetic_git(&root, &["add", "src", "d.ts"]);
    std::fs::remove_file(root.join("src/gone e.ts")).expect("deleted, not staged");
    let files = system(root.path()).tracked(&["*.ts", "*.tsx"]);
    assert_eq!(
        files.expect("git lists the files"),
        vec![
            "d.ts".to_owned(),
            "src/a.ts".to_owned(),
            "src/b.tsx".to_owned(),
            "src/naïve.ts".to_owned()
        ],
        "tracked TypeScript only, a name out of ASCII as it is (git quotes it by default), \
         none deleted from the work tree"
    );
    let outside = scratch("no-git");
    let error = system(outside.path()).tracked(&["*.ts", "*.tsx"]);
    assert!(
        error.is_err_and(|failure| failure
            .to_string()
            .contains("exited with code 128: fatal: not a git repository")),
        "not a repository, in git's words"
    );
}

#[test]
fn runs_a_real_command_for_its_stdout_and_its_stderr() {
    let mut runner = system(fixture("coverage-locality"));
    let shown = |ran: Ran| {
        (
            ran.stdout,
            ran.stderr,
            ran.ended.map_err(|failure| failure.to_string()),
        )
    };
    let script = "echo result; echo 'note: first' >&2; echo 'error: why' >&2; exit 2";
    assert_eq!(
        shown(runner.run_for_stdout(&Cmd::new("sh", &["-c", script], Dir::Root))),
        (
            "result\n".to_owned(),
            "note: first\nerror: why\n".to_owned(),
            Err(format!("`sh -c {script}` exited with code 2: error: why"))
        ),
        "both streams kept, the reason of the failure from its stderr"
    );
    assert_eq!(
        shown(runner.run_for_stdout(&Cmd::new("sh", &["-c", "exit 3"], Dir::Root))),
        (
            String::new(),
            String::new(),
            Err("`sh -c exit 3` exited with code 3".to_owned())
        ),
        "a failure that says nothing"
    );
    assert_eq!(
        shown(runner.run_for_stdout(&Cmd::new("sh", &["-c", "printf '\\377' >&2"], Dir::Root))),
        (String::new(), "\u{fffd}".to_owned(), Ok(())),
        "a stderr that is not text, replaced"
    );
    let missing = runner.run_for_stdout(&Cmd::new("no-such-program-xtask", &[], Dir::Root));
    assert_eq!(
        shown(missing),
        (
            String::new(),
            String::new(),
            Err(
                "cannot run `no-such-program-xtask`: No such file or directory (os error 2)"
                    .to_owned()
            )
        ),
        "a program that does not start"
    );
    assert_eq!(
        shown(runner.run_for_stdout(&Cmd::new(
            "sh",
            &["-c", "printf '\\377'; echo 'error: kept' >&2"],
            Dir::Root
        ))),
        (
            String::new(),
            "error: kept\n".to_owned(),
            Err("`sh -c printf '\\377'; echo 'error: kept' >&2` wrote output that is not UTF-8 text"
                .to_owned())
        ),
        "stdout that is not text"
    );
}

#[test]
fn system_runner_captures_output_and_failures() {
    let mut runner = system(fixture("coverage-locality"));
    let echo = Cmd {
        program: "cargo",
        args: vec!["--version".to_owned()],
        dir: Dir::Root,
        env: Vec::new(),
    };
    assert!(
        runner
            .capture(&echo)
            .is_ok_and(|out| out.starts_with("cargo ")),
        "stdout captured"
    );
    let failing = Cmd {
        program: "cargo",
        args: vec!["no-such-command".to_owned()],
        dir: Dir::Root,
        env: Vec::new(),
    };
    assert_eq!(
        runner.capture(&failing).map_err(|failure| failure.to_string()),
        Err("`cargo no-such-command` exited with code 101: error: no such command: `no-such-command`".to_owned()),
        "a failing command, its code and its own reason"
    );
    let unknown = Cmd {
        program: "git",
        args: vec!["log".to_owned(), "deadbeefdeadbeef..HEAD".to_owned()],
        dir: Dir::Root,
        env: Vec::new(),
    };
    assert!(
        runner.capture(&unknown).is_err_and(|failure| {
            let text = failure.to_string();
            text.starts_with("`git log deadbeefdeadbeef..HEAD` exited with code 128: fatal: ")
                && text.matches("git log").count() == 1
        }),
        "git's own reason, the command named once"
    );
    let missing = Cmd {
        program: "no-such-program-xyz",
        args: vec![],
        dir: Dir::Root,
        env: Vec::new(),
    };
    assert_eq!(
        runner.capture(&missing).map_err(|failure| format!(
            "{} {}",
            matches!(failure.failed, Failed::NotStarted(_)),
            failure
        )),
        Err(
            "true cannot run `no-such-program-xyz`: No such file or directory (os error 2)"
                .to_owned()
        ),
        "a program that cannot start"
    );
    let binary = Cmd {
        program: "printf",
        args: vec!["\\377".to_owned()],
        dir: Dir::Root,
        env: Vec::new(),
    };
    assert_eq!(
        runner
            .capture(&binary)
            .map_err(|failure| failure.to_string()),
        Err("`printf \\377` wrote output that is not UTF-8 text".to_owned()),
        "output that is not text"
    );
}

#[test]
fn explains_why_a_command_failed() {
    let reason = |stderr: &str| crate::error_reason(stderr.as_bytes());
    assert_eq!(
        reason(
            "error: no such command: `mutants`\n\nhelp: view all installed commands with `cargo --list`\n"
        ),
        Some("error: no such command: `mutants`".to_owned()),
        "cargo: the error, not the help after it"
    );
    assert_eq!(
        reason(
            "info: syncing channel updates for 'nightly-2026-11-01'\nerror: toolchain 'nightly-2026-11-01' is not installed\n"
        ),
        Some("error: toolchain 'nightly-2026-11-01' is not installed".to_owned()),
        "rustup: the error, not the information before it"
    );
    assert_eq!(
        reason(
            "\n  fatal: ambiguous argument 'x..HEAD': unknown revision\nUse '--' to separate paths\n"
        ),
        Some("fatal: ambiguous argument 'x..HEAD': unknown revision".to_owned()),
        "git: the fatal line, trimmed"
    );
    assert_eq!(
        reason("\nsomething went wrong\nthen more\n"),
        Some("something went wrong".to_owned()),
        "otherwise the first line said"
    );
    assert_eq!(reason(" \n\n"), None, "nothing said");
    let failure = |exit: Exit, said: Option<&str>| {
        CommandFailure {
            line: "x y".to_owned(),
            failed: Failed::Ended {
                exit,
                reason: said.map(str::to_owned),
            },
        }
        .to_string()
    };
    assert_eq!(
        failure(Exit::Signal(11_i32), Some("core dumped")),
        "`x y` was stopped by signal 11 (SIGSEGV): core dumped",
        "a named signal and a reason"
    );
    assert_eq!(
        failure(Exit::Signal(31_i32), None),
        "`x y` was stopped by signal 31",
        "a signal without a portable name"
    );
    assert_eq!(
        failure(Exit::Code(2_i32), None),
        "`x y` exited with code 2",
        "a code alone"
    );
}

#[test]
fn starts_commands_with_no_input_and_git_untranslated() {
    let locale = |program: &str| {
        crate::runner::command(program, Path::new("."), &[])
            .get_envs()
            .find(|(name, _)| *name == "LC_ALL")
            .map(|(_, value)| value.map(OsString::from))
    };
    assert_eq!(
        locale("git"),
        Some(Some(OsString::from("C"))),
        "git's messages in English, which error_reason reads"
    );
    assert_eq!(locale("cargo"), None, "the other commands keep the locale");
    let mut runner = system(fixture("coverage-locality"));
    let reads = Cmd::new(
        "sh",
        &["-c", "if read line; then echo read; else echo none; fi"],
        Dir::Root,
    );
    assert_eq!(
        runner.capture(&reads),
        Ok("none\n".to_owned()),
        "a command waiting for input finds none, instead of blocking"
    );
    let terminal = Cmd::new("sh", &["-c", "test ! -t 0"], Dir::Root);
    assert_eq!(
        runner.run(&terminal),
        Ok(()),
        "a command run, its output on the terminal, does not read the terminal"
    );
}

#[test]
fn gives_a_command_its_own_variables_alone() {
    let mut runner = system(fixture("coverage-locality"));
    let shown = Cmd::new(
        "sh",
        &["-c", "printf '%s' \"${POYETIK_PROBE-unset}\""],
        Dir::Root,
    );
    let set = shown
        .clone()
        .with_env("POYETIK_PROBE", "one".to_owned())
        .with_env("POYETIK_OTHER", "two".to_owned());
    assert_eq!(
        set.line(),
        "POYETIK_PROBE=one POYETIK_OTHER=two sh -c printf '%s' \"${POYETIK_PROBE-unset}\"",
        "its variables shown before the program, in order"
    );
    assert_eq!(runner.capture(&set), Ok("one".to_owned()), "captured: set");
    assert_eq!(
        runner.capture(&shown),
        Ok("unset".to_owned()),
        "for that command alone"
    );
    let ran = runner.run_for_stdout(&set);
    assert_eq!(
        (ran.stdout.as_str(), ran.ended.is_ok()),
        ("one", true),
        "run for its output: set"
    );
    let failing = Cmd::new("sh", &["-c", "test \"$POYETIK_PROBE\" = one"], Dir::Root);
    assert_eq!(
        runner.run(&failing).map_err(|failure| failure.to_string()),
        Err("`sh -c test \"$POYETIK_PROBE\" = one` exited with code 1".to_owned()),
        "run: unset, it fails"
    );
    assert_eq!(
        runner.run(&failing.with_env("POYETIK_PROBE", "one".to_owned())),
        Ok(()),
        "run: set"
    );
}

/// A value of each setting of [`crate::NEUTRAL_GIT_CONFIG`] that changes what git prints,
/// as a user's configuration may hold it.
const HOSTILE_GIT_CONFIG: [(&str, &str); 45] = [
    ("grep.lineNumber", "true"),
    ("grep.column", "true"),
    ("grep.fullName", "true"),
    ("grep.extendedRegexp", "true"),
    ("grep.patternType", "perl"),
    ("color.ui", "always"),
    ("color.diff", "always"),
    ("color.status", "always"),
    ("color.branch", "always"),
    ("color.grep", "always"),
    ("color.showBranch", "always"),
    ("color.interactive", "always"),
    ("diff.colorMoved", "zebra"),
    ("log.decorate", "full"),
    ("log.date", "iso"),
    ("log.mailmap", "false"),
    ("format.pretty", "oneline"),
    ("status.short", "true"),
    ("status.branch", "true"),
    ("log.showSignature", "true"),
    ("log.abbrevCommit", "true"),
    ("log.follow", "true"),
    ("log.showRoot", "false"),
    ("diff.noprefix", "true"),
    ("diff.mnemonicPrefix", "true"),
    ("diff.suppressBlankEmpty", "true"),
    ("diff.relative", "true"),
    ("diff.renames", "copies"),
    ("diff.context", "0"),
    ("diff.interHunkContext", "9"),
    ("diff.algorithm", "histogram"),
    ("diff.indentHeuristic", "false"),
    ("core.quotePath", "false"),
    ("status.showUntrackedFiles", "no"),
    ("status.relativePaths", "false"),
    // Finding H11-1 of the fifth deep review of visual block 2 (#34): a user's attributes
    // marking Markdown binary turned the breaking log of the changelog into one line, which
    // the bang gate passed, and the release search's words into `Binary file … matches`.
    ("core.attributesFile", HOSTILE_ATTRIBUTES),
    // Findings H1-3, H11-4 and H17-9 of the fifth deep review of visual block 2 (#34): a log
    // printed in another encoding (the history gate's own format), abbreviated hashes, a
    // long status's comment marks and hints, and the prefixes of newer gits.
    ("i18n.logOutputEncoding", "ISO-8859-1"),
    ("core.abbrev", "12"),
    ("status.displayCommentPrefix", "true"),
    ("status.aheadBehind", "false"),
    ("advice.statusHints", "false"),
    ("color.advice", "always"),
    ("diff.srcPrefix", "x/"),
    ("diff.dstPrefix", "y/"),
    // Finding MJ-1 of the seventh deep review of visual block 2 (#34): git 2.54 compares the
    // long status with the push branch too.
    ("status.compareBranches", "@{push}"),
];

/// The user's attributes file of [`HOSTILE_GIT_CONFIG`], relative to the repository's root
/// where the reads run, in git's own directory, so that no read lists it: Markdown binary.
const HOSTILE_ATTRIBUTES: &str = ".git/hostile-attributes";

#[test]
fn reads_git_whatever_the_user_s_configuration() {
    assert_eq!(
        HOSTILE_GIT_CONFIG.map(|(key, _)| key),
        crate::NEUTRAL_GIT_CONFIG.map(|(key, _)| key),
        "a hostile value for every setting neutralized, in its order"
    );
    let dir = scratch("neutral-git");
    std::fs::create_dir_all(dir.join("sub")).expect("a directory");
    std::fs::write(dir.join("sub/a.rs"), "fn Foo() {}\n\n\nx\n").expect("a file");
    std::fs::write(dir.join("sub/a.md"), "Foo\n").expect("a Markdown file");
    let _init = hermetic_git(&dir, &["init", "--quiet", "--initial-branch=main"]);
    let _add = hermetic_git(&dir, &["add", "."]);
    let _first = hermetic_git(&dir, &["commit", "--quiet", "-m", "first"]);
    std::fs::write(dir.join("sub/a.rs"), "fn Foo() {}\n\n\ny\n").expect("a change");
    std::fs::write(dir.join("sub/a.md"), "Foo, changed\n").expect("a Markdown change");
    std::fs::write(dir.join("sub/é.rs"), "z\n").expect("a file named in UTF-8");
    let _second = hermetic_git(&dir, &["commit", "--quiet", "-am", "second, café ×"]);
    std::fs::write(dir.join("new.rs"), "n\n").expect("an untracked file");
    // Each read in git's plainest form, none of the options xtask's own reads add.
    let reads: [&[&str]; 11] = [
        &["grep", "-h", "--only-matching", "-e", "Foo", "HEAD", "--"],
        &["log", "--format=%H %s", "HEAD"],
        &["log", "--patch", "--format=%H", "HEAD"],
        &["log", "HEAD"],
        &["show", "HEAD"],
        &["diff", "HEAD~1", "HEAD"],
        &["status", "--short", "--untracked-files"],
        &["status"],
        &["branch"],
        &["grep", "-n", "-e", "Foo", "HEAD", "--"],
        &["ls-files", "--others", "--cached"],
    ];
    let read_all = || -> Vec<String> {
        let mut runner = system(dir.path());
        reads
            .iter()
            .map(|args| {
                runner
                    .capture(&Cmd::new("git", args, Dir::Root))
                    .expect("git reads")
            })
            .collect()
    };
    let plain = read_all();
    let raw_grep = hermetic_git(&dir, reads[0]);
    // Finding F11-1 of the fourth deep review of visual block 2 (#34): `color.ui` alone left
    // `color.diff=always` coloring a diff, and `log.decorate` and `format.pretty` changed a
    // log read without its own format.
    let raw_log = hermetic_git(&dir, reads[3]);
    let raw_diff = hermetic_git(&dir, reads[5]);
    // The log's encoding last, after the raw reads below, which read UTF-8.
    let (encoding, others): (Vec<_>, Vec<_>) = HOSTILE_GIT_CONFIG
        .into_iter()
        .partition(|(key, _)| *key == "i18n.logOutputEncoding");
    for (key, value) in others {
        let _set = hermetic_git(&dir, &["config", key, value]);
    }
    std::fs::write(dir.join(HOSTILE_ATTRIBUTES), "*.md binary\n").expect("the attributes");
    assert_ne!(
        hermetic_git(&dir, reads[0]),
        raw_grep,
        "the hostile configuration changes what git prints without xtask's settings"
    );
    assert_ne!(
        hermetic_git(&dir, reads[3]),
        raw_log,
        "a log without a format"
    );
    assert_ne!(hermetic_git(&dir, reads[5]), raw_diff, "a diff colored");
    assert!(
        hermetic_git(&dir, reads[2]).contains("Binary files "),
        "Markdown binary"
    );
    for (key, value) in encoding {
        let _set = hermetic_git(&dir, &["config", key, value]);
    }
    let mut latin = hermetic_git_command(&dir, &["log", "-1", "--format=%s"]);
    assert_eq!(
        String::from_utf8(latin.output().expect("git runs").stdout)
            .map_err(|error| error.to_string()),
        Err("invalid utf-8 sequence of 1 bytes from index 11".to_owned()),
        "a message printed in ISO-8859-1"
    );
    assert_eq!(
        read_all(),
        plain,
        "every read as with no configuration of the user's"
    );
}

/// Whether `key` is a setting of a family of [`crate::GIT_OUTPUT_FAMILIES`]: a name of one
/// of its prefixes with no slot nor subsection, or the family's own name, in any case, as
/// git reads names.
fn of_a_family(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    crate::GIT_OUTPUT_FAMILIES.iter().any(|family| {
        key.strip_prefix(&family.to_ascii_lowercase())
            .is_some_and(|rest| {
                family.ends_with('.') && !rest.contains(['.', '<']) || rest.is_empty()
            })
    })
}

/// How many rows of [`crate::NEUTRAL_GIT_CONFIG`] and [`crate::GIT_KEYS_NO_GATE_READS`]
/// name `key`, compared ASCII-case-insensitively, as git compares the names of settings.
fn rows_naming(key: &str) -> usize {
    crate::NEUTRAL_GIT_CONFIG
        .iter()
        .chain(&crate::GIT_KEYS_NO_GATE_READS)
        .filter(|(row, _)| row.eq_ignore_ascii_case(key))
        .count()
}

/// The settings of a family that `listed` (one per line, as `git help --config` prints
/// them) holds and neither table classes.
fn unclassed(listed: &str) -> Vec<&str> {
    listed
        .lines()
        .filter(|key| of_a_family(key))
        .filter(|key| rows_naming(key) == 0)
        .collect()
}

#[test]
fn classes_a_setting_in_any_case_as_git_does() {
    // Finding MJ-1 of the seventh deep review of visual block 2 (#34): git 2.45 lists
    // `diff.noPrefix` where 2.39 listed `diff.noprefix`, and git reads both as one name.
    let listed = "diff.noPrefix\nCOLOR.UI\nlog.graphIndent\nlog.unknownProbe\ncore.abbrev\n";
    assert_eq!(unclassed(listed), ["log.unknownProbe"]);
}

#[test]
fn classes_every_output_setting_git_lists() {
    // Findings H1-3, H11-4 and H17-9 of the fifth deep review of visual block 2 (#34): the
    // neutral settings missed some the claimed reads show. Every setting of the families
    // that change git's output, as this machine's git lists them, is neutralized or
    // reviewed as changing no read of a gate.
    let dir = scratch("git-settings");
    let listed = hermetic_git(&dir, &["help", "--config"]);
    assert_eq!(
        unclassed(&listed),
        Vec::<&str>::new(),
        "every listed setting classed"
    );
    // What git lists is read, its families found in it: keys every git since 2.30 lists, and a
    // floor under the 70 of git 2.39.2, which this was written with.
    for key in ["color.ui", "log.date", "diff.noprefix"] {
        assert!(of_a_family(key), "{key}");
        assert!(
            listed.lines().any(|line| line.eq_ignore_ascii_case(key)),
            "{key} listed"
        );
    }
    // Git 2.48 and 2.49 list no backticked name of their documentation, `diff.*` among them
    // (git's generator, fixed in 2.50 by e1b81f54d): neither passes this floor.
    assert!(
        listed.lines().filter(|key| of_a_family(key)).count() > 60,
        "the families hold the settings git lists"
    );
    for (key, reason) in crate::GIT_KEYS_NO_GATE_READS {
        assert!(of_a_family(key) && rows_naming(key) == 1, "{key}");
        assert!(!reason.is_empty(), "{key}");
    }
    for (key, _) in crate::NEUTRAL_GIT_CONFIG {
        assert!(of_a_family(key), "{key}: a neutral setting of a family");
        assert_eq!(rows_naming(key), 1, "{key}: one row");
    }
    assert!(
        !of_a_family("color.diff.meta"),
        "a color's slot is no setting of its own"
    );
    assert!(
        !of_a_family("diff.<driver>.binary"),
        "a driver's setting neither"
    );
    assert!(
        !of_a_family("core.abbreviation"),
        "a name only, never its prefix"
    );
}

#[test]
fn ignores_the_user_s_git_configuration() {
    // Finding H11-3 of the fifth deep review of visual block 2 (#34): `GIT_CONFIG_GLOBAL`,
    // which keeps the tests from the user's configuration, needs git 2.32; an older git
    // ignores it silently. The git that runs is asked for a setting of a user's
    // configuration it must not read.
    let home = scratch("git-home");
    std::fs::write(home.join(".gitconfig"), "[poyetik]\n\tprobe = user\n").expect("a config");
    let probe = |hermetic: bool| {
        let mut git = hermetic_git_command(&home, &["config", "--get", "poyetik.probe"]);
        let _home = git.env("HOME", home.path());
        if !hermetic {
            let _user = git
                .env_remove("XDG_CONFIG_HOME")
                .env_remove("GIT_CONFIG_GLOBAL");
        }
        String::from_utf8(git.output().expect("git runs").stdout).expect("UTF-8")
    };
    assert_eq!(
        probe(false),
        "user\n",
        "the user's file, read without the variables"
    );
    assert_eq!(probe(true), "", "none read under them");
}
