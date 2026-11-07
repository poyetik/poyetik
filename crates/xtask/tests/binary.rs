//! Runs the real `xtask` binary.

#[cfg(test)]
mod tests {
    use std::process::Command;

    const USAGE: &[u8] = b"usage: cargo xtask <check|commit-msg <file>|help>";

    #[test]
    fn prints_usage_and_fails_without_a_command() {
        let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .output()
            .expect("the binary runs");
        assert!(!output.status.success(), "no command is an error");
        assert!(output.stderr.starts_with(USAGE), "usage on stderr");
        assert!(output.stdout.is_empty(), "nothing on stdout");
    }

    #[test]
    fn prints_help_and_succeeds() {
        let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .arg("help")
            .output()
            .expect("the binary runs");
        assert!(output.status.success(), "help succeeds");
        assert!(output.stdout.starts_with(USAGE), "usage on stdout");
        assert!(output.stderr.is_empty(), "nothing on stderr");
    }
}
