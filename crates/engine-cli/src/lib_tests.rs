//! Tests of case rendering, WAV encoding and the commands, with an in-memory file system.

use alloc::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use engine_core::SampleRate;

use super::{
    CaseError, CliError, Disk, Files, HEADER_LEN, Rendered, USAGE, arguments, render_case, run,
    wav_bytes, wav_header,
};

const SINE: &str = r#"{"sample_rate":48000,"frames":100}"#;

const PATCH: &str = r#"{"format":1,
  "nodes":[{"id":1,"box":"osc~"},{"id":2,"box":"dac~"}],
  "connections":[{"from":[1,0],"to":[2,0]},{"from":[1,0],"to":[2,1]}]}"#;

/// Files kept in memory.
#[derive(Debug, Default)]
struct Memory {
    files: BTreeMap<PathBuf, Vec<u8>>,
    read_only: bool,
    no_dirs: bool,
    unreadable: Option<PathBuf>,
    dirs: Vec<PathBuf>,
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
    /// Refuses a file larger than `limit` like [`Disk`], with a message naming the limit.
    fn read(&mut self, path: &Path, limit: usize) -> io::Result<String> {
        if self.unreadable.as_deref() == Some(path) {
            return Err(io::Error::other("permission denied"));
        }
        let bytes = self
            .files
            .get(path)
            .ok_or_else(|| io::Error::other("no such file"))?;
        if bytes.len() > limit {
            return Err(io::Error::other(format!("larger than {limit} bytes")));
        }
        String::from_utf8(bytes.clone()).map_err(io::Error::other)
    }

    fn write(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        if self.read_only {
            return Err(io::Error::other("read-only"));
        }
        drop(self.files.insert(path.to_path_buf(), bytes.to_vec()));
        Ok(())
    }

    fn create_dir_all(&mut self, dir: &Path) -> io::Result<()> {
        if self.no_dirs {
            return Err(io::Error::other("cannot create directories"));
        }
        self.dirs.push(dir.to_path_buf());
        Ok(())
    }

