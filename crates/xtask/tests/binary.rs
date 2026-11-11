//! Runs the real `xtask` binary: the command-line contract of every tool of the repository
//! (exit codes 0, 1 and 2; one `error:` line on stderr; a closed stdout is not an error), and
//! the git hooks through real git (on Unix). One test target, so that every build of a
//! mutant compiles and links one test binary for both.

#![forbid(unsafe_code)]

/// The command-line contract of the binary.
#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::process::{Command, Output, Stdio};

    /// The first line of the usage.
    const USAGE: &str = "usage: cargo xtask <check [--full [--part <part>]]|list [--full [--parts]]|mutants <target>|bless [--case-changed <name>]... [--battery-changed] [--from-nothing]|bless-benchmarks|commit-msg <file>|pre-push [pair...]|help>\n";

    /// Runs the binary with `args`, its stdout and stderr captured.
    fn xtask(args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(args)
            .output()
            .expect("the binary runs")
    }

    /// Reads an output as text.
    fn text(bytes: &[u8]) -> String {
        String::from_utf8(bytes.to_vec()).expect("UTF-8")
    }

    /// A file of this test, below the system's temporary directory, holding `content`.
    fn file(name: &str, content: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("xtask-binary-{name}-{}", std::process::id()));
        std::fs::write(&path, content).expect("file written");
        path
    }

    #[test]
    fn refuses_a_missing_command_in_one_line() {
        let output = xtask(&[]);
        assert_eq!(output.status.code(), Some(2_i32), "a usage error");
        assert_eq!(
            text(&output.stderr),
            "error: no command given (see: cargo xtask help)\n",
            "one line on stderr"
        );
        assert!(output.stdout.is_empty(), "nothing on stdout");
    }

    #[test]
    fn refuses_an_unknown_command_in_one_line() {
        let output = xtask(&["chek"]);
        assert_eq!(output.status.code(), Some(2_i32), "a usage error");
        assert_eq!(
            text(&output.stderr),
            "error: unknown command \"chek\" (see: cargo xtask help)\n",
            "one line on stderr, naming the command"
        );
        assert!(output.stdout.is_empty(), "nothing on stdout");
    }

    #[cfg(unix)]
    #[test]
    fn refuses_an_argument_that_is_not_text_without_panicking() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt as _;
        let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .arg(OsStr::from_bytes(b"\xff"))
            .output()
            .expect("the binary runs");
        assert_eq!(
            output.status.code(),
            Some(2_i32),
            "a usage error, not a panic"
        );
        assert_eq!(
            text(&output.stderr),
            "error: argument is not valid UTF-8: \u{fffd} (see: cargo xtask help)\n",
            "one line on stderr"
        );
    }

    #[test]
    fn prints_help_and_succeeds() {
        let output = xtask(&["help"]);
        assert!(output.status.success(), "help succeeds");
        assert!(text(&output.stdout).starts_with(USAGE), "usage on stdout");
        assert!(output.stderr.is_empty(), "nothing on stderr");
    }

    #[test]
    fn refuses_a_commit_message_in_one_line_with_the_form_on_stdout() {
        let path = file("message", "Update stuff\n");
        let output = xtask(&["commit-msg", path.to_str().expect("UTF-8 path")]);
        std::fs::remove_file(&path).expect("cleanup");
        assert_eq!(output.status.code(), Some(1_i32), "the check failed");
        assert_eq!(
            text(&output.stderr),
            "error: invalid commit message: missing \"<type>: \" prefix\n",
            "one line on stderr"
        );
        assert!(
            text(&output.stdout).starts_with("expected: <type>(<scope>)!: <summary>"),
            "the expected form on stdout"
        );
    }

    #[test]
    fn names_a_commit_message_file_it_cannot_read() {
        let output = xtask(&["commit-msg", "/no/such/message"]);
        assert_eq!(output.status.code(), Some(1_i32), "the check failed");
        let stderr = text(&output.stderr);
        assert!(
            stderr.starts_with("error: cannot read /no/such/message: ")
                && stderr.ends_with('\n')
                && stderr.lines().count() == 1,
            "one line naming the file: {stderr}"
        );
    }

    #[test]
    fn ignores_a_closed_stdout() {
        let (reader, writer) = std::io::pipe().expect("a pipe");
        drop(reader);
        let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .arg("list")
            .stdout(writer)
            .stderr(Stdio::piped())
            .output()
            .expect("the binary runs");
        assert_eq!(output.status.code(), Some(0_i32), "listing succeeds");
        assert!(output.stderr.is_empty(), "nothing on stderr");
    }

    /// A closed stdout stops no command xtask starts: a stand-in for `pnpm`, the first
    /// command `bless-benchmarks` runs after the preflight, writes lines on its stdout, then
    /// records that it finished and fails, so that nothing runs after it. Before the
    /// children's stdout went to xtask's stderr, the first line killed it (SIGPIPE).
    #[cfg(unix)]
    #[test]
    fn keeps_the_commands_it_runs_alive_on_a_closed_stdout() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("xtask-binary-pipe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch created");
        let record = dir.join("record.txt");
        let stub = dir.join("pnpm");
        std::fs::write(
            &stub,
            format!(
                "#!/bin/sh\nif [ \"$1\" = --version ]; then echo 10.0.0; exit 0; fi\ni=0\nwhile [ $i -lt 1000 ]; do echo \"line $i\"; i=$((i+1)); done\necho finished > '{}'\nexit 3\n",
                record.display()
            ),
        )
        .expect("stub written");
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
            .expect("stub executable");
        let path = format!(
            "{}:{}",
            dir.display(),
            std::env::var("PATH").expect("a PATH")
        );
        let (reader, writer) = std::io::pipe().expect("a pipe");
        drop(reader);
        let mut command = Command::new(env!("CARGO_BIN_EXE_xtask"));
        // The variables a tool running these tests sets (cargo-mutants' lint cap, the
        // coverage's instrumentation), which the preflight would refuse before the command.
        for (name, _) in std::env::vars_os() {
            if name.to_str().is_some_and(xtask::refused_variable) {
                let _without = command.env_remove(name);
            }
        }
        let output = command
            .arg("bless-benchmarks")
            .env("PATH", path)
            .stdout(writer)
            .stderr(Stdio::piped())
            .output()
            .expect("the binary runs");
        let finished = std::fs::read_to_string(&record);
        std::fs::remove_dir_all(&dir).expect("cleanup");
        let stderr = text(&output.stderr);
        assert_eq!(
            finished.ok().as_deref(),
            Some("finished\n"),
            "the command ran to its end: {stderr}"
        );
        assert!(
            stderr.contains("line 999\n"),
            "its output on xtask's stderr: {stderr}"
        );
        assert_eq!(
            stderr.lines().last(),
            Some("error: Build the engine module: `pnpm run build:wasm` exited with code 3"),
            "the failure the stand-in chose, not a signal"
        );
        assert_eq!(output.status.code(), Some(1_i32), "the work failed");
    }
}

