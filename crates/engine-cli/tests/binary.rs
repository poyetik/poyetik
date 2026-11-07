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
        std::fs::create_dir_all(dir.join("cases")).expect("temporary directory");
        let case = Path::new(&conformance()).join("cases/sine-440.json");
        let copied = std::fs::copy(case, dir.join("cases/sine-440.json")).expect("copy the case");
        assert!(copied > 0, "the case is copied");
        std::fs::write(dir.join("references.json"), r#"{"sine-440":"0"}"#).expect("references");
        let output = Command::new(env!("CARGO_BIN_EXE_engine-cli"))
            .args(["check", &dir.display().to_string()])
            .output()
            .expect("the binary runs");
        std::fs::remove_dir_all(&dir).expect("cleanup");
        assert!(!output.status.success(), "a mismatch fails");
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .starts_with("MISMATCH sine-440: expected 0, got "),
            "explained"
        );
        assert_eq!(
            output.stderr, b"conformance failed: 1 problem(s)\n",
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

    #[test]
    fn prints_usage_and_fails_without_a_command() {
        let output = Command::new(env!("CARGO_BIN_EXE_engine-cli"))
            .output()
            .expect("the binary runs");
        assert!(!output.status.success(), "no command is an error");
        assert!(
            output.stderr.starts_with(b"usage: engine-cli <command>"),
            "usage on stderr"
        );
    }
}