    /// Lists in reverse name order: a real directory has no order either.
    fn list(&mut self, dir: &Path) -> io::Result<Vec<PathBuf>> {
        let entries: Vec<PathBuf> = self
            .files
            .keys()
            .rev()
            .filter(|path| path.parent() == Some(dir))
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
    words.iter().map(|word| (*word).to_owned()).collect()
}

fn run_with(memory: &mut Memory, words: &[&str]) -> (Result<(), String>, String) {
    let mut out = Vec::new();
    let result = run(&args(words), memory, &mut out).map_err(|error| error.to_string());
    (result, String::from_utf8(out).expect("utf-8 output"))
}

/// How a usage error reads: its reason, then where to find the usage.
fn usage(reason: &str) -> String {
    format!("{reason} (see: engine-cli help)")
}

fn sine_hash() -> String {
    render_case(SINE, PATCH)
        .expect("valid case")
        .hash()
        .to_string()
}

#[test]
fn renders_a_case() {
    let rendered = render_case(SINE, PATCH).expect("valid case");
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
    let no_output = r#"{"format":1,"nodes":[{"id":1,"box":"osc~"}],"connections":[]}"#;
    let case = |message: &str| CaseError::Case(message.to_owned());
    let patch = |message: &str| CaseError::Patch(message.to_owned());
    for (case_text, patch_text, expected) in [
        (
            "{",
            PATCH,
            case("invalid case: EOF while parsing an object at line 1 column 1"),
        ),
        (
            r#"{"sample_rate":48000}"#,
            PATCH,
            case("invalid case: missing field `frames` at line 1 column 21"),
        ),
        (
            r#"{"sample_rate":48000,"frames":1,"extra":1}"#,
            PATCH,
            case(
                "invalid case: unknown field `extra`, expected `sample_rate` or `frames` at line 1 column 39",
            ),
        ),
        (
            r#"{"sample_rate":1,"frames":1}"#,
            PATCH,
            case("invalid sample rate: 1 Hz (supported: 8000 to 384000 Hz)"),
        ),
        (
            SINE,
            r#"{"format":2,"nodes":[],"connections":[]}"#,
            patch("unsupported patch format 2 (expected 1)"),
        ),
        (
            SINE,
            r#"{"format":1,"nodes":[{"id":1,"box":"x~"}],"connections":[]}"#,
            patch("box 1: unknown box type \"x~\""),
        ),
        (
            SINE,
            no_output,
            patch("the patch has no audio output: nothing to render"),
        ),
    ] {
        assert_eq!(
            render_case(case_text, patch_text).map(|_| ()),
            Err(expected),
            "{case_text} {patch_text}"
        );
    }
    let error = patch("p");
    assert_eq!(error.to_string(), "p", "the message alone");
    assert_eq!(
        error.in_files(Path::new("c.json"), Path::new("p.json")),
        "p.json: p",
        "the patch file is named"
    );
    assert_eq!(
        case("c").in_files(Path::new("c.json"), Path::new("p.json")),
        "c.json: c",
        "the case file is named"
    );
}

#[test]
fn accepts_durations_from_one_frame_to_the_limit_only() {
    let with_frames = |frames: &str| format!(r#"{{"sample_rate":48000,"frames":{frames}}}"#);
    for frames in ["0", "536870913", "18446744073709551615"] {
        assert_eq!(
            super::prepare_case(&with_frames(frames), PATCH).map(|_| ()),
            Err(CaseError::Case(format!(
                "invalid case: frames must be between 1 and 536870912, got {frames}"
            ))),
            "{frames} frames"
        );
    }
    assert!(
        super::prepare_case(&with_frames("1"), PATCH).is_ok(),
        "one frame"
    );
    // The longest case passes the duration check; in stereo, only the WAV size stops it.
    assert_eq!(
        super::prepare_case(&with_frames("536870912"), PATCH).map(|_| ()),
        Err(CaseError::Case(
            "too long for WAV: 1073741824 samples".to_owned()
        )),
        "2^29 frames are a valid duration"
    );
    assert_eq!(super::MAX_FRAMES, 1 << 29_u32, "2^29 frames");
}

#[test]
fn rejects_every_case_of_the_shared_invalid_corpus() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/invalid/cases");
    let mut count = 0_u32;
    for entry in std::fs::read_dir(&dir).expect("the invalid corpus") {
        let path = entry.expect("an entry").path();
        let case = std::fs::read_to_string(&path).expect("a case file");
        assert!(
            matches!(super::prepare_case(&case, PATCH), Err(CaseError::Case(_))),
            "{} is rejected",
            path.display()
        );
        count += 1;
    }
    assert!(count >= 2, "{count} invalid cases");
}

#[test]
fn reads_the_patch_bytes_unchanged() {
    // The same parameter written twice is refused by the patch reader; a reader that
    // went through a JSON value first would silently keep the last one.
    let repeated = PATCH.replace(
        r#"{"id":1,"box":"osc~"}"#,
        r#"{"id":1,"box":"osc~","params":{"frequency":1,"frequency":2}}"#,
    );
    let error = render_case(SINE, &repeated)
        .expect_err("repeated parameter")
        .to_string();
    assert!(
        error.contains("parameter `frequency` is given twice"),
        "{error}"
    );
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
        wav_header(48_000, 0, 0).map(|bytes| bytes[46..50].to_vec()),
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
    let mut memory = Memory::with(&[("case.json", SINE), ("patch.json", PATCH)]);
    let (result, out) = run_with(&mut memory, &["render", "case.json", "patch.json"]);
    assert_eq!(
        (result, out),
        (Ok(()), format!("{}\n", sine_hash())),
        "hash only"
    );
    let (with_wav, _) = run_with(
        &mut memory,
        &["render", "case.json", "patch.json", "--wav", "out.wav"],
    );
    assert_eq!(with_wav, Ok(()), "with a WAV file");
    assert_eq!(
        memory.files[Path::new("out.wav")].len(),
        HEADER_LEN + 800,
        "100 stereo frames"
    );
}

#[test]
fn render_command_reports_errors() {
    let mut memory = Memory::with(&[
        ("case.json", SINE),
        ("patch.json", PATCH),
        ("bad.json", "{"),
    ]);
    let expects = usage("render expects <case.json> <patch.json> [--wav <file.wav>]");
    for (words, error) in [
        (&["render"][..], expects.clone()),
        (&["render", "case.json"][..], expects.clone()),
        (
            &["render", "case.json", "patch.json", "--mp3", "x"][..],
            usage("unknown option \"--mp3\""),
        ),
        (
            &["render", "case.json", "patch.json", "-w"][..],
            usage("unknown option \"-w\""),
        ),
        (
            &["render", "case.json", "patch.json", "--wav"][..],
            expects.clone(),
        ),
        (
            &["render", "case.json", "patch.json", "extra"][..],
            expects.clone(),
        ),
        (
            &["render", "case.json", "patch.json", "--wav", "a.wav", "b"][..],
            expects,
        ),
        (
            &["render", "missing.json", "patch.json"][..],
            "cannot read missing.json: no such file".to_owned(),
        ),
        (
            &["render", "case.json", "missing.json"][..],
            "cannot read missing.json: no such file".to_owned(),
        ),
    ] {
        assert_eq!(run_with(&mut memory, words).0, Err(error), "{words:?}");
    }
    assert_eq!(
        run_with(&mut memory, &["render", "bad.json", "patch.json"]).0,
        Err("bad.json: invalid case: EOF while parsing an object at line 1 column 1".to_owned()),
        "the case file is named"
    );
    assert_eq!(
        run_with(&mut memory, &["render", "case.json", "bad.json"]).0,
        Err("bad.json: invalid patch: EOF while parsing an object at line 1 column 1".to_owned()),
        "the patch file is named"
    );
    memory.read_only = true;
    assert_eq!(
        run_with(
            &mut memory,
            &["render", "case.json", "patch.json", "--wav", "out.wav"]
        )
        .0,
        Err("cannot write out.wav: read-only".to_owned()),
        "unwritable WAV"
    );
}

fn conformance(references: &str) -> Memory {
    Memory::with(&[
        ("conf/cases/sine.json", SINE),
        ("conf/patches/sine.json", PATCH),
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
        Err("conformance failed in conf: 2 problem(s)".to_owned()),
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
        Err("conformance failed in conf: 1 problem(s)".to_owned()),
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
            .is_err_and(|error| error.starts_with("invalid conf/references.json: ")),
        "invalid references"
    );
    let mut no_refs = Memory::with(&[
        ("conf/cases/sine.json", SINE),
        ("conf/patches/sine.json", PATCH),
    ]);
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
}

#[test]
fn check_command_reports_every_case_that_fails_to_render() {
    // Two broken cases, one in its case file, one in its patch, between valid ones: both
    // are reported, and the valid cases are still checked.
    let mut memory = Memory::with(&[
        ("conf/cases/a.json", "{"),
        ("conf/patches/a.json", PATCH),
        ("conf/cases/b.json", SINE),
        (
            "conf/patches/b.json",
            r#"{"format":1,"nodes":[{"id":1,"box":"foo~"}],"connections":[]}"#,
        ),
        ("conf/cases/sine.json", SINE),
        ("conf/patches/sine.json", PATCH),
        (
            "conf/references.json",
            &format!(r#"{{"a":"0","b":"0","sine":"{}"}}"#, sine_hash()),
        ),
    ]);
    let (result, out) = run_with(&mut memory, &["check", "conf"]);
    assert_eq!(
        result,
        Err("conformance failed in conf: 2 problem(s)".to_owned()),
        "both failures counted, once each"
    );
    assert_eq!(
        out,
        format!(
            "FAILED   a: conf/cases/a.json: invalid case: EOF while parsing an object at line 1 column 1\n\
             FAILED   b: conf/patches/b.json: box 1: unknown box type \"foo~\"\n\
             ok       sine {}\n",
            sine_hash()
        ),
        "every case reported, in order"
    );
    let mut broken = Memory::with(&[
        ("conf/cases/a.json", "{"),
        ("conf/patches/a.json", PATCH),
        ("conf/references.json", "{}"),
    ]);
    assert_eq!(
        run(&args(&["check", "conf"]), &mut broken, &mut Broken).map_err(|error| error.to_string()),
        Err("cannot write to stdout: closed".to_owned()),
        "a broken output is a failure"
    );
}

#[test]
fn bless_command_writes_sorted_references() {
    let mut memory = Memory::with(&[
        ("conf/cases/sine.json", SINE),
        ("conf/patches/sine.json", PATCH),
        ("conf/cases/a.json", SINE),
        ("conf/patches/a.json", PATCH),
    ]);
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
    for (words, reason) in [
        (&[][..], "missing command"),
        (&["play", "x"][..], "unknown command \"play\""),
        (&["play"][..], "unknown command \"play\""),
        (&["check"][..], "check expects <dir>"),
        (&["bless", "a", "b"][..], "bless expects <dir>"),
        (&["export", "a"][..], "export expects <dir> <out>"),
        (&["help", "render"][..], "help expects no argument"),
    ] {
        assert_eq!(
            run_with(&mut Memory::default(), words).0,
            Err(usage(reason)),
            "{words:?}"
        );
    }
}

#[test]
fn help_prints_the_usage_on_the_output() {
    assert_eq!(
        run_with(&mut Memory::default(), &["help"]),
        (Ok(()), format!("{USAGE}\n")),
        "usage on stdout"
    );
    assert_eq!(
        run(&args(&["help"]), &mut Memory::default(), &mut Broken)
            .map_err(|error| error.to_string()),
        Err("cannot write to stdout: closed".to_owned()),
        "a broken output is a failure"
    );
    for command in ["render", "check", "bless", "export", "help"] {
        assert!(
            USAGE
                .lines()
                .any(|line| line.starts_with(&format!("  {command} "))),
            "{command} is described"
        );
    }
}

#[test]
fn reads_arguments_as_utf8_text() {
    use std::ffi::OsString;
    assert_eq!(
        arguments(vec![OsString::from("check"), OsString::from("dir")]),
        Ok(args(&["check", "dir"])),
        "text arguments"
    );
    assert_eq!(arguments(Vec::new()), Ok(Vec::new()), "no argument");
}

#[cfg(unix)]
#[test]
fn refuses_arguments_that_are_not_utf8() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    assert_eq!(
        arguments(vec![
            OsString::from("render"),
            OsString::from_vec(b"case\xff.json".to_vec()),
        ]),
        Err(CliError::Usage(
            "argument is not valid UTF-8: case\u{fffd}.json".to_owned()
        )),
        "a usage error naming the argument"
    );
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
        run(&args(&["check", "conf"]), &mut memory, &mut Broken).map_err(|error| error.to_string()),
        Err("cannot write to stdout: closed".to_owned()),
        "check"
    );
    let mut bad = conformance(r#"{"sine":"0"}"#);
    assert_eq!(
        run(&args(&["check", "conf"]), &mut bad, &mut Broken).map_err(|error| error.to_string()),
        Err("cannot write to stdout: closed".to_owned()),
        "mismatch line"
    );
    let mut missing = conformance("{}");
    assert_eq!(
        run(&args(&["check", "conf"]), &mut missing, &mut Broken)
            .map_err(|error| error.to_string()),
        Err("cannot write to stdout: closed".to_owned()),
        "missing line"
    );
    let mut stale = Memory::with(&[
        ("conf/cases/sine.json", SINE),
        ("conf/patches/sine.json", PATCH),
        (
            "conf/references.json",
            &format!(r#"{{"sine":"{}","z":"0"}}"#, sine_hash()),
        ),
    ]);
    let mut once = Limited(1);
    assert_eq!(
        run(&args(&["check", "conf"]), &mut stale, &mut once).map_err(|error| error.to_string()),
        Err("cannot write to stdout: closed".to_owned()),
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
    assert_eq!(disk.read(&file, 5).ok(), Some("hello".to_owned()), "read");
    assert_eq!(
        disk.read(&file, 4)
            .map_err(|error| (error.kind(), error.to_string())),
        Err((
            io::ErrorKind::FileTooLarge,
            "larger than 4 bytes".to_owned()
        )),
        "one byte over the limit"
    );
    assert!(disk.write(&file, b"\xff").is_ok(), "write bytes");
    assert_eq!(
        disk.read(&file, 5).map_err(|error| error.kind()),
        Err(io::ErrorKind::InvalidData),
        "not UTF-8"
    );
    assert_eq!(disk.list(&dir).ok(), Some(vec![file.clone()]), "list");
    assert!(
        disk.list(&dir.join("missing")).is_err(),
        "missing directory"
    );
    assert!(disk.read(&dir.join("missing"), 5).is_err(), "missing file");
    let nested = dir.join("x").join("y");
    assert!(disk.create_dir_all(&nested).is_ok(), "create directories");
    assert!(nested.is_dir(), "created");
    assert!(disk.create_dir_all(&nested).is_ok(), "already there");
    assert!(
        disk.create_dir_all(&file.join("z")).is_err(),
        "under a file"
    );
    assert!(
        disk.write(&dir.join("missing").join("x"), b"").is_err(),
        "unwritable"
    );
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

#[cfg(unix)]
#[test]
fn disk_reads_at_most_one_byte_past_the_limit() {
    // An endless file: reading it whole would never end.
    let error = Disk
        .read(Path::new("/dev/zero"), 1_000)
        .expect_err("too large");
    assert_eq!(error.to_string(), "larger than 1000 bytes", "refused");
}

#[test]
fn refuses_files_larger_than_their_limit() {
    assert_eq!(
        (super::MAX_CASE_BYTES, super::MAX_REFERENCES_BYTES),
        (65_536, 1_048_576),
        "the documented limits: 64 KiB and 1 MiB"
    );
    let case = format!("{SINE}{}", " ".repeat(super::MAX_CASE_BYTES - SINE.len()));
    let patch = format!(
        "{PATCH}{}",
        " ".repeat(engine_patch::MAX_PATCH_BYTES - PATCH.len())
    );
    let hashes = format!(r#"{{"c":"{}"}}"#, sine_hash());
    let references = format!(
        "{hashes}{}",
        " ".repeat(super::MAX_REFERENCES_BYTES - hashes.len())
    );
    let mut memory = Memory::with(&[
        ("c.json", &case),
        ("p.json", &patch),
        ("conf/cases/c.json", &case),
        ("conf/patches/c.json", &patch),
        ("conf/references.json", &references),
    ]);
    assert_eq!(
        run_with(&mut memory, &["render", "c.json", "p.json"]).0,
        Ok(()),
        "exactly at the limits"
    );
    assert_eq!(
        run_with(&mut memory, &["check", "conf"]).0,
        Ok(()),
        "every file of a corpus at its limit"
    );
    for (file, limit) in [
        ("c.json", super::MAX_CASE_BYTES),
        ("p.json", engine_patch::MAX_PATCH_BYTES),
    ] {
        let mut over = Memory::with(&[("c.json", &case), ("p.json", &patch)]);
        over.files
            .get_mut(Path::new(file))
            .expect("the file")
            .push(b' ');
        assert_eq!(
            run_with(&mut over, &["render", "c.json", "p.json"]).0,
            Err(format!("cannot read {file}: larger than {limit} bytes")),
            "{file}: one byte over"
        );
    }
    for (file, limit) in [
        ("conf/cases/c.json", super::MAX_CASE_BYTES),
        ("conf/patches/c.json", engine_patch::MAX_PATCH_BYTES),
        ("conf/references.json", super::MAX_REFERENCES_BYTES),
    ] {
        let mut over = Memory::with(&[
            ("conf/cases/c.json", &case),
            ("conf/patches/c.json", &patch),
            ("conf/references.json", &references),
        ]);
        over.files
            .get_mut(Path::new(file))
            .expect("the file")
            .push(b' ');
        assert_eq!(
            run_with(&mut over, &["check", "conf"]).0,
            Err(format!("cannot read {file}: larger than {limit} bytes")),
            "{file}: one byte over"
        );
    }
}

#[test]
fn prepares_a_case_before_rendering() {
    let prepared = super::prepare_case(SINE, PATCH).expect("valid case");
    assert_eq!(prepared.channels(), 2, "channels");
    assert_eq!(prepared.sample_count(), 200, "100 frames x 2 channels");
    assert_eq!(
        prepared.render(),
        render_case(SINE, PATCH).expect("valid case"),
        "same render"
    );
}

#[test]
fn refuses_a_case_too_long_for_a_wav_file_before_rendering() {
    // Only prepares the case: a broken size check fails at once instead of rendering.
    // The longest stereo case, 2^29 frames, is 4 GiB of samples: one byte too many.
    let huge = SINE.replace(r#""frames":100"#, r#""frames":536870912"#);
    assert_ne!(huge, SINE, "the duration was replaced");
    assert_eq!(
        super::prepare_case(&huge, PATCH).map(|prepared| prepared.sample_count()),
        Err(CaseError::Case(
            "too long for WAV: 1073741824 samples".to_owned()
        )),
        "rejected while preparing"
    );
    let prepared = super::prepare_case(SINE, PATCH).expect("valid case");
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

/// Two cases, `a` and `b`, the second one an octave up.
fn two_cases() -> Memory {
    Memory::with(&[
        ("conf/cases/b.json", SINE),
        (
            "conf/patches/b.json",
            &PATCH.replace(
                "{\"id\":1,\"box\":\"osc~\"}",
                "{\"id\":1,\"box\":\"osc~\",\"params\":{\"frequency\":880}}",
            ),
        ),
        ("conf/cases/a.json", SINE),
        ("conf/patches/a.json", PATCH),
        ("conf/cases/notes.txt", "ignored"),
    ])
}

#[test]
fn export_command_writes_the_wav_of_every_case_in_order() {
    let mut memory = two_cases();
    let (result, out) = run_with(&mut memory, &["export", "conf", "out/wav"]);
    assert_eq!(result, Ok(()), "{out}");
    assert_eq!(
        memory.dirs,
        vec![PathBuf::from("out/wav")],
        "output directory"
    );
    let rendered_a = render_case(SINE, PATCH).expect("valid case");
    let rendered_b = render_case(SINE, &memory.text("conf/patches/b.json")).expect("valid case");
    assert_ne!(
        rendered_a.samples, rendered_b.samples,
        "two different cases"
    );
    assert_eq!(
        memory.files[Path::new("out/wav/a.wav")],
        wav_bytes(&rendered_a).expect("fits"),
        "a"
    );
    assert_eq!(
        memory.files[Path::new("out/wav/b.wav")],
        wav_bytes(&rendered_b).expect("fits"),
        "b"
    );
    assert_eq!(
        out,
        format!(
            "wrote    out/wav/a.wav {}\nwrote    out/wav/b.wav {}\n",
            rendered_a.hash(),
            rendered_b.hash()
        ),
        "sorted by name"
    );
    assert!(
        !memory.files.contains_key(Path::new("out/wav/notes.wav")),
        "only JSON cases"
    );
}

#[test]
fn export_command_reports_errors() {
    let mut no_dirs = two_cases();
    no_dirs.no_dirs = true;
    assert_eq!(
        run_with(&mut no_dirs, &["export", "conf", "out"]).0,
        Err("cannot create out: cannot create directories".to_owned()),
        "directory"
    );
    let mut read_only = two_cases();
    read_only.read_only = true;
    assert_eq!(
        run_with(&mut read_only, &["export", "conf", "out"]).0,
        Err("cannot write out/a.wav: read-only".to_owned()),
        "write"
    );
    let mut invalid = Memory::with(&[
        ("conf/cases/bad.json", "{}"),
        ("conf/patches/bad.json", PATCH),
    ]);
    assert!(
        run_with(&mut invalid, &["export", "conf", "out"])
            .0
            .is_err_and(|error| error.starts_with("conf/cases/bad.json: invalid case: ")),
        "invalid case file named"
    );
    let mut missing = Memory::default();
    assert_eq!(
        run_with(&mut missing, &["export", "conf", "out"]).0,
        Err("cannot list conf/cases: no such directory".to_owned()),
        "no cases"
    );
    let mut unreadable = two_cases();
    unreadable.unreadable = Some(PathBuf::from("conf/cases/a.json"));
    assert_eq!(
        run_with(&mut unreadable, &["export", "conf", "out"]).0,
        Err("cannot read conf/cases/a.json: permission denied".to_owned()),
        "unreadable case"
    );
    assert_eq!(
        run_with(&mut two_cases(), &["export", "conf"]).0,
        Err(usage("export expects <dir> <out>")),
        "missing output"
    );
}

#[test]
fn pairs_every_case_with_its_patch() {
    let mut no_patch = Memory::with(&[
        ("conf/cases/sine.json", SINE),
        ("conf/cases/lonely.json", SINE),
        ("conf/patches/sine.json", PATCH),
        ("conf/references.json", "{}"),
    ]);
    assert_eq!(
        run_with(&mut no_patch, &["check", "conf"]).0,
        Err("cannot read conf/patches/lonely.json: no such file".to_owned()),
        "a case without its patch"
    );
    let mut orphan = Memory::with(&[
        ("conf/cases/sine.json", SINE),
        ("conf/patches/sine.json", PATCH),
        ("conf/patches/zombie.json", PATCH),
        ("conf/patches/notes.txt", "ignored"),
        ("conf/references.json", "{}"),
    ]);
    assert_eq!(
        run_with(&mut orphan, &["check", "conf"]).0,
        Err("zombie: patch without a case: conf/patches/zombie.json".to_owned()),
        "a patch without its case"
    );
    let mut no_patches = Memory::with(&[("conf/cases/sine.json", SINE)]);
    assert_eq!(
        run_with(&mut no_patches, &["check", "conf"]).0,
        Err("cannot list conf/patches: no such directory".to_owned()),
        "no patch directory"
    );
}

#[test]
fn tells_usage_errors_from_failures() {
    let mut memory = Memory::default();
    let unknown = run(&args(&["nope"]), &mut memory, &mut Vec::new());
    assert_eq!(
        unknown,
        Err(CliError::Usage("unknown command \"nope\"".to_owned())),
        "usage"
    );
    let failed = run(&args(&["check", "conf"]), &mut memory, &mut Vec::new());
    assert_eq!(
        failed,
        Err(CliError::Failed(
            "cannot list conf/cases: no such directory".to_owned()
        )),
        "failure"
    );
    assert_eq!(
        unknown.map_err(|error| (error.exit_code(), error.to_string())),
        Err((2, usage("unknown command \"nope\""))),
        "exit code 2, pointing to the help"
    );
    assert_eq!(
        failed.map_err(|error| (error.exit_code(), error.to_string())),
        Err((1, "cannot list conf/cases: no such directory".to_owned())),
        "exit code 1, the message alone"
    );
}

#[test]
fn reports_an_error_as_one_escaped_line() {
    assert_eq!(
        CliError::Failed("p.json: box 1: unknown box type \"x~\"".to_owned()).line(),
        "error: p.json: box 1: unknown box type \"x~\"",
        "plain text is unchanged, quotes included"
    );
    assert_eq!(
        CliError::Usage("unknown command \"\u{1b}[2J\"".to_owned()).line(),
        "error: unknown command \"\\u{1b}[2J\" (see: engine-cli help)",
        "a usage error, escaped too"
    );
    let hostile =
        "a\nb\rc\td\u{7f}e\u{85}f\u{202e}g\u{2066}h\u{200f}i\u{200e}i\u{61c}j\u{2028}k\u{2029}l";
    assert_eq!(
        CliError::Failed(hostile.to_owned()).line(),
        "error: a\\u{a}b\\u{d}c\\u{9}d\\u{7f}e\\u{85}f\\u{202e}g\\u{2066}h\\u{200f}i\\u{200e}i\\u{61c}j\\u{2028}k\\u{2029}l",
        "control, bidi and line-separator characters are escaped"
    );
    assert_eq!(
        CliError::Failed("é ü 音".to_owned()).line(),
        "error: é ü 音",
        "other text is kept"
    );
}

#[test]
fn bounds_the_length_of_an_error_line() {
    let line = CliError::Failed("x".repeat(engine_patch::MAX_MESSAGE_CHARS + 1)).line();
    assert_eq!(
        line,
        format!(
            "error: {}…",
            "x".repeat(engine_patch::MAX_MESSAGE_CHARS - 1)
        ),
        "the message is cut, the prefix kept"
    );
}

/// An output whose reader is gone, or that fails another way.
struct Pipe(io::ErrorKind);
impl io::Write for Pipe {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::from(self.0))
    }
    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::from(self.0))
    }
}

#[test]
fn ignores_a_closed_reader_but_not_other_output_errors() {
    use io::Write as _;
    let mut closed = super::BestEffort(Pipe(io::ErrorKind::BrokenPipe));
    assert_eq!(
        closed.write(b"abc").map_err(|error| error.kind()),
        Ok(3),
        "write"
    );
    assert_eq!(
        closed.flush().map_err(|error| error.kind()),
        Ok(()),
        "flush"
    );
    let mut full = super::BestEffort(Pipe(io::ErrorKind::StorageFull));
    assert_eq!(
        full.write(b"abc").map_err(|error| error.kind()),
        Err(io::ErrorKind::StorageFull),
        "write"
    );
    assert_eq!(
        full.flush().map_err(|error| error.kind()),
        Err(io::ErrorKind::StorageFull),
        "flush"
    );
    let mut fine = super::BestEffort(Vec::new());
    assert_eq!(
        fine.write(b"ab").map_err(|error| error.kind()),
        Ok(2),
        "passes through"
    );
    assert_eq!(fine.0, b"ab", "written");
}

#[test]
fn fails_on_an_empty_corpus() {
    // A corpus without a case proves nothing: check and export refuse it.
    let mut empty = Memory::with(&[
        ("conf/cases/notes.txt", "no case here"),
        ("conf/patches/notes.txt", "no patch here"),
        ("conf/references.json", "{}"),
    ]);
    for command in ["check", "bless"] {
        assert_eq!(
            run_with(&mut empty, &[command, "conf"]).0,
            Err("no conformance case in conf/cases".to_owned()),
            "{command}"
        );
    }
    assert_eq!(
        run_with(&mut empty, &["export", "conf", "out"]).0,
        Err("no conformance case in conf/cases".to_owned()),
        "export"
    );
}
