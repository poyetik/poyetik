//! Runs the real `xtask` binary: the command-line contract of every tool of the repository
//! (exit codes 0, 1 and 2; one `error:` line on stderr; a closed stdout is not an error).

#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::process::{Command, Output, Stdio};

    /// The first line of the usage.
    const USAGE: &str =
        "usage: cargo xtask <check|list|bless|commit-msg <file>|pre-push [commit...]|help>\n";

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
}
