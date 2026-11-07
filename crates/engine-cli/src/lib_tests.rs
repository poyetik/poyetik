//! Tests of case rendering, WAV encoding and the commands, with an in-memory file system.

use alloc::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use engine_core::SampleRate;

use super::{Disk, Files, HEADER_LEN, Rendered, USAGE, render_case, run, wav_bytes, wav_header};

const SINE: &str = r#"{"sample_rate":48000,"frames":100,"patch":{"format":1,
  "nodes":[{"id":1,"box":"osc~"},{"id":2,"box":"dac~"}],
  "connections":[{"from":[1,0],"to":[2,0]},{"from":[1,0],"to":[2,1]}]}}"#;

/// Files kept in memory.
#[derive(Debug, Default)]
struct Memory {
    files: BTreeMap<PathBuf, Vec<u8>>,
    read_only: bool,
    unreadable: Option<PathBuf>,
}

impl Memory {
    fn with(entries: &[(&str, &str)]) -> Self {
        let mut memory = Self::default();
        for (path, text) in entries {
            drop(
                memory
                    .files
                    .insert(PathBuf::from(path), text.as_bytes().to_vec()),
            );
        }
        memory
    }

    fn text(&self, path: &str) -> String {
        String::from_utf8(self.files[Path::new(path)].clone()).expect("utf-8 file")
    }
}

impl Files for Memory {
    fn read(&mut self, path: &Path) -> io::Result<String> {
        if self.unreadable.as_deref() == Some(path) {
            return Err(io::Error::other("permission denied"));
        }
        let bytes = self
            .files
            .get(path)
            .ok_or_else(|| io::Error::other("no such file"))?;
        String::from_utf8(bytes.clone()).map_err(io::Error::other)
    }

    fn write(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        if self.read_only {
            return Err(io::Error::other("read-only"));
        }
        drop(self.files.insert(path.to_path_buf(), bytes.to_vec()));
        Ok(())
    }

    fn list(&mut self, dir: &Path) -> io::Result<Vec<PathBuf>> {
        let entries: Vec<PathBuf> = self
            .files
            .keys()
            .filter(|p| p.parent() == Some(dir))
            .cloned()
            .collect();
        if entries.is_empty() {
            Err(io::Error::other("no such directory"))
        } else {
            Ok(entries)
        }
    }
}

fn args(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| (*w).to_owned()).collect()
}

fn run_with(memory: &mut Memory, words: &[&str]) -> (Result<(), String>, String) {
    let mut out = Vec::new();
    let result = run(&args(words), memory, &mut out);
    (result, String::from_utf8(out).expect("utf-8 output"))
}

fn sine_hash() -> String {
    render_case(SINE).expect("valid case").hash().to_string()
}

#[test]
fn renders_a_case() {
    let rendered = render_case(SINE).expect("valid case");
    assert_eq!(rendered.rate, SampleRate::HZ_48000, "rate");
    assert_eq!(rendered.channels, 2, "dac~ is stereo");
    assert_eq!(rendered.samples.len(), 200, "100 frames x 2 channels");
    assert_eq!(rendered.samples[0].to_bits(), 0.0_f32.to_bits(), "phase 0");
    assert_eq!(
        rendered.samples[2], rendered.samples[3],
        "same signal on both channels"
    );
}

