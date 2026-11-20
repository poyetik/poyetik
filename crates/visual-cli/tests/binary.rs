//! Runs the real `visual-cli` binary on the repository's visual conformance corpus.

#![forbid(unsafe_code)]

extern crate alloc;

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
        // Every case but the largest image, 2^25 pixels: unoptimized, as tests build, it costs
        // most of the corpus's time (about 15 s of 18); `cargo xtask check` checks the whole
        // corpus with an optimized build ("Visual conformance (native)").
        let largest = "square-largest-image-at-50";
        let dir = scratch("corpus");
        std::fs::create_dir_all(dir.join("cases")).expect("cases directory");
        for entry in std::fs::read_dir(Path::new(&corpus()).join("cases")).expect("the cases") {
            let path = entry.expect("an entry").path();
            // Only case files: a file manager's `.DS_Store`, which git ignores, is none.
            if path.extension() == Some(OsStr::new("json"))
                && path.file_stem() != Some(OsStr::new(largest))
            {
                let name = path.file_name().expect("a file name");
                let copied = std::fs::copy(&path, dir.join("cases").join(name)).expect("copied");
                assert!(copied > 0, "{} copied", path.display());
            }
        }
        let references = std::fs::read_to_string(Path::new(&corpus()).join("references.json"))
            .expect("the references");
        let (removed, kept): (Vec<&str>, Vec<&str>) = references
            .lines()
            .partition(|line| line.contains(&format!("\"{largest}\"")));
        let mut kept = kept.join("\n");
        kept.push('\n');
        assert!(
            matches!(removed.as_slice(), [line] if line.ends_with(',')),
            "one reference left out, the largest image's, not the last line (a comma after \
             it): {removed:?}"
        );
        std::fs::write(dir.join("references.json"), kept).expect("written");
        let display = std::fs::copy(
            Path::new(&corpus()).join("display.json"),
            dir.join("display.json"),
        )
        .expect("the display corpus copied");
        assert!(display > 0, "the display corpus copied");
        let output = visual_cli(&[OsStr::new("check"), dir.as_os_str()]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let said = String::from_utf8_lossy(&output.stdout);
        let mut cases: Vec<String> = std::fs::read_dir(Path::new(&corpus()).join("cases"))
            .expect("the cases")
            .filter_map(|entry| {
                let name = entry.expect("an entry").file_name();
                name.to_string_lossy()
                    .strip_suffix(".json")
                    .map(str::to_owned)
            })
            .collect();
        cases.sort();
        let reference_names: Vec<String> =
            serde_json::from_str::<alloc::collections::BTreeMap<String, String>>(&references)
                .expect("the references")
                .into_keys()
                .collect();
        assert_eq!(
            cases, reference_names,
            "every case of the corpus has its reference"
        );
        cases.retain(|name| name != largest);
        // Then the display corpus, the repository's file the native encoding gives.
        cases.push("display.json".to_owned());
        let checked: Vec<&str> = said
            .lines()
            .map(|line| {
                line.strip_prefix("ok       ")
                    .and_then(|rest| rest.split_once(' '))
                    .map_or("", |(name, _)| name)
            })
            .collect();
        assert_eq!(
            checked, cases,
            "every case passes, each once, in order: {said}"
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
        let display = std::fs::copy(
            Path::new(&corpus()).join("display.json"),
            dir.join("display.json"),
        )
        .expect("the display corpus copied");
        assert!(display > 0, "the display corpus copied");
        let output = visual_cli(&[OsStr::new("check"), dir.as_os_str()]);
        assert_eq!(output.status.code(), Some(1_i32), "a failure");
        let rendered = visual_cli(&[
            OsStr::new("render"),
            dir.join("cases/square.json").as_os_str(),
        ]);
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).lines().next(),
            Some(
                format!(
                    "MISMATCH square: {}, reference 0",
                    String::from_utf8_lossy(&rendered.stdout).trim_end()
                )
                .as_str()
            ),
            "the case named on stdout, with both hashes"
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            format!(
                "error: 1 problem(s), listed above; when the renders are meant to change, \
                 `cargo run -p visual-cli -- bless {}` rewrites the references and display.json\n",
                dir.display()
            ),
            "one error line, and how to accept the change if it is meant"
        );
    }

    /// What opens a runnable hint: written in two pieces, so that the hints gate reads no
    /// hint here.
    const HINT_OPENING: &str = concat!("(see", ": ");

    /// The command of the first runnable hint in `text` ([`HINT_OPENING`] to its closing
    /// parenthesis), split into words: it names no quoted path in these tests.
    fn hint_words(text: &str) -> Vec<String> {
        let (_, after) = text.split_once(HINT_OPENING).expect("a runnable hint");
        let (command, _) = after.split_once(')').expect("the hint's end");
        command.split(' ').map(str::to_owned).collect()
    }

    /// Runs the hint's `words` in `root`, as a user at the repository's root types it: the
    /// git commands it names, with no git setting of the process running the tests (a hook's
    /// `GIT_DIR`) leaking in.
    fn run_hint(root: &Path, words: &[String]) -> Output {
        let (program, rest) = words.split_first().expect("a command");
        assert_eq!(program, "git", "a git command: {words:?}");
        Command::new(program)
            .args(rest)
            .current_dir(root)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .expect("git runs")
    }

    #[test]
    fn restores_a_corpus_file_by_the_hint_it_gives() {
        // A repository holding a blessed corpus, each file then changed: the check names the
        // git command that restores it, from the repository's root, and once it is run the
        // check passes again.
        let root = scratch("restore");
        let dir = root.join("corpus");
        std::fs::create_dir_all(dir.join("cases")).expect("cases directory");
        let case = Path::new(&corpus()).join("cases/square-one-column-at-1000.json");
        let copied = std::fs::copy(case, dir.join("cases/column.json")).expect("copied");
        assert!(copied > 0, "the case is copied");
        let in_root = |args: &[&str]| {
            Command::new(env!("CARGO_BIN_EXE_visual-cli"))
                .args(args)
                .current_dir(&root)
                .output()
                .expect("the binary runs")
        };
        assert!(in_root(&["bless", "corpus"]).status.success(), "blessed");
        for words in [&["init", "--quiet"][..], &["add", "corpus"][..]] {
            let words: Vec<String> = core::iter::once("git")
                .chain(words.iter().copied())
                .map(str::to_owned)
                .collect();
            let done = run_hint(&root, &words);
            assert!(done.status.success(), "{words:?}: {done:?}");
        }
        std::fs::write(dir.join("references.json"), "{}").expect("changed");
        let refused = in_root(&["check", "corpus"]);
        assert_eq!(refused.status.code(), Some(1_i32), "references refused");
        let references = hint_words(&String::from_utf8_lossy(&refused.stderr));
        assert_eq!(
            references,
            ["git", "checkout", "--", "corpus/references.json"]
        );
        assert!(run_hint(&root, &references).status.success(), "restored");
        let display = std::fs::read_to_string(dir.join("display.json")).expect("the corpus");
        std::fs::write(
            dir.join("display.json"),
            display.replacen("ff\"]", "fe\"]", 1),
        )
        .expect("changed");
        let mismatch = in_root(&["check", "corpus"]);
        assert_eq!(
            mismatch.status.code(),
            Some(1_i32),
            "display corpus refused"
        );
        let said = String::from_utf8_lossy(&mismatch.stdout);
        let line = said
            .lines()
            .find(|line| line.starts_with("MISMATCH display.json"))
            .expect("the display corpus's line");
        let restore = hint_words(line);
        assert_eq!(restore, ["git", "checkout", "--", "corpus/display.json"]);
        assert_eq!(
            hint_words(&String::from_utf8_lossy(&mismatch.stderr)),
            restore,
            "the summary names the same command"
        );
        assert!(run_hint(&root, &restore).status.success(), "restored");
        let checked = in_root(&["check", "corpus"]);
        assert_eq!(
            (
                checked.status.code(),
                String::from_utf8_lossy(&checked.stderr)
            ),
            (Some(0_i32), "".into()),
            "the hints followed, the check passes"
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
        assert_eq!(output.status.code(), Some(0_i32), "blessed: {output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!(
                "wrote    1 reference(s) to {}\nwrote    {} pixel(s) to {}\n",
                dir.join("references.json").display(),
                visual_cli::display_corpus().len(),
                dir.join("display.json").display()
            ),
            "says what it wrote"
        );
        let checked = visual_cli(&[OsStr::new("check"), dir.as_os_str()]);
        assert_eq!(
            (
                checked.status.code(),
                String::from_utf8_lossy(&checked.stderr)
            ),
            (Some(0_i32), "".into()),
            "then checked"
        );
    }

    #[test]
    fn prints_the_hash_of_one_case() {
        let case = Path::new(&corpus()).join("cases/square-320x180-at-60.json");
        let output = visual_cli(&[OsStr::new("render"), case.as_os_str()]);
        assert_eq!(output.status.code(), Some(0_i32), "rendered: {output:?}");
        let said = String::from_utf8_lossy(&output.stdout);
        let references =
            std::fs::read_to_string(Path::new(&corpus()).join("references.json")).expect("read");
        let reference = references
            .lines()
            .find_map(|line| line.trim().strip_prefix("\"square-320x180-at-60\": \""))
            .and_then(|rest| rest.split_once('"'))
            .map(|(hash, _)| format!("{hash}\n"));
        assert_eq!(
            Some(said.into_owned()),
            reference,
            "the reference hash of that case, on one line"
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
                &format!("error: {reason} (see: cargo run -p visual-cli -- help)"),
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            assert_error(
                &visual_cli(&[OsStr::new("render"), OsStr::from_bytes(b"case\xff.json")]),
                2,
                "error: argument 2 is not valid UTF-8 (see: cargo run -p visual-cli -- help)",
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
        // Two small cases, so that the check writes several lines: the whole corpus would
        // only cost time (`checks_the_repository_corpus` renders it).
        let dir = scratch("unread");
        std::fs::create_dir_all(dir.join("cases")).expect("cases directory");
        for name in ["square-one-column-at-1000", "square-320x180-at-60"] {
            let case = Path::new(&corpus()).join(format!("cases/{name}.json"));
            let copied =
                std::fs::copy(case, dir.join(format!("cases/{name}.json"))).expect("copied");
            assert!(copied > 0, "the case is copied");
        }
        assert!(
            visual_cli(&[OsStr::new("bless"), dir.as_os_str()])
                .status
                .success(),
            "blessed"
        );
        // The read end is closed before the binary writes: every write gets a broken pipe.
        let (reader, writer) = std::io::pipe().expect("a pipe");
        drop(reader);
        let output = Command::new(env!("CARGO_BIN_EXE_visual-cli"))
            .arg("check")
            .arg(dir.as_os_str())
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
