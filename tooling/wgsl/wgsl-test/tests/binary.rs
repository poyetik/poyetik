//! Runs the real `wgsl-test` binary: its usage, its exit codes, its problem lines and its
//! one error line, and a closed stdout.

#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::process::{Command, Stdio};

    /// Runs the binary with `args` in the crate's `tests/fixtures/` directory, and returns
    /// its exit code, stdout and stderr.
    fn binary(args: &[&str]) -> (Option<i32>, String, String) {
        let output = Command::new(env!("CARGO_BIN_EXE_wgsl-test"))
            .args(args)
            .current_dir(fixtures())
            .stdin(Stdio::null())
            .output()
            .expect("the binary runs");
        (
            output.status.code(),
            String::from_utf8(output.stdout).expect("UTF-8"),
            String::from_utf8(output.stderr).expect("UTF-8"),
        )
    }

    /// The directory of the fixtures.
    fn fixtures() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
    }

    #[test]
    fn prints_its_usage_and_refuses_a_usage_error_with_code_2() {
        let (code, out, err) = binary(&["help"]);
        assert_eq!((code, err.as_str()), (Some(0_i32), ""), "help succeeds");
        assert!(out.starts_with("usage: wgsl-test <command>\n"), "{out}");
        assert_eq!(
            binary(&["run"]),
            (
                Some(2_i32),
                String::new(),
                "error: unknown command \"run\" (see: wgsl-test help)\n".to_owned()
            ),
            "one error line, exit code 2"
        );
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let output = Command::new(env!("CARGO_BIN_EXE_wgsl-test"))
                .arg("validate")
                .arg(std::ffi::OsStr::from_bytes(b"valid\xff.wgsl"))
                .current_dir(fixtures())
                .output()
                .expect("the binary runs");
            assert_eq!(
                (
                    output.status.code(),
                    String::from_utf8(output.stdout).expect("UTF-8"),
                    String::from_utf8(output.stderr).expect("UTF-8")
                ),
                (
                    Some(2_i32),
                    String::new(),
                    "error: argument 2 is not valid UTF-8 (see: wgsl-test help)\n".to_owned()
                ),
                "an argument that is not UTF-8, a usage error rather than another file read"
            );
        }
    }

    #[test]
    fn accepts_a_valid_file_silently() {
        assert_eq!(
            binary(&["validate", "valid.wgsl"]),
            (Some(0_i32), String::new(), String::new()),
            "nothing to say"
        );
    }

    #[test]
    fn says_each_refused_file_on_stdout_then_one_error_line_with_code_1() {
        let (code, out, err) = binary(&["validate", "invalid.wgsl", "valid.wgsl", "missing.wgsl"]);
        assert_eq!(code, Some(1_i32), "{out}{err}");
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2, "one line per refused file: {out}");
        assert!(
            lines
                .first()
                .is_some_and(|line| line.starts_with("invalid.wgsl:4:12: invalid: ")),
            "the invalid file at the expression at fault: {out}"
        );
        assert!(
            lines
                .get(1)
                .is_some_and(|line| line.starts_with("missing.wgsl: cannot read: ")),
            "the missing file: {out}"
        );
        assert_eq!(
            err, "error: 2 of 3 files refused, listed above\n",
            "one summary line"
        );
    }

    #[test]
    fn keeps_its_exit_code_when_stdout_is_closed() {
        let mut child = Command::new(env!("CARGO_BIN_EXE_wgsl-test"))
            .args(["validate", "invalid.wgsl", "invalid.wgsl", "invalid.wgsl"])
            .current_dir(fixtures())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("the binary starts");
        drop(child.stdout.take());
        let mut err = String::new();
        let _read = child
            .stderr
            .take()
            .expect("its stderr")
            .read_to_string(&mut err)
            .expect("UTF-8");
        let status = child.wait().expect("the binary ends");
        assert_eq!(status.code(), Some(1_i32), "the work's own code: {err}");
        assert_eq!(
            err, "error: 3 of 3 files refused, listed above\n",
            "the summary, the closed stdout no error"
        );
    }
}