#[test]
fn rejects_invalid_cases() {
    for (case, start) in [
        ("{", "invalid case: "),
        (r#"{"sample_rate":48000,"frames":1}"#, "invalid case: "),
        (
            r#"{"sample_rate":48000,"frames":1,"patch":{},"extra":1}"#,
            "invalid case: ",
        ),
        (
            r#"{"sample_rate":1,"frames":1,"patch":{}}"#,
            "invalid sample rate: 1 Hz",
        ),
        (
            r#"{"sample_rate":48000,"frames":1,"patch":{"format":2,"nodes":[],"connections":[]}}"#,
            "unsupported patch format 2",
        ),
        (
            r#"{"sample_rate":48000,"frames":1,"patch":{"format":1,"nodes":[{"id":1,"box":"x~"}],"connections":[]}}"#,
            "node 1: unknown box type",
        ),
    ] {
        let error = render_case(case).expect_err("invalid case");
        assert!(error.starts_with(start), "{error}");
    }
}

#[test]
fn writes_a_float_wav_header() {
    let header = wav_header(48_000, 2, 6).expect("valid sizes");
    let expected: Vec<u8> = [
        &b"RIFF"[..],
        &74_u32.to_le_bytes(),
        b"WAVE",
        b"fmt ",
        &18_u32.to_le_bytes(),
        &3_u16.to_le_bytes(),
        &2_u16.to_le_bytes(),
        &48_000_u32.to_le_bytes(),
        &384_000_u32.to_le_bytes(),
        &8_u16.to_le_bytes(),
        &32_u16.to_le_bytes(),
        &0_u16.to_le_bytes(),
        b"fact",
        &4_u32.to_le_bytes(),
        &3_u32.to_le_bytes(),
        b"data",
        &24_u32.to_le_bytes(),
    ]
    .concat();
    assert_eq!(header.to_vec(), expected, "every field");
    assert_eq!(HEADER_LEN, 58, "header size");
    assert_eq!(
        wav_header(48_000, 0, 0).map(|h| h[46..50].to_vec()),
        Ok(0_u32.to_le_bytes().to_vec()),
        "no channel, no frame"
    );
}

#[test]
fn rejects_sizes_beyond_the_wav_format() {
    assert_eq!(
        wav_header(48_000, 70_000, 0),
        Err("too many channels for WAV: 70000".to_owned()),
        "u16 channels"
    );
    assert_eq!(
        wav_header(48_000, 20_000, 0),
        Err("too many channels for WAV: 20000".to_owned()),
        "block align overflow"
    );
    assert_eq!(
        wav_header(384_000, 16_000, 0),
        Err("sample rate too high for WAV: 384000".to_owned()),
        "byte rate overflow"
    );
    let too_long = |samples: usize| Err(format!("too long for WAV: {samples} samples"));
    assert_eq!(
        wav_header(48_000, 1, 1_usize << 30_u32),
        too_long(1_usize << 30_u32),
        "4 GiB of data"
    );
    assert_eq!(
        wav_header(48_000, 1, 1_073_741_812),
        too_long(1_073_741_812),
        "RIFF size overflow"
    );
    assert_eq!(
        wav_header(48_000, 1, 1_usize << 32_u32),
        too_long(1_usize << 32_u32),
        "beyond u32"
    );
    assert!(
        wav_header(48_000, 1, 1_073_741_811).is_ok(),
        "largest size that fits"
    );
}

#[test]
fn encodes_samples_after_the_header() {
    let rendered = Rendered {
        rate: SampleRate::HZ_48000,
        channels: 1,
        samples: vec![0.5, -1.0],
    };
    let bytes = wav_bytes(&rendered).expect("valid render");
    assert_eq!(bytes.len(), HEADER_LEN + 8, "header and two samples");
    assert_eq!(
        &bytes[HEADER_LEN..],
        [0.5_f32.to_le_bytes(), (-1.0_f32).to_le_bytes()].concat(),
        "little-endian f32"
    );
    let too_wide = Rendered {
        channels: 70_000,
        ..rendered
    };
    assert!(wav_bytes(&too_wide).is_err(), "header errors are reported");
}

#[test]
fn render_command_prints_the_hash_and_writes_wav() {
    let mut memory = Memory::with(&[("case.json", SINE)]);
    let (result, out) = run_with(&mut memory, &["render", "case.json"]);
    assert_eq!(
        (result, out),
        (Ok(()), format!("{}\n", sine_hash())),
        "hash only"
    );
    let (with_wav, _) = run_with(&mut memory, &["render", "case.json", "--wav", "out.wav"]);
    assert_eq!(with_wav, Ok(()), "with a WAV file");
    assert_eq!(
        memory.files[Path::new("out.wav")].len(),
        HEADER_LEN + 800,
        "100 stereo frames"
    );
}

#[test]
fn render_command_reports_errors() {
    let mut memory = Memory::with(&[("case.json", SINE), ("bad.json", "{")]);
    for (words, error) in [
        (&["render"][..], USAGE.to_owned()),
        (&["render", "case.json", "--mp3", "x"][..], USAGE.to_owned()),
        (&["render", "case.json", "--wav"][..], USAGE.to_owned()),
        (
            &["render", "missing.json"][..],
            "cannot read missing.json: no such file".to_owned(),
        ),
    ] {
        assert_eq!(run_with(&mut memory, words).0, Err(error), "{words:?}");
    }
    assert!(
        run_with(&mut memory, &["render", "bad.json"])
            .0
            .is_err_and(|e| e.starts_with("invalid case")),
        "invalid case"
    );
    memory.read_only = true;
    assert_eq!(
        run_with(&mut memory, &["render", "case.json", "--wav", "out.wav"]).0,
        Err("cannot write out.wav: read-only".to_owned()),
        "unwritable WAV"
    );
}

fn conformance(references: &str) -> Memory {
    Memory::with(&[
        ("conf/cases/sine.json", SINE),
        ("conf/cases/notes.txt", "ignored"),
        ("conf/references.json", references),
    ])
}

#[test]
fn check_command_accepts_matching_references() {
    let mut memory = conformance(&format!(r#"{{"sine":"{}"}}"#, sine_hash()));
    let (result, out) = run_with(&mut memory, &["check", "conf"]);
    assert_eq!(result, Ok(()), "all match");
    assert_eq!(
        out,
        format!("ok       sine {}\n", sine_hash()),
        "one line per case"
    );
}

#[test]
fn check_command_reports_every_problem() {
    let mut memory = conformance(r#"{"sine":"0000","gone":"1111"}"#);
    let (result, out) = run_with(&mut memory, &["check", "conf"]);
    assert_eq!(
        result,
        Err("conformance failed: 2 problem(s)".to_owned()),
        "two problems"
    );
    assert_eq!(
        out,
        format!(
            "MISMATCH sine: expected 0000, got {}\nSTALE    gone: reference without a case\n",
            sine_hash()
        ),
        "mismatch and stale reference"
    );
    let mut missing = conformance("{}");
    let (missing_result, missing_out) = run_with(&mut missing, &["check", "conf"]);
    assert_eq!(
        missing_result,
        Err("conformance failed: 1 problem(s)".to_owned()),
        "missing reference"
    );
    assert_eq!(
        missing_out, "MISSING  sine: no reference hash (run bless)\n",
        "explained"
    );
}

#[test]
fn check_command_reports_unreadable_inputs() {
    let mut invalid = conformance("[1]");
    assert!(
        run_with(&mut invalid, &["check", "conf"])
            .0
            .is_err_and(|e| e.starts_with("invalid conf/references.json: ")),
        "invalid references"
    );
    let mut no_refs = Memory::with(&[("conf/cases/sine.json", SINE)]);
    assert_eq!(
        run_with(&mut no_refs, &["check", "conf"]).0,
        Err("cannot read conf/references.json: no such file".to_owned()),
        "no references"
    );
    let mut no_cases = Memory::default();
    assert_eq!(
        run_with(&mut no_cases, &["check", "conf"]).0,
        Err("cannot list conf/cases: no such directory".to_owned()),
        "no cases"
    );
    let mut bad_case =
        Memory::with(&[("conf/cases/bad.json", "{"), ("conf/references.json", "{}")]);
    assert!(
        run_with(&mut bad_case, &["check", "conf"])
            .0
            .is_err_and(|e| e.starts_with("bad: invalid case")),
        "case named"
    );
}

#[test]
fn bless_command_writes_sorted_references() {
    let mut memory = Memory::with(&[("conf/cases/sine.json", SINE), ("conf/cases/a.json", SINE)]);
    let (result, out) = run_with(&mut memory, &["bless", "conf"]);
    assert_eq!(result, Ok(()), "blessed");
    assert_eq!(
        out, "blessed 2 case(s) in conf/references.json\n",
        "summary"
    );
    let hash = sine_hash();
    assert_eq!(
        memory.text("conf/references.json"),
        format!("{{\n  \"a\": \"{hash}\",\n  \"sine\": \"{hash}\"\n}}\n"),
        "sorted JSON"
    );
    assert_eq!(
        run_with(&mut memory, &["check", "conf"]).0,
        Ok(()),
        "check agrees"
    );
    memory.read_only = true;
    assert_eq!(
        run_with(&mut memory, &["bless", "conf"]).0,
        Err("cannot write conf/references.json: read-only".to_owned()),
        "unwritable"
    );
    assert_eq!(
        run_with(&mut Memory::default(), &["bless", "conf"]).0,
        Err("cannot list conf/cases: no such directory".to_owned()),
        "no cases"
    );
}

#[test]
fn rejects_unknown_commands() {
    for words in [
        &[][..],
        &["play", "x"][..],
        &["play"][..],
        &["check"][..],
        &["bless", "a", "b"][..],
    ] {
        assert_eq!(
            run_with(&mut Memory::default(), words).0,
            Err(USAGE.to_owned()),
            "{words:?}"
        );
    }
}

/// An output that always fails.
struct Broken;
impl io::Write for Broken {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("closed"))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn reports_a_broken_output() {
    let mut memory = conformance(&format!(r#"{{"sine":"{}"}}"#, sine_hash()));
    assert_eq!(
        run(&args(&["check", "conf"]), &mut memory, &mut Broken),
        Err("closed".to_owned()),
        "check"
    );
    let mut bad = conformance(r#"{"sine":"0"}"#);
    assert_eq!(
        run(&args(&["check", "conf"]), &mut bad, &mut Broken),
        Err("closed".to_owned()),
        "mismatch line"
    );
    let mut missing = conformance("{}");
    assert_eq!(
        run(&args(&["check", "conf"]), &mut missing, &mut Broken),
        Err("closed".to_owned()),
        "missing line"
    );
    let mut stale = Memory::with(&[
        ("conf/cases/sine.json", SINE),
        (
            "conf/references.json",
            &format!(r#"{{"sine":"{}","z":"0"}}"#, sine_hash()),
        ),
    ]);
    let mut once = Limited(1);
    assert_eq!(
        run(&args(&["check", "conf"]), &mut stale, &mut once),
        Err("closed".to_owned()),
        "stale line"
    );
}

/// An output that accepts `n` complete lines, then fails.
struct Limited(usize);
impl io::Write for Limited {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.0 == 0 {
            return Err(io::Error::other("closed"));
        }
        if buf.contains(&b'\n') {
            self.0 = self.0.saturating_sub(1);
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn disk_reads_writes_and_lists() {
    let dir = std::env::temp_dir().join(format!("engine-cli-disk-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temporary directory");
    let file = dir.join("a.txt");
    let mut disk = Disk;
    assert!(disk.write(&file, b"hello").is_ok(), "write");
    assert_eq!(disk.read(&file).ok(), Some("hello".to_owned()), "read");
    assert_eq!(disk.list(&dir).ok(), Some(vec![file.clone()]), "list");
    assert!(
        disk.list(&dir.join("missing")).is_err(),
        "missing directory"
    );
    assert!(disk.read(&dir.join("missing")).is_err(), "missing file");
    assert!(
        disk.write(&dir.join("missing").join("x"), b"").is_err(),
        "unwritable"
    );
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn prepares_a_case_before_rendering() {
    let prepared = super::prepare_case(SINE).expect("valid case");
    assert_eq!(prepared.rate(), SampleRate::HZ_48000, "rate");
    assert_eq!(prepared.channels(), 2, "channels");
    assert_eq!(prepared.sample_count(), 200, "100 frames x 2 channels");
    assert_eq!(
        prepared.render(),
        render_case(SINE).expect("valid case"),
        "same render"
    );
}

#[test]
fn refuses_a_case_too_long_for_a_wav_file_before_rendering() {
    // Only prepares the case: a broken size check fails at once instead of rendering.
    let huge = SINE.replace(r#""frames":100"#, r#""frames":4000000000"#);
    assert_eq!(
        super::prepare_case(&huge).map(|p| p.sample_count()),
        Err("too long for WAV: 8000000000 samples".to_owned()),
        "rejected while preparing"
    );
    let prepared = super::prepare_case(SINE).expect("valid case");
    assert_eq!(
        prepared.wav_header(),
        &wav_header(48_000, 2, 200).expect("valid sizes"),
        "header kept"
    );
}

#[test]
fn writes_wav_files_from_a_header_and_samples() {
    let header = wav_header(48_000, 1, 2).expect("valid sizes");
    let file = super::wav_file(&header, &[0.5, -1.0]);
    assert_eq!(&file[..HEADER_LEN], &header, "header first");
    assert_eq!(
        &file[HEADER_LEN..],
        [0.5_f32.to_le_bytes(), (-1.0_f32).to_le_bytes()].concat(),
        "then samples"
    );
}

#[test]
fn check_command_reports_unreadable_cases() {
    let mut memory = conformance("{}");
    memory.unreadable = Some(PathBuf::from("conf/cases/sine.json"));
    assert_eq!(
        run_with(&mut memory, &["check", "conf"]).0,
        Err("cannot read conf/cases/sine.json: permission denied".to_owned()),
        "the file is named"
    );
}

#[cfg(unix)]
#[test]
fn check_command_rejects_case_names_that_are_not_utf8() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let name = Path::new("conf/cases").join(OsStr::from_bytes(b"bad\xff.json"));
    let mut memory = Memory::default();
    drop(memory.files.insert(name, SINE.as_bytes().to_vec()));
    let error = run_with(&mut memory, &["check", "conf"])
        .0
        .expect_err("invalid name");
    assert!(
        error.starts_with("invalid case file name: conf/cases/bad"),
        "{error}"
    );
}
