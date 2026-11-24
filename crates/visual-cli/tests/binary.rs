//! Runs the real `visual-cli` binary on the repository's visual conformance corpus.

#![forbid(unsafe_code)]

extern crate alloc;

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::Path;
    use std::process::{Command, Output, Stdio};

    use visual_testkit::matrix::{HINT_DIRECTORIES, HINT_REPOSITORY, HINT_STAGED};

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
                 bless rewrites the references and display.json (see: cargo run -p visual-cli -- bless {})\n",
                dir.display()
            ),
            "one error line, and how to accept the change if it is meant"
        );
    }

    /// What opens a runnable hint: written in two pieces, so that the hints gate reads no
    /// hint here.
    const HINT_OPENING: &str = concat!("(see", ": ");

    /// The command of every runnable hint in `text`, [`HINT_OPENING`] to its closing
    /// parenthesis: the paths it names hold none.
    fn hint_commands(text: &str) -> Vec<String> {
        text.split(HINT_OPENING)
            .skip(1)
            .map(|after| {
                let (command, _) = after.split_once(')').expect("the hint's end");
                command.to_owned()
            })
            .collect()
    }

    /// The command of the first runnable hint in `text`.
    fn hint_command(text: &str) -> String {
        hint_commands(text)
            .into_iter()
            .next()
            .expect("a runnable hint")
    }

    /// The variables under which a test runs git without the machine's or the user's
    /// configuration (a `core.autocrlf` of the user's would write the restored file with
    /// other line ends): a copy of xtask's `HERMETIC_GIT`, which its hermetic git gate holds
    /// this file to. `GIT_CONFIG_GLOBAL` needs git 2.32 or later.
    const HERMETIC_GIT: [(&str, &str); 4] = [
        ("GIT_CONFIG_NOSYSTEM", "1"),
        ("GIT_CONFIG_GLOBAL", "/dev/null"),
        ("GIT_ATTR_NOSYSTEM", "1"),
        ("XDG_CONFIG_HOME", "/dev/null"),
    ];

    /// The variables git sets for a hook that name one repository, removed so that the
    /// process running the tests (a hook's `GIT_DIR`) leaks none in: a copy of xtask's
    /// `LOCAL_GIT_VARIABLES`, which its hermetic git gate holds this file to.
    const LOCAL_GIT_VARIABLES: [&str; 14] = [
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_CONFIG",
        "GIT_CONFIG_PARAMETERS",
        "GIT_CONFIG_COUNT",
        "GIT_OBJECT_DIRECTORY",
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_IMPLICIT_WORK_TREE",
        "GIT_GRAFT_FILE",
        "GIT_INDEX_FILE",
        "GIT_REPLACE_REF_BASE",
        "GIT_PREFIX",
        "GIT_SHALLOW_FILE",
        "GIT_COMMON_DIR",
    ];

    /// Runs `command` in `dir` through a POSIX shell, as a user types it, git reading no
    /// configuration of the machine's or the user's ([`HERMETIC_GIT`]) and no setting of the
    /// process running the tests ([`LOCAL_GIT_VARIABLES`]).
    fn shell(dir: &Path, command: &str) -> Output {
        let mut sh = Command::new("sh");
        let _hermetic = sh.args(["-c", command]).current_dir(dir).envs(HERMETIC_GIT);
        for variable in LOCAL_GIT_VARIABLES {
            let _without = sh.env_remove(variable);
        }
        sh.output().expect("sh runs")
    }

    /// Runs the command of a hint in `dir` as a user types it ([`shell`]), `cargo run -p
    /// visual-cli --` being the binary tested: no scratch repository holds the workspace.
    fn follow(dir: &Path, command: &str) -> Output {
        shell(
            dir,
            &format!(
                "cargo() {{ [ \"$1 $2 $3 $4\" = 'run -p visual-cli --' ] && shift 4 && '{}' \"$@\"; }}; {command}",
                env!("CARGO_BIN_EXE_visual-cli")
            ),
        )
    }

    /// A repository ([`HINT_REPOSITORY`], `name` keeping tests run at once apart) holding
    /// `corpus/`, one case blessed, committed.
    fn committed_corpus(name: &str) -> Scratch {
        let root = scratch(&format!("{HINT_REPOSITORY} {name}"));
        let dir = root.join("corpus");
        std::fs::create_dir_all(dir.join("cases")).expect("cases directory");
        let case = Path::new(&corpus()).join("cases/square-one-column-at-1000.json");
        let copied = std::fs::copy(case, dir.join("cases/column.json")).expect("copied");
        assert!(copied > 0, "the case is copied");
        let blessed = follow(&root, "cargo run -p visual-cli -- bless corpus");
        assert!(blessed.status.success(), "{blessed:?}");
        let git = "git -c user.name=t -c user.email=t@t";
        let made = shell(
            &root,
            &format!("{git} init --quiet && {git} add corpus && {git} commit --quiet -m held"),
        );
        assert!(made.status.success(), "{made:?}");
        root
    }

    /// What `check` says of the corpus of `root`, its stdout then its stderr, and whether it
    /// passes.
    fn checked(root: &Path) -> (String, bool) {
        let output = follow(root, "cargo run -p visual-cli -- check corpus");
        let said = [output.stdout, output.stderr].concat();
        (
            String::from_utf8(said).expect("UTF-8"),
            output.status.success(),
        )
    }

    #[test]
    fn loses_no_untracked_file_by_a_hint() {
        // Findings H7-8 of the fifth deep review of visual block 2 (#34) and K13-3 of the
        // sixth: a directory in a file's place was told to be made readable, then to be
        // removed, with a restore which deletes the directory and what it holds. Every hint of
        // the output, followed, leaves the directory's untracked file; the directory moved
        // away, the hints lead to a passing check.
        let root = committed_corpus("untracked");
        for file in ["corpus/references.json", "corpus/display.json"] {
            let place = root.join(file);
            std::fs::remove_file(&place).expect("removed");
            std::fs::create_dir(&place).expect("a directory in its place");
            std::fs::write(place.join("untracked"), "kept").expect("written");
            let (said, passed) = checked(&root);
            assert!(!passed, "{said}");
            let hints = hint_commands(&said);
            assert!(!hints.is_empty(), "{said}");
            for hint in hints {
                let followed = follow(&root, &hint);
                assert!(
                    std::fs::read(place.join("untracked")).is_ok_and(|held| held == b"kept"),
                    "{hint}: {followed:?}"
                );
            }
            std::fs::rename(&place, root.join("moved away")).expect("moved away");
            let (missing, _) = checked(&root);
            let restored = follow(&root, &hint_command(&missing));
            assert!(restored.status.success(), "{missing}: {restored:?}");
            let (again, restored_passes) = checked(&root);
            assert!(restored_passes, "{file}: {again}");
            std::fs::remove_dir_all(root.join("moved away")).expect("removed");
        }
    }

    #[test]
    fn restores_a_corpus_file_by_the_hint_it_gives() {
        // A repository holding a blessed corpus, each file then damaged, staged or not: the check
        // names the git command that restores it, which, run through a shell from every
        // directory of the shared test matrix, restores it from the last commit, and the check passes again. The
        // repository's path holds a space and a quote (findings H1-1, H14-3 and H1-7 of the
        // fifth deep review of visual block 2 (#34): the hint restored from the index, from
        // the root only).
        let root = committed_corpus("restored");
        let dir = root.join("corpus");
        let in_root = |args: &[&str]| {
            Command::new(env!("CARGO_BIN_EXE_visual-cli"))
                .args(args)
                .current_dir(&root)
                .output()
                .expect("the binary runs")
        };
        for directory in HINT_DIRECTORIES {
            for stage in HINT_STAGED {
                let below = root.join(directory);
                std::fs::create_dir_all(&below).expect("a subdirectory");
                let staged = |file: &str| {
                    if stage {
                        let done = shell(&root, &format!("git add corpus/{file}"));
                        assert!(done.status.success(), "{done:?}");
                    }
                };
                std::fs::write(dir.join("references.json"), "{}").expect("changed");
                staged("references.json");
                let refused = in_root(&["check", "corpus"]);
                assert_eq!(refused.status.code(), Some(1_i32), "references refused");
                let references = hint_command(&String::from_utf8_lossy(&refused.stderr));
                assert!(
                    references.starts_with("git checkout HEAD -- '/"),
                    "from the last commit, by its absolute path, quoted: {references}"
                );
                let restored = shell(&below, &references);
                assert!(restored.status.success(), "{references}: {restored:?}");
                let display =
                    std::fs::read_to_string(dir.join("display.json")).expect("the corpus");
                std::fs::write(
                    dir.join("display.json"),
                    display.replacen("ff\"]", "fe\"]", 1),
                )
                .expect("changed");
                staged("display.json");
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
                let restore = hint_command(line);
                assert_eq!(
                    hint_command(&String::from_utf8_lossy(&mismatch.stderr)),
                    restore,
                    "the summary names the same command"
                );
                let again = shell(&below, &restore);
                assert!(again.status.success(), "{restore}: {again:?}");
                let checked = in_root(&["check", "corpus"]);
                assert_eq!(
                    (
                        checked.status.code(),
                        String::from_utf8_lossy(&checked.stderr)
                    ),
                    (Some(0_i32), "".into()),
                    "the hints followed from {directory:?}, staged {stage}, the check passes"
                );
            }
        }
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
