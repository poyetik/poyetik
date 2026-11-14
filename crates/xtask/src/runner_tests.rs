//! Tests of how the check runs commands and reads files: the runner, its failures and its real
//! implementation.

use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::process::Command;

use crate::tests::{HERMETIC, fixture, hermetic_git, scratch, system};
use crate::{Cmd, CommandFailure, Dir, Exit, Failed, Ran, Runner};

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
        let mut command = Command::new("git");
        let _hermetic = command
            .args(["status", "--porcelain", "--untracked-files=all"])
            .current_dir(&repository)
            .envs(HERMETIC)
            .env("HOME", &home);
        if !hermetic {
            let _user = command.env_remove("XDG_CONFIG_HOME");
        }
        for variable in crate::LOCAL_GIT_VARIABLES {
            let _without = command.env_remove(variable);
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
    };
    assert!(
        runner
            .run(&missing)
            .is_err_and(|failure| matches!(failure.failed, Failed::NotStarted(_))
                && failure
                    .to_string()
                    .starts_with("cannot run `no-such-program-xyz`: ")),
        "a program that cannot start"
    );
    // Commands that print nothing, so that the checks' output holds no line of a test.
    #[cfg(unix)]
    {
        let shell = |script: &str| Cmd {
            program: "sh",
            args: vec!["-c".to_owned(), script.to_owned()],
            dir: Dir::Root,
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
    assert!(runner.read("no-such-file").is_err(), "missing file");
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
    assert!(
        runner.write("a/b/c.diff/d", "").is_err(),
        "cannot create a directory where a file is"
    );
    assert!(runner.write("a/b", "").is_err(), "cannot write a directory");
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
    assert!(
        runner.fresh_dir("file/below").is_err(),
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
    assert!(
        missing.stdout.is_empty()
            && missing.stderr.is_empty()
            && missing.ended.is_err_and(|failure| failure
                .to_string()
                .starts_with("cannot run `no-such-program-xtask`: ")),
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
    };
    assert!(
        runner
            .capture(&missing)
            .is_err_and(|failure| matches!(failure.failed, Failed::NotStarted(_))),
        "a program that cannot start"
    );
    let binary = Cmd {
        program: "printf",
        args: vec!["\\377".to_owned()],
        dir: Dir::Root,
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
