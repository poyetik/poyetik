//! Tests of case rendering, WAV encoding and the commands, with an in-memory file system.

use alloc::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use engine_core::{RenderFormat, SampleRate};

use engine_fpenv::with_default_mode;

use super::{
    CaseError, CliError, Disk, Files, HEADER_LEN, Rendered, RenderedSamples, USAGE, arguments, run,
    wav_header,
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
    unwritable: Option<PathBuf>,
    dirs: Vec<PathBuf>,
    /// How many times each path was read.
    reads: BTreeMap<PathBuf, usize>,
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
        let count = self.reads.entry(path.to_path_buf()).or_default();
        *count = count.saturating_add(1);
        if self.unreadable.as_deref() == Some(path) {
            return Err(io::Error::other("permission denied"));
        }
        let bytes = self
            .files
            .get(path)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such file"))?;
        if bytes.len() > limit {
            return Err(io::Error::other(format!("larger than {limit} bytes")));
        }
        String::from_utf8(bytes.clone()).map_err(io::Error::other)
    }

    fn write(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        if self.read_only || self.unwritable.as_deref() == Some(path) {
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

/// Prepares a case in the default floating-point mode, as `dispatch` does.
fn prepare(case: &str, patch: &str, format: RenderFormat) -> Result<super::Prepared, CaseError> {
    with_default_mode(|mode| super::prepare_case(case, patch, format, mode))
}

/// Prepares and renders a case in the default floating-point mode, as the commands do.
fn render_case(case: &str, patch: &str, format: RenderFormat) -> Result<Rendered, CaseError> {
    with_default_mode(|mode| {
        super::prepare_case(case, patch, format, mode).map(|prepared| prepared.render(mode))
    })
}

/// The render format of `rendered`'s samples.
fn format_of(rendered: &Rendered) -> RenderFormat {
    rendered.samples.as_samples().format()
}

/// The hash of the sine case in `format`.
fn sine_hash(format: RenderFormat) -> String {
    render_case(SINE, PATCH, format)
        .expect("valid case")
        .hash()
        .expect("whole frames")
        .to_string()
}

/// A render built by hand whose samples are not whole frames is refused, never hashed:
/// by the render, and in words by the command line.
#[test]
fn refuses_to_hash_samples_that_are_not_whole_frames() {
    let rendered = Rendered {
        rate: SampleRate::HZ_48000,
        channels: 2,
        samples: RenderedSamples::Binary32(vec![0.0; 3]),
        wav_header: [0; HEADER_LEN],
    };
    let error = rendered.hash().expect_err("a cut frame");
    assert_eq!(
        (error.samples(), error.channels()),
        (3, 2),
        "what was given"
    );
    assert_eq!(
        super::cli::hash_of(&rendered),
        Err("cannot hash the render: 3 samples are not whole frames of 2 channels".to_owned()),
        "in words"
    );
    let whole = Rendered {
        samples: RenderedSamples::Binary32(vec![0.0; 4]),
        ..rendered
    };
    assert_eq!(
        super::cli::hash_of(&whole),
        Ok(whole.hash().expect("whole frames")),
        "a render is hashed"
    );
}

/// The references of the sine case, as `bless` writes them, in every format: a JSON object
/// of the hash of each format, by name.
fn sine_references() -> String {
    let hashes: Vec<String> = RenderFormat::ALL
        .iter()
        .map(|format| format!(r#""{}":"{}""#, format.name(), sine_hash(*format)))
        .collect();
    format!("{{{}}}", hashes.join(","))
}

/// The samples of a render, widened to binary64 (exactly), and its format.
fn widened(rendered: &Rendered) -> Vec<f64> {
    match &rendered.samples {
        RenderedSamples::Binary64(samples) => samples.clone(),
        RenderedSamples::Binary64AsBinary32(samples) | RenderedSamples::Binary32(samples) => {
            samples.iter().copied().map(f64::from).collect()
        }
    }
}

#[test]
fn renders_a_case_in_every_format() {
    for format in RenderFormat::ALL {
        let rendered = render_case(SINE, PATCH, format).expect("valid case");
        assert_eq!(rendered.rate(), SampleRate::HZ_48000, "rate");
        assert_eq!(rendered.channels(), 2, "dac~ is stereo");
        assert!(
            core::ptr::eq(
                core::ptr::from_ref(rendered.samples()),
                core::ptr::from_ref(&rendered.samples)
            ),
            "its own samples, by address: an equality of floats would not compare bits"
        );
        assert_eq!(format_of(&rendered), format, "its format");
        let samples = widened(&rendered);
        assert_eq!(samples.len(), 200, "100 frames x 2 channels");
        assert_eq!(samples[0].to_bits(), 0, "phase 0");
        assert_eq!(samples[2], samples[3], "same signal on both channels");
    }
    // Each format its own samples: binary64 given as binary32 is the binary64 render
    // rounded, and differs from the binary32 render, computed in binary32.
    let render = |format| render_case(SINE, PATCH, format).expect("valid case");
    let given: Vec<f64> = widened(&render(RenderFormat::Binary64))
        .into_iter()
        .map(|sample| {
            f64::from(<f32 as engine_core::host::Delivered<
                engine_core::Binary64,
            >>::deliver(
                <engine_core::Binary64 as engine_core::host::Raw>::from_raw(sample),
            ))
        })
        .collect();
    assert_eq!(
        widened(&render(RenderFormat::Binary64AsBinary32)),
        given,
        "rounded"
    );
    assert_ne!(
        widened(&render(RenderFormat::Binary32)),
        given,
        "computed in binary32"
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
        for format in RenderFormat::ALL {
            assert_eq!(
                render_case(case_text, patch_text, format).map(|_| ()),
                Err(expected.clone()),
                "{format}: {case_text} {patch_text}"
            );
        }
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
            prepare(&with_frames(frames), PATCH, RenderFormat::Binary32).map(|_| ()),
            Err(CaseError::Case(format!(
                "invalid case: frames must be between 1 and 536870912, got {frames}"
            ))),
            "{frames} frames"
        );
    }
    for format in RenderFormat::ALL {
        assert!(
            prepare(&with_frames("1"), PATCH, format).is_ok(),
            "one frame"
        );
        // The longest case passes the duration check; in stereo, only the WAV size stops it.
        assert_eq!(
            prepare(&with_frames("536870912"), PATCH, format).map(|_| ()),
            Err(CaseError::Case(
                "too long for WAV: 1073741824 samples".to_owned()
            )),
            "2^29 frames are a valid duration"
        );
    }
    assert_eq!(super::MAX_FRAMES, 1 << 29_u32, "2^29 frames");
}

#[test]
fn rejects_every_case_of_the_shared_invalid_corpus() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/invalid/cases");
    let mut count = 0_u32;
    for entry in std::fs::read_dir(&dir).expect("the invalid corpus") {
        let path = entry.expect("an entry").path();
        // Read as the commands read it, through the size limit: the corpus holds a file
        // above it.
        let refused = match Disk.read(&path, super::MAX_CASE_BYTES) {
            Err(error) => error.kind() == io::ErrorKind::FileTooLarge,
            Ok(case) => RenderFormat::ALL
                .into_iter()
                .all(|format| matches!(prepare(&case, PATCH, format), Err(CaseError::Case(_)))),
        };
        assert!(refused, "{} is rejected", path.display());
        count += 1;
    }
    assert!(count >= 8, "{count} invalid cases");
}

#[test]
fn reads_the_patch_bytes_unchanged() {
    // The same parameter written twice is refused by the patch reader; a reader that
    // went through a JSON value first would silently keep the last one.
    let repeated = PATCH.replace(
        r#"{"id":1,"box":"osc~"}"#,
        r#"{"id":1,"box":"osc~","params":{"frequency":1,"frequency":2}}"#,
    );
    let error = render_case(SINE, &repeated, RenderFormat::Binary64)
        .expect_err("repeated parameter")
        .to_string();
    assert!(
        error.contains("parameter `frequency` is given twice"),
        "{error}"
    );
}

#[test]
fn writes_a_float_wav_header() {
    let header = wav_header(48_000, 2, 6, RenderFormat::Binary32).expect("valid sizes");
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
    assert_eq!(
        wav_header(48_000, 2, 6, RenderFormat::Binary64AsBinary32),
        Ok(header),
        "binary32 samples given by a binary64 render"
    );
    let wide = wav_header(48_000, 2, 6, RenderFormat::Binary64).expect("valid sizes");
    let expected_wide: Vec<u8> = [
        &b"RIFF"[..],
        &98_u32.to_le_bytes(),
        b"WAVE",
        b"fmt ",
        &18_u32.to_le_bytes(),
        &3_u16.to_le_bytes(),
        &2_u16.to_le_bytes(),
        &48_000_u32.to_le_bytes(),
        &768_000_u32.to_le_bytes(),
        &16_u16.to_le_bytes(),
        &64_u16.to_le_bytes(),
        &0_u16.to_le_bytes(),
        b"fact",
        &4_u32.to_le_bytes(),
        &3_u32.to_le_bytes(),
        b"data",
        &48_u32.to_le_bytes(),
    ]
    .concat();
    assert_eq!(wide.to_vec(), expected_wide, "binary64: 8 bytes a sample");
    assert_eq!(HEADER_LEN, 58, "header size");
    assert_eq!(
        wav_header(48_000, 0, 0, RenderFormat::Binary64).map(|bytes| bytes[46..50].to_vec()),
        Ok(0_u32.to_le_bytes().to_vec()),
        "no channel, no frame"
    );
}

#[test]
fn rejects_sizes_beyond_the_wav_format() {
    let narrow = |channels: usize, samples: usize| {
        wav_header(48_000, channels, samples, RenderFormat::Binary32)
    };
    assert_eq!(
        narrow(70_000, 0),
        Err("too many channels for WAV: 70000".to_owned()),
        "u16 channels"
    );
    assert_eq!(
        narrow(20_000, 0),
        Err("too many channels for WAV: 20000".to_owned()),
        "block align overflow"
    );
    assert_eq!(
        wav_header(384_000, 16_000, 0, RenderFormat::Binary32),
        Err("sample rate too high for WAV: 384000".to_owned()),
        "byte rate overflow"
    );
    let too_long = |samples: usize| Err(format!("too long for WAV: {samples} samples"));
    assert_eq!(
        narrow(1, 1_usize << 30_u32),
        too_long(1_usize << 30_u32),
        "4 GiB of data"
    );
    assert_eq!(
        narrow(1, 1_073_741_812),
        too_long(1_073_741_812),
        "RIFF size overflow"
    );
    assert_eq!(
        narrow(1, 1_usize << 32_u32),
        too_long(1_usize << 32_u32),
        "beyond u32"
    );
    assert!(narrow(1, 1_073_741_811).is_ok(), "largest size that fits");
    // Binary64: 8 bytes a sample, so every limit at half the count.
    let wide = |rate: u32, channels: usize, samples: usize| {
        wav_header(rate, channels, samples, RenderFormat::Binary64)
    };
    assert_eq!(
        wide(48_000, 8_192, 0),
        Err("too many channels for WAV: 8192".to_owned()),
        "binary64 block align overflow"
    );
    assert!(wide(48_000, 8_191, 0).is_ok(), "the widest binary64 frame");
    assert_eq!(
        wide(384_000, 1_399, 0),
        Err("sample rate too high for WAV: 384000".to_owned()),
        "binary64 byte rate overflow"
    );
    assert!(
        wide(384_000, 1_398, 0).is_ok(),
        "the largest binary64 byte rate"
    );
    assert_eq!(
        wide(48_000, 1, 1_usize << 29_u32),
        too_long(1_usize << 29_u32),
        "4 GiB of binary64 data"
    );
    assert_eq!(
        wide(48_000, 1, 536_870_906),
        too_long(536_870_906),
        "binary64 RIFF size overflow"
    );
    assert!(
        wide(48_000, 1, 536_870_905).is_ok(),
        "the largest binary64 size that fits"
    );
}

#[test]
fn encodes_samples_after_their_header() {
    let mono = |samples: RenderedSamples| {
        let count = samples.as_samples().len();
        let format = samples.as_samples().format();
        Rendered {
            rate: SampleRate::HZ_48000,
            channels: 1,
            samples,
            wav_header: wav_header(48_000, 1, count, format).expect("valid sizes"),
        }
    };
    let rendered = mono(RenderedSamples::Binary32(vec![0.5, -1.0]));
    let encoded = rendered.wav();
    assert_eq!(encoded.len(), HEADER_LEN + 8, "header and two samples");
    assert_eq!(&encoded[..HEADER_LEN], &rendered.wav_header, "header first");
    assert_eq!(
        &encoded[HEADER_LEN..],
        [0.5_f32.to_le_bytes(), (-1.0_f32).to_le_bytes()].concat(),
        "little-endian f32"
    );
    let given = mono(RenderedSamples::Binary64AsBinary32(vec![0.25]));
    assert_eq!(
        given.wav()[HEADER_LEN..].to_vec(),
        0.25_f32.to_le_bytes().to_vec(),
        "binary32 samples of a binary64 render"
    );
    let wide = mono(RenderedSamples::Binary64(vec![0.1_f64, -0.0_f64]));
    let wide_bytes = wide.wav();
    assert_eq!(
        wide_bytes.len(),
        HEADER_LEN + 16,
        "header and two binary64 samples"
    );
    assert_eq!(
        &wide_bytes[34..36],
        64_u16.to_le_bytes(),
        "64 bits a sample"
    );
    assert_eq!(
        &wide_bytes[HEADER_LEN..],
        [0.1_f64.to_le_bytes(), (-0.0_f64).to_le_bytes()].concat(),
        "little-endian f64"
    );
}

#[test]
fn render_command_prints_the_hash_and_writes_wav() {
    let mut memory = Memory::with(&[("case.json", SINE), ("patch.json", PATCH)]);
    let (result, out) = run_with(&mut memory, &["render", "case.json", "patch.json"]);
    assert_eq!(
        (result, out),
        (Ok(()), format!("{}\n", sine_hash(RenderFormat::Binary64))),
        "the hash of the default format, binary64"
    );
    let (with_wav, _) = run_with(
        &mut memory,
        &["render", "case.json", "patch.json", "--wav", "out.wav"],
    );
    assert_eq!(with_wav, Ok(()), "with a WAV file");
    assert_eq!(
        memory.files[Path::new("out.wav")].len(),
        HEADER_LEN + 1600,
        "100 stereo frames of binary64"
    );
    for format in RenderFormat::ALL {
        for words in [
            &[
                "render",
                "case.json",
                "patch.json",
                "--format",
                format.name(),
            ][..],
            &[
                "render",
                "case.json",
                "patch.json",
                "--wav",
                "f.wav",
                "--format",
                format.name(),
            ],
            &[
                "render",
                "case.json",
                "patch.json",
                "--format",
                format.name(),
                "--wav",
                "f.wav",
            ],
        ] {
            assert_eq!(
                run_with(&mut memory, words),
                (Ok(()), format!("{}\n", sine_hash(format))),
                "{words:?}"
            );
        }
        let width = usize::from(format.sample_bits() / 8);
        assert_eq!(
            memory.files[Path::new("f.wav")].len(),
            HEADER_LEN + 200 * width,
            "{format}: 100 stereo frames"
        );
    }
}

/// An option's value that is itself an option is refused, never read as a file name: `--wav
/// --format` wrote a file named `--format`.
#[test]
fn render_command_never_takes_an_option_for_a_value() {
    let mut memory = Memory::with(&[("case.json", SINE), ("patch.json", PATCH)]);
    for (words, option) in [
        (
            &["render", "case.json", "patch.json", "--wav", "--format"][..],
            "--wav",
        ),
        (
            &[
                "render",
                "case.json",
                "patch.json",
                "--wav",
                "--format",
                "binary32",
            ][..],
            "--wav",
        ),
        (
            &["render", "case.json", "patch.json", "--wav", "--wav"][..],
            "--wav",
        ),
    ] {
        assert_eq!(
            run_with(&mut memory, words),
            (
                Err(usage(&format!("option \"{option}\" needs a value"))),
                String::new()
            ),
            "{words:?}"
        );
    }
    assert_eq!(
        memory.files.keys().collect::<Vec<_>>(),
        [Path::new("case.json"), Path::new("patch.json")],
        "nothing written"
    );
    assert_eq!(
        run_with(
            &mut memory,
            &["render", "case.json", "patch.json", "--wav", "-out.wav"]
        )
        .0,
        Ok(()),
        "a file name that starts with a dash but is no option is a value"
    );
    assert!(memory.files.contains_key(Path::new("-out.wav")), "written");
}

#[test]
fn render_command_reports_errors() {
    let mut memory = Memory::with(&[
        ("case.json", SINE),
        ("patch.json", PATCH),
        ("bad.json", "{"),
    ]);
    let expects =
        usage("render expects <case.json> <patch.json> [--format <format>] [--wav <file.wav>]");
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
            usage("option \"--wav\" needs a value"),
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
            &["render", "case.json", "patch.json", "--format"][..],
            usage("option \"--format\" needs a value"),
        ),
        (
            &[
                "render",
                "case.json",
                "patch.json",
                "--format",
                "--wav",
                "a.wav",
            ][..],
            usage("option \"--format\" needs a value"),
        ),
        (
            &[
                "render",
                "case.json",
                "patch.json",
                "--wav",
                "a.wav",
                "--wav",
                "b.wav",
            ][..],
            usage("option \"--wav\" given twice"),
        ),
        (
            &[
                "render",
                "case.json",
                "patch.json",
                "--format",
                "binary32",
                "--format",
                "binary64",
            ][..],
            usage("option \"--format\" given twice"),
        ),
        (
            &[
                "render",
                "case.json",
                "patch.json",
                "--format",
                "binary16",
                "--format",
                "binary64",
            ][..],
            usage("option \"--format\" given twice"),
        ),
        (
            &["render", "case.json", "patch.json", "--format", "binary16"][..],
            usage(
                "unknown render format \"binary16\" (binary64, binary64-as-binary32 or binary32)",
            ),
        ),
    ] {
        assert_eq!(run_with(&mut memory, words).0, Err(error), "{words:?}");
    }
}