/// The git hooks, run by real git: what reaches `cargo xtask` when a push or a commit
/// starts from a linked worktree, as the CI section of `CLAUDE.md` recommends.
#[cfg(test)]
#[cfg(unix)]
mod hooks {
    use std::fs;
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use xtask::LOCAL_GIT_VARIABLES;

    /// The variables of `received` (the stub's record, one `NAME=value` per line) that describe a
    /// repository: none may reach the check.
    fn local_variables(received: &str) -> Vec<&str> {
        received
            .lines()
            .filter_map(|line| line.split_once('=').map(|(name, _)| name))
            .filter(|name| LOCAL_GIT_VARIABLES.contains(name))
            .collect()
    }

    /// A fresh directory of this test, emptied first.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xtask-hooks-{name}-{}", std::process::id()));
        if dir.exists() {
            fs::remove_dir_all(&dir).expect("old scratch removed");
        }
        fs::create_dir_all(&dir).expect("scratch created");
        dir
    }

    /// Runs git in `dir`, hermetically: no variable of an enclosing git process leaks in.
    fn git(dir: &Path, args: &[&str], path: &str) {
        let mut command = Command::new("git");
        let _same = command
            .args(args)
            .current_dir(dir)
            .env("PATH", path)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HOME", dir);
        for variable in LOCAL_GIT_VARIABLES {
            let _without = command.env_remove(variable);
        }
        let status = command.status().expect("git runs");
        assert!(status.success(), "git {args:?} in {}", dir.display());
    }

    /// A stand-in for `cargo` that writes the git variables it received to `record`.
    fn stub_cargo(bin: &Path, record: &Path) {
        fs::create_dir_all(bin).expect("bin created");
        let script = bin.join("cargo");
        let body = format!(
            "#!/bin/sh\nenv | grep '^GIT_' | grep -v -e '^GIT_AUTHOR' -e '^GIT_COMMITTER' -e '^GIT_CONFIG_NOSYSTEM' > '{record}'\necho \"ARGS $*\" >> '{record}'\nexit 0\n",
            record = record.display()
        );
        fs::write(&script, body).expect("stub written");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("stub executable");
    }

    /// The repository's own hooks, committed in `repo` under `.githooks` and enabled through
    /// `core.hooksPath`, as in the project: a relative hooks path is resolved in each worktree.
    fn install_hooks(repo: &Path, path: &str) {
        let hooks = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.githooks");
        let installed = repo.join(".githooks");
        fs::create_dir_all(&installed).expect("hooks dir");
        for hook in ["pre-push", "commit-msg"] {
            let target = installed.join(hook);
            let _bytes = fs::copy(hooks.join(hook), &target).expect("hook copied");
            fs::set_permissions(&target, fs::Permissions::from_mode(0o755))
                .expect("hook executable");
        }
        git(repo, &["config", "core.hooksPath", ".githooks"], path);
    }

    #[test]
    fn a_push_from_a_linked_worktree_passes_no_git_variable_to_the_check() {
        let root = scratch("push");
        let record = root.join("record.txt");
        let bin = root.join("bin");
        stub_cargo(&bin, &record);
        let path = format!(
            "{}:/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin",
            bin.display()
        );
        let main = root.join("main");
        fs::create_dir_all(&main).expect("main created");
        git(&main, &["init", "-q", "-b", "main"], &path);
        install_hooks(&main, &path);
        fs::write(main.join("a.txt"), "a\n").expect("file written");
        git(&main, &["add", "a.txt", ".githooks"], &path);
        git(
            &main,
            &["commit", "-q", "--no-verify", "-m", "docs: start"],
            &path,
        );
        let remote = root.join("remote.git");
        git(
            &root,
            &["init", "-q", "--bare", remote.to_str().expect("utf-8")],
            &path,
        );
        let worktree = root.join("worktree");
        git(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                worktree.to_str().expect("utf-8"),
                "-b",
                "work",
            ],
            &path,
        );
        git(
            &worktree,
            &["push", "-q", remote.to_str().expect("utf-8"), "work"],
            &path,
        );
        let received = fs::read_to_string(&record).expect("the hook ran cargo");
        assert_eq!(local_variables(&received), Vec::<&str>::new(), "{received}");
        assert!(
            received
                .lines()
                .any(|line| line.starts_with("ARGS xtask pre-push ")
                    && line.ends_with(" refs/heads/work")
                    && line.split(' ').count() == 5),
            "each pushed commit with the remote ref it updates: {received}"
        );
        git(&worktree, &["tag", "v1.0.0"], &path);
        git(
            &worktree,
            &["push", "-q", remote.to_str().expect("utf-8"), "v1.0.0"],
            &path,
        );
        let tagged = fs::read_to_string(&record).expect("the hook ran cargo");
        assert!(
            tagged
                .lines()
                .any(|line| line.starts_with("ARGS xtask pre-push ")
                    && line.ends_with(" refs/tags/v1.0.0")),
            "a pushed tag is named: {tagged}"
        );
        fs::remove_dir_all(&root).expect("scratch removed");
    }

    #[test]
    fn a_commit_from_a_linked_worktree_passes_no_git_variable_to_the_message_check() {
        let root = scratch("commit");
        let record = root.join("record.txt");
        let bin = root.join("bin");
        stub_cargo(&bin, &record);
        let path = format!(
            "{}:/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin",
            bin.display()
        );
        let main = root.join("main");
        fs::create_dir_all(&main).expect("main created");
        git(&main, &["init", "-q", "-b", "main"], &path);
        install_hooks(&main, &path);
        fs::write(main.join("a.txt"), "a\n").expect("file written");
        git(&main, &["add", "a.txt", ".githooks"], &path);
        git(
            &main,
            &["commit", "-q", "--no-verify", "-m", "docs: start"],
            &path,
        );
        let worktree = root.join("worktree");
        git(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                worktree.to_str().expect("utf-8"),
                "-b",
                "work",
            ],
            &path,
        );
        fs::write(worktree.join("b.txt"), "b\n").expect("file written");
        git(&worktree, &["add", "b.txt"], &path);
        git(&worktree, &["commit", "-q", "-m", "docs: more"], &path);
        let received = fs::read_to_string(&record).expect("the hook ran cargo");
        assert_eq!(local_variables(&received), Vec::<&str>::new(), "{received}");
        fs::remove_dir_all(&root).expect("scratch removed");
    }
}
