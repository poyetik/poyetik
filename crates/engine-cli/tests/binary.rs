//! Runs the real `engine-cli` binary on the repository's conformance corpus.

#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

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

    /// A fresh, empty directory for one test, `engine-cli-<name>-<process id>` ([`Scratch`]).
    #[expect(
        clippy::disallowed_methods,
        reason = "the one place engine-cli's binary tests name the temporary directory: the guard removes what it \
                  creates"
    )]
    fn scratch(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("engine-cli-{name}-{}", std::process::id()));
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

    /// The repository's conformance corpus.
    fn conformance() -> String {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../conformance")
            .display()
            .to_string()
    }

    /// `json`, a JSON document, as bless writes it.
    fn canonical(json: &str) -> String {
        format!(
            "{:#}\n",
            serde_json::from_str::<serde_json::Value>(json).expect("JSON")
        )
    }

    /// The repository's record of what its references were blessed under, with `cases`
    /// (the versions of each case's box types) in place of its own.
    fn record(cases: &str) -> String {
        let mut recorded: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(Path::new(&conformance()).join("algorithms.json"))
                .expect("the repository's record"),
        )
        .expect("JSON");
        recorded["cases"] = serde_json::from_str(cases).expect("JSON");
        format!("{recorded:#}\n")
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
        let dir = scratch("wrong");
        for kind in ["cases", "patches"] {
            std::fs::create_dir_all(dir.join(kind)).expect("temporary directory");
            let file = Path::new(&conformance()).join(kind).join("sine-440.json");
            let copied =
                std::fs::copy(file, dir.join(kind).join("sine-440.json")).expect("copy the file");
            assert!(copied > 0, "the {kind} file is copied");
        }
        std::fs::write(
            dir.join("references.json"),
            canonical(r#"{"sine-440":{"binary64":"0"}}"#),
        )
        .expect("references");
        std::fs::write(
            dir.join("algorithms.json"),
            record(r#"{"sine-440":{"dac~":1,"osc~":1}}"#),
        )
        .expect("algorithm versions");
        let output = Command::new(env!("CARGO_BIN_EXE_engine-cli"))
            .args(["check", &dir.display().to_string()])
            .output()
            .expect("the binary runs");
        assert_eq!(
            output.status.code(),
            Some(1_i32),
            "a mismatch fails with exit code 1"
        );
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .starts_with("MISMATCH sine-440 binary64: expected 0, got "),
            "explained"
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            format!(
                "error: conformance failed in {}: 3 problem(s)\n",
                dir.display()
            ),
            "summary on stderr, naming the directory: a mismatch, two references missing"
        );
    }

    #[test]
    fn exports_the_repository_corpus_as_wav_files() {
        let dir = scratch("export");
        let output = Command::new(env!("CARGO_BIN_EXE_engine-cli"))
            .args([
                "export",
                &conformance(),
                &dir.join("wav").display().to_string(),
            ])
            .output()
            .expect("the binary runs");
        let read = |format: &str| {
            std::fs::read(dir.join(format!("wav/sine-440.{format}.wav"))).expect("written")
        };
        let (wide, given, narrow) = (
            read("binary64"),
            read("binary64-as-binary32"),
            read("binary32"),
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        // 58-byte header, then 48000 frames of 2 channels of 8- or 4-byte floats.
        assert_eq!(wide.len(), 58 + 48_000 * 2 * 8, "binary64 size");
        for wav in [&given, &narrow] {
            assert_eq!(wav.len(), 58 + 48_000 * 2 * 4, "binary32 size");
        }
        for wav in [&wide, &given, &narrow] {
            assert!(wav.starts_with(b"RIFF"), "a WAV file");
        }
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
        let dir = scratch("fault");
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
        assert_error(
            &unknown_box,
            1,
            &format!(
                "error: {}: box 1: unknown box type \"foo~\"",
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
    fn escapes_untrusted_text_into_one_error_line() {
        let dir = scratch("escape");
        std::fs::create_dir_all(&dir).expect("temporary directory");
        let case = Path::new(&conformance()).join("cases/sine-440.json");
        let (hostile_box, hostile_field) = (dir.join("box.json"), dir.join("field.json"));
        std::fs::write(
            &hostile_box,
            r#"{"format":1,"nodes":[{"id":1,"box":"x\u001b[31m\nsecond"}],"connections":[]}"#,
        )
        .expect("patch");
        std::fs::write(
            &hostile_field,
            r#"{"format":1,"nodes":[],"connections":[],"a\u001b\n":1}"#,
        )
        .expect("patch");
        let outputs: Vec<std::process::Output> = [&hostile_box, &hostile_field]
            .iter()
            .map(|patch| {
                engine_cli(&[
                    std::ffi::OsStr::new("render"),
                    case.as_os_str(),
                    patch.as_os_str(),
                ])
            })
            .collect();
        for output in outputs {
            let stderr = &output.stderr;
            assert_eq!(output.status.code(), Some(1_i32), "a failure");
            assert!(stderr.starts_with(b"error: "), "an error line");
            assert_eq!(
                stderr.iter().position(|&byte| byte < 0x20 || byte == 0x7f),
                Some(stderr.len() - 1),
                "no control byte but the final newline: {}",
                String::from_utf8_lossy(stderr)
            );
        }
    }

    /// Whether `character` could break a line or drive a terminal: a control,
    /// bidirectional formatting or line-separator character.
    fn unsafe_character(character: char) -> bool {
        character.is_control()
            || matches!(
                character,
                '\u{61c}'
                    | '\u{200e}'
                    | '\u{200f}'
                    | '\u{202a}'..='\u{202e}'
                    | '\u{2066}'..='\u{2069}'
                    | '\u{2028}'
                    | '\u{2029}'
            )
    }

    #[test]
    fn checks_a_hostile_corpus_with_one_safe_line_per_problem() {
        let dir = scratch("hostile");
        for kind in ["cases", "patches"] {
            std::fs::create_dir_all(dir.join(kind)).expect("temporary directory");
        }
        let case = std::fs::read_to_string(Path::new(&conformance()).join("cases/sine-440.json"))
            .expect("case");
        let patch =
            std::fs::read_to_string(Path::new(&conformance()).join("patches/sine-440.json"))
                .expect("patch");
        let huge_box = format!(
            r#"{{"format":1,"nodes":[{{"id":1,"box":"{}"}}],"connections":[]}}"#,
            "x".repeat(2_000_000)
        );
        let forged_box = r#"{"format":1,"nodes":[{"id":1,"box":"y\nok       sine-440 0\u001b[2K\u202e"}],"connections":[]}"#;
        for (name, text) in [
            ("a\nok       a 0", forged_box),
            ("b\u{1b}[2J", patch.as_str()),
            ("c\u{202e}", huge_box.as_str()),
        ] {
            std::fs::write(dir.join("cases").join(format!("{name}.json")), &case).expect("case");
            std::fs::write(dir.join("patches").join(format!("{name}.json")), text).expect("patch");
        }
        std::fs::write(
            dir.join("references.json"),
            canonical(
                "{\"b\\u001b[2J\":{\"binary64\":\"0\\nok       forged\\u001b[31m\\u202e\"},\
                 \"gone\\nok       gone\\u0007\":{}}",
            ),
        )
        .expect("references");
        std::fs::write(
            dir.join("algorithms.json"),
            record(r#"{"b\u001b[2J":{"dac~":1,"osc~":1}}"#),
        )
        .expect("algorithm versions");
        let output = engine_cli(&[std::ffi::OsStr::new("check"), dir.as_os_str()]);
        assert_eq!(output.status.code(), Some(1_i32), "a failure");
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            format!(
                "error: conformance failed in {}: 10 problem(s)\n",
                dir.display()
            ),
            "ten problems: two cases failing in three formats each, a mismatch, two missing references, a stale reference"
        );
        let stdout = String::from_utf8(output.stdout).expect("UTF-8 output");
        let lines: Vec<&str> = stdout
            .strip_suffix('\n')
            .expect("the last line ends")
            .split('\n')
            .collect();
        assert_eq!(lines.len(), 10, "one line per problem: {stdout:?}");
        for line in lines {
            assert!(
                !line.chars().any(unsafe_character),
                "no control or bidirectional character: {line:?}"
            );
            assert!(
                line.chars().count() <= engine_patch::MAX_MESSAGE_CHARS,
                "bounded: {} characters",
                line.chars().count()
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn refuses_an_endless_case_file_instead_of_reading_it_whole() {
        let patch = Path::new(&conformance()).join("patches/sine-440.json");
        let output = engine_cli(&[
            std::ffi::OsStr::new("render"),
            std::ffi::OsStr::new("/dev/zero"),
            patch.as_os_str(),
        ]);
        assert_error(
            &output,
            1,
            "error: cannot read /dev/zero: larger than 65536 bytes",
        );
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
