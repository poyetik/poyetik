//! Runs the real `visual-gpu` binary: its usage, its exit codes and its one error line.

#![forbid(unsafe_code)]

extern crate alloc;

#[cfg(test)]
mod tests {
    use std::process::{Command, Stdio};

    use visual_testkit::matrix::{HINT_DIRECTORIES, HINT_REPOSITORY, HINT_STAGED};

    /// Runs the binary with `args` in `dir`, and returns its exit code, stdout and stderr.
    fn binary_in(dir: &std::path::Path, args: &[&str]) -> (Option<i32>, String, String) {
        let output = Command::new(env!("CARGO_BIN_EXE_visual-gpu"))
            .args(args)
            .current_dir(dir)
            .stdin(Stdio::null())
            .output()
            .expect("the binary runs");
        (
            output.status.code(),
            String::from_utf8(output.stdout).expect("UTF-8"),
            String::from_utf8(output.stderr).expect("UTF-8"),
        )
    }

    /// Runs the binary with `args` in the crate's `tests/` directory: for its commands that
    /// read no baseline.
    fn binary(args: &[&str]) -> (Option<i32>, String, String) {
        binary_in(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests"),
            args,
        )
    }

    /// A directory one test owns, removed with its files when dropped, whatever the test does.
    #[derive(Debug)]
    #[must_use = "the directory is removed when the guard is dropped"]
    struct Scratch(std::path::PathBuf);

    impl core::ops::Deref for Scratch {
        type Target = std::path::Path;
        fn deref(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            // Nothing better can be done in a drop: a directory a test removed itself is gone.
            drop(std::fs::remove_dir_all(&self.0));
        }
    }

