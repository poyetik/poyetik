//! Runs the real `visual-cli` binary on the repository's visual conformance corpus.

#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::Path;
    use std::process::{Command, Output, Stdio};

    /// A directory of one test below the system's temporary directory, removed with
    /// everything in it when the guard is dropped: when the test returns or unwinds, a failing
    /// test or a panicking mutant included. A test process killed (a mutant stopped at its
    /// timeout) or aborted runs no drop and leaves it; the next test of the same name in a
    /// process of the same id removes it when it starts.
    #[derive(Debug)]
    #[must_use = "the directory is removed when the guard is dropped"]
    struct Scratch(std::path::PathBuf);

    impl AsRef<Path> for Scratch {
        fn as_ref(&self) -> &Path {
            &self.0
        }
    }

    impl core::ops::Deref for Scratch {
        type Target = Path;
        fn deref(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            // Nothing better can be done in a drop: a directory a test removed itself is gone.
            drop(std::fs::remove_dir_all(&self.0));
        }
    }

    /// A fresh, empty directory for one test, `visual-cli-<name>-<process id>` ([`Scratch`]).
    #[expect(
        clippy::disallowed_methods,
        reason = "the one place visual-cli's binary tests name the temporary directory: the \
                  guard removes what it creates"
    )]
    fn scratch(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("visual-cli-{name}-{}", std::process::id()));
        if let Err(error) = std::fs::remove_dir_all(&dir) {
            assert_eq!(error.kind(), std::io::ErrorKind::NotFound, "clean start");
        }
        std::fs::create_dir_all(&dir).expect("scratch directory");
        Scratch(dir)
    }

    #[test]
    fn removes_a_scratch_directory_whatever_the_test_does() {
        let kept = scratch("guard");
        let dropped = kept.to_path_buf();
        std::fs::write(kept.join("a.txt"), "a").expect("written");
        drop(kept);
        assert!(!dropped.exists(), "removed with its files when dropped");
        let unwound = std::panic::catch_unwind(|| {
            let held = scratch("guard-unwound");
            let inside = held.to_path_buf();
            std::fs::write(held.join("a.txt"), "a").expect("written");
            std::panic::panic_any(inside);
        });
        let left = unwound
            .expect_err("the test panicked")
            .downcast::<std::path::PathBuf>()
            .expect("the path");
        assert!(!left.exists(), "removed when the test unwinds");
    }

    /// The repository's visual conformance corpus.
    fn corpus() -> String {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../conformance/visual")
            .display()
            .to_string()
    }

    /// Runs the binary with `args`.
    fn visual_cli(args: &[&OsStr]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_visual-cli"))
            .args(args)
            .output()
            .expect("the binary runs")
    }

    /// Checks the command-line contract for an error: exit `code`, nothing on stdout, and
    /// exactly the line `expected` on stderr.
    fn assert_error(output: &Output, code: i32, expected: &str) {
        assert_eq!(output.status.code(), Some(code), "exit code of {expected}");
        assert!(output.stdout.is_empty(), "nothing on stdout for {expected}");
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            format!("{expected}\n"),
            "one error line"
        );
    }

    #[test]
    fn checks_the_repository_corpus() {
        let output = visual_cli(&[OsStr::new("check"), OsStr::new(&corpus())]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let said = String::from_utf8_lossy(&output.stdout);
        assert!(
            said.lines().count() >= 4 && said.lines().all(|line| line.starts_with("ok       ")),
            "every case passes: {said}"
        );
        assert!(output.stderr.is_empty(), "nothing on stderr");
    }

    #[test]
    fn fails_with_exit_code_1_and_one_line_when_a_reference_differs() {
        let dir = scratch("wrong");
        std::fs::create_dir_all(dir.join("cases")).expect("cases directory");
        let case = Path::new(&corpus()).join("cases/square-320x180-at-60.json");
        let copied = std::fs::copy(case, dir.join("cases/square.json")).expect("copied");
        assert!(copied > 0, "the case is copied");
        std::fs::write(dir.join("references.json"), "{\n  \"square\": \"0\"\n}\n")
            .expect("references");
        let output = visual_cli(&[OsStr::new("check"), dir.as_os_str()]);
        assert_eq!(output.status.code(), Some(1_i32), "a failure");
        assert!(
            String::from_utf8_lossy(&output.stdout).starts_with("MISMATCH square: "),
            "the case named on stdout"
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            "error: 1 of the visual cases do not match their references, listed above\n",
            "one error line"
        );
    }

    #[test]
    fn blesses_then_checks_a_copy_of_the_corpus() {
        let dir = scratch("bless");
        std::fs::create_dir_all(dir.join("cases")).expect("cases directory");
        let case = Path::new(&corpus()).join("cases/square-one-column-at-1000.json");
        let copied = std::fs::copy(case, dir.join("cases/column.json")).expect("copied");
        assert!(copied > 0, "the case is copied");
        let output = visual_cli(&[OsStr::new("bless"), dir.as_os_str()]);
        assert!(output.status.success(), "blessed");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!(
                "wrote    1 references to {}\n",
                dir.join("references.json").display()
            ),
            "says what it wrote"
        );
        assert!(
            visual_cli(&[OsStr::new("check"), dir.as_os_str()])
                .status
                .success(),
            "then checked"
        );
    }

    #[test]
    fn prints_the_hash_of_one_case() {
        let case = Path::new(&corpus()).join("cases/square-320x180-at-60.json");
        let output = visual_cli(&[OsStr::new("render"), case.as_os_str()]);
        assert!(output.status.success(), "rendered");
        let said = String::from_utf8_lossy(&output.stdout);
        let references =
            std::fs::read_to_string(Path::new(&corpus()).join("references.json")).expect("read");
        assert!(
            said.ends_with('\n') && references.contains(&format!("\"{}\"", said.trim_end())),
            "the reference hash of the case: {said}"
        );
    }

    #[test]
    fn refuses_bad_command_lines_with_one_error_line_and_exit_code_2() {
        for (args, reason) in [
            (&[][..], "no command"),
            (&["chek"][..], "unknown command \"chek\""),
            (&["check"][..], "check expects <dir>"),
        ] {
            let os_args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
            assert_error(
                &visual_cli(&os_args),
                2,
                &format!("error: {reason} (see: visual-cli help)"),
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            assert_error(
                &visual_cli(&[OsStr::new("render"), OsStr::from_bytes(b"case\xff.json")]),
                2,
                "error: argument 2 is not valid UTF-8 (see: visual-cli help)",
            );
        }
    }

    #[test]
    fn fails_with_exit_code_1_naming_the_file() {
        assert_error(
            &visual_cli(&[OsStr::new("render"), OsStr::new("missing.json")]),
            1,
            "error: cannot read missing.json: No such file or directory (os error 2)",
        );
    }

    #[cfg(unix)]
    #[test]
    fn refuses_an_endless_case_file_instead_of_reading_it_whole() {
        assert_error(
            &visual_cli(&[OsStr::new("render"), OsStr::new("/dev/zero")]),
            1,
            "error: cannot read /dev/zero: larger than 65536 bytes",
        );
    }

    #[test]
    fn prints_the_usage_on_stdout_for_help() {
        let output = visual_cli(&[OsStr::new("help")]);
        assert!(output.status.success(), "help succeeds");
        assert!(
            String::from_utf8_lossy(&output.stdout).starts_with("usage: visual-cli <command>\n"),
            "the usage, on stdout"
        );
        assert!(output.stderr.is_empty(), "nothing on stderr");
    }

    #[test]
    fn succeeds_when_nobody_reads_its_output() {
        // The read end is closed before the binary writes: every write gets a broken pipe.
        let (reader, writer) = std::io::pipe().expect("a pipe");
        drop(reader);
        let output = Command::new(env!("CARGO_BIN_EXE_visual-cli"))
            .args(["check", &corpus()])
            .stdout(writer)
            .stderr(Stdio::piped())
            .output()
            .expect("the binary runs");
        assert_eq!(
            output.status.code(),
            Some(0_i32),
            "the check passed: exit code 0"
        );
        assert!(output.stderr.is_empty(), "no error line");
    }
}