#[test]
fn render_command_names_the_file_at_fault() {
    let mut memory = Memory::with(&[
        ("case.json", SINE),
        ("patch.json", PATCH),
        ("bad.json", "{"),
    ]);
    for words in [
        &["render", "missing.json", "patch.json"][..],
        &["render", "case.json", "missing.json"][..],
    ] {
        assert_eq!(
            run_with(&mut memory, words).0,
            Err("cannot read missing.json: no such file".to_owned()),
            "{words:?}"
        );
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

/// The algorithm versions of the box types of [`PATCH`], as `bless` records them for the
/// case `name`.
fn sine_algorithms(name: &str) -> String {
    format!(r#""{name}":{{"dac~":1,"osc~":1}}"#)
}

/// `json`, a JSON document, as bless writes it: indented by two spaces, keys sorted, a
/// final newline.
fn canonical(json: &str) -> String {
    format!(
        "{:#}\n",
        serde_json::from_str::<serde_json::Value>(json).expect("JSON")
    )
}

/// The fingerprint of the registered box type `name`, at its current version, through the
/// engine's instances.
fn fingerprint_of(name: &str) -> String {
    engine_battery::fingerprint::<
        engine_objects::BuiltIn<engine_core::Binary64>,
        engine_objects::BuiltIn<engine_core::Binary32>,
    >(
        engine_objects::REGISTRY
            .iter()
            .find(|spec| spec.name == name)
            .expect("a registered box type"),
    )
    .expect("its parameter sets")
}

/// The record bless writes for the algorithm versions `cases` (a JSON object of each
/// case's box types' versions): render hash 2, render rules 1, battery 1 and the
/// fingerprints of the registered box types at version 1.
fn record(cases: &str) -> String {
    canonical(&format!(
        r#"{{"cases":{cases},"fingerprint_battery":2,"fingerprints":{{"dac~":{{"1":"{}"}},"osc~":{{"1":"{}"}}}},"render_hash":"render-hash/2","render_rules":1}}"#,
        fingerprint_of("dac~"),
        fingerprint_of("osc~")
    ))
}

fn conformance(references: &str) -> Memory {
    Memory::with(&[
        ("conf/cases/sine.json", SINE),
        ("conf/patches/sine.json", PATCH),
        ("conf/cases/notes.txt", "ignored"),
        ("conf/references.json", &canonical(references)),
        (
            "conf/algorithms.json",
            &record(&format!("{{{}}}", sine_algorithms("sine"))),
        ),
    ])
}

/// `bless` records the versions and the hashes of one reading of each case: a file edited
/// while it runs cannot give versions of one text and hashes of another.
#[test]
fn bless_command_reads_each_case_once() {
    let mut memory = conformance("{}");
    drop(memory.files.remove(Path::new("conf/references.json")));
    drop(memory.files.remove(Path::new("conf/algorithms.json")));
    assert_eq!(
        run_with(&mut memory, &["bless", "conf", "--from-nothing"]).0,
        Ok(()),
        "blessed"
    );
    for path in ["conf/cases/sine.json", "conf/patches/sine.json"] {
        assert_eq!(
            memory.reads.get(Path::new(path)),
            Some(&1),
            "{path} read once"
        );
    }
}

#[test]
fn check_command_accepts_matching_references() {
    let mut memory = conformance(&format!(r#"{{"sine":{}}}"#, sine_references()));
    let (result, out) = run_with(&mut memory, &["check", "conf"]);
    assert_eq!(result, Ok(()), "all match");
    let lines = RenderFormat::ALL
        .map(|format| format!("ok       sine {format} {}\n", sine_hash(format)))
        .concat();
    assert_eq!(out, lines, "one line per case and format");
}

#[test]
fn check_command_reports_every_problem() {
    let binary32 = sine_hash(RenderFormat::Binary32);
    let mut memory = conformance(&format!(
        r#"{{"sine":{{"binary64":"0000","binary32":"{binary32}","binary16":"2222"}},"gone":{{"binary64":"1111"}}}}"#
    ));
    let (result, out) = run_with(&mut memory, &["check", "conf"]);
    assert_eq!(
        result,
        Err("conformance failed in conf: 4 problem(s)".to_owned()),
        "four problems"
    );
    assert_eq!(
        out,
        format!(
            "MISMATCH sine binary64: expected 0000, got {} (the same algorithm versions: its \
             bits changed without a new version)\n\
             MISSING  sine binary64-as-binary32: no reference hash (run `cargo xtask bless`)\n\
             ok       sine binary32 {binary32}\n\
             STALE    sine binary16: reference of no render format\n\
             STALE    gone: reference without a case\n",
            sine_hash(RenderFormat::Binary64)
        ),
        "mismatch, missing format, reference of no format and stale case"
    );
    let mut missing = conformance("{}");
    let (missing_result, missing_out) = run_with(&mut missing, &["check", "conf"]);
    assert_eq!(
        missing_result,
        Err("conformance failed in conf: 3 problem(s)".to_owned()),
        "missing references"
    );
    assert_eq!(
        missing_out,
        "MISSING  sine binary64: no reference hash (run `cargo xtask bless`)\n\
         MISSING  sine binary64-as-binary32: no reference hash (run `cargo xtask bless`)\n\
         MISSING  sine binary32: no reference hash (run `cargo xtask bless`)\n",
        "explained, format by format"
    );
}

#[test]
fn check_command_refuses_algorithm_versions_other_than_blessed() {
    let references = format!(r#"{{"sine":{}}}"#, sine_references());
    let check = |algorithms: &str, hashes: &str| {
        let mut memory = conformance(hashes);
        drop(memory.files.insert(
            PathBuf::from("conf/algorithms.json"),
            algorithms.as_bytes().to_vec(),
        ));
        run_with(&mut memory, &["check", "conf"])
    };
    let ok = RenderFormat::ALL
        .map(|format| format!("ok       sine {format} {}\n", sine_hash(format)))
        .concat();
    // A new version of a box type without new references: refused, naming both versions.
    let (bumped, bumped_out) = check(&record(r#"{"sine":{"dac~":1,"osc~":2}}"#), &references);
    assert_eq!(
        bumped,
        Err("conformance failed in conf: 1 problem(s)".to_owned()),
        "a version other than blessed"
    );
    assert_eq!(
        bumped_out,
        format!(
            "{ok}VERSION  sine osc~: at algorithm version 1, the references were blessed with \
             version 2 (check the renders of the new version, then run `cargo xtask bless`)\n"
        ),
        "explained"
    );
    // A box type without a recorded version, a version of a box type the patch does not
    // use, a case without versions, versions without a case.
    let (partial, partial_out) = check(
        &record(r#"{"sine":{"osc~":1,"env~":1},"gone":{"osc~":1}}"#),
        &references,
    );
    assert_eq!(
        partial,
        Err("conformance failed in conf: 3 problem(s)".to_owned()),
        "missing and stale versions"
    );
    assert_eq!(
        partial_out,
        format!(
            "{ok}MISSING  sine dac~: no algorithm version (run `cargo xtask bless`)\n\
             STALE    sine env~: algorithm version of a box type the case does not use\n\
             STALE    gone: algorithm versions without a case\n"
        ),
        "each named"
    );
    let (none, none_out) = check(&record("{}"), &references);
    assert_eq!(
        none,
        Err("conformance failed in conf: 1 problem(s)".to_owned()),
        "no versions"
    );
    assert_eq!(
        none_out,
        format!(
            "{ok}MISSING  sine: no algorithm versions (`cargo xtask bless --case-changed sine`, or \
             restore the record from git)\n"
        ),
        "explained"
    );
    // Bits changed under a new version: the mismatch says nothing of a missing version,
    // which the version line explains.
    let changed = format!(
        r#"{{"sine":{}}}"#,
        sine_references().replace(&sine_hash(RenderFormat::Binary32), "0000")
    );
    let (both, both_out) = check(&record(r#"{"sine":{"dac~":1,"osc~":2}}"#), &changed);
    assert_eq!(
        both,
        Err("conformance failed in conf: 2 problem(s)".to_owned()),
        "the mismatch and the version"
    );
    assert!(
        both_out.contains(&format!(
            "MISMATCH sine binary32: expected 0000, got {}\n",
            sine_hash(RenderFormat::Binary32)
        )),
        "a plain mismatch: {both_out}"
    );
    assert!(
        both_out.contains("VERSION  sine osc~: at algorithm version 1"),
        "with the version: {both_out}"
    );
    for (algorithms, error) in [
        ("[1]", "invalid conf/algorithms.json: "),
        (r#"{"sine":{"osc~":-1}}"#, "invalid conf/algorithms.json: "),
    ] {
        assert!(
            check(algorithms, &references)
                .0
                .is_err_and(|failure| failure.starts_with(error)),
            "{algorithms}"
        );
    }
    // A missing record: every_hint_leads_to_a_directory_that_checks.
}

#[test]
fn reads_the_algorithm_versions_of_a_readable_patch_only() {
    // A patch that cannot be read: its renders fail and say why, no version is compared,
    // and bless stops on it, naming the patch, before writing anything.
    let mut memory = Memory::with(&[
        ("conf/cases/sine.json", SINE),
        ("conf/patches/sine.json", "{"),
        ("conf/references.json", "{}\n"),
        ("conf/algorithms.json", &record("{}")),
    ]);
    let (result, out) = run_with(&mut memory, &["check", "conf"]);
    assert_eq!(
        result,
        Err("conformance failed in conf: 3 problem(s)".to_owned()),
        "one failed render per format, no version line"
    );
    assert!(
        out.lines().all(|line| line.starts_with("FAILED   sine ")),
        "{out}"
    );
    let (blessed, _) = run_with(&mut memory, &["bless", "conf"]);
    assert!(
        blessed.is_err_and(|error| error.starts_with("conf/patches/sine.json: ")),
        "bless names the patch"
    );
    assert_eq!(
        memory.text("conf/algorithms.json"),
        record("{}"),
        "nothing written"
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
        Err(
            "cannot read conf/references.json: no such file (restore conf/references.json from \
             git; a new directory: `cargo xtask bless --from-nothing`)"
                .to_owned()
        ),
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
            &canonical(&format!(
                r#"{{"a":{{}},"b":{{}},"sine":{}}}"#,
                sine_references()
            )),
        ),
        (
            "conf/algorithms.json",
            &record(&format!(
                "{{{},{}}}",
                sine_algorithms("a"),
                sine_algorithms("sine")
            )),
        ),
    ]);
    let (result, out) = run_with(&mut memory, &["check", "conf"]);
    assert_eq!(
        result,
        Err("conformance failed in conf: 6 problem(s)".to_owned()),
        "both failures counted, once per format each"
    );
    let failed = |name: &str, reason: &str| -> String {
        RenderFormat::ALL
            .map(|format| format!("FAILED   {name} {format}: {reason}\n"))
            .concat()
    };
    let ok = RenderFormat::ALL
        .map(|format| format!("ok       sine {format} {}\n", sine_hash(format)))
        .concat();
    assert_eq!(
        out,
        format!(
            "{}{}{ok}",
            failed(
                "a",
                "conf/cases/a.json: invalid case: EOF while parsing an object at line 1 column 1"
            ),
            failed("b", "conf/patches/b.json: box 1: unknown box type \"foo~\""),
        ),
        "every case reported in every format, in order"
    );
    let mut broken = Memory::with(&[
        ("conf/cases/a.json", "{"),
        ("conf/patches/a.json", PATCH),
        ("conf/references.json", "{}\n"),
        ("conf/algorithms.json", &record("{}")),
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
    let (result, out) = run_with(&mut memory, &["bless", "conf", "--from-nothing"]);
    assert_eq!(result, Ok(()), "blessed");
    assert_eq!(
        out, "blessed 2 case(s) in conf/references.json and conf/algorithms.json\n",
        "summary"
    );
    let hashes = format!(
        "{{\n    \"binary32\": \"{}\",\n    \"binary64\": \"{}\",\n    \"binary64-as-binary32\": \"{}\"\n  }}",
        sine_hash(RenderFormat::Binary32),
        sine_hash(RenderFormat::Binary64),
        sine_hash(RenderFormat::Binary64AsBinary32)
    );
    assert_eq!(
        memory.text("conf/references.json"),
        format!("{{\n  \"a\": {hashes},\n  \"sine\": {hashes}\n}}\n"),
        "sorted JSON: cases, then the names of the formats"
    );
    let versions = "{\n      \"dac~\": 1,\n      \"osc~\": 1\n    }";
    assert_eq!(
        memory.text("conf/algorithms.json"),
        format!(
            "{{\n  \"cases\": {{\n    \"a\": {versions},\n    \"sine\": {versions}\n  }},\n  \
             \"fingerprint_battery\": 2,\n  \"fingerprints\": {{\n    \"dac~\": {{\n      \
             \"1\": \"{}\"\n    }},\n    \"osc~\": {{\n      \"1\": \"{}\"\n    }}\n  }},\n  \
             \"render_hash\": \"render-hash/2\",\n  \"render_rules\": 1\n}}\n",
            fingerprint_of("dac~"),
            fingerprint_of("osc~")
        ),
        "the versions of every box type of every case, the fingerprints, the render hash and \
         rules, sorted"
    );
    assert_eq!(
        run_with(&mut memory, &["check", "conf"]).0,
        Ok(()),
        "check agrees"
    );
    memory.unwritable = Some(PathBuf::from("conf/algorithms.json"));
    assert_eq!(
        run_with(&mut memory, &["bless", "conf"]).0,
        Err(
            "cannot write conf/algorithms.json: read-only (conf/references.json was written: \
             run `cargo xtask bless` again)"
                .to_owned()
        ),
        "unwritable algorithm versions, the references written"
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
fn bless_command_refuses_bits_changed_without_a_new_version() {
    let blessed = conformance(&format!(r#"{{"sine":{}}}"#, sine_references()));
    let references = blessed.text("conf/references.json");
    let algorithms = blessed.text("conf/algorithms.json");
    // A reference that changed under the same versions: refused, nothing written.
    let moved = canonical(&format!(
        r#"{{"sine":{}}}"#,
        sine_references().replace(&sine_hash(RenderFormat::Binary32), "0000")
    ));
    let mut memory = conformance(&moved);
    let (result, out) = run_with(&mut memory, &["bless", "conf"]);
    assert_eq!(
        result,
        Err("bless refused in conf: 1 problem(s), nothing written".to_owned()),
        "refused"
    );
    assert_eq!(
        out,
        "CHANGED  sine binary32: its reference changed under the same algorithm versions (a \
         new version, or `cargo xtask bless --case-changed sine`)\n",
        "named"
    );
    assert_eq!(
        memory.text("conf/references.json"),
        moved,
        "nothing written"
    );
    // Named as changed on purpose: blessed.
    let (named, _) = run_with(&mut memory, &["bless", "conf", "--case-changed", "sine"]);
    assert_eq!(named, Ok(()), "the case named");
    assert_eq!(memory.text("conf/references.json"), references, "rewritten");
    assert_eq!(
        memory.text("conf/algorithms.json"),
        algorithms,
        "the same record"
    );
    // A name that is no case is refused.
    let (unknown, unknown_out) =
        run_with(&mut memory, &["bless", "conf", "--case-changed", "nine"]);
    assert_eq!(
        (unknown, unknown_out.as_str()),
        (
            Err("bless refused in conf: 1 problem(s), nothing written".to_owned()),
            "UNKNOWN  --case-changed nine: no such case\n"
        ),
        "a name of no case"
    );
    // A fingerprint changed at a recorded version: refused by check and bless alike.
    let changed = algorithms.replace(&fingerprint_of("osc~"), &"0".repeat(64));
    drop(memory.files.insert(
        PathBuf::from("conf/algorithms.json"),
        changed.as_bytes().to_vec(),
    ));
    let bits = "BITS     osc~ 1: its outputs changed at the same algorithm version (a new version \
                needed)\n";
    let (checked, checked_out) = run_with(&mut memory, &["check", "conf"]);
    assert_eq!(
        checked,
        Err("conformance failed in conf: 1 problem(s)".to_owned()),
        "check refuses it"
    );
    assert!(checked_out.ends_with(bits), "{checked_out}");
    let (refused, refused_out) =
        run_with(&mut memory, &["bless", "conf", "--case-changed", "sine"]);
    assert_eq!(
        (refused, refused_out.as_str()),
        (
            Err("bless refused in conf: 1 problem(s), nothing written".to_owned()),
            bits
        ),
        "bless refuses it, whatever the case named"
    );
}

#[test]
fn bless_command_spends_only_the_versions_of_the_merge_base() {
    let mut memory = conformance(&format!(r#"{{"sine":{}}}"#, sine_references()));
    let algorithms = memory.text("conf/algorithms.json");
    let changed = algorithms.replace(&fingerprint_of("osc~"), &"0".repeat(64));
    drop(memory.files.insert(
        PathBuf::from("conf/algorithms.json"),
        changed.as_bytes().to_vec(),
    ));
    let bits = "BITS     osc~ 1: its outputs changed at the same algorithm version (a new version \
                needed)\n";
    // A version the merge base did not spend may take other bits; one it spent may not.
    drop(
        memory
            .files
            .insert(PathBuf::from("base.json"), b"null\n".to_vec()),
    );
    let (unspent, _) = run_with(
        &mut memory,
        &[
            "bless",
            "conf",
            "--case-changed",
            "sine",
            "--spent",
            "base.json",
        ],
    );
    assert_eq!(unspent, Ok(()), "nothing spent at the merge base");
    assert_eq!(
        memory.text("conf/algorithms.json"),
        algorithms,
        "re-blessed"
    );
    drop(
        memory
            .files
            .insert(PathBuf::from("base.json"), changed.as_bytes().to_vec()),
    );
    assert_eq!(
        run_with(&mut memory, &["bless", "conf", "--spent", "base.json"]),
        (
            Err("bless refused in conf: 1 problem(s), nothing written".to_owned()),
            bits.to_owned()
        ),
        "spent at the merge base"
    );
    assert_eq!(
        run_with(&mut memory, &["bless", "conf", "--spent", "none.json"]).0,
        Err(
            "cannot read the spent record none.json: no such file (`cargo xtask bless` writes \
             it: run that)"
                .to_owned()
        ),
        "a record of the merge base to read"
    );
    drop(
        memory
            .files
            .insert(PathBuf::from("base.json"), b"[1]".to_vec()),
    );
    assert!(
        run_with(&mut memory, &["bless", "conf", "--spent", "base.json"])
            .0
            .is_err_and(
                |error| error.starts_with("invalid spent record base.json: ")
                    && error.ends_with(" (`cargo xtask bless` writes it: run that)")
            ),
        "an invalid record of the merge base"
    );
}

/// Files bless cannot compare with are refused; a first bless, with neither, only when said.
#[test]
fn bless_command_refuses_files_it_cannot_compare_with() {
    let mut memory = conformance(&format!(r#"{{"sine":{}}}"#, sine_references()));
    let algorithms = memory.text("conf/algorithms.json");
    let references = memory.text("conf/references.json");
    // An invalid record and the hints of every refusal:
    // every_hint_leads_to_a_directory_that_checks.
    memory.unreadable = Some(PathBuf::from("conf/references.json"));
    assert_eq!(
        run_with(&mut memory, &["bless", "conf"]).0,
        Err("cannot read conf/references.json: permission denied".to_owned()),
        "unreadable references"
    );
    memory.unreadable = Some(PathBuf::from("conf/algorithms.json"));
    assert_eq!(
        run_with(&mut memory, &["bless", "conf"]).0,
        Err("cannot read conf/algorithms.json: permission denied".to_owned()),
        "unreadable record"
    );
    memory.unreadable = None;
    let refused = Err("bless refused in conf: 1 problem(s), nothing written".to_owned());
    drop(memory.files.remove(Path::new("conf/algorithms.json")));
    for told in [&["bless", "conf"][..], &["bless", "conf", "--from-nothing"]] {
        assert_eq!(
            run_with(&mut memory, told),
            (
                refused.clone(),
                "MISSING  conf/algorithms.json: missing beside conf/references.json (restore \
                 conf/algorithms.json from git: bless compares with both)\n"
                    .to_owned()
            ),
            "the record alone missing: {told:?}"
        );
    }
    drop(memory.files.insert(
        PathBuf::from("conf/algorithms.json"),
        algorithms.clone().into_bytes(),
    ));
    drop(memory.files.remove(Path::new("conf/references.json")));
    assert_eq!(
        run_with(&mut memory, &["bless", "conf"]),
        (
            refused.clone(),
            "MISSING  conf/references.json: missing beside conf/algorithms.json (restore \
             conf/references.json from git: bless compares with both)\n"
                .to_owned()
        ),
        "the references alone missing"
    );
    drop(memory.files.remove(Path::new("conf/algorithms.json")));
    assert_eq!(
        run_with(&mut memory, &["bless", "conf", "--case-changed", "typo"]),
        (
            refused.clone(),
            "MISSING  conf/references.json and conf/algorithms.json: none to compare with (a \
             first bless: `cargo xtask bless --from-nothing`)\n"
                .to_owned()
        ),
        "neither, unsaid"
    );
    assert_eq!(
        run_with(
            &mut memory,
            &["bless", "conf", "--from-nothing", "--case-changed", "typo"]
        ),
        (
            refused.clone(),
            "UNKNOWN  --case-changed typo: no such case\n".to_owned()
        ),
        "the names checked from nothing too"
    );
    assert_eq!(
        run_with(&mut memory, &["bless", "conf", "--from-nothing"]).0,
        Ok(()),
        "from nothing, said"
    );
    assert_eq!(memory.text("conf/algorithms.json"), algorithms, "recorded");
    assert_eq!(
        memory.text("conf/references.json"),
        references,
        "referenced"
    );
    assert_eq!(
        run_with(&mut memory, &["bless", "conf", "--from-nothing"]),
        (
            refused,
            "OPTION   --from-nothing: conf/references.json and conf/algorithms.json exist, to \
             compare with (run `cargo xtask bless` without it)\n"
                .to_owned()
        ),
        "from nothing, with both to compare with"
    );
}

/// The conformance directory as committed: the sine case, its references and its record.
fn committed() -> Memory {
    conformance(&format!(r#"{{"sine":{}}}"#, sine_references()))
}

/// What `cargo xtask <command> <options>` runs: `engine-cli <command> conf --spent
/// spent.json`, the record on main written to `spent.json` first (here, the committed one),
/// then the options.
fn xtask(memory: &mut Memory, command: &str, options: &[&str]) -> (Result<(), String>, String) {
    drop(memory.files.insert(
        PathBuf::from("spent.json"),
        committed().files[Path::new("conf/algorithms.json")].clone(),
    ));
    let words: Vec<&str> = [command, "conf", "--spent", "spent.json"]
        .into_iter()
        .chain(options.iter().copied())
        .collect();
    run_with(memory, &words)
}

/// What `cargo xtask bless <options>` runs ([`xtask`]).
fn xtask_bless(memory: &mut Memory, options: &[&str]) -> (Result<(), String>, String) {
    xtask(memory, "bless", options)
}

/// Does what `line` says to do, the first of its alternatives: run the `cargo xtask`
/// command it names (after removing the file it says to remove first), or restore the file
/// it names from git (its committed contents).
fn follow(memory: &mut Memory, line: &str) {
    let command = line.find("`cargo xtask ");
    let restore = line.find("restore ");
    if let Some(start) = command.filter(|start| restore.is_none_or(|other| *start < other)) {
        if let Some((_, removed)) = line.split_once("remove ") {
            let path = removed.split(',').next().expect("a path");
            assert!(
                memory.files.remove(Path::new(path)).is_some(),
                "{line}: {path} to remove"
            );
        }
        let named = line
            .get(start..)
            .and_then(|from| from.split('`').nth(1))
            .expect("a closed command");
        let words: Vec<&str> = named.split_whitespace().skip(2).collect();
        let (task, options) = words.split_first().expect("a command of xtask");
        assert_eq!(xtask(memory, task, options).0, Ok(()), "{line}: followed");
    } else {
        let start = restore.expect("a hint");
        let path = line
            .get(start..)
            .expect("the hint")
            .trim_start_matches("restore ")
            .split(' ')
            .next()
            .expect("a path");
        drop(memory.files.insert(
            PathBuf::from(path),
            committed().files[Path::new(path)].clone(),
        ));
    }
}

/// How a test brings a conformance directory into a state and runs the command that
/// refuses it: what it returned and printed.
type Setup = fn(&mut Memory) -> (Result<(), String>, String);

/// Every state a command refuses with a hint: its name, how it is reached, and the end of
/// the line that gives the hint, exactly.
const HINTED: [(&str, Setup, &str); 15] = [
    (
        "references missing",
        |memory| {
            drop(memory.files.remove(Path::new("conf/references.json")));
            run_with(memory, &["check", "conf"])
        },
        "cannot read conf/references.json: no such file (restore conf/references.json \
         from git; a new directory: `cargo xtask bless --from-nothing`)",
    ),
    (
        "record missing",
        |memory| {
            drop(memory.files.remove(Path::new("conf/algorithms.json")));
            run_with(memory, &["check", "conf"])
        },
        "cannot read conf/algorithms.json: no such file (restore conf/algorithms.json \
         from git; a new directory: `cargo xtask bless --from-nothing`)",
    ),
    (
        "both missing, at bless",
        |memory| {
            drop(memory.files.remove(Path::new("conf/references.json")));
            drop(memory.files.remove(Path::new("conf/algorithms.json")));
            xtask_bless(memory, &[])
        },
        "MISSING  conf/references.json and conf/algorithms.json: none to compare with (a \
         first bless: `cargo xtask bless --from-nothing`)",
    ),
    (
        "references alone missing, at bless",
        |memory| {
            drop(memory.files.remove(Path::new("conf/references.json")));
            xtask_bless(memory, &[])
        },
        "MISSING  conf/references.json: missing beside conf/algorithms.json (restore \
         conf/references.json from git: bless compares with both)",
    ),
    (
        "invalid references",
        |memory| {
            drop(
                memory
                    .files
                    .insert(PathBuf::from("conf/references.json"), b"[1".to_vec()),
            );
            run_with(memory, &["check", "conf"])
        },
        "(restore conf/references.json from git)",
    ),
    (
        "invalid record, at bless",
        |memory| {
            drop(
                memory
                    .files
                    .insert(PathBuf::from("conf/algorithms.json"), b"[1".to_vec()),
            );
            xtask_bless(memory, &[])
        },
        "(bless compares with it: restore conf/algorithms.json from git)",
    ),
    (
        "references not as bless writes them",
        |memory| {
            let text = format!("{} ", memory.text("conf/references.json"));
            drop(
                memory
                    .files
                    .insert(PathBuf::from("conf/references.json"), text.into_bytes()),
            );
            run_with(memory, &["check", "conf"])
        },
        "conf/references.json is not as bless writes it (run `cargo xtask bless`)",
    ),
    (
        "a case with references and no versions",
        |memory| {
            drop(memory.files.insert(
                PathBuf::from("conf/algorithms.json"),
                record("{}").into_bytes(),
            ));
            run_with(memory, &["check", "conf"])
        },
        "MISSING  sine: no algorithm versions (`cargo xtask bless --case-changed sine`, or \
         restore the record from git)",
    ),
    (
        "a new case: no references and no versions",
        |memory| {
            drop(memory.files.insert(
                PathBuf::from("conf/algorithms.json"),
                record("{}").into_bytes(),
            ));
            drop(
                memory
                    .files
                    .insert(PathBuf::from("conf/references.json"), b"{}\n".to_vec()),
            );
            run_with(memory, &["check", "conf"])
        },
        "MISSING  sine: no algorithm versions (run `cargo xtask bless`)",
    ),
    (
        "a format without a reference",
        |memory| {
            let references = canonical(&format!(
                r#"{{"sine":{}}}"#,
                sine_references().replace(
                    &format!(r#","binary32":"{}""#, sine_hash(RenderFormat::Binary32)),
                    ""
                )
            ));
            drop(memory.files.insert(
                PathBuf::from("conf/references.json"),
                references.into_bytes(),
            ));
            run_with(memory, &["check", "conf"])
        },
        "MISSING  sine binary32: no reference hash (run `cargo xtask bless`)",
    ),
    (
        "another battery",
        |memory| {
            let text = memory
                .text("conf/algorithms.json")
                .replace(r#""fingerprint_battery": 2"#, r#""fingerprint_battery": 1"#);
            drop(
                memory
                    .files
                    .insert(PathBuf::from("conf/algorithms.json"), text.into_bytes()),
            );
            run_with(memory, &["check", "conf"])
        },
        "BATTERY  the fingerprints were computed with battery 1, the engine's is 2 (run \
         `cargo xtask bless --battery-changed`)",
    ),
    (
        "a first bless that wrote the references alone",
        |memory| {
            drop(memory.files.remove(Path::new("conf/references.json")));
            drop(memory.files.remove(Path::new("conf/algorithms.json")));
            memory.unwritable = Some(PathBuf::from("conf/algorithms.json"));
            let refused = xtask_bless(memory, &["--from-nothing"]);
            memory.unwritable = None;
            refused
        },
        "cannot write conf/algorithms.json: read-only (conf/references.json was written: \
         remove conf/references.json, then run `cargo xtask bless --from-nothing`)",
    ),
    (
        "a bless that wrote the references alone",
        |memory| {
            memory.unwritable = Some(PathBuf::from("conf/algorithms.json"));
            let refused = xtask_bless(memory, &[]);
            memory.unwritable = None;
            refused
        },
        "cannot write conf/algorithms.json: read-only (conf/references.json was written: \
         run `cargo xtask bless` again)",
    ),
    (
        "an invalid spent record",
        |memory| {
            drop(
                memory
                    .files
                    .insert(PathBuf::from("spent.json"), b"[1".to_vec()),
            );
            run_with(memory, &["bless", "conf", "--spent", "spent.json"])
        },
        "(`cargo xtask bless` writes it: run that)",
    ),
    (
        "an invalid spent record, at check",
        |memory| {
            drop(
                memory
                    .files
                    .insert(PathBuf::from("spent.json"), b"null ]".to_vec()),
            );
            run_with(memory, &["check", "conf", "--spent", "spent.json"])
        },
        "(`cargo xtask check` writes it: run that)",
    ),
];

/// Every state a command refuses with a hint, the hint asserted as written, then followed:
/// what it says to run or restore succeeds, and the directory checks again.
#[test]
fn every_hint_leads_to_a_directory_that_checks() {
    for (state, setup, hint) in HINTED {
        let mut memory = committed();
        let (result, out) = setup(&mut memory);
        let said: Vec<String> = result
            .err()
            .into_iter()
            .chain(out.lines().map(str::to_owned))
            .collect();
        let line = said
            .iter()
            .find(|line| line.ends_with(hint))
            .unwrap_or_else(|| panic!("{state}: {hint} in {said:?}"));
        follow(&mut memory, line);
        assert_eq!(
            run_with(&mut memory, &["check", "conf"]).0,
            Ok(()),
            "{state}: checks once followed"
        );
    }
}

#[test]
fn check_command_compares_the_fingerprints_with_the_record_on_main() {
    let mut memory = committed();
    assert_eq!(
        xtask(&mut memory, "check", &[]).0,
        Ok(()),
        "the record on main itself"
    );
    drop(
        memory
            .files
            .insert(PathBuf::from("null.json"), b"null\n".to_vec()),
    );
    assert_eq!(
        run_with(&mut memory, &["check", "conf", "--spent", "null.json"]).0,
        Ok(()),
        "no record on main"
    );
    let osc = fingerprint_of("osc~");
    let main = record(&format!("{{{}}}", sine_algorithms("sine"))).replace(&osc, "0000");
    drop(
        memory
            .files
            .insert(PathBuf::from("main.json"), main.into_bytes()),
    );
    let (result, out) = run_with(&mut memory, &["check", "conf", "--spent", "main.json"]);
    assert_eq!(
        result,
        Err("conformance failed in conf: 1 problem(s)".to_owned()),
        "a version spent on main given other bits"
    );
    assert!(
        out.ends_with(
            "BITS     osc~ 1: other bits than the record on main holds at this spent version (a \
             new version of osc~, then run `cargo xtask bless`)\n"
        ),
        "{out}"
    );
    assert_eq!(
        run_with(&mut memory, &["check", "conf", "--spent", "gone.json"]).0,
        Err(
            "cannot read the spent record gone.json: no such file (`cargo xtask check` writes \
             it: run that)"
                .to_owned()
        ),
        "a record on main to read"
    );
    for words in [
        &["check", "conf", "--spent"][..],
        &["check", "conf", "--other", "main.json"],
        &[
            "check",
            "conf",
            "--spent",
            "main.json",
            "--spent",
            "main.json",
        ],
    ] {
        assert_eq!(
            run_with(&mut memory, words).0,
            Err(usage("check expects <dir> [--spent <record>]")),
            "{words:?}"
        );
    }
}

#[test]
fn check_command_names_a_mismatch_under_other_render_rules_as_such() {
    // Under other render rules, a mismatch is not said to be bits changed without a new
    // version: the rules version line explains it.
    let mut memory = conformance(&format!(
        r#"{{"sine":{}}}"#,
        sine_references().replace(&sine_hash(RenderFormat::Binary32), "0000")
    ));
    let record = memory
        .text("conf/algorithms.json")
        .replace("\"render_rules\": 1", "\"render_rules\": 0");
    drop(
        memory
            .files
            .insert(PathBuf::from("conf/algorithms.json"), record.into_bytes()),
    );
    let (result, out) = run_with(&mut memory, &["check", "conf"]);
    assert_eq!(
        result,
        Err("conformance failed in conf: 2 problem(s)".to_owned()),
        "the mismatch and the rules"
    );
    assert!(
        out.contains(&format!(
            "MISMATCH sine binary32: expected 0000, got {}\n",
            sine_hash(RenderFormat::Binary32)
        )),
        "a plain mismatch: {out}"
    );
    assert!(
        out.ends_with(
            "RULES    the references were rendered under render rules version 0, the engine's is \
             1 (run `cargo xtask bless`)\n"
        ),
        "{out}"
    );
}

#[test]
fn check_command_reads_its_files_only_as_bless_writes_them() {
    let mut memory = conformance(&format!(r#"{{"sine":{}}}"#, sine_references()));
    let references = memory.text("conf/references.json");
    for (file, text) in [
        ("conf/references.json", references.replace('\n', "")),
        (
            "conf/references.json",
            references.replacen("{\n", "{\n  \"sine\": {},\n", 1),
        ),
        (
            "conf/algorithms.json",
            format!("{} ", memory.text("conf/algorithms.json")),
        ),
    ] {
        let mut edited = conformance(&format!(r#"{{"sine":{}}}"#, sine_references()));
        drop(edited.files.insert(PathBuf::from(file), text.into_bytes()));
        assert_eq!(
            run_with(&mut edited, &["check", "conf"]).0,
            Err(format!(
                "{file} is not as bless writes it (run `cargo xtask bless`)"
            )),
            "{file}: a hand edit or a repeated key"
        );
    }
    assert_eq!(
        run_with(&mut memory, &["check", "conf"]).0,
        Ok(()),
        "as written"
    );
    memory.unreadable = Some(PathBuf::from("conf/algorithms.json"));
    assert_eq!(
        run_with(&mut memory, &["check", "conf"]).0,
        Err("cannot read conf/algorithms.json: permission denied".to_owned()),
        "no hint where bless would not help"
    );
}

#[test]
fn rejects_unknown_commands() {
    for (words, reason) in [
        (&[][..], "missing command"),
        (&["play", "x"][..], "unknown command \"play\""),
        (&["play"][..], "unknown command \"play\""),
        (&["check"][..], "check expects <dir> [--spent <record>]"),
        (
            &["bless"][..],
            "bless expects <dir> [--case-changed <name>]... [--spent <record>] \
             [--battery-changed] [--from-nothing]",
        ),
        (&["bless", "a", "b"][..], "unknown option \"b\""),
        (
            &["bless", "a", "--case-changed", "x", "--case-changed"][..],
            "bless expects <dir> [--case-changed <name>]... [--spent <record>] \
             [--battery-changed] [--from-nothing]",
        ),
        (
            &["bless", "a", "--spent"][..],
            "bless expects <dir> [--case-changed <name>]... [--spent <record>] \
             [--battery-changed] [--from-nothing]",
        ),
        (
            &["bless", "a", "--case", "x"][..],
            "unknown option \"--case\"",
        ),
        (
            &["bless", "a", "--spent", "x", "--spent", "x"][..],
            "option \"--spent\" given twice",
        ),
        (
            &["bless", "a", "--battery-changed", "--battery-changed"][..],
            "option \"--battery-changed\" given twice",
        ),
        (
            &[
                "bless",
                "a",
                "--from-nothing",
                "--case-changed",
                "x",
                "--from-nothing",
            ][..],
            "option \"--from-nothing\" given twice",
        ),
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

/// The usage is a constant, so it cannot be built from `RenderFormat::ALL`: this test
/// compares the two copies, and the default.
#[test]
fn the_usage_names_every_render_format_and_the_default() {
    let usage = USAGE.split_whitespace().collect::<Vec<_>>().join(" ");
    for format in RenderFormat::ALL {
        assert!(
            usage.contains(&format!("{} ", format.name()))
                || usage.contains(&format!("{},", format.name()))
                || usage.contains(&format!("{})", format.name())),
            "{format} is named"
        );
    }
    assert!(
        usage.contains("(binary64, the default,"),
        "binary64 is the default"
    );
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
    let mut memory = conformance(&format!(r#"{{"sine":{}}}"#, sine_references()));
    assert_eq!(
        run(&args(&["check", "conf"]), &mut memory, &mut Broken).map_err(|error| error.to_string()),
        Err("cannot write to stdout: closed".to_owned()),
        "check"
    );
    let mut bad = conformance(r#"{"sine":{"binary64":"0"}}"#);
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
            &canonical(&format!(r#"{{"sine":{},"z":{{}}}}"#, sine_references())),
        ),
        (
            "conf/algorithms.json",
            &record(&format!("{{{}}}", sine_algorithms("sine"))),
        ),
    ]);
    let mut three = Limited(3);
    assert_eq!(
        run(&args(&["check", "conf"]), &mut stale, &mut three).map_err(|error| error.to_string()),
        Err("cannot write to stdout: closed".to_owned()),
        "stale line"
    );
    let mut unknown_format = conformance(&format!(
        r#"{{"sine":{}}}"#,
        sine_references().replace("binary32\"", "binary16\"")
    ));
    let mut three_more = Limited(3);
    assert_eq!(
        run(
            &args(&["check", "conf"]),
            &mut unknown_format,
            &mut three_more
        )
        .map_err(|error| error.to_string()),
        Err("cannot write to stdout: closed".to_owned()),
        "a line on a reference of no render format"
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

/// A directory of one test below the system's temporary directory, removed with
/// everything in it when the guard is dropped: when the test returns or unwinds, a failing
/// test or a panicking mutant included. A test process killed (a mutant stopped at its
/// timeout) or aborted runs no drop and leaves it; the next test of the same name in a
/// process of the same id removes it when it starts.
#[derive(Debug)]
#[must_use = "the directory is removed when the guard is dropped"]
struct Scratch(PathBuf);

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

/// A fresh, empty directory for one test, `engine-cli-<name>-<process id>` ([`Scratch`]).
#[expect(
    clippy::disallowed_methods,
    reason = "the one place engine-cli's unit tests name the temporary directory: the guard removes what it \
              creates"
)]
fn scratch(name: &str) -> Scratch {
    let dir = std::env::temp_dir().join(format!("engine-cli-{name}-{}", std::process::id()));
    if let Err(error) = std::fs::remove_dir_all(&dir) {
        assert_eq!(error.kind(), io::ErrorKind::NotFound, "clean start");
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
        .downcast::<PathBuf>()
        .expect("the path");
    assert!(!left.exists(), "removed when the test unwinds");
}

#[test]
fn disk_reads_writes_and_lists() {
    let dir = scratch("disk");
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
    let hashes = canonical(&format!(r#"{{"c":{}}}"#, sine_references()));
    let references = format!(
        "{hashes}{}",
        " ".repeat(super::MAX_REFERENCES_BYTES - hashes.len())
    );
    let recorded = record(&format!("{{{}}}", sine_algorithms("c")));
    let algorithms = format!(
        "{recorded}{}",
        " ".repeat(super::MAX_REFERENCES_BYTES - recorded.len())
    );
    let mut memory = Memory::with(&[
        ("c.json", &case),
        ("p.json", &patch),
        ("conf/cases/c.json", &case),
        ("conf/patches/c.json", &patch),
        ("conf/references.json", &hashes),
        ("conf/algorithms.json", &recorded),
    ]);
    assert_eq!(
        run_with(&mut memory, &["render", "c.json", "p.json"]).0,
        Ok(()),
        "exactly at the limits"
    );
    assert_eq!(
        run_with(&mut memory, &["check", "conf"]).0,
        Ok(()),
        "the case and the patch of a corpus at their limit"
    );
    // The files bless writes are read at their limit (then refused, padded, since they
    // are not as bless writes them).
    for (file, padded) in [
        ("conf/references.json", &references),
        ("conf/algorithms.json", &algorithms),
    ] {
        let mut at_limit = conformance(&format!(r#"{{"c":{}}}"#, sine_references()));
        drop(
            at_limit
                .files
                .insert(PathBuf::from(file), padded.clone().into_bytes()),
        );
        assert_eq!(
            run_with(&mut at_limit, &["check", "conf"]).0,
            Err(format!(
                "{file} is not as bless writes it (run `cargo xtask bless`)"
            )),
            "{file}: read at its limit"
        );
    }
    for (file, limit, full) in [
        ("conf/cases/c.json", super::MAX_CASE_BYTES, &case),
        ("conf/patches/c.json", engine_patch::MAX_PATCH_BYTES, &patch),
        (
            "conf/references.json",
            super::MAX_REFERENCES_BYTES,
            &references,
        ),
        (
            "conf/algorithms.json",
            super::MAX_REFERENCES_BYTES,
            &algorithms,
        ),
    ] {
        let mut over = Memory::with(&[
            ("conf/cases/c.json", &case),
            ("conf/patches/c.json", &patch),
            ("conf/references.json", &hashes),
            ("conf/algorithms.json", &recorded),
        ]);
        drop(
            over.files
                .insert(PathBuf::from(file), format!("{full} ").into_bytes()),
        );
        assert_eq!(
            run_with(&mut over, &["check", "conf"]).0,
            Err(format!("cannot read {file}: larger than {limit} bytes")),
            "{file}: one byte over"
        );
    }
}

#[test]
fn prepares_a_case_before_rendering() {
    for format in RenderFormat::ALL {
        let prepared = prepare(SINE, PATCH, format).expect("valid case");
        assert_eq!(prepared.sample_count(), 200, "100 frames x 2 channels");
        let rendered = with_default_mode(|mode| prepared.render(mode));
        assert_eq!(
            (rendered.channels, format_of(&rendered)),
            (2, format),
            "its channels and format"
        );
    }
}

#[test]
fn refuses_a_case_too_long_for_a_wav_file_before_rendering() {
    // Only prepares the case: a broken size check fails at once instead of rendering.
    // The longest stereo case, 2^29 frames, is 4 GiB of samples: one byte too many.
    let huge = SINE.replace(r#""frames":100"#, r#""frames":536870912"#);
    assert_ne!(huge, SINE, "the duration was replaced");
    for format in RenderFormat::ALL {
        assert_eq!(
            prepare(&huge, PATCH, format).map(|prepared| prepared.sample_count()),
            Err(CaseError::Case(
                "too long for WAV: 1073741824 samples".to_owned()
            )),
            "rejected while preparing"
        );
        let prepared = prepare(SINE, PATCH, format).expect("valid case");
        assert_eq!(
            with_default_mode(|mode| prepared.render(mode)).wav()[..HEADER_LEN],
            wav_header(48_000, 2, 200, format).expect("valid sizes"),
            "header kept"
        );
    }
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
    let mut lines = Vec::new();
    for (name, patch) in [
        ("a", PATCH.to_owned()),
        ("b", memory.text("conf/patches/b.json")),
    ] {
        for format in RenderFormat::ALL {
            let rendered = render_case(SINE, &patch, format).expect("valid case");
            let path = format!("out/wav/{name}.{format}.wav");
            assert_eq!(memory.files[Path::new(&path)], rendered.wav(), "{path}");
            lines.push(format!(
                "wrote    {path} {}\n",
                rendered.hash().expect("whole frames")
            ));
        }
    }
    assert_ne!(
        memory.files[Path::new("out/wav/a.binary64.wav")],
        memory.files[Path::new("out/wav/b.binary64.wav")],
        "two different cases"
    );
    assert_eq!(
        out,
        lines.concat(),
        "sorted by name, then in the order of the formats"
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
        Err("cannot write out/a.binary64.wav: read-only".to_owned()),
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
    for (refused, what) in [(missing, "no cases"), (unreadable, "an unreadable case")] {
        assert_eq!(
            refused.dirs,
            Vec::<PathBuf>::new(),
            "{what}: the corpus is listed and read before the output directory is created"
        );
    }
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
    for length in [
        engine_patch::MAX_MESSAGE_CHARS,
        engine_patch::MAX_MESSAGE_CHARS + 1,
    ] {
        let reason = "x".repeat(length);
        let bounded = engine_patch::safe_message(&reason);
        assert_eq!(
            CliError::Usage(reason).line(),
            format!("error: {bounded} (see: engine-cli help)"),
            "a usage error keeps the way to its usage, after its reason is bounded ({length})"
        );
    }
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
    assert_eq!(
        fine.flush().map_err(|error| error.kind()),
        Ok(()),
        "flush passes through"
    );
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

/// Whether `character` could break a line or drive a terminal: a control, bidirectional
/// formatting or line-separator character (written independently of `engine-patch`).
fn unsafe_character(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{61c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
                | '\u{2028}' | '\u{2029}'
        )
}

/// Checks that `out` is `lines` safe lines: each ends with the only line feed, holds no
/// other unsafe character and at most `MAX_MESSAGE_CHARS` characters.
fn assert_safe_lines(out: &str, lines: usize) {
    assert!(out.ends_with('\n'), "every line ends: {out:?}");
    let split: Vec<&str> = out.trim_end_matches('\n').split('\n').collect();
    assert_eq!(split.len(), lines, "one line per report: {out:?}");
    for line in split {
        assert!(
            !line.chars().any(unsafe_character),
            "no control or bidirectional character: {line:?}"
        );
        assert!(
            line.chars().count() <= engine_patch::MAX_MESSAGE_CHARS,
            "bounded: {} characters",
            line.chars().count()
        );
    }
}

/// Box type of a patch, crafted to forge a verdict line and drive the terminal.
const FORGED_BOX: &str = "y\\nok       sine-440 0\\u001b[2K\\u202e";

/// A conformance directory whose untrusted text attacks the output: case names (file
/// names), a box type quoted by an error, a 2 MB box type, and the keys and values of
/// `references.json`. Six problems: two failed renders, a mismatch, a missing
/// reference and two stale ones.
fn hostile_corpus() -> Memory {
    let huge_box = format!(
        r#"{{"format":1,"nodes":[{{"id":1,"box":"{}"}}],"connections":[]}}"#,
        "x".repeat(2_000_000)
    );
    let forged_box =
        format!(r#"{{"format":1,"nodes":[{{"id":1,"box":"{FORGED_BOX}"}}],"connections":[]}}"#);
    let references = "{\"b\\u001b[2J\\nok       b\":{},\
         \"b\\u001b[2J\":{\"binary64\":\"0\\nok       forged\\u001b[31m\\u202e\",\
         \"\\u001b[2Jbinary16\\nok\":\"0\"},\
         \"gone\\nok       gone\\u0007\":{}}";
    Memory::with(&[
        ("conf/cases/a\nok       a 0.json", SINE),
        ("conf/patches/a\nok       a 0.json", &forged_box),
        ("conf/cases/b\u{1b}[2J.json", SINE),
        ("conf/patches/b\u{1b}[2J.json", PATCH),
        ("conf/cases/c\u{202e}.json", SINE),
        ("conf/patches/c\u{202e}.json", &huge_box),
        ("conf/cases/d\u{1b}]0;title\u{7}.json", SINE),
        ("conf/patches/d\u{1b}]0;title\u{7}.json", PATCH),
        ("conf/references.json", &canonical(references)),
        (
            "conf/algorithms.json",
            &record("{\"gone\\nok       gone\\u0007\":{}}"),
        ),
    ])
}

#[test]
fn check_command_prints_one_safe_line_per_problem() {
    let mut memory = hostile_corpus();
    let (result, out) = run_with(&mut memory, &["check", "conf"]);
    assert_eq!(
        result,
        Err("conformance failed in conf: 18 problem(s)".to_owned()),
        "every problem counted, in every format"
    );
    assert_safe_lines(&out, 18);
    assert!(
        out.contains("FAILED   a\\u{a}ok       a 0 binary64: "),
        "the case name is escaped: {out}"
    );
    assert!(
        out.contains("unknown box type \"y\\u{a}ok       sine-440 0\\u{1b}[2K\\u{202e}\""),
        "the box type is escaped: {out}"
    );
    assert!(
        out.contains(&format!(
            "MISMATCH b\\u{{1b}}[2J binary64: expected 0\\u{{a}}ok       forged\\u{{1b}}[31m\\u{{202e}}, got {}",
            sine_hash(RenderFormat::Binary64)
        )),
        "the reference is escaped: {out}"
    );
    assert!(
        out.contains(
            "STALE    b\\u{1b}[2J \\u{1b}[2Jbinary16\\u{a}ok: reference of no render format"
        ),
        "the name of a format is escaped: {out}"
    );
    assert!(
        out.contains("STALE    gone\\u{a}ok       gone\\u{7}: reference without a case"),
        "the reference key is escaped: {out}"
    );
    assert!(
        out.contains("STALE    gone\\u{a}ok       gone\\u{7}: algorithm versions without a case"),
        "the key of the algorithm versions is escaped: {out}"
    );
}

#[test]
fn bless_and_export_print_safe_lines() {
    let mut memory = Memory::with(&[
        ("conf\u{1b}[2J/cases/a\n\u{202e}.json", SINE),
        ("conf\u{1b}[2J/patches/a\n\u{202e}.json", PATCH),
    ]);
    let (blessed, bless_out) = run_with(&mut memory, &["bless", "conf\u{1b}[2J", "--from-nothing"]);
    assert_eq!(blessed, Ok(()), "blessed");
    assert_eq!(
        bless_out,
        "blessed 1 case(s) in conf\\u{1b}[2J/references.json and conf\\u{1b}[2J/algorithms.json\n",
        "the directory is escaped"
    );
    let (exported, export_out) = run_with(&mut memory, &["export", "conf\u{1b}[2J", "out"]);
    assert_eq!(exported, Ok(()), "exported");
    let lines = RenderFormat::ALL
        .map(|format| {
            format!(
                "wrote    out/a\\u{{a}}\\u{{202e}}.{format}.wav {}\n",
                sine_hash(format)
            )
        })
        .concat();
    assert_eq!(export_out, lines, "the path is escaped");
}