    /// A fresh directory of its own outside the repository, `visual-gpu-<name>-<process id>`
    /// ([`Scratch`]), holding a baseline of a GPU no machine has: `name` keeps tests run at
    /// once apart.
    #[expect(
        clippy::disallowed_methods,
        reason = "the one place visual-gpu's binary tests name the temporary directory: the guard removes what it \
                  creates"
    )]
    fn with_another_gpus_baseline(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("visual-gpu-{name}-{}", std::process::id()));
        if let Err(error) = std::fs::remove_dir_all(&dir) {
            assert_eq!(error.kind(), std::io::ErrorKind::NotFound, "clean start");
        }
        std::fs::create_dir_all(dir.join("spec")).expect("a scratch directory");
        std::fs::create_dir_all(dir.join(".git")).expect("the walk's end");
        std::fs::write(
            dir.join("spec/gpu-time-baseline.txt"),
            visual_gpu::bench::format_baseline(&alloc::collections::BTreeMap::from([(
                "No such GPU (none)".to_owned(),
                visual_gpu::bench::Held::new(1, "macOS 26.5.2".to_owned(), "-".to_owned()),
            )])),
        )
        .expect("a baseline written");
        Scratch(dir)
    }

    #[test]
    fn removes_its_scratch_directory_whatever_the_test_does() {
        let kept = with_another_gpus_baseline("guard");
        let dropped = kept.to_path_buf();
        assert!(
            dropped.join("spec/gpu-time-baseline.txt").exists(),
            "its baseline"
        );
        // Finding PG-5 of the eighth deep review of visual block 2 (#34): the scratch is its own
        // repository's root, never the real one.
        assert_eq!(
            visual_gpu::bench::root_from(&dropped.join("sub")),
            dropped,
            "its own root"
        );
        drop(kept);
        assert!(!dropped.exists(), "removed with its files when dropped");
    }

    #[test]
    fn prints_its_usage_and_refuses_a_usage_error_with_code_2() {
        let (code, out, err) = binary(&["help"]);
        assert_eq!((code, err.as_str()), (Some(0_i32), ""), "help succeeds");
        assert!(
            out.starts_with("usage: visual-gpu <check|bless|help>\n"),
            "{out}"
        );
        assert_eq!(
            binary(&["run"]),
            (
                Some(2_i32),
                String::new(),
                "error: unknown command \"run\" (see: cargo run --manifest-path \"$(git rev-parse --show-toplevel)/crates/visual-gpu/Cargo.toml\" -- help)\n".to_owned()
            ),
            "one error line, exit code 2"
        );
    }

    #[test]
    fn renders_nothing_on_a_gpu_without_a_baseline() {
        let dir = with_another_gpus_baseline("shown");
        // From a subdirectory too, as its hints run (a new finding of the seventh deep review
        // of visual block 2 (#34)).
        let below = dir.join("sub dir");
        std::fs::create_dir(&below).expect("a subdirectory");
        assert_eq!(binary_in(&below, &["check"]), binary_in(&dir, &["check"]));
        let (code, out, err) = binary_in(&dir, &["check"]);
        assert_eq!((code, err.as_str()), (Some(0_i32), ""), "{out}");
        assert!(
            out.ends_with(
                "no baseline (the baseline holds \"No such GPU (none)\"): its time is not checked, nothing rendered\n"
            ),
            "{out}"
        );
        std::fs::remove_file(dir.join("spec/gpu-time-baseline.txt")).expect("removed");
        assert_eq!(
            binary_in(&dir, &["check"]),
            (
                Some(1_i32),
                String::new(),
                "error: spec/gpu-time-baseline.txt is missing: no time can be checked: \
                 restore it (see: git checkout HEAD -- :/spec/gpu-time-baseline.txt), or bless it, \
                 only on AC power, outside Low Power Mode, on the machine whose GPU it holds \
                 (see: cargo xtask bless-benchmarks)\n"
                    .to_owned()
            ),
            "no baseline file: the check fails"
        );
    }

    /// What opens a runnable hint: written in two pieces, so that the hints gate reads no
    /// hint here.
    const HINT_OPENING: &str = concat!("(see", ": ");

    /// The variables under which a test runs git without the machine's or the user's
    /// configuration: a copy of xtask's `HERMETIC_GIT`, which its hermetic git gate holds
    /// this file to. `GIT_CONFIG_GLOBAL` needs git 2.32 or later.
    const HERMETIC_GIT: [(&str, &str); 4] = [
        ("GIT_CONFIG_NOSYSTEM", "1"),
        ("GIT_CONFIG_GLOBAL", "/dev/null"),
        ("GIT_ATTR_NOSYSTEM", "1"),
        ("XDG_CONFIG_HOME", "/dev/null"),
    ];

    /// The variables git sets for a hook that name one repository, removed so that the
    /// process running the tests leaks none in: a copy of xtask's `LOCAL_GIT_VARIABLES`,
    /// which its hermetic git gate holds this file to.
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
    fn shell(dir: &std::path::Path, command: &str) -> std::process::Output {
        let mut sh = Command::new("sh");
        let _hermetic = sh.args(["-c", command]).current_dir(dir).envs(HERMETIC_GIT);
        for variable in LOCAL_GIT_VARIABLES {
            let _without = sh.env_remove(variable);
        }
        sh.output().expect("sh runs")
    }

    #[test]
    fn says_a_current_directory_it_cannot_read_in_one_error_line() {
        // Finding PB-8 of the eighth deep review of visual block 2 (#34): a directory removed
        // under the command was read as no directory, and refused as one without spec/.
        let outside = with_another_gpus_baseline("gone");
        std::fs::create_dir(outside.join("gone")).expect("a directory");
        let bin = env!("CARGO_BIN_EXE_visual-gpu").replace('\'', r"'\''");
        let ran = shell(
            &outside,
            &format!("cd gone && rmdir \"$PWD\" && exec '{bin}' check"),
        );
        assert_eq!(
            (
                ran.status.code(),
                String::from_utf8(ran.stdout).expect("UTF-8"),
                String::from_utf8(ran.stderr).expect("UTF-8")
            ),
            (
                Some(1_i32),
                String::new(),
                "error: cannot read the current directory: No such file or directory (os error 2)\n"
                    .to_owned()
            )
        );
    }

    #[test]
    fn restores_the_baseline_by_the_hint_it_gives() {
        // Findings H1-1 and H14-3 of the fifth deep review of visual block 2 (#34): the hint
        // restored from the index, which kept a damaged file once staged, and named a path
        // from the root, which git refuses from a subdirectory. A repository whose path holds
        // a space and a quote, its baseline damaged, staged or not: the hint, run from every
        // directory of the shared test matrix, restores it, and the check passes again.
        let root = with_another_gpus_baseline(HINT_REPOSITORY);
        let git = "git -c user.name=t -c user.email=t@t";
        let made = shell(
            &root,
            &format!("{git} init --quiet && {git} add spec && {git} commit --quiet -m held"),
        );
        assert!(made.status.success(), "{made:?}");
        // Damaged, then missing (finding K5-5 of the sixth deep review of visual block 2 (#34);
        // the bless of a missing one, which measures, is followed on the fake world).
        let damages = HINT_STAGED.map(|staged| [(staged, false), (staged, true)]);
        let baseline = root.join("spec/gpu-time-baseline.txt");
        for directory in HINT_DIRECTORIES {
            for &(staged, missing) in damages.as_flattened() {
                if missing {
                    std::fs::remove_file(&baseline).expect("removed");
                } else {
                    std::fs::write(&baseline, "damaged\n").expect("damaged");
                }
                if staged {
                    assert!(shell(&root, "git add spec").status.success(), "staged");
                }
                let (code, _, err) = binary_in(&root, &["check"]);
                assert_eq!(code, Some(1_i32), "{err}");
                let (_, after) = err.split_once(HINT_OPENING).expect("a hint");
                let (command, _) = after.split_once(')').expect("its end");
                let below = root.join(directory);
                std::fs::create_dir_all(&below).expect("a subdirectory");
                let restored = shell(&below, command);
                assert!(restored.status.success(), "{command}: {restored:?}");
                let (passed, out, said) = binary_in(&root, &["check"]);
                assert_eq!(
                    (passed, said.as_str()),
                    (Some(0_i32), ""),
                    "{directory:?}, staged {staged}, missing {missing}: {out}"
                );
            }
        }
        // A directory in its place, holding an untracked file: no hint of the line restores
        // the file, which would delete it (K13-3). Moved away, then every hint of the line
        // followed from every directory (findings MC-6 and MI-6 of the seventh), `cargo run`
        // standing for this binary run where the hint runs: the check says the file missing,
        // as above, and the untracked file is kept.
        let cargo = format!(
            "cargo() {{ [ \"$*\" = \"run --manifest-path $(git rev-parse --show-toplevel)/crates/visual-gpu/Cargo.toml -- check\" ] && '{}' check; }}; ",
            // Finding PB-14 of the eighth deep review of visual block 2 (#34).
            env!("CARGO_BIN_EXE_visual-gpu").replace('\'', r"'\''")
        );
        for directory in HINT_DIRECTORIES {
            std::fs::remove_file(&baseline).expect("removed");
            std::fs::create_dir_all(baseline.join("mine")).expect("a directory in its place");
            std::fs::write(baseline.join("mine/notes"), "kept").expect("an untracked file");
            let (code, _, err) = binary_in(&root, &["check"]);
            assert_eq!(code, Some(1_i32), "{err}");
            let moved = shell(&root, "mv spec/gpu-time-baseline.txt moved");
            assert!(moved.status.success(), "{moved:?}");
            let missing = binary_in(&root, &["check"]);
            for hint in err.split(HINT_OPENING).skip(1) {
                // Up to the parenthesis that closes the hint, `$(…)` inside it kept.
                let mut depth = 0_i32;
                let end = hint
                    .find(|character| {
                        depth += i32::from(character == '(') - i32::from(character == ')');
                        depth < 0_i32
                    })
                    .expect("its end");
                let command = hint.get(..end).expect("a command");
                let followed = shell(&root.join(directory), &format!("{cargo}{command}"));
                assert_eq!(
                    (
                        followed.status.code(),
                        String::from_utf8_lossy(&followed.stderr)
                    ),
                    (missing.0, missing.2.as_str().into()),
                    "{command}"
                );
            }
            let kept = std::fs::read_to_string(root.join("moved/mine/notes"));
            assert_eq!(kept.ok().as_deref(), Some("kept"), "no untracked file lost");
            std::fs::remove_dir_all(root.join("moved")).expect("set aside");
            assert!(
                shell(&root, "git checkout HEAD -- spec").status.success(),
                "restored"
            );
        }
    }

    #[test]
    fn ignores_a_closed_stdout() {
        let dir = with_another_gpus_baseline("closed");
        for args in [["help"], ["check"]] {
            let (reader, writer) = std::io::pipe().expect("a pipe");
            drop(reader);
            let output = Command::new(env!("CARGO_BIN_EXE_visual-gpu"))
                .args(args)
                .current_dir(&*dir)
                .stdin(Stdio::null())
                .stdout(writer)
                .stderr(Stdio::piped())
                .output()
                .expect("the binary runs");
            assert_eq!(output.status.code(), Some(0_i32), "{args:?} succeeds");
            assert!(output.stderr.is_empty(), "{args:?}: nothing on stderr");
        }
    }

    #[test]
    fn fails_with_one_error_line_and_code_1() {
        // Findings K13-11, K3-2 and L3-n2 of the sixth deep review of visual block 2 (#34): run
        // where no baseline can be written, it measured every run first, and the test took any
        // failure for that one. Refused before a GPU is opened, by its exact line.
        // The scratch's spec/ above a repository's root, which is never read.
        let outside = with_another_gpus_baseline("outside");
        let repository = outside.join("repo");
        std::fs::create_dir_all(repository.join(".git")).expect("a repository");
        for command in ["check", "bless"] {
            assert_eq!(
                binary_in(&repository, &[command]),
                (
                    Some(1_i32),
                    String::new(),
                    "error: cannot read spec/gpu-time-baseline.txt: no spec directory at the root of the repository it runs in (here when it runs in none): run it inside the repository that holds the baseline\n"
                        .to_owned()
                ),
                "{command}"
            );
        }
    }
}
