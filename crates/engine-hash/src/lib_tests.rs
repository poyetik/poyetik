//! Tests of the render hash, against the shared vectors.

extern crate alloc;

use alloc::string::ToString;
use alloc::vec::Vec;

use engine_core::{ArithmeticMode, MODE, RenderFormat, SampleRate};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{NotWholeFrames, RenderHash, Samples, TAG, render_hash};

const RATE: SampleRate = SampleRate::HZ_48000;

/// The known-answer vectors of the specification, shared with the TypeScript
/// implementations and computed independently of all of them.
const VECTORS: &str = include_str!("../../../spec/render-hash-vectors.json");

/// Reads a field of a vector as an unsigned integer.
fn number(vector: &Value, key: &str) -> u64 {
    vector[key].as_u64().expect("an unsigned integer")
}

/// Reads a field of a vector as text.
fn text<'json>(vector: &'json Value, key: &str) -> &'json str {
    vector[key].as_str().expect("a string")
}

/// Decodes hexadecimal digits, two per byte.
fn hex_bytes(digits: &str) -> Vec<u8> {
    digits
        .as_bytes()
        .chunks(2)
        .map(|pair| {
            u8::from_str_radix(core::str::from_utf8(pair).expect("ASCII"), 16).expect("hexadecimal")
        })
        .collect()
}

/// The bit patterns of a vector's samples, as written.
fn sample_bits(vector: &Value) -> Vec<u64> {
    vector["samples"]
        .as_array()
        .expect("a list of samples")
        .iter()
        .map(|bits| u64::from_str_radix(bits.as_str().expect("a string"), 16).expect("hex"))
        .collect()
}

/// The arithmetic mode of a vector: 16 hexadecimal digits.
fn mode_bits(vector: &Value) -> u64 {
    let digits = text(vector, "arithmetic_mode");
    assert_eq!(digits.len(), 16, "16 digits");
    u64::from_str_radix(digits, 16).expect("hexadecimal")
}

/// Narrows bit patterns to binary32 values, refusing a wider one.
fn narrow(bits: &[u64]) -> Vec<f32> {
    bits.iter()
        .map(|&bits| f32::from_bits(u32::try_from(bits).expect("32 bits")))
        .collect()
}

/// The vectors file, parsed, refusing a key this suite does not read: a new kind of
/// vector or refusal fails here until it is read.
fn spec() -> Value {
    let json: Value = serde_json::from_str(VECTORS).expect("valid JSON");
    let mut keys: Vec<&str> = json
        .as_object()
        .expect("an object")
        .keys()
        .map(alloc::string::String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["refused_modes", "refused_shapes", "scheme", "vectors"],
        "every key of the vectors file is read"
    );
    json
}

/// The render a vector describes: its rate, channels, mode, format and sample bits.
struct Input {
    /// Its sample rate.
    rate: SampleRate,
    /// Its channel count.
    channels: usize,
    /// Its arithmetic mode.
    mode: ArithmeticMode,
    /// Its render format.
    format: RenderFormat,
    /// The bit patterns of its samples.
    bits: Vec<u64>,
}

/// Reads the render a vector describes.
fn input(vector: &Value) -> Input {
    Input {
        rate: SampleRate::new(u32::try_from(number(vector, "rate")).expect("u32"))
            .expect("a valid rate"),
        channels: usize::try_from(number(vector, "channels")).expect("usize"),
        mode: ArithmeticMode::from_bits(mode_bits(vector)).expect("a known mode"),
        format: RenderFormat::ALL
            .into_iter()
            .find(|format| format.name() == text(vector, "format"))
            .expect("a render format"),
        bits: sample_bits(vector),
    }
}

/// Hashes the render a vector describes, or says why it is not one.
fn hash_of(vector: &Value) -> Result<RenderHash, NotWholeFrames> {
    let Input {
        rate,
        channels,
        mode,
        format,
        bits,
    } = input(vector);
    match format {
        RenderFormat::Binary64 => {
            let wide: Vec<f64> = bits.iter().copied().map(f64::from_bits).collect();
            render_hash(rate, channels, Samples::Binary64(&wide), mode)
        }
        RenderFormat::Binary64AsBinary32 => render_hash(
            rate,
            channels,
            Samples::Binary64AsBinary32(&narrow(&bits)),
            mode,
        ),
        RenderFormat::Binary32 => {
            render_hash(rate, channels, Samples::Binary32(&narrow(&bits)), mode)
        }
    }
}

/// Hashes one channel of binary64 samples at 48 kHz in the strict mode.
fn mono(samples: &[f64]) -> RenderHash {
    render_hash(RATE, 1, Samples::Binary64(samples), MODE).expect("whole frames")
}

#[test]
fn reproduces_the_known_answer_vectors_of_the_specification() {
    // If one changes, every reference hash changes: the scheme needs a new tag.
    let json = spec();
    assert_eq!(text(&json, "scheme").as_bytes(), TAG, "same scheme");
    let vectors = json["vectors"].as_array().expect("a list of vectors");
    assert!(vectors.len() >= 8, "every vector is read");
    let mut formats = Vec::new();
    for vector in vectors {
        let name = text(vector, "name");
        formats.push(text(vector, "format"));
        let expected = text(vector, "sha256");
        assert_eq!(
            hash_of(vector).map(|hash| hash.to_string()).as_deref(),
            Ok(expected),
            "{name}"
        );
        assert_eq!(
            Sha256::digest(hex_bytes(text(vector, "bytes"))).as_slice(),
            hex_bytes(expected),
            "{name}: the bytes of the vector give its hash"
        );
    }
    for format in RenderFormat::ALL {
        assert!(formats.contains(&format.name()), "a vector of {format}");
    }
}

