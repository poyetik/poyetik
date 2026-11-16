//! Runs the real `visual-gpu` binary: its usage, its exit codes and its one error line.

#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::process::{Command, Stdio};

    /// Runs the binary with `args` in the crate's `tests/` directory (no baseline there, nor
    /// any `spec/` to write one into), and returns
    /// its exit code, stdout and stderr.
    fn binary(args: &[&str]) -> (Option<i32>, String, String) {
        let output = Command::new(env!("CARGO_BIN_EXE_visual-gpu"))
            .args(args)
            .current_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests"))
            .stdin(Stdio::null())
            .output()
            .expect("the binary runs");
        (
            output.status.code(),
            String::from_utf8(output.stdout).expect("UTF-8"),
            String::from_utf8(output.stderr).expect("UTF-8"),
        )
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
                "error: unknown command \"run\" (see: visual-gpu help)\n".to_owned()
            ),
            "one error line, exit code 2"
        );
    }

    #[test]
    fn checks_this_gpu_without_a_baseline_by_showing_it() {
        let (code, out, err) = binary(&["check"]);
        assert_eq!((code, err.as_str()), (Some(0_i32), ""), "{out}");
        assert!(
            out.ends_with("no baseline: shown, not checked\n")
                || out == "this GPU has no clock its programs can read: no time\n",
            "{out}"
        );
    }

    #[test]
    fn fails_with_one_error_line_and_code_1() {
        let (code, out, err) = binary(&["bless"]);
        // No `spec/` in the crate's `tests/`: the baseline cannot be written (or, on a GPU
        // without a clock, there is nothing to bless).
        assert_eq!(code, Some(1_i32), "{out}{err}");
        assert!(
            err.starts_with("error: ") && err.ends_with('\n') && err.lines().count() == 1,
            "{err}"
        );
    }
}
