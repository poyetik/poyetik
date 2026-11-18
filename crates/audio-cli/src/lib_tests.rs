//! Tests of case rendering, WAV encoding and the commands, with an in-memory file system.

use alloc::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use audio_core::{RenderFormat, Replaced, SampleRate};

use fpenv::with_default_mode;

use crate::hint::Hint;

use super::{
    CaseError, CliError, Disk, Files, HEADER_LEN, Rendered, RenderedSamples, USAGE, arguments, run,
    wav_header,
};

/// Declares the kinds of hint, `HintKind`, from the list that declares [`Hint`]
/// (`with_hints`), with every one of them in `HintKind::ALL`, and `Hint::kind`.
macro_rules! declare_kinds {
    ($(
        $(#[doc = $doc:literal])+
        $name:ident $({ $($(#[doc = $field_doc:literal])+ $field:ident: $type:ty),+ $(,)? })?
    ),+ $(,)?) => {
        /// The kind of a [`Hint`], without what it names.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        enum HintKind {
            $(
                $(#[doc = $doc])+
                $name,
            )+
        }

        impl HintKind {
            /// Every kind, in the order of their declaration.
            const ALL: &[Self] = &[$(Self::$name),+];
        }

        impl Hint<'_> {
            /// The kind of the hint.
            const fn kind(&self) -> HintKind {
                match self {
                    $(Self::$name { .. } => HintKind::$name,)+
                }
            }
        }
    };
}

with_hints!(declare_kinds);

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
    /// The spent record, when not the committed one ([`main_record`]).
    main: Option<Vec<u8>>,
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
    let result =
        run(&args(words), memory, &mut out, &mut Vec::new()).map_err(|error| error.to_string());
    (result, String::from_utf8(out).expect("utf-8 output"))
}

/// How a usage error reads: its reason, then where to find the usage.
fn usage(reason: &str) -> String {
    format!("{reason} (see: audio-cli help)")
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
        replaced: Replaced::default(),
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
            f64::from(
                <f32 as audio_core::host::Delivered<audio_core::Binary64>>::deliver(
                    <audio_core::Binary64 as audio_core::host::Raw>::from_raw(sample),
                ),
            )
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
fn refuses_a_case_or_an_output_stage_written_as_an_array() {
    // A derived reader also takes an array of the fields in order: `[48000, 1]` would read
    // as 48 kHz and one frame. The oracle refuses both as not a JSON object.
    let case = |message: &str| CaseError::Case(message.to_owned());
    for (case_text, expected) in [
        (
            "[48000, 1]",
            case(
                "invalid case: invalid type: sequence, expected a case: a JSON object with \
                 sample_rate, frames and optionally output at line 1 column 0",
            ),
        ),
        (
            r#"{"sample_rate":48000,"frames":1,"output":["protected",1.5,"precise"]}"#,
            case(
                "invalid case: invalid type: sequence, expected an output stage: a JSON object \
                 with mode, and lookahead and detector when protected at line 1 column 41",
            ),
        ),
    ] {
        for format in RenderFormat::ALL {
            assert_eq!(
                render_case(case_text, PATCH, format).map(|_| ()),
                Err(expected.clone()),
                "{format}: {case_text}"
            );
        }
    }
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
                "invalid case: unknown field `extra`, expected one of `sample_rate`, `frames`, `output` at line 1 column 39",
            ),
        ),
        (
            r#"{"sample_rate":48000,"frames":1,"output":null}"#,
            PATCH,
            case(
                "invalid case: invalid type: null, expected an output stage: a JSON object with \
                 mode, and lookahead and detector when protected at line 1 column 45",
            ),
        ),
        (
            "7",
            PATCH,
            case(
                "invalid case: invalid type: integer `7`, expected a case: a JSON object with \
                 sample_rate, frames and optionally output at line 1 column 1",
            ),
        ),
        (
            r#"{"sample_rate":48000,"frames":1,"output":{"mode":"loud","lookahead":1.5,"detector":"standard"}}"#,
            PATCH,
            case(
                "invalid case: unknown variant `loud`, expected `raw` or `protected` at line 1 column 55",
            ),
        ),
        (
            r#"{"sample_rate":48000,"frames":1,"output":{"mode":"protected","lookahead":1.5,"detector":"exact"}}"#,
            PATCH,
            case(
                "invalid case: unknown variant `exact`, expected one of `standard`, `precise`, `mastering` at line 1 column 96",
            ),
        ),
        (
            r#"{"sample_rate":48000,"frames":1,"output":{"mode":"protected","lookahead":0.25,"detector":"standard"}}"#,
            PATCH,
            case("invalid case: limiter lookahead 0.25 ms is out of range (0.5 to 5 ms)"),
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
fn refuses_a_setting_of_a_raw_output_and_a_protected_one_without_both() {
    // A raw stage carries no limiter, so no setting, whatever its value; a protected one
    // needs its lookahead and its detector.
    let case = |message: &str| CaseError::Case(message.to_owned());
    for (case_text, expected) in [
        (
            r#"{"sample_rate":48000,"frames":1,"output":{"mode":"raw","lookahead":1.5,"detector":"standard"}}"#,
            case("invalid case: a raw output stage takes no lookahead at line 1 column 93"),
        ),
        (
            r#"{"sample_rate":48000,"frames":1,"output":{"mode":"raw","lookahead":0.25}}"#,
            case("invalid case: a raw output stage takes no lookahead at line 1 column 72"),
        ),
        (
            r#"{"sample_rate":48000,"frames":1,"output":{"mode":"raw","detector":"standard"}}"#,
            case("invalid case: a raw output stage takes no detector at line 1 column 77"),
        ),
        (
            r#"{"sample_rate":48000,"frames":1,"output":{"mode":"raw","lookahead":null}}"#,
            case("invalid case: invalid type: null, expected f64 at line 1 column 71"),
        ),
        (
            r#"{"sample_rate":48000,"frames":1,"output":{"mode":"protected","detector":"standard"}}"#,
            case("invalid case: missing field `lookahead` at line 1 column 83"),
        ),
        (
            r#"{"sample_rate":48000,"frames":1,"output":{"mode":"protected","lookahead":1.5}}"#,
            case("invalid case: missing field `detector` at line 1 column 77"),
        ),
    ] {
        for format in RenderFormat::ALL {
            assert_eq!(
                render_case(case_text, PATCH, format).map(|_| ()),
                Err(expected.clone()),
                "{format}: {case_text}"
            );
        }
    }
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
            replaced: Replaced::default(),
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

/// The fingerprint of every registered box type at its current version, through the
/// engine's instances, by name: computed once for the whole test binary, since the battery
/// is the slowest part of these tests and gives the same value whichever test asks first.
static FINGERPRINTS: std::sync::LazyLock<BTreeMap<&'static str, String>> =
    std::sync::LazyLock::new(|| {
        audio_boxes::REGISTRY
            .iter()
            .map(|spec| {
                (
                    spec.name(),
                    audio_battery::fingerprint::<
                        audio_boxes::BuiltIn<audio_core::Binary64>,
                        audio_boxes::BuiltIn<audio_core::Binary32>,
                    >(spec)
                    .expect("the engine creates every registered box type"),
                )
            })
            .collect()
    });

/// The fingerprint of the registered box type `name`, at its current version, through the
/// engine's instances ([`FINGERPRINTS`]).
fn fingerprint_of(name: &str) -> String {
    FINGERPRINTS
        .get(name)
        .expect("a registered box type")
        .clone()
}

/// The record bless writes for the algorithm versions `cases` (a JSON object of each
/// case's box types' versions): render hash 2, render rules 1, battery 1 and the
/// fingerprints of the registered box types at version 1.
fn record(cases: &str) -> String {
    canonical(&format!(
        r#"{{"cases":{cases},"fingerprint_battery":2,"fingerprints":{{"dac~":{{"1":"{}"}},"osc~":{{"1":"{}"}}}},"render_hash":"render-hash/2","render_rules":3}}"#,
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
             STALE    sine binary16: reference of no render format (run `cargo xtask bless`)\n\
             STALE    gone: reference without a case (run `cargo xtask bless`)\n",
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
             STALE    sine env~: algorithm version of a box type the case does not use (run \
             `cargo xtask bless`)\n\
             STALE    gone: algorithm versions without a case (run `cargo xtask bless`)\n"
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
             restore conf/algorithms.json from git)\n"
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
    // A missing record: every_hint_is_followed_from_a_state_that_gives_it.
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
        run(
            &args(&["check", "conf"]),
            &mut broken,
            &mut Broken,
            &mut Vec::new()
        )
        .map_err(|error| error.to_string()),
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
             \"render_hash\": \"render-hash/2\",\n  \"render_rules\": 3\n}}\n",
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
        "CHANGED  sine binary32: its reference changed under the same algorithm versions (a new \
         version, then run `cargo xtask bless`, or `cargo xtask bless --case-changed sine`)\n",
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
    let bits = "BITS     osc~ 1: its outputs changed at the same algorithm version (a new \
                version of osc~, then run `cargo xtask bless`)\n";
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
    let bits = "BITS     osc~ 1: its outputs changed at the same algorithm version (a new \
                version of osc~, then run `cargo xtask bless`)\n";
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
    // every_hint_is_followed_from_a_state_that_gives_it.
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

/// The spent record: the one `memory` holds there, else the committed one.
fn main_record(memory: &Memory) -> Vec<u8> {
    memory
        .main
        .clone()
        .unwrap_or_else(|| committed().files[Path::new("conf/algorithms.json")].clone())
}

/// What `cargo xtask <command> <options>` runs: `audio-cli <command> conf --spent
/// spent.json`, the spent record ([`main_record`]) written to `spent.json` first, then the
/// options.
fn xtask(memory: &mut Memory, command: &str, options: &[&str]) -> (Result<(), String>, String) {
    let main = main_record(memory);
    drop(memory.files.insert(PathBuf::from("spent.json"), main));
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

/// `record`, a record as bless writes it, edited by `edit`, as bless writes it.
fn edited(record: &[u8], edit: fn(&mut serde_json::Value)) -> Vec<u8> {
    let mut value: serde_json::Value = serde_json::from_slice(record).expect("a record");
    edit(&mut value);
    format!("{value:#}\n").into_bytes()
}

/// Edits the record of `conf` and the spent record alike: how a test makes or undoes a
/// change of the engine, which it cannot make. The engine moving from one value to
/// another (a version, the render rules, the render hash, the battery) is both records
/// holding the value it moves from.
fn edit_records(memory: &mut Memory, edit: fn(&mut serde_json::Value)) {
    memory.main = Some(edited(&main_record(memory), edit));
    let here = edited(&memory.files[Path::new("conf/algorithms.json")], edit);
    drop(
        memory
            .files
            .insert(PathBuf::from("conf/algorithms.json"), here),
    );
}

/// Moves every version of osc~ in `record` by `by`: down by one, the engine's osc~ at a new
/// version; up by one, back.
fn move_osc(record: &mut serde_json::Value, by: i64) {
    let shift = |version: &serde_json::Value| {
        version
            .as_i64()
            .and_then(|version| version.checked_add(by))
            .expect("a version")
    };
    if let Some(cases) = record["cases"].as_object_mut() {
        for boxes in cases.values_mut() {
            if let Some(version) = boxes.get_mut("osc~") {
                *version = shift(version).into();
            }
        }
    }
    let prints = record["fingerprints"]["osc~"].take();
    let moved: serde_json::Map<String, serde_json::Value> = prints
        .as_object()
        .expect("the fingerprints of osc~")
        .iter()
        .map(|(version, print)| {
            let version: serde_json::Value = version.parse::<i64>().expect("a version").into();
            (shift(&version).to_string(), print.clone())
        })
        .collect();
    record["fingerprints"]["osc~"] = moved.into();
}

/// The engine's osc~ at a new version ([`move_osc`]).
fn osc_newer(record: &mut serde_json::Value) {
    move_osc(record, -1);
}

/// The engine's osc~ back at its version ([`move_osc`]).
fn osc_older(record: &mut serde_json::Value) {
    move_osc(record, 1);
}

/// The battery the engine had before its current one, 1.
fn older_battery(record: &mut serde_json::Value) {
    record["fingerprint_battery"] = 1_u32.into();
}

/// The render rules the engine had before its current ones, 0.
fn older_rules(record: &mut serde_json::Value) {
    record["render_rules"] = 0_u32.into();
}

/// The engine's render rules, 3.
fn current_rules(record: &mut serde_json::Value) {
    record["render_rules"] = 3_u32.into();
}

/// The render hash the engine had before its current one.
fn older_hash(record: &mut serde_json::Value) {
    record["render_hash"] = "render-hash/1".into();
}

/// The engine's render hash.
fn current_hash(record: &mut serde_json::Value) {
    record["render_hash"] = "render-hash/2".into();
}

/// The fingerprint of osc~ at version 1 the engine had before a change of its code.
fn other_osc_bits(record: &mut serde_json::Value) {
    record["fingerprints"]["osc~"]["1"] = "0000".into();
}

/// The steps of the alternatives of `hint`, as [`crate::hint`] writes them: alternatives
/// separated by `, or ` or `; `, the steps of each by `, then `.
fn alternatives(hint: &str) -> Vec<Vec<&str>> {
    hint.split(", or ")
        .flat_map(|alternative| alternative.split("; "))
        .map(|alternative| alternative.split(", then ").collect())
        .collect()
}

/// Does the steps of an alternative of a hint: runs the `cargo xtask` command a step names,
/// removes the file it says to remove, restores the file it says to restore from git (its
/// committed contents); a step of the code or a review, said in words, is `change`'s.
fn follow(memory: &mut Memory, steps: &[&str], change: Change) {
    for step in steps {
        if let Some((_, command)) = step.split_once("`cargo xtask ") {
            let named = command.split('`').next().expect("a closed command");
            let words: Vec<&str> = named.split_whitespace().collect();
            let (task, options) = words.split_first().expect("a command of xtask");
            assert_eq!(xtask(memory, task, options).0, Ok(()), "{step}: followed");
        } else if let Some(path) = step.strip_prefix("remove ") {
            assert!(
                memory.files.remove(Path::new(path)).is_some(),
                "{step}: {path} to remove"
            );
        } else if let Some((_, restored)) = step.split_once("restore ") {
            let path = restored.split(' ').next().expect("a path");
            drop(memory.files.insert(
                PathBuf::from(path),
                committed().files[Path::new(path)].clone(),
            ));
        } else {
            change(memory, step);
        }
    }
}

/// How a test brings a conformance directory into a state and runs the command that
/// refuses it: what it returned and printed.
type Setup = Box<dyn Fn(&mut Memory) -> (Result<(), String>, String)>;

/// How a test makes a step of a hint said in words: a change of the code, which it
/// emulates on the records ([`edit_records`]), or a review, which changes nothing.
type Change = fn(&mut Memory, &str);

/// A step in words where a hint should have none.
fn no_change(_: &mut Memory, step: &str) {
    panic!("{step}: a step in words");
}

/// "check the renders of the new version": a review.
fn review(_: &mut Memory, step: &str) {
    assert_eq!(step, "check the renders of the new version", "a review");
}

/// "a new version of osc~" (or "a new version", in a case that only uses osc~ and dac~,
/// whose code did not change).
fn new_version(memory: &mut Memory, step: &str) {
    assert!(
        ["a new version of osc~", "a new version"].contains(&step),
        "{step}: a new version"
    );
    edit_records(memory, osc_newer);
}

/// The steps in words of a battery change blessed alone, the other change made by `redo`
/// and undone by `undo`.
fn battery_alone(memory: &mut Memory, step: &str, undo: fn(&mut Memory), redo: fn(&mut Memory)) {
    match step {
        "bless the battery change alone first: set the other changes aside" => undo(memory),
        "commit" => {}
        "bring the other changes back" => redo(memory),
        other => panic!("{other}: not a step of a battery change"),
    }
}

/// The render rules changed with the battery.
fn rules_apart(memory: &mut Memory, step: &str) {
    battery_alone(
        memory,
        step,
        |memory| edit_records(memory, current_rules),
        |memory| edit_records(memory, older_rules),
    );
}

/// The render hash changed with the battery.
fn hash_apart(memory: &mut Memory, step: &str) {
    battery_alone(
        memory,
        step,
        |memory| edit_records(memory, current_hash),
        |memory| edit_records(memory, older_hash),
    );
}

/// The version of osc~ changed with the battery.
fn version_apart(memory: &mut Memory, step: &str) {
    battery_alone(
        memory,
        step,
        |memory| edit_records(memory, osc_older),
        |memory| edit_records(memory, osc_newer),
    );
}

/// No fingerprint of dac~ at its version: the engine's dac~ at a version no record holds,
/// which no case fails on, since the case's versions do not move.
fn without_dac(record: &mut serde_json::Value) {
    drop(
        record["fingerprints"]
            .as_object_mut()
            .expect("the fingerprints")
            .remove("dac~"),
    );
}

/// The fingerprint of dac~ at its version back.
fn with_dac(record: &mut serde_json::Value) {
    record["fingerprints"]["dac~"] = serde_json::json!({ "1": fingerprint_of("dac~") });
}

/// A version of dac~ the record does not hold, with the battery.
fn dac_apart(memory: &mut Memory, step: &str) {
    battery_alone(
        memory,
        step,
        |memory| edit_records(memory, with_dac),
        |memory| edit_records(memory, without_dac),
    );
}

/// The references without the binary32 one of the case: a reference lost, in a merge.
fn no_binary32(memory: &mut Memory) {
    let mut references: serde_json::Value =
        serde_json::from_slice(&memory.files[Path::new(REFERENCES)]).expect("references");
    drop(
        references["sine"]
            .as_object_mut()
            .expect("the references of sine")
            .remove("binary32"),
    );
    drop(memory.files.insert(
        PathBuf::from(REFERENCES),
        format!("{references:#}\n").into_bytes(),
    ));
}

/// The references back as committed.
fn restore_references(memory: &mut Memory) {
    drop(memory.files.insert(
        PathBuf::from(REFERENCES),
        committed().files[Path::new(REFERENCES)].clone(),
    ));
}

/// The binary32 reference of the case lost with the battery change.
fn missing_apart(memory: &mut Memory, step: &str) {
    battery_alone(memory, step, restore_references, no_binary32);
}

/// The binary32 render of the case changed with the battery.
fn reference_apart(memory: &mut Memory, step: &str) {
    battery_alone(memory, step, restore_references, old_binary32);
}

/// A case added with the battery change.
fn case_apart(memory: &mut Memory, step: &str) {
    battery_alone(memory, step, remove_new_case, add_new_case);
}

/// The references as before a change of the binary32 render of the case.
fn old_binary32(memory: &mut Memory) {
    let references = canonical(&format!(
        r#"{{"sine":{}}}"#,
        sine_references().replace(&sine_hash(RenderFormat::Binary32), "0000")
    ));
    drop(memory.files.insert(
        PathBuf::from("conf/references.json"),
        references.into_bytes(),
    ));
}

/// A second case, `new`, added to the corpus.
fn add_new_case(memory: &mut Memory) {
    for (path, text) in [
        ("conf/cases/new.json", SINE),
        ("conf/patches/new.json", PATCH),
    ] {
        drop(
            memory
                .files
                .insert(PathBuf::from(path), text.as_bytes().to_vec()),
        );
    }
}

/// The case `new` removed from the corpus.
fn remove_new_case(memory: &mut Memory) {
    for path in ["conf/cases/new.json", "conf/patches/new.json"] {
        assert!(memory.files.remove(Path::new(path)).is_some(), "{path}");
    }
}

/// A state a command refuses with a hint, and which alternative of it the test follows.
struct Hinted {
    /// What the state is.
    state: &'static str,
    /// The hint it gives, as the test writes it.
    hint: Hint<'static>,
    /// How it is reached.
    setup: Setup,
    /// The end of the line that gives the hint, exactly.
    line: String,
    /// Other lines the command prints, exactly: what a battery change came with.
    causes: Vec<String>,
    /// The alternative of the hint followed, counted from 0.
    alternative: usize,
    /// How the steps in words are made.
    change: Change,
}

/// The record of `conf`, edited by `edit`, then `command` run.
fn with_record(
    memory: &mut Memory,
    edit: fn(&mut serde_json::Value),
    command: &[&str],
) -> (Result<(), String>, String) {
    let text = edited(&memory.files[Path::new("conf/algorithms.json")], edit);
    drop(
        memory
            .files
            .insert(PathBuf::from("conf/algorithms.json"), text),
    );
    run_with(memory, command)
}

/// Where the record of the corpus lives.
const RECORD: &str = "conf/algorithms.json";

/// What `bless` names when one or both of its writes fail.
const REFERENCES: &str = "conf/references.json";

/// Every state of `kind` a command refuses with its hint: each state names its hint, and
/// between them they follow every alternative of it. An exhaustive `match`: a hint without
/// a state does not compile.
#[expect(clippy::too_many_lines, reason = "one table, one row per state")]
fn hinted(kind: HintKind) -> Vec<Hinted> {
    let record_file = Path::new(RECORD);
    let references = Path::new(REFERENCES);
    match kind {
        HintKind::CaseChangedOrRestore => [0, 1]
            .map(|alternative| {
                row(
                    "a case with references and no versions",
                    Hint::CaseChangedOrRestore {
                        case: "sine",
                        record: record_file,
                    },
                    |memory| {
                        drop(
                            memory
                                .files
                                .insert(PathBuf::from(RECORD), record("{}").into_bytes()),
                        );
                        run_with(memory, &["check", "conf"])
                    },
                    "MISSING  sine: no algorithm versions (`cargo xtask bless --case-changed \
                     sine`, or restore conf/algorithms.json from git)",
                    alternative,
                    no_change,
                )
            })
            .into(),
        HintKind::NewCase => vec![row(
            "a new case: no references and no versions",
            Hint::NewCase,
            |memory| {
                drop(
                    memory
                        .files
                        .insert(PathBuf::from(RECORD), record("{}").into_bytes()),
                );
                drop(
                    memory
                        .files
                        .insert(PathBuf::from(REFERENCES), b"{}\n".to_vec()),
                );
                run_with(memory, &["check", "conf"])
            },
            "MISSING  sine: no algorithm versions (run `cargo xtask bless`)",
            0,
            no_change,
        )],
        HintKind::NewVersion => vec![row(
            "osc~ at a new version",
            Hint::NewVersion,
            |memory| {
                edit_records(memory, osc_newer);
                xtask(memory, "check", &[])
            },
            "VERSION  sine osc~: at algorithm version 1, the references were blessed with \
             version 0 (check the renders of the new version, then run `cargo xtask bless`)",
            0,
            review,
        )],
        HintKind::UnversionedBox => vec![row(
            "a box type of a case without its version",
            Hint::UnversionedBox,
            |memory| {
                with_record(
                    memory,
                    |record| {
                        drop(
                            record["cases"]["sine"]
                                .as_object_mut()
                                .map(|boxes| boxes.remove("dac~")),
                        );
                    },
                    &["check", "conf"],
                )
            },
            "MISSING  sine dac~: no algorithm version (run `cargo xtask bless`)",
            0,
            no_change,
        )],
        HintKind::NoReference => vec![row(
            "a format without a reference",
            Hint::NoReference,
            |memory| {
                let without = canonical(&format!(
                    r#"{{"sine":{}}}"#,
                    sine_references().replace(
                        &format!(r#","binary32":"{}""#, sine_hash(RenderFormat::Binary32)),
                        ""
                    )
                ));
                drop(
                    memory
                        .files
                        .insert(PathBuf::from(REFERENCES), without.into_bytes()),
                );
                run_with(memory, &["check", "conf"])
            },
            "MISSING  sine binary32: no reference hash (run `cargo xtask bless`)",
            0,
            no_change,
        )],
        HintKind::RestoreBeside => vec![
            row(
                "references alone missing, at bless",
                Hint::RestoreBeside { file: references },
                |memory| {
                    drop(memory.files.remove(Path::new(REFERENCES)));
                    xtask_bless(memory, &[])
                },
                "MISSING  conf/references.json: missing beside conf/algorithms.json (restore \
                 conf/references.json from git: bless compares with both)",
                0,
                no_change,
            ),
            row(
                "record alone missing, at bless",
                Hint::RestoreBeside { file: record_file },
                |memory| {
                    drop(memory.files.remove(Path::new(RECORD)));
                    xtask_bless(memory, &[])
                },
                "MISSING  conf/algorithms.json: missing beside conf/references.json (restore \
                 conf/algorithms.json from git: bless compares with both)",
                0,
                no_change,
            ),
        ],
        HintKind::FirstBless => vec![row(
            "both missing, at bless",
            Hint::FirstBless,
            |memory| {
                drop(memory.files.remove(Path::new(REFERENCES)));
                drop(memory.files.remove(Path::new(RECORD)));
                xtask_bless(memory, &[])
            },
            "MISSING  conf/references.json and conf/algorithms.json: none to compare with (a \
             first bless: `cargo xtask bless --from-nothing`)",
            0,
            no_change,
        )],
        HintKind::WithoutFromNothing => vec![row(
            "--from-nothing with both files",
            Hint::WithoutFromNothing,
            |memory| xtask_bless(memory, &["--from-nothing"]),
            "OPTION   --from-nothing: conf/references.json and conf/algorithms.json exist, to \
             compare with (run `cargo xtask bless` without it)",
            0,
            no_change,
        )],
        HintKind::WithoutBatteryChanged => vec![row(
            "--battery-changed with the battery unchanged",
            Hint::WithoutBatteryChanged,
            |memory| xtask_bless(memory, &["--battery-changed"]),
            "BATTERY  --battery-changed: the battery is unchanged (2) (run `cargo xtask bless` \
             without it)",
            0,
            no_change,
        )],
        HintKind::Stale => stale_rows(),
        HintKind::RemoveThenFirstBless => vec![row(
            "a first bless that wrote the references alone",
            Hint::RemoveThenFirstBless {
                written: references,
            },
            |memory| {
                drop(memory.files.remove(Path::new(REFERENCES)));
                drop(memory.files.remove(Path::new(RECORD)));
                memory.unwritable = Some(PathBuf::from(RECORD));
                let refused = xtask_bless(memory, &["--from-nothing"]);
                memory.unwritable = None;
                refused
            },
            "cannot write conf/algorithms.json: read-only (conf/references.json was written: \
             remove conf/references.json, then run `cargo xtask bless --from-nothing`)",
            0,
            no_change,
        )],
        HintKind::BlessAgain => vec![
            row(
                "a bless that wrote the references alone",
                Hint::BlessAgain {
                    changed: &[],
                    battery_changed: false,
                },
                |memory| {
                    memory.unwritable = Some(PathBuf::from(RECORD));
                    let refused = xtask_bless(memory, &[]);
                    memory.unwritable = None;
                    refused
                },
                "cannot write conf/algorithms.json: read-only (conf/references.json was \
                 written: run `cargo xtask bless` again)",
                0,
                no_change,
            ),
            row(
                "a bless of a battery change that wrote the references alone",
                Hint::BlessAgain {
                    changed: &[],
                    battery_changed: true,
                },
                |memory| {
                    edit_records(memory, older_battery);
                    memory.unwritable = Some(PathBuf::from(RECORD));
                    let refused = xtask_bless(memory, &["--battery-changed"]);
                    memory.unwritable = None;
                    refused
                },
                "cannot write conf/algorithms.json: read-only (conf/references.json was \
                 written: run `cargo xtask bless --battery-changed` again)",
                0,
                no_change,
            ),
            row(
                "a bless of a case changed that wrote the references alone",
                Hint::BlessAgain {
                    changed: named_sine(),
                    battery_changed: false,
                },
                |memory| {
                    old_binary32(memory);
                    memory.unwritable = Some(PathBuf::from(RECORD));
                    let refused = xtask_bless(memory, &["--case-changed", "sine"]);
                    memory.unwritable = None;
                    refused
                },
                "cannot write conf/algorithms.json: read-only (conf/references.json was \
                 written: run `cargo xtask bless --case-changed sine` again)",
                0,
                no_change,
            ),
        ],
        HintKind::RestoreOrFirstBless => vec![
            row(
                "references missing",
                Hint::RestoreOrFirstBless { file: references },
                |memory| {
                    drop(memory.files.remove(Path::new(REFERENCES)));
                    run_with(memory, &["check", "conf"])
                },
                "cannot read conf/references.json: no such file (restore \
                 conf/references.json from git; a new directory: `cargo xtask bless \
                 --from-nothing`)",
                0,
                no_change,
            ),
            row(
                "record missing",
                Hint::RestoreOrFirstBless { file: record_file },
                |memory| {
                    drop(memory.files.remove(Path::new(RECORD)));
                    run_with(memory, &["check", "conf"])
                },
                "cannot read conf/algorithms.json: no such file (restore \
                 conf/algorithms.json from git; a new directory: `cargo xtask bless \
                 --from-nothing`)",
                0,
                no_change,
            ),
            row(
                "a new directory",
                Hint::RestoreOrFirstBless { file: references },
                |memory| {
                    drop(memory.files.remove(Path::new(REFERENCES)));
                    drop(memory.files.remove(Path::new(RECORD)));
                    run_with(memory, &["check", "conf"])
                },
                "cannot read conf/references.json: no such file (restore \
                 conf/references.json from git; a new directory: `cargo xtask bless \
                 --from-nothing`)",
                1,
                no_change,
            ),
        ],
        HintKind::RestoreInvalid => vec![
            row(
                "invalid references, at check",
                Hint::RestoreInvalid { file: references },
                |memory| {
                    drop(
                        memory
                            .files
                            .insert(PathBuf::from(REFERENCES), b"[1".to_vec()),
                    );
                    run_with(memory, &["check", "conf"])
                },
                "(restore conf/references.json from git)",
                0,
                no_change,
            ),
            row(
                "invalid record, at check",
                Hint::RestoreInvalid { file: record_file },
                |memory| {
                    drop(memory.files.insert(PathBuf::from(RECORD), b"[1".to_vec()));
                    run_with(memory, &["check", "conf"])
                },
                "(restore conf/algorithms.json from git)",
                0,
                no_change,
            ),
        ],
        HintKind::Rewrite => vec![row(
            "references not as bless writes them",
            Hint::Rewrite,
            |memory| {
                let text = format!("{} ", memory.text(REFERENCES));
                drop(
                    memory
                        .files
                        .insert(PathBuf::from(REFERENCES), text.into_bytes()),
                );
                run_with(memory, &["check", "conf"])
            },
            "conf/references.json is not as bless writes it (run `cargo xtask bless`)",
            0,
            no_change,
        )],
        HintKind::RestoreCompared => vec![row(
            "invalid record, at bless",
            Hint::RestoreCompared { file: record_file },
            |memory| {
                drop(memory.files.insert(PathBuf::from(RECORD), b"[1".to_vec()));
                xtask_bless(memory, &[])
            },
            "(bless compares with it: restore conf/algorithms.json from git)",
            0,
            no_change,
        )],
        HintKind::WriteSpent => vec![
            row(
                "an invalid spent record",
                Hint::WriteSpent {
                    writer: "cargo xtask bless",
                },
                |memory| {
                    drop(
                        memory
                            .files
                            .insert(PathBuf::from("spent.json"), b"[1".to_vec()),
                    );
                    run_with(memory, &["bless", "conf", "--spent", "spent.json"])
                },
                "(`cargo xtask bless` writes it: run that)",
                0,
                no_change,
            ),
            row(
                "an invalid spent record, at check",
                Hint::WriteSpent {
                    writer: "cargo xtask check",
                },
                |memory| {
                    drop(
                        memory
                            .files
                            .insert(PathBuf::from("spent.json"), b"null ]".to_vec()),
                    );
                    run_with(memory, &["check", "conf", "--spent", "spent.json"])
                },
                "(`cargo xtask check` writes it: run that)",
                0,
                no_change,
            ),
            row(
                "a missing spent record, at check",
                Hint::WriteSpent {
                    writer: "cargo xtask check",
                },
                |memory| run_with(memory, &["check", "conf", "--spent", "spent.json"]),
                "cannot read the spent record spent.json: no such file (`cargo xtask check` \
                 writes it: run that)",
                0,
                no_change,
            ),
        ],
        HintKind::Hash => vec![row(
            "another render hash",
            Hint::Hash,
            |memory| {
                edit_records(memory, older_hash);
                xtask(memory, "check", &[])
            },
            "HASH     the references were hashed with render-hash/1, the engine with \
             render-hash/2 (run `cargo xtask bless`)",
            0,
            no_change,
        )],
        HintKind::Rules => vec![row(
            "other render rules",
            Hint::Rules,
            |memory| {
                edit_records(memory, older_rules);
                xtask(memory, "check", &[])
            },
            "RULES    the references were rendered under render rules version 0, the engine's \
             is 3 (run `cargo xtask bless`)",
            0,
            no_change,
        )],
        HintKind::NoFingerprint => vec![row(
            "no fingerprint at the current version",
            Hint::NoFingerprint,
            |memory| {
                with_record(
                    memory,
                    |record| {
                        drop(
                            record["fingerprints"]
                                .as_object_mut()
                                .map(|prints| prints.remove("osc~")),
                        );
                    },
                    &["check", "conf"],
                )
            },
            "MISSING  osc~: no fingerprint at algorithm version 1 (run `cargo xtask bless`)",
            0,
            no_change,
        )],
        HintKind::Battery => vec![
            row(
                "another battery",
                Hint::Battery,
                |memory| {
                    edit_records(memory, older_battery);
                    xtask(memory, "check", &[])
                },
                BATTERY_ONLY,
                0,
                no_change,
            ),
            row(
                "another battery and a version of a box type the case does not use",
                Hint::Battery,
                |memory| {
                    edit_records(memory, older_battery);
                    edit_json(memory, RECORD, |record| {
                        record["cases"]["sine"]["old~"] = 1_u32.into();
                    });
                    xtask(memory, "check", &[])
                },
                BATTERY_ONLY,
                0,
                no_change,
            ),
            row(
                "another battery and a reference of no render format",
                Hint::Battery,
                |memory| {
                    edit_records(memory, older_battery);
                    edit_json(memory, REFERENCES, |hashes| {
                        hashes["sine"]["binary16"] = "0000".into();
                    });
                    xtask(memory, "check", &[])
                },
                BATTERY_ONLY,
                0,
                no_change,
            ),
        ],
        HintKind::BatteryAlone => battery_alone_rows(),
        HintKind::SpentBits => vec![row(
            "other bits than main at a spent version",
            Hint::SpentBits { name: "osc~" },
            |memory| {
                memory.main = Some(edited(&main_record(memory), other_osc_bits));
                xtask(memory, "check", &[])
            },
            "BITS     osc~ 1: other bits than the spent record holds at this version (a new \
             version of osc~, then run `cargo xtask bless`)",
            0,
            new_version,
        )],
        HintKind::BitsChanged => vec![row(
            "the outputs of osc~ changed at its version, at check",
            Hint::BitsChanged { name: "osc~" },
            |memory| {
                edit_records(memory, other_osc_bits);
                xtask(memory, "check", &[])
            },
            "BITS     osc~ 1: its outputs changed at the same algorithm version (a new version \
             of osc~, then run `cargo xtask bless`)",
            0,
            new_version,
        )],
        HintKind::BitsKept => vec![row(
            "the outputs of osc~ changed at its version, at bless",
            Hint::BitsKept { name: "osc~" },
            |memory| {
                edit_records(memory, other_osc_bits);
                xtask_bless(memory, &[])
            },
            "BITS     osc~ 1: its outputs changed at the same algorithm version (a new version \
             of osc~, then run `cargo xtask bless`)",
            0,
            new_version,
        )],
        HintKind::NoVersionsAtBless => [0, 1]
            .map(|alternative| {
                row(
                    "a case with references and no versions, at bless",
                    Hint::NoVersionsAtBless {
                        case: "sine",
                        record: record_file,
                    },
                    |memory: &mut Memory| {
                        drop(
                            memory
                                .files
                                .insert(PathBuf::from(RECORD), record("{}").into_bytes()),
                        );
                        xtask_bless(memory, &[])
                    },
                    "MISSING  sine: a reference but no versions recorded in the record bless \
                     replaces (`cargo xtask bless --case-changed sine`, or restore \
                     conf/algorithms.json from git)",
                    alternative,
                    no_change,
                )
            })
            .into(),
        HintKind::ChangedReference => {
            let changes: [(usize, Change); 2] = [(0, new_version), (1, no_change)];
            changes
                .map(|(alternative, change)| {
                    row(
                        "a reference changed under the same versions",
                        Hint::ChangedReference { case: "sine" },
                        |memory: &mut Memory| {
                            old_binary32(memory);
                            xtask_bless(memory, &[])
                        },
                        "CHANGED  sine binary32: its reference changed under the same algorithm \
                     versions (a new version, then run `cargo xtask bless`, or `cargo xtask \
                     bless --case-changed sine`)",
                        alternative,
                        change,
                    )
                })
                .into()
        }
    }
}

/// `edit` made to the JSON document `path` of `memory`, written as bless writes it.
fn edit_json(memory: &mut Memory, path: &str, edit: fn(&mut serde_json::Value)) {
    let text = edited(&memory.files[Path::new(path)], edit);
    drop(memory.files.insert(PathBuf::from(path), text));
}

/// A state of check with something stale: what it is, the file edited, the line check
/// gives without its hint, and the edit.
type Stale = (
    &'static str,
    &'static str,
    &'static str,
    fn(&mut serde_json::Value),
);

/// Every state of check where something is recorded that nothing has, which a bless drops.
fn stale_rows() -> Vec<Hinted> {
    let stale: [Stale; 5] = [
        (
            "a fingerprint of no box type",
            RECORD,
            "STALE    old~: fingerprint of no box type",
            |record| record["fingerprints"]["old~"] = serde_json::json!({ "1": "0000" }),
        ),
        (
            "a version of a box type the case does not use",
            RECORD,
            "STALE    sine old~: algorithm version of a box type the case does not use",
            |record| record["cases"]["sine"]["old~"] = 1_u32.into(),
        ),
        (
            "versions without a case",
            RECORD,
            "STALE    gone: algorithm versions without a case",
            |record| record["cases"]["gone"] = serde_json::json!({ "osc~": 1_u32 }),
        ),
        (
            "a reference of no render format",
            REFERENCES,
            "STALE    sine binary16: reference of no render format",
            |references| references["sine"]["binary16"] = "0000".into(),
        ),
        (
            "a reference without a case",
            REFERENCES,
            "STALE    gone: reference without a case",
            |references| references["gone"] = serde_json::json!({ "binary64": "0000" }),
        ),
    ];
    stale
        .into_iter()
        .map(|(state, path, line, edit)| {
            row(
                state,
                Hint::Stale,
                move |memory| {
                    edit_json(memory, path, edit);
                    xtask(memory, "check", &[])
                },
                &format!("{line} (run `cargo xtask bless`)"),
                0,
                no_change,
            )
        })
        .collect()
}

/// The line of check for a battery change that comes with nothing a bless changing the
/// battery refuses.
const BATTERY_ONLY: &str = "BATTERY  the fingerprints were computed with battery 1, the engine's \
                            is 2 (run `cargo xtask bless --battery-changed`)";

/// The procedure that blesses a battery change alone first: `then`, the end of the hint
/// after `--battery-changed`'s bless.
const ALONE: &str = "(bless the battery change alone first: set the other changes aside, then \
                     run `cargo xtask bless --battery-changed`, then commit, then bring the \
                     other changes back, then run `cargo xtask bless`)";

/// `["sine"]`, for the whole test run: the cases named in a hint the test writes.
fn named_sine() -> &'static [String] {
    Vec::leak(vec!["sine".to_owned()])
}

/// A state a command refuses with a hint ([`Hinted`]).
fn row(
    state: &'static str,
    hint: Hint<'static>,
    setup: impl Fn(&mut Memory) -> (Result<(), String>, String) + 'static,
    line: &str,
    alternative: usize,
    change: Change,
) -> Hinted {
    Hinted {
        state,
        hint,
        setup: Box::new(setup),
        line: line.to_owned(),
        causes: Vec::new(),
        alternative,
        change,
    }
}

/// A change made with a battery change, which no one bless accepts with it.
struct WithBattery {
    /// What it is.
    what: &'static str,
    /// How it is made, after the battery change.
    make: fn(&mut Memory),
    /// How its steps in words are made.
    apart: Change,
    /// The line of `bless` that refuses it, without a hint, with the option or without.
    cause: &'static str,
    /// The line of `check` that says it, without a hint: its command would be refused.
    at_check: fn() -> String,
    /// Whether the last bless of the procedure names the case `sine`.
    named: bool,
}

/// Every kind of change that can come with a battery change: a new version of a box type
/// a case uses, of one no case uses, other render rules, another render hash, a reference
/// changed under the same versions, a reference missing, a new case.
const WITH_BATTERY: [WithBattery; 7] = [
    WithBattery {
        what: "the version of osc~",
        make: |memory| edit_records(memory, osc_newer),
        apart: version_apart,
        cause: "VERSION  osc~: version 1 is not recorded",
        at_check: || "VERSION  osc~: version 1 is not recorded".to_owned(),
        named: false,
    },
    WithBattery {
        what: "a version of dac~ no case fails on",
        make: |memory| edit_records(memory, without_dac),
        apart: dac_apart,
        cause: "VERSION  dac~: version 1 is not recorded",
        at_check: || "VERSION  dac~: version 1 is not recorded".to_owned(),
        named: false,
    },
    WithBattery {
        what: "the render rules",
        make: |memory| edit_records(memory, older_rules),
        apart: rules_apart,
        cause: "RULES    --battery-changed: the render rules changed too, from version 0 to 3",
        at_check: || {
            "RULES    the references were rendered under render rules version 0, the engine's \
             is 3"
                .to_owned()
        },
        named: false,
    },
    WithBattery {
        what: "the render hash",
        make: |memory| edit_records(memory, older_hash),
        apart: hash_apart,
        cause: "HASH     --battery-changed: the render hash changed too, from render-hash/1 to \
                render-hash/2",
        at_check: || {
            "HASH     the references were hashed with render-hash/1, the engine with \
             render-hash/2"
                .to_owned()
        },
        named: false,
    },
    WithBattery {
        what: "a render",
        make: old_binary32,
        apart: reference_apart,
        cause: "CHANGED  sine binary32: its reference changed",
        at_check: || {
            format!(
                "MISMATCH sine binary32: expected 0000, got {} (the same algorithm versions: its \
                 bits changed without a new version)",
                sine_hash(RenderFormat::Binary32)
            )
        },
        named: true,
    },
    WithBattery {
        what: "a reference lost",
        make: no_binary32,
        apart: missing_apart,
        cause: "CHANGED  sine binary32: no reference in the references bless replaces",
        at_check: || "MISSING  sine binary32: no reference hash".to_owned(),
        named: false,
    },
    WithBattery {
        what: "a case added",
        make: add_new_case,
        apart: case_apart,
        cause: "CHANGED  new: a case the record bless replaces does not hold",
        at_check: || "MISSING  new: no algorithm versions".to_owned(),
        named: false,
    },
];

/// Every state where a battery change comes with another change ([`WITH_BATTERY`]), each
/// at check, at bless and at bless with `--battery-changed`: the line of the battery gives
/// the procedure once, after the line of the other change, which bless gives without a
/// hint, with the option or without.
fn battery_alone_rows() -> Vec<Hinted> {
    let battery = "BATTERY  the fingerprints were computed with battery 1, the engine's is 2";
    let at_bless = format!("{battery}, and a battery change blesses no reference and no version");
    let commands: [(&str, &[&str], &str); 3] = [
        ("check", &[], battery),
        ("bless", &[], &at_bless),
        ("bless", &["--battery-changed"], &at_bless),
    ];
    WITH_BATTERY
        .iter()
        .flat_map(|with| {
            commands.map(|(command, options, line)| {
                let (hint, alone) = if with.named {
                    (
                        Hint::BatteryAlone {
                            changed: named_sine(),
                        },
                        ALONE.replace(
                            "run `cargo xtask bless`)",
                            "run `cargo xtask bless --case-changed sine`)",
                        ),
                    )
                } else {
                    (Hint::BatteryAlone { changed: &[] }, ALONE.to_owned())
                };
                let make = with.make;
                Hinted {
                    state: with.what,
                    hint,
                    setup: Box::new(move |memory| {
                        edit_records(memory, older_battery);
                        make(memory);
                        xtask(memory, command, options)
                    }),
                    line: format!("{line} {alone}"),
                    causes: vec![if command == "check" {
                        (with.at_check)()
                    } else {
                        with.cause.to_owned()
                    }],
                    alternative: 0,
                    change: with.apart,
                }
            })
        })
        .collect()
}

/// A stale entry made in a file: the file, and the edit.
type StaleEdit = (&'static str, fn(&mut serde_json::Value));

/// At check, while the battery differs, no line but the battery's names a command: every
/// other one would be refused until the battery change is blessed, so each is said
/// without its hint, whatever came with the battery (stale entries included).
#[test]
fn check_names_no_refused_command_while_the_battery_differs() {
    let stale: [StaleEdit; 5] = [
        (RECORD, |record| {
            record["fingerprints"]["old~"] = serde_json::json!({ "1": "0000" });
        }),
        (RECORD, |record| {
            record["cases"]["sine"]["old~"] = 1_u32.into();
        }),
        (RECORD, |record| {
            record["cases"]["gone"] = serde_json::json!({ "osc~": 1_u32 });
        }),
        (REFERENCES, |hashes| {
            hashes["sine"]["binary16"] = "0000".into();
        }),
        (REFERENCES, |hashes| {
            hashes["gone"] = serde_json::json!({ "binary64": "0000" });
        }),
    ];
    let refuses = |what: &str, make: &dyn Fn(&mut Memory)| {
        let mut memory = committed();
        edit_records(&mut memory, older_battery);
        make(&mut memory);
        let (result, out) = xtask(&mut memory, "check", &[]);
        assert!(result.is_err(), "{what}: refused");
        let commands: Vec<&str> = out
            .lines()
            .filter(|line| !line.starts_with("BATTERY") && line.contains("cargo xtask"))
            .collect();
        assert_eq!(commands, Vec::<&str>::new(), "{what}: {out}");
        assert_eq!(
            out.lines()
                .filter(|line| line.starts_with("BATTERY"))
                .count(),
            1,
            "{what}: the battery's line, with its hint: {out}"
        );
    };
    for with in &WITH_BATTERY {
        refuses(with.what, &with.make);
    }
    for (path, edit) in stale {
        refuses(path, &|memory| edit_json(memory, path, edit));
    }
}

/// At bless, with `--battery-changed` or without, a change made with a battery change is
/// said once, without a hint, before the procedure; a name of `--case-changed` that is no
/// case is said on its own line, never in the procedure's command.
#[test]
fn bless_says_what_came_with_the_battery_without_a_hint() {
    let battery = "BATTERY  the fingerprints were computed with battery 1, the engine's is 2, \
                   and a battery change blesses no reference and no version";
    let named = ALONE.replace(
        "run `cargo xtask bless`)",
        "run `cargo xtask bless --case-changed sine`)",
    );
    for with in &WITH_BATTERY {
        for options in [&[][..], &["--battery-changed"]] {
            let mut memory = committed();
            edit_records(&mut memory, older_battery);
            (with.make)(&mut memory);
            let alone = if with.named { &named } else { ALONE };
            assert_eq!(
                xtask(&mut memory, "bless", options).1,
                format!("{}\n{battery} {alone}\n", with.cause),
                "{}, {options:?}",
                with.what
            );
        }
    }
    let mut memory = committed();
    edit_records(&mut memory, older_battery);
    old_binary32(&mut memory);
    assert_eq!(
        xtask(&mut memory, "bless", &["--case-changed", "a$(touch x) b"]).1,
        format!(
            "UNKNOWN  --case-changed a$(touch x) b: no such case\nCHANGED  sine binary32: its \
             reference changed\n{battery} {named}\n"
        ),
        "a name that is no case"
    );
}

#[test]
fn check_names_once_a_case_whose_references_changed_with_the_battery() {
    let mut memory = committed();
    edit_records(&mut memory, older_battery);
    let references = canonical(&format!(
        r#"{{"sine":{}}}"#,
        sine_references()
            .replace(&sine_hash(RenderFormat::Binary32), "0000")
            .replace(&sine_hash(RenderFormat::Binary64), "1111")
    ));
    drop(
        memory
            .files
            .insert(PathBuf::from(REFERENCES), references.into_bytes()),
    );
    let (result, out) = xtask(&mut memory, "check", &[]);
    assert_eq!(
        result,
        Err("conformance failed in conf: 3 problem(s)".to_owned()),
        "two mismatches and the battery"
    );
    assert!(
        out.ends_with(&format!(
            "{}\n",
            ALONE.replace(
                "run `cargo xtask bless`)",
                "run `cargo xtask bless --case-changed sine`)"
            )
        )),
        "two references of the case changed, the case named once: {out}"
    );
}

/// Every state a command refuses with a hint, for every kind of hint: the line asserted as
/// written, holding its hint, then the alternative followed, after which the directory
/// checks against the spent record; between them, the states of a kind follow every
/// alternative of its hint.
#[test]
fn every_hint_is_followed_from_a_state_that_gives_it() {
    for kind in HintKind::ALL {
        let rows = hinted(*kind);
        let mut followed = alloc::collections::BTreeSet::new();
        let mut count = None;
        for row in rows {
            let state = row.state;
            let hint = row.hint.to_string();
            assert_eq!(row.hint.kind(), *kind, "{state}: a hint of its kind");
            assert!(
                row.line.ends_with(&format!("{hint})")),
                "{state}: {hint} in {}",
                row.line
            );
            let mut memory = committed();
            let (result, out) = (row.setup)(&mut memory);
            let said: Vec<String> = result
                .err()
                .into_iter()
                .chain(out.lines().map(str::to_owned))
                .collect();
            assert!(
                said.iter().any(|line| line.ends_with(&row.line)),
                "{state}: {} in {said:?}",
                row.line
            );
            for cause in &row.causes {
                assert!(said.contains(cause), "{state}: {cause} in {said:?}");
            }
            assert!(
                said.iter()
                    .filter(|line| line.contains(" first: set the other changes aside"))
                    .count()
                    <= 1,
                "{state}: the procedure said once at most in {said:?}"
            );
            let alternatives = alternatives(&hint);
            follow(&mut memory, &alternatives[row.alternative], row.change);
            assert_eq!(
                xtask(&mut memory, "check", &[]).0,
                Ok(()),
                "{state}: checks once followed"
            );
            let _first = followed.insert(row.alternative);
            assert!(
                count
                    .replace(alternatives.len())
                    .is_none_or(|count| count == alternatives.len()),
                "{state}: as many alternatives as the other states of {kind:?}"
            );
        }
        assert_eq!(
            followed,
            (0..count.unwrap_or_default()).collect(),
            "{kind:?}: a state for each alternative"
        );
        assert!(count.is_some(), "{kind:?}: a state");
    }
}

#[test]
fn check_command_compares_the_fingerprints_with_the_record_on_main() {
    let mut memory = committed();
    assert_eq!(
        xtask(&mut memory, "check", &[]).0,
        Ok(()),
        "the spent record itself"
    );
    drop(
        memory
            .files
            .insert(PathBuf::from("null.json"), b"null\n".to_vec()),
    );
    assert_eq!(
        run_with(&mut memory, &["check", "conf", "--spent", "null.json"]).0,
        Ok(()),
        "no spent record"
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
            "BITS     osc~ 1: other bits than the spent record holds at this version (a new \
             version of osc~, then run `cargo xtask bless`)\n"
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
        "a spent record to read"
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

/// A spent record of another battery cannot be compared with: check says so, in a line
/// that is no problem, and says nothing when it compares.
#[test]
fn check_command_says_when_it_cannot_compare_with_the_record_on_main() {
    let mut memory = committed();
    let ok = RenderFormat::ALL
        .map(|format| format!("ok       sine {format} {}\n", sine_hash(format)))
        .concat();
    let older = memory
        .text("conf/algorithms.json")
        .replace(r#""fingerprint_battery": 2"#, r#""fingerprint_battery": 1"#);
    drop(
        memory
            .files
            .insert(PathBuf::from("main.json"), older.into_bytes()),
    );
    assert_eq!(
        run_with(&mut memory, &["check", "conf", "--spent", "main.json"]),
        (
            Ok(()),
            format!(
                "{ok}skipped  the spent record: its fingerprints were computed with battery 1, \
                 these with battery 2 (compared once the battery change reaches main)\n"
            )
        ),
        "another battery on main: not compared, and said"
    );
    assert_eq!(
        xtask(&mut memory, "check", &[]),
        (Ok(()), ok.clone()),
        "the same battery: compared, nothing said"
    );
    drop(
        memory
            .files
            .insert(PathBuf::from("null.json"), b"null\n".to_vec()),
    );
    assert_eq!(
        run_with(&mut memory, &["check", "conf", "--spent", "null.json"]),
        (Ok(()), ok),
        "no spent record: nothing to compare with, nothing said"
    );
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
        .replace("\"render_rules\": 3", "\"render_rules\": 0");
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
             3 (run `cargo xtask bless`)\n"
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
            "option \"--case-changed\" needs a value",
        ),
        (
            &["bless", "a", "--spent"][..],
            "option \"--spent\" needs a value",
        ),
        // A value is never one of the options: each option given as the value of another.
        (
            &["bless", "a", "--case-changed", "--battery-changed"][..],
            "option \"--case-changed\" needs a value",
        ),
        (
            &["bless", "a", "--case-changed", "--from-nothing"][..],
            "option \"--case-changed\" needs a value",
        ),
        (
            &["bless", "a", "--case-changed", "--spent", "x"][..],
            "option \"--case-changed\" needs a value",
        ),
        (
            &["bless", "a", "--spent", "--case-changed", "x"][..],
            "option \"--spent\" needs a value",
        ),
        (
            &["bless", "a", "--spent", "--battery-changed"][..],
            "option \"--spent\" needs a value",
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
        run(
            &args(&["help"]),
            &mut Memory::default(),
            &mut Broken,
            &mut Vec::new()
        )
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
fn the_usage_says_what_each_command_does_with_non_finite_samples() {
    // Each command's part of the usage: from its line, `  <command> `, to the next one, its
    // words on one line.
    let mut parts: Vec<(String, String)> = Vec::new();
    for line in USAGE.lines().skip(1) {
        let words = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.starts_with("   ") {
            let (_, part) = parts.last_mut().expect("a command first");
            part.push(' ');
            part.push_str(&words);
        } else {
            let command = words.split(' ').next().expect("a command").to_owned();
            parts.push((command, words));
        }
    }
    for (command, words) in [
        (
            "render",
            "warns on stderr of non-finite samples (NaN or infinite), each rendered as zero, \
             the exit code unchanged",
        ),
        (
            "check",
            "fails a case whose render replaced a non-finite sample, even when its hash \
             matches its reference",
        ),
        ("bless", "a case whose render replaced a non-finite sample"),
        (
            "export",
            "refuses each render that replaced a non-finite sample, naming it on its line, \
             writes every other one, then fails counting them; a case that cannot be \
             prepared stops it, leaving the files written before it; files already in <out> \
             stay, an earlier file of a render it refuses among them",
        ),
    ] {
        let part = parts
            .iter()
            .find(|(name, _)| name == command)
            .map(|(_, part)| part.as_str())
            .expect("the command is described");
        assert!(part.contains(words), "{command}: {words:?} in {part:?}");
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
        run(
            &args(&["check", "conf"]),
            &mut memory,
            &mut Broken,
            &mut Vec::new()
        )
        .map_err(|error| error.to_string()),
        Err("cannot write to stdout: closed".to_owned()),
        "check"
    );
    let mut bad = conformance(r#"{"sine":{"binary64":"0"}}"#);
    assert_eq!(
        run(
            &args(&["check", "conf"]),
            &mut bad,
            &mut Broken,
            &mut Vec::new()
        )
        .map_err(|error| error.to_string()),
        Err("cannot write to stdout: closed".to_owned()),
        "mismatch line"
    );
    let mut missing = conformance("{}");
    assert_eq!(
        run(
            &args(&["check", "conf"]),
            &mut missing,
            &mut Broken,
            &mut Vec::new()
        )
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
        run(
            &args(&["check", "conf"]),
            &mut stale,
            &mut three,
            &mut Vec::new()
        )
        .map_err(|error| error.to_string()),
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
            &mut three_more,
            &mut Vec::new()
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

/// A fresh, empty directory for one test, `audio-cli-<name>-<process id>` ([`Scratch`]).
#[expect(
    clippy::disallowed_methods,
    reason = "the one place audio-cli's unit tests name the temporary directory: the guard removes what it \
              creates"
)]
fn scratch(name: &str) -> Scratch {
    let dir = std::env::temp_dir().join(format!("audio-cli-{name}-{}", std::process::id()));
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
        " ".repeat(audio_patch::MAX_PATCH_BYTES - PATCH.len())
    );
    let hashes = canonical(&format!(r#"{{"c":{}}}"#, sine_references()));
    let references = format!(
        "{hashes}{}",
        " ".repeat(super::MAX_REFERENCES_BYTES - hashes.len())
    );
    let recorded = record(&format!("{{{}}}", sine_algorithms("c")));
    let algorithms = format!("{recorded}{}", " ".repeat(1_048_576 - recorded.len()));
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
        ("conf/patches/c.json", audio_patch::MAX_PATCH_BYTES, &patch),
        (
            "conf/references.json",
            super::MAX_REFERENCES_BYTES,
            &references,
        ),
        ("conf/algorithms.json", 1_048_576, &algorithms),
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

/// The spent record, an `algorithms.json` read from git: read at 1 MiB, refused one byte
/// past it.
#[test]
fn refuses_a_spent_record_larger_than_its_limit() {
    let spent = record(&format!("{{{}}}", sine_algorithms("sine")));
    let at_limit = format!("{spent}{}", " ".repeat(1_048_576 - spent.len()));
    for (text, result) in [
        (at_limit.clone(), Ok(())),
        (
            format!("{at_limit} "),
            Err(
                "cannot read the spent record spent.json: larger than 1048576 bytes \
                 (`cargo xtask check` writes it: run that)"
                    .to_owned(),
            ),
        ),
    ] {
        let mut memory = committed();
        drop(
            memory
                .files
                .insert(PathBuf::from("spent.json"), text.into_bytes()),
        );
        assert_eq!(
            run_with(&mut memory, &["check", "conf", "--spent", "spent.json"]).0,
            result,
            "the spent record at its limit, then one byte past"
        );
    }
}

#[test]
fn prepares_a_case_before_rendering() {
    for format in RenderFormat::ALL {
        let prepared = prepare(SINE, PATCH, format).expect("valid case");
        let rendered = with_default_mode(|mode| prepared.render(mode));
        assert_eq!(
            (rendered.channels, format_of(&rendered)),
            (2, format),
            "its channels and format"
        );
        assert_eq!(
            rendered.samples.as_samples().len(),
            200,
            "100 frames x 2 channels"
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
            prepare(&huge, PATCH, format).map(|_| ()),
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
    assert!(error.starts_with("conf/cases/bad"), "{error}");
    assert!(error.ends_with(&format!(": {NOT_A_CASE_NAME}")), "{error}");
}

/// What `check` says of a case or patch file whose name is not a case name.
const NOT_A_CASE_NAME: &str = "not a case name (lowercase ASCII letters, digits, dots and \
                               hyphens, starting with a letter or a digit)";

#[test]
fn reads_only_case_names_a_command_can_hold_unquoted() {
    for name in [
        "a",
        "0",
        "z9",
        "a.b-c",
        "sine-1.1754944e-38-at-8k",
        "9-.",
        "0.-",
    ] {
        let mut memory = Memory::with(&[
            (&format!("conf/cases/{name}.json"), SINE),
            (&format!("conf/patches/{name}.json"), PATCH),
        ]);
        let said = run_with(&mut memory, &["check", "conf"]).0;
        assert!(
            said.as_ref()
                .is_err_and(|error| error.starts_with("cannot read conf/references.json: ")),
            "{name}: read as a case, {said:?}"
        );
    }
    for name in [
        "A", "sinE", "my case", "-x", ".x", "a_b", "é", "a$(b)", "a\u{0}", "..",
    ] {
        for (case, patch) in [(name, "a"), ("a", name)] {
            let mut memory = Memory::with(&[
                (&format!("conf/cases/{case}.json"), SINE),
                (&format!("conf/patches/{patch}.json"), PATCH),
            ]);
            let folder = if case == name { "cases" } else { "patches" };
            assert_eq!(
                run_with(&mut memory, &["check", "conf"]).0,
                Err(format!("conf/{folder}/{name}.json: {NOT_A_CASE_NAME}")),
                "{name:?} in {folder}"
            );
        }
    }
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
    let unknown = run(
        &args(&["nope"]),
        &mut memory,
        &mut Vec::new(),
        &mut Vec::new(),
    );
    assert_eq!(
        unknown,
        Err(CliError::Usage("unknown command \"nope\"".to_owned())),
        "usage"
    );
    let failed = run(
        &args(&["check", "conf"]),
        &mut memory,
        &mut Vec::new(),
        &mut Vec::new(),
    );
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
        "error: unknown command \"\\u{1b}[2J\" (see: audio-cli help)",
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
    let line = CliError::Failed("x".repeat(audio_patch::MAX_MESSAGE_CHARS + 1)).line();
    assert_eq!(
        line,
        format!("error: {}…", "x".repeat(audio_patch::MAX_MESSAGE_CHARS - 1)),
        "the message is cut, the prefix kept"
    );
    for length in [
        audio_patch::MAX_MESSAGE_CHARS,
        audio_patch::MAX_MESSAGE_CHARS + 1,
    ] {
        let reason = "x".repeat(length);
        let bounded = audio_patch::safe_message(&reason);
        assert_eq!(
            CliError::Usage(reason).line(),
            format!("error: {bounded} (see: audio-cli help)"),
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
/// formatting or line-separator character (written independently of `audio-patch`).
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
            line.chars().count() <= audio_patch::MAX_MESSAGE_CHARS,
            "bounded: {} characters",
            line.chars().count()
        );
    }
}

/// Box type of a patch, crafted to forge a verdict line and drive the terminal.
const FORGED_BOX: &str = "y\\nok       sine-440 0\\u001b[2K\\u202e";

/// A conformance directory whose untrusted text attacks the output (case names are case
/// names, refused otherwise where they are read): a box type quoted by an error, a 2 MB box
/// type, and the keys and values of `references.json`. Six problems: two failed renders, a
/// mismatch, a missing reference and two stale ones.
fn hostile_corpus() -> Memory {
    let huge_box = format!(
        r#"{{"format":1,"nodes":[{{"id":1,"box":"{}"}}],"connections":[]}}"#,
        "x".repeat(2_000_000)
    );
    let forged_box =
        format!(r#"{{"format":1,"nodes":[{{"id":1,"box":"{FORGED_BOX}"}}],"connections":[]}}"#);
    let references = "{\"b\\u001b[2J\\nok       b\":{},\
         \"b\":{\"binary64\":\"0\\nok       forged\\u001b[31m\\u202e\",\
         \"\\u001b[2Jbinary16\\nok\":\"0\"},\
         \"gone\\nok       gone\\u0007\":{}}";
    Memory::with(&[
        ("conf/cases/a.json", SINE),
        ("conf/patches/a.json", &forged_box),
        ("conf/cases/b.json", SINE),
        ("conf/patches/b.json", PATCH),
        ("conf/cases/c.json", SINE),
        ("conf/patches/c.json", &huge_box),
        ("conf/cases/d.json", SINE),
        ("conf/patches/d.json", PATCH),
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
    let mut named = Memory::with(&[("conf/cases/a\nok       a 0\u{1b}[2J.json", SINE)]);
    assert_eq!(
        run(
            &args(&["check", "conf"]),
            &mut named,
            &mut Vec::new(),
            &mut Vec::new()
        )
        .map_err(|error| error.line()),
        Err(format!(
            "error: conf/cases/a\\u{{a}}ok       a 0\\u{{1b}}[2J.json: {NOT_A_CASE_NAME}"
        )),
        "a name that is no case name is escaped in its refusal"
    );
    assert!(
        out.contains("unknown box type \"y\\u{a}ok       sine-440 0\\u{1b}[2K\\u{202e}\""),
        "the box type is escaped: {out}"
    );
    assert!(
        out.contains(&format!(
            "MISMATCH b binary64: expected 0\\u{{a}}ok       forged\\u{{1b}}[31m\\u{{202e}}, got {}",
            sine_hash(RenderFormat::Binary64)
        )),
        "the reference is escaped: {out}"
    );
    assert!(
        out.contains("STALE    b \\u{1b}[2Jbinary16\\u{a}ok: reference of no render format"),
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
        ("conf\u{1b}[2J/cases/a.json", SINE),
        ("conf\u{1b}[2J/patches/a.json", PATCH),
    ]);
    let (blessed, bless_out) = run_with(&mut memory, &["bless", "conf\u{1b}[2J", "--from-nothing"]);
    assert_eq!(blessed, Ok(()), "blessed");
    assert_eq!(
        bless_out,
        "blessed 1 case(s) in conf\\u{1b}[2J/references.json and conf\\u{1b}[2J/algorithms.json\n",
        "the directory is escaped"
    );
    let (exported, export_out) =
        run_with(&mut memory, &["export", "conf\u{1b}[2J", "out\n\u{202e}"]);
    assert_eq!(exported, Ok(()), "exported");
    let lines = RenderFormat::ALL
        .map(|format| {
            format!(
                "wrote    out\\u{{a}}\\u{{202e}}/a.{format}.wav {}\n",
                sine_hash(format)
            )
        })
        .concat();
    assert_eq!(export_out, lines, "the path is escaped");
}

/// A render of two samples by hand, whose output stage replaced `replaced`.
fn replacing(replaced: Replaced) -> Rendered {
    Rendered {
        rate: SampleRate::HZ_48000,
        channels: 1,
        samples: RenderedSamples::Binary32(vec![0.0; 2]),
        wav_header: [0; HEADER_LEN],
        replaced,
    }
}

#[test]
fn words_what_the_output_stage_replaced() {
    use audio_core::FirstReplaced;
    assert_eq!(
        super::cli::replaced_words(&replacing(Replaced::default())),
        None
    );
    assert_eq!(
        super::cli::replaced_words(&replacing(Replaced::new(1, Some(FirstReplaced {
                frame: 7,
                channel: 0
            })).expect("a count and a first sample that agree"))),
        Some("1 non-finite sample (NaN or infinite) rendered as zero, at frame 7 of channel 0 (both counted from 0)".to_owned())
    );
    assert_eq!(
        super::cli::replaced_words(&replacing(
            Replaced::new(
                u64::MAX,
                Some(FirstReplaced {
                    frame: u64::MAX,
                    channel: 3
                })
            )
            .expect("a count and a first sample that agree")
        )),
        Some(format!(
            "{} non-finite samples (NaN or infinite) rendered as zero, the first at frame {} of \
             channel 3 (both counted from 0)",
            u64::MAX,
            u64::MAX
        ))
    );
}

#[test]
fn warns_of_replaced_samples_on_render_and_refuses_them_in_conformance() {
    use audio_core::FirstReplaced;
    let replaced = replacing(
        Replaced::new(
            2,
            Some(FirstReplaced {
                frame: 3,
                channel: 1,
            }),
        )
        .expect("a count and a first sample that agree"),
    );
    let mut warnings = Vec::new();
    super::cli::warn_replaced(
        &mut warnings,
        ("cases/a\u{1b}.json", "patches/a\n.json"),
        &replaced,
    );
    assert_eq!(
        String::from_utf8(warnings).expect("UTF-8"),
        "warning: cases/a\\u{1b}.json with patches/a\\u{a}.json: 2 non-finite samples (NaN or infinite) rendered as zero, the \
         first at frame 3 of channel 1 (both counted from 0): from a box of this patch\n",
        "one line, the case and patch files escaped"
    );
    let mut none = Vec::new();
    super::cli::warn_replaced(
        &mut none,
        ("cases/a.json", "patches/a.json"),
        &replacing(Replaced::default()),
    );
    assert!(none.is_empty(), "nothing replaced, nothing said");
    // A closed stderr changes nothing.
    super::cli::warn_replaced(&mut Broken, ("cases/a.json", "patches/a.json"), &replaced);
    assert_eq!(
        super::cli::conformant(&replaced, "sine-440", RenderFormat::Binary32).map(|_| ()),
        Err(
            "FAILED   sine-440 binary32: 2 non-finite samples (NaN or infinite) rendered as zero, \
             the first at frame 3 of channel 1 (both counted from 0): a defect of a box, which \
             no conformance case may hold"
                .to_owned()
        ),
        "a conformance case refused"
    );
    let clean = replacing(Replaced::default());
    assert!(
        super::cli::conformant(&clean, "sine-440", RenderFormat::Binary32)
            .is_ok_and(|kept| core::ptr::eq(kept, &raw const clean)),
        "a clean render kept as it is"
    );
}

#[test]
fn check_fails_a_render_with_replaced_samples_as_bless_does() {
    use audio_core::FirstReplaced;
    let replaced = replacing(
        Replaced::new(
            1,
            Some(FirstReplaced {
                frame: 0,
                channel: 0,
            }),
        )
        .expect("a count and a first sample that agree"),
    );
    assert_eq!(
        super::cli::conformant_hash(&replaced),
        Err(
            "1 non-finite sample (NaN or infinite) rendered as zero, at frame 0 of channel 0 \
             (both counted from 0): a defect of a box, which no conformance case may hold"
                .to_owned()
        ),
        "the defect, the line naming the case"
    );
    let clean = replacing(Replaced::default());
    let expected = clean
        .hash()
        .expect("a render of two samples hashes")
        .to_string();
    assert_eq!(
        super::cli::conformant_hash(&clean),
        Ok(expected),
        "a clean render's hash"
    );
}

#[test]
fn reads_the_decimals_of_a_case_correctly_rounded() {
    // A lookahead whose nearest binary64 is 0x3ff9_5f47_e53a_7889: serde_json's default
    // parser, which is not correctly rounded, reads it one ulp above. A case's values are
    // read as the oracle and the browsers read them (correctly rounded) because audio-cli
    // asks for serde_json's `float_roundtrip` itself, not by the chance of another crate.
    let read: f64 = serde_json::from_str("1.58576192419380597").expect("a number");
    assert_eq!(
        read.to_bits(),
        0x3ff9_5f47_e53a_7889,
        "the nearest binary64"
    );
}

#[test]
fn renders_a_case_through_its_output_stage() {
    // Raw when absent or named; protected through the limiter, delayed by its latency: at
    // 48 kHz, 0.5 ms of lookahead (24 frames) and the standard detector's 6.
    let raw = render_case(
        r#"{"sample_rate":48000,"frames":100}"#,
        PATCH,
        RenderFormat::Binary64,
    )
    .expect("raw");
    let named = render_case(
        r#"{"sample_rate":48000,"frames":100,"output":{"mode":"raw"}}"#,
        PATCH,
        RenderFormat::Binary64,
    )
    .expect("raw, named");
    assert_eq!(raw.hash(), named.hash(), "raw either way");
    let protected = render_case(
        r#"{"sample_rate":48000,"frames":100,"output":{"mode":"protected","lookahead":0.5,"detector":"standard"}}"#,
        PATCH,
        RenderFormat::Binary64,
    )
    .expect("protected");
    let RenderedSamples::Binary64(samples) = protected.samples() else {
        panic!("binary64 samples");
    };
    assert!(
        samples[..60].iter().all(|sample| sample.to_bits() == 0),
        "30 frames of silence in stereo"
    );
    assert!(
        samples[60..].iter().any(|sample| *sample != 0.0_f64),
        "then the limited sine"
    );
    assert_ne!(protected.hash(), raw.hash(), "another render");
    // The precise detector looks 16 frames behind: 40 frames of silence in stereo, 80
    // samples, at 0.5 ms of lookahead.
    let precise = render_case(
        r#"{"sample_rate":48000,"frames":100,"output":{"mode":"protected","lookahead":0.5,"detector":"precise"}}"#,
        PATCH,
        RenderFormat::Binary32,
    )
    .expect("protected, precise");
    let RenderedSamples::Binary32(narrow) = precise.samples() else {
        panic!("binary32 samples");
    };
    assert!(
        narrow[..80].iter().all(|sample| sample.to_bits() == 0),
        "40 frames of silence"
    );
    assert!(
        narrow[80..].iter().any(|sample| *sample != 0.0_f32),
        "then the limited sine"
    );
}

#[test]
fn renders_every_mode_and_detector_in_every_format() {
    // Raw named, then protected with each detector, in each render format: silence over
    // the latency (0.5 ms, 24 frames, and the detector's 6, 16 or 64), then sound.
    for format in RenderFormat::ALL {
        for (output, latency) in [
            (r#"{"mode":"raw"}"#, 0_usize),
            (
                r#"{"mode":"protected","lookahead":0.5,"detector":"standard"}"#,
                30,
            ),
            (
                r#"{"mode":"protected","lookahead":0.5,"detector":"precise"}"#,
                40,
            ),
            (
                r#"{"mode":"protected","lookahead":0.5,"detector":"mastering"}"#,
                88,
            ),
        ] {
            let case = format!(r#"{{"sample_rate":48000,"frames":200,"output":{output}}}"#);
            let rendered = render_case(&case, PATCH, format).expect("a render");
            let silent = |count: usize| match rendered.samples() {
                RenderedSamples::Binary64(samples) => samples
                    .iter()
                    .take(count)
                    .all(|sample| sample.to_bits() == 0),
                RenderedSamples::Binary64AsBinary32(samples)
                | RenderedSamples::Binary32(samples) => samples
                    .iter()
                    .take(count)
                    .all(|sample| sample.to_bits() == 0),
            };
            // Stereo: two samples a frame; the sine starts at zero, so its first frame is
            // silent too.
            assert!(
                silent(latency.saturating_mul(2)),
                "{format} {output}: silent over the latency"
            );
            assert!(
                !silent(latency.saturating_add(2).saturating_mul(2)),
                "{format} {output}: then sound"
            );
        }
    }
}

/// Outputs silence but a NaN at the one frame its parameter `frame` names, counted from the
/// first frame it renders: a defect no real box has, so that the commands meet a render
/// whose output stage replaced a sample (`spec/engine-design.md`, decision 12). Test only,
/// in [`NAN_REGISTRY`]; the third such box beside `audio-core`'s and the plan laws'
/// `nonfinite~` (#24 step 38).
#[derive(Debug)]
struct NanAt {
    /// The frame of the NaN, as its parameter gives it.
    at: f64,
    /// The first frame of the next block.
    next: u32,
}

impl<S: audio_core::Sample, const N: usize> audio_core::Object<S, N> for NanAt {
    fn process(
        &mut self,
        _inputs: &[audio_core::Block<S, N>],
        outputs: &mut [audio_core::Block<S, N>],
    ) {
        let value = |raw: f64| S::from_binary64(audio_core::host::Raw::from_raw(raw));
        for output in outputs.iter_mut() {
            for (sample, frame) in output.iter_mut().zip(self.next..) {
                *sample = if f64::from(frame).to_bits() == self.at.to_bits() {
                    value(f64::NAN)
                } else {
                    value(0.0_f64)
                };
            }
        }
        self.next = self
            .next
            .wrapping_add(u32::try_from(N).expect("a block of fewer than 2^32 frames"));
    }
}

impl audio_core::Create for NanAt {
    const BOX_MODE: audio_core::BoxMode = audio_core::BoxMode::LiveCapable;
    const PARAMS: &'static [audio_core::ParamSpec] = &[audio_core::param!(
        "frame",
        "The frame whose sample is a NaN",
        0.0,
        0.0,
        16_777_216.0
    )];

    fn create<S: audio_core::Sample, const N: usize>(
        _rate: SampleRate,
        params: audio_core::Params<'_>,
    ) -> Box<dyn audio_core::Object<S, N>> {
        let at = params.first().map_or(0.0_f64, |&frame| {
            <audio_core::Binary64 as audio_core::host::Raw>::raw(frame)
        });
        Box::new(Self { at, next: 0 })
    }
}

/// The `nan-at~` box type ([`NanAt`]).
static NAN_AT: audio_core::ObjectSpec = audio_core::ObjectSpec::new::<NanAt>(
    "nan-at~",
    "A NaN at one frame, silence elsewhere",
    &[],
    &[audio_core::PortSpec {
        name: "out",
        description: "Silence, and a NaN at the frame given",
    }],
    &[],
    audio_core::AlgorithmVersion::FIRST,
);

/// The engine's box types, and `nan-at~`.
static NAN_REGISTRY: [&audio_core::ObjectSpec; 3] = [&audio_boxes::OSC, &audio_boxes::DAC, &NAN_AT];

/// A patch whose `nan-at~` gives a NaN at `frame`, on the left channel of `dac~`.
fn nan_patch(frame: u32) -> String {
    format!(
        r#"{{"format":1,
  "nodes":[{{"id":1,"box":"nan-at~","params":{{"frame":{frame}}}}},{{"id":2,"box":"dac~"}}],
  "connections":[{{"from":[1,0],"to":[2,0]}}]}}"#
    )
}

/// Runs `words` with the box types of [`NAN_REGISTRY`]: the result, stdout and stderr.
fn run_nan(memory: &mut Memory, words: &[&str]) -> (Result<(), String>, String, String) {
    let (mut out, mut warnings) = (Vec::new(), Vec::new());
    let result = super::cli::run_in(
        &NAN_REGISTRY,
        &args(words),
        memory,
        (&mut out, &mut warnings),
    )
    .map_err(|error| error.to_string());
    let text = |bytes: Vec<u8>| String::from_utf8(bytes).expect("utf-8 output");
    (result, text(out), text(warnings))
}

/// The line the commands print for the NaN of [`nan_patch`] at frame 7 in case `name` and
/// `format`, as `check` prints a failed case.
fn nan_defect(name: &str, format: RenderFormat) -> String {
    format!(
        "FAILED   {name} {format}: 1 non-finite sample (NaN or infinite) rendered as zero, at \
         frame 7 of channel 0 (both counted from 0): a defect of a box, which no conformance \
         case may hold\n"
    )
}

#[test]
fn renders_a_case_whose_box_gives_a_nan_and_warns() {
    // The NaN replaced by zero: the render is the silence the same patch renders with its
    // NaN past the end, and stderr says what was replaced, the exit code unchanged.
    let silent = nan_patch(1000);
    let mut memory = Memory::with(&[
        ("a.json", SINE),
        ("nan.json", &nan_patch(7)),
        ("silent.json", &silent),
    ]);
    let (result, out, warnings) = run_nan(&mut memory, &["render", "a.json", "nan.json"]);
    assert_eq!(result, Ok(()), "rendered");
    let (_, silence, quiet) = run_nan(&mut memory, &["render", "a.json", "silent.json"]);
    assert_eq!(out, silence, "the NaN rendered as zero");
    assert_eq!(
        warnings,
        "warning: a.json with nan.json: 1 non-finite sample (NaN or infinite) rendered as zero, at \
         frame 7 of channel 0 (both counted from 0): from a box of this patch\n",
        "one warning"
    );
    assert_eq!(quiet, "", "no NaN, no warning");
}

#[test]
fn warns_only_of_a_nan_its_render_delivers_raw_and_limited() {
    // A NaN at frame 100: delivered by a raw render of 101 frames, not by one of 100, whose
    // last block computes it all the same; through a limiter whose latency is 30 frames at
    // 48 kHz (0.5 ms, 24 frames, and the standard detector's 6), delivered only by a render
    // of 131 frames.
    let limited = |frames: u32| {
        format!(
            r#"{{"sample_rate":48000,"frames":{frames},
  "output":{{"mode":"protected","lookahead":0.5,"detector":"standard"}}}}"#
        )
    };
    let raw = |frames: u32| format!(r#"{{"sample_rate":48000,"frames":{frames}}}"#);
    for (case, warned) in [
        (raw(100), false),
        (raw(101), true),
        (limited(130), false),
        (limited(131), true),
    ] {
        let mut memory = Memory::with(&[("a.json", &case), ("nan.json", &nan_patch(100))]);
        let (result, _, warnings) = run_nan(&mut memory, &["render", "a.json", "nan.json"]);
        assert_eq!(result, Ok(()), "{case}: rendered");
        let expected = if warned {
            "warning: a.json with nan.json: 1 non-finite sample (NaN or infinite) rendered as zero, at \
             frame 100 of channel 0 (both counted from 0): from a box of this patch\n"
        } else {
            ""
        };
        assert_eq!(warnings, expected, "{case}");
    }
}

#[test]
fn check_fails_a_case_whose_box_gives_a_nan_even_when_its_hash_matches() {
    // Blessed with the NaN past the end, the case renders silence; with the NaN at frame 7,
    // the output stage replaces it by zero, the same silence, the same hash: check fails it
    // all the same, in every format.
    let mut memory = Memory::with(&[
        ("conf/cases/nan.json", SINE),
        ("conf/patches/nan.json", &nan_patch(1000)),
    ]);
    let (blessed, ..) = run_nan(&mut memory, &["bless", "conf", "--from-nothing"]);
    assert_eq!(blessed, Ok(()), "blessed without a NaN");
    let (passed, clean, _) = run_nan(&mut memory, &["check", "conf"]);
    assert_eq!(passed, Ok(()), "{clean}");
    drop(memory.files.insert(
        PathBuf::from("conf/patches/nan.json"),
        nan_patch(7).into_bytes(),
    ));
    let (result, out, _) = run_nan(&mut memory, &["check", "conf"]);
    assert_eq!(
        result,
        Err("conformance failed in conf: 3 problem(s)".to_owned()),
        "every format fails"
    );
    let lines = RenderFormat::ALL
        .map(|format| nan_defect("nan", format))
        .concat();
    assert_eq!(out, lines, "each format failed, though its hash matches");
}

#[test]
fn bless_refuses_every_case_whose_box_gives_a_nan_at_once() {
    // A clean case between two whose box gives a NaN: every render of both refused, each on
    // its line, then one error, nothing written; one such case alone, its three formats.
    for (cases, refused) in [
        (&["nan", "one", "two"][..], &["nan", "two"][..]),
        (&["nan"], &["nan"]),
    ] {
        let mut memory = Memory::default();
        for name in cases {
            let patch = if *name == "one" {
                nan_patch(1000)
            } else {
                nan_patch(7)
            };
            drop(memory.files.insert(
                PathBuf::from(format!("conf/cases/{name}.json")),
                SINE.as_bytes().to_vec(),
            ));
            drop(memory.files.insert(
                PathBuf::from(format!("conf/patches/{name}.json")),
                patch.into_bytes(),
            ));
        }
        let (result, out, _) = run_nan(&mut memory, &["bless", "conf", "--from-nothing"]);
        assert_eq!(
            result,
            Err(format!(
                "bless refused in conf: {} problem(s), nothing written",
                3 * refused.len()
            )),
            "{cases:?}"
        );
        let lines: String = refused
            .iter()
            .flat_map(|name| RenderFormat::ALL.map(|format| nan_defect(name, format)))
            .collect();
        assert_eq!(
            out, lines,
            "{cases:?}: every render refused, each on its line"
        );
        assert!(
            !memory.files.contains_key(Path::new("conf/references.json"))
                && !memory.files.contains_key(Path::new("conf/algorithms.json")),
            "{cases:?}: nothing written"
        );
    }
}

#[test]
fn export_refuses_every_render_whose_box_gives_a_nan_and_writes_the_others() {
    // Each render of the two cases whose box gives a NaN refused on its line, before its
    // WAV file; the clean cases around them written; then one error counting them.
    let mut memory = Memory::with(&[
        ("conf/cases/a.json", SINE),
        ("conf/patches/a.json", PATCH),
        ("conf/cases/nan.json", SINE),
        ("conf/patches/nan.json", &nan_patch(7)),
        ("conf/cases/other.json", SINE),
        ("conf/patches/other.json", &nan_patch(7)),
        ("conf/cases/z.json", SINE),
        ("conf/patches/z.json", PATCH),
    ]);
    let (result, out, _) = run_nan(&mut memory, &["export", "conf", "out"]);
    assert_eq!(
        result,
        Err("export failed in conf: 6 render(s) refused".to_owned()),
        "refused"
    );
    let wrote = |name: &str| {
        RenderFormat::ALL
            .map(|format| format!("wrote    out/{name}.{format}.wav {}\n", sine_hash(format)))
            .concat()
    };
    let refused = |name: &str| {
        RenderFormat::ALL
            .map(|format| nan_defect(name, format))
            .concat()
    };
    assert_eq!(
        out,
        [wrote("a"), refused("nan"), refused("other"), wrote("z")].concat(),
        "the clean cases written, every refused render said"
    );
    assert!(
        !memory
            .files
            .keys()
            .any(|path| path.starts_with("out") && path.to_string_lossy().contains("nan")),
        "no WAV file of the refused case"
    );
}

#[test]
fn reads_each_detector_of_a_case_by_its_name() {
    for (name, detector) in [
        ("standard", audio_core::Detector::Standard),
        ("precise", audio_core::Detector::Precise),
        ("mastering", audio_core::Detector::Mastering),
    ] {
        let read: super::case::CaseDetector =
            serde_json::from_str(&format!("\"{name}\"")).expect("a detector's name");
        assert_eq!(read.0, detector, "{name}");
    }
    assert!(
        serde_json::from_str::<super::case::CaseDetector>("\"Precise\"").is_err(),
        "a name only, as written"
    );
}