#[test]
fn refuses_every_shape_of_the_specification_that_is_not_whole_frames() {
    let json = spec();
    let refused = json["refused_shapes"].as_array().expect("a list of shapes");
    assert!(refused.len() >= 2, "every refused shape is read");
    for vector in refused {
        let name = text(vector, "name");
        let error = hash_of(vector).expect_err(name);
        assert_eq!(
            error.to_string(),
            text(vector, "reason"),
            "{name}: its reason"
        );
        assert_eq!(
            (error.samples(), error.channels()),
            (
                u64::try_from(sample_bits(vector).len()).expect("u64"),
                number(vector, "channels")
            ),
            "{name}: what it was given"
        );
    }
}

#[test]
fn refuses_every_unknown_mode_of_the_specification() {
    let json = spec();
    let refused = json["refused_modes"].as_array().expect("a list of modes");
    assert!(refused.len() >= 4, "every refused mode is read");
    for vector in refused {
        let digits = text(vector, "unknown_flags");
        let name = text(vector, "name");
        let error = ArithmeticMode::from_bits(mode_bits(vector)).expect_err(name);
        assert_eq!(
            error.bits(),
            u64::from_str_radix(digits, 16).expect("hexadecimal"),
            "{name}: refused, naming the unknown flags alone"
        );
        assert_eq!(
            error.to_string(),
            alloc::format!("unknown arithmetic options: 0x{digits}"),
            "{name}: in the words of every reader"
        );
    }
}

#[test]
fn reports_a_shape_that_is_not_whole_frames_as_an_error() {
    /// Compiles only for an error type.
    const fn error<E: core::error::Error>() {}
    error::<NotWholeFrames>();
}

#[test]
fn shows_64_lowercase_hex_digits() {
    let text = mono(&[]).to_string();
    assert_eq!(text.len(), 64, "32 bytes");
    assert!(
        text.bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "{text}"
    );
}

#[test]
fn covers_every_bit_of_every_sample() {
    let hash = mono;
    let base = hash(&[0.25_f64, 0.5_f64]);
    assert_ne!(
        base,
        hash(&[0.25_f64, f64::from_bits(0.5_f64.to_bits() + 1)]),
        "1 ulp of binary64"
    );
    assert_ne!(hash(&[0.0_f64]), hash(&[-0.0_f64]), "sign of zero");
    assert_ne!(base, hash(&[0.5_f64, 0.25_f64]), "order");
    assert_ne!(base, hash(&[0.25_f64]), "length");
    let narrow = |samples: &[f32]| render_hash(RATE, 1, Samples::Binary32(samples), MODE);
    assert_ne!(
        narrow(&[0.25, 0.5]),
        narrow(&[0.25, f32::from_bits(0.5_f32.to_bits() + 1)]),
        "1 ulp of binary32"
    );
}

#[test]
fn covers_the_format_and_the_mode() {
    let base = render_hash(RATE, 2, Samples::Binary32(&[0.25, 0.5]), MODE);
    assert_ne!(
        base,
        render_hash(RATE, 1, Samples::Binary32(&[0.25, 0.5]), MODE),
        "channel count"
    );
    let rate = SampleRate::new(44_100).expect("valid rate");
    assert_ne!(
        base,
        render_hash(rate, 2, Samples::Binary32(&[0.25, 0.5]), MODE),
        "sample rate"
    );
    assert_ne!(
        base,
        render_hash(RATE, 2, Samples::Binary64AsBinary32(&[0.25, 0.5]), MODE),
        "precision"
    );
    assert_ne!(
        base,
        render_hash(RATE, 2, Samples::Binary64(&[0.25_f64, 0.5_f64]), MODE),
        "sample width"
    );
    let fused = ArithmeticMode::from_bits(1).expect("a known option");
    assert_ne!(
        base,
        render_hash(RATE, 2, Samples::Binary32(&[0.25, 0.5]), fused),
        "arithmetic mode"
    );
    assert_eq!(
        base,
        render_hash(RATE, 2, Samples::Binary32(&[0.25, 0.5]), MODE),
        "deterministic"
    );
}

#[test]
fn knows_the_format_and_the_length_of_its_samples() {
    let wide = Samples::Binary64(&[1.0_f64, 2.0_f64]);
    assert_eq!(wide.format(), RenderFormat::Binary64, "binary64");
    assert_eq!((wide.len(), wide.is_empty()), (2, false), "two samples");
    let given = Samples::Binary64AsBinary32(&[1.0]);
    assert_eq!(given.format(), RenderFormat::Binary64AsBinary32, "given");
    assert_eq!((given.len(), given.is_empty()), (1, false), "one sample");
    let narrow = Samples::Binary32(&[]);
    assert_eq!(narrow.format(), RenderFormat::Binary32, "binary32");
    assert_eq!((narrow.len(), narrow.is_empty()), (0, true), "none");
}
