//! Runs the real `engine-cli` binary on the repository's conformance corpus.

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    fn conformance() -> String {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../conformance")
            .display()
            .to_string()
    }

    #[test]
    fn checks_the_repository_corpus() {
        let output = Command::new(env!("CARGO_BIN_EXE_engine-cli"))
            .args(["check", &conformance()])
            .output()
            .expect("the binary runs");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("ok       sine-440 "),
            "sine-440 passes"
        );
    }

    #[test]
    fn fails_on_a_wrong_reference_hash() {
        let dir = std::env::temp_dir().join(format!("engine-cli-wrong-{}", std::process::id()));
        for kind in ["cases", "patches"] {
            std::fs::create_dir_all(dir.join(kind)).expect("temporary directory");
            let file = Path::new(&conformance()).join(kind).join("sine-440.json");
            let copied =
                std::fs::copy(file, dir.join(kind).join("sine-440.json")).expect("copy the file");
            assert!(copied > 0, "the {kind} file is copied");
        }
        std::fs::write(dir.join("references.json"), r#"{"sine-440":"0"}"#).expect("references");
        let output = Command::new(env!("CARGO_BIN_EXE_engine-cli"))
            .args(["check", &dir.display().to_string()])
            .output()
            .expect("the binary runs");
        std::fs::remove_dir_all(&dir).expect("cleanup");
        assert_eq!(
            output.status.code(),
            Some(1_i32),
            "a mismatch fails with exit code 1"
        );
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .starts_with("MISMATCH sine-440: expected 0, got "),
            "explained"
        );
        assert_eq!(
            output.stderr, b"error: conformance failed: 1 problem(s)\n",
            "summary on stderr"
        );
    }

    #[test]
    fn exports_the_repository_corpus_as_wav_files() {
        let dir = std::env::temp_dir().join(format!("engine-cli-export-{}", std::process::id()));
        let output = Command::new(env!("CARGO_BIN_EXE_engine-cli"))
            .args([
                "export",
                &conformance(),
                &dir.join("wav").display().to_string(),
            ])
            .output()
            .expect("the binary runs");
        let wav = std::fs::read(dir.join("wav/sine-440.wav")).expect("written");
        std::fs::remove_dir_all(&dir).expect("cleanup");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        // 58-byte header, then 48000 frames of 2 channels of 4-byte floats.
        assert_eq!(wav.len(), 58 + 48_000 * 2 * 4, "size");
        assert!(wav.starts_with(b"RIFF"), "a WAV file");
    }

    /// Runs the binary with `args`.
    fn engine_cli(args: &[&std::ffi::OsStr]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_engine-cli"))
            .args(args)
            .output()
            .expect("the binary runs")
    }

    /// Checks the command-line contract for an error: exit `code`, nothing on stdout, and
    /// exactly the line `expected` on stderr.
    fn assert_error(output: &std::process::Output, code: i32, expected: &str) {
        assert_eq!(output.status.code(), Some(code), "exit code of {expected}");
        assert!(output.stdout.is_empty(), "nothing on stdout for {expected}");
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            format!("{expected}\n"),
            "one error line"
        );
    }

    #[test]
    fn refuses_bad_command_lines_with_one_error_line_and_exit_code_2() {
        for (args, reason) in [
            (&[][..], "missing command"),
            (&["chek"][..], "unknown command \"chek\""),
            (
                &["render", "a.json", "b.json", "--wvv", "x"][..],
                "unknown option \"--wvv\"",
            ),
            (&["check"][..], "check expects <dir>"),
            (&["check", "a", "b"][..], "check expects <dir>"),
        ] {
            let os_args: Vec<&std::ffi::OsStr> = args.iter().map(std::ffi::OsStr::new).collect();
            assert_error(
                &engine_cli(&os_args),
                2,
                &format!("error: {reason} (see: engine-cli help)"),
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn refuses_an_argument_that_is_not_utf8_with_exit_code_2() {
        use std::os::unix::ffi::OsStrExt;
        let output = engine_cli(&[
            std::ffi::OsStr::new("render"),
            std::ffi::OsStr::from_bytes(b"case\xff.json"),
            std::ffi::OsStr::new("patch.json"),
        ]);
        assert_error(
            &output,
            2,
            "error: argument is not valid UTF-8: case\u{fffd}.json (see: engine-cli help)",
        );
    }

    #[test]
    fn prints_the_usage_on_stdout_for_help() {
        let output = engine_cli(&[std::ffi::OsStr::new("help")]);
        assert_eq!(output.status.code(), Some(0_i32), "success");
        assert!(output.stderr.is_empty(), "nothing on stderr");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("{}\n", engine_cli::USAGE),
            "the usage"
        );
    }

    #[test]
    fn names_the_file_at_fault_in_one_error_line_with_exit_code_1() {
        let dir = std::env::temp_dir().join(format!("engine-cli-fault-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temporary directory");
        let case = Path::new(&conformance()).join("cases/sine-440.json");
        let patch = dir.join("patch.json");
        std::fs::write(
            &patch,
            r#"{"format":1,"nodes":[{"id":1,"box":"foo~"}],"connections":[]}"#,
        )
        .expect("patch");
        let missing = dir.join("missing.json");
        let unknown_box = engine_cli(&[
            std::ffi::OsStr::new("render"),
            case.as_os_str(),
            patch.as_os_str(),
        ]);
        let unreadable = engine_cli(&[
            std::ffi::OsStr::new("render"),
            missing.as_os_str(),
            patch.as_os_str(),
        ]);
        std::fs::remove_dir_all(&dir).expect("cleanup");
        assert_error(
            &unknown_box,
            1,
            &format!(
                "error: {}: node 1: unknown box type \"foo~\"",
                patch.display()
            ),
        );
        let stderr = String::from_utf8_lossy(&unreadable.stderr);
        assert!(
            stderr.starts_with(&format!("error: cannot read {}: ", missing.display())),
            "{stderr}"
        );
        assert_eq!(stderr.lines().count(), 1, "one line: {stderr}");
        assert_eq!(unreadable.status.code(), Some(1_i32), "a failure");
    }

    #[test]
    fn succeeds_when_nobody_reads_its_output() {
        // The read end is closed before the binary writes: every write gets a broken pipe.
        let (reader, writer) = std::io::pipe().expect("a pipe");
        drop(reader);
        let output = Command::new(env!("CARGO_BIN_EXE_engine-cli"))
            .args(["check", &conformance()])
            .stdout(writer)
            .stderr(std::process::Stdio::piped())
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
