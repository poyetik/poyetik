//! Runs the real `visual-gpu` binary: its usage, its exit codes and its one error line.

#![forbid(unsafe_code)]

extern crate alloc;

#[cfg(test)]
mod tests {
    use std::process::{Command, Stdio};

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

    /// Runs the binary with `args` in the crate's `tests/` directory (no baseline there, nor
    /// any `spec/` to write one into).
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
                "error: unknown command \"run\" (see: cargo run --manifest-path crates/visual-gpu/Cargo.toml -- help)\n".to_owned()
            ),
            "one error line, exit code 2"
        );
    }

    #[test]
    fn renders_nothing_on_a_gpu_without_a_baseline() {
        let dir = with_another_gpus_baseline("shown");
        let (code, out, err) = binary_in(&dir, &["check"]);
        assert_eq!((code, err.as_str()), (Some(0_i32), ""), "{out}");
        assert!(
            out.ends_with(
                "no baseline (the baseline holds No such GPU (none)): its time is not checked, nothing rendered\n"
            ),
            "{out}"
        );
        assert_eq!(
            binary(&["check"]),
            (
                Some(1_i32),
                String::new(),
                "error: spec/gpu-time-baseline.txt is missing: no time can be checked: \
                 restore it (see: git checkout -- spec/gpu-time-baseline.txt), or bless it, \
                 only on AC power, outside Low Power Mode, on the machine whose GPU it holds \
                 (see: cargo xtask bless-benchmarks)\n"
                    .to_owned()
            ),
            "no baseline file: the check fails"
        );
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
        let (code, out, err) = binary(&["bless"]);
        // No `spec/` in the crate's `tests/`: the baseline cannot be written (the checks run
        // on a GPU with a clock, `runs_the_checks_on_a_gpu_with_a_clock`).
        assert_eq!(code, Some(1_i32), "{out}{err}");
        assert!(
            err.starts_with("error: ") && err.ends_with('\n') && err.lines().count() == 1,
            "{err}"
        );
    }
}
