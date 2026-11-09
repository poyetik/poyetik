//! Canonical hash of a rendered signal.
//!
//! The hash covers the exact bit pattern of every sample, so two renders have the same
//! hash only if they are bit-for-bit identical. It is how conformance is proven across
//! targets: native, WASM, every browser and every SDK must reproduce the reference
//! hashes of `conformance/references.json`.
//!
//! The scheme is specified in `spec/render-hash.md`, with known-answer vectors in
//! `spec/render-hash-vectors.json`. In short, the hashed bytes, all little-endian: the
//! ASCII tag `render-hash/2`, the processing precision and the width of the hashed samples
//! (one byte each, 32 or 64), the arithmetic mode (`u64` flags), the sample rate (`u32`),
//! the channel count (`u64`), the sample count (frames times channels, `u64`), then every
//! sample's bit pattern (`u32` or `u64`), channels interleaved.

#![no_std]
#![forbid(unsafe_code)]

use core::fmt;

use engine_core::{ArithmeticMode, RenderFormat, SampleRate};
use sha2::{Digest, Sha256};

/// Identifies the hashing scheme; changes whenever the hashed bytes change.
const TAG: &[u8] = b"render-hash/2";

/// The SHA-256 hash of a render, shown as 64 lowercase hexadecimal digits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RenderHash([u8; 32]);

impl fmt::Display for RenderHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// The samples of a render, interleaved by channel, in its render format.
///
/// The width the hash records is always that of the samples given (64 bits for binary64,
/// 32 otherwise): a hash cannot claim binary64 samples for binary32 ones. The precision it
/// records for binary32 samples is the caller's statement of which render made them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Samples<'render> {
    /// Computed and given in binary64.
    Binary64(&'render [f64]),
    /// Computed in binary64, given in binary32.
    Binary64AsBinary32(&'render [f32]),
    /// Computed and given in binary32.
    Binary32(&'render [f32]),
}

impl Samples<'_> {
    /// The render format of these samples.
    #[must_use]
    pub const fn format(&self) -> RenderFormat {
        match self {
            Self::Binary64(_) => RenderFormat::Binary64,
            Self::Binary64AsBinary32(_) => RenderFormat::Binary64AsBinary32,
            Self::Binary32(_) => RenderFormat::Binary32,
        }
    }

    /// The number of samples.
    #[must_use]
    pub const fn len(&self) -> usize {
        match self {
            Self::Binary64(samples) => samples.len(),
            Self::Binary64AsBinary32(samples) | Self::Binary32(samples) => samples.len(),
        }
    }

    /// Whether there is no sample.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Converts a length to `u64`: lossless on every supported target (`usize` is at most
/// 64 bits wide).
fn length(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

/// Hashes a render: `samples` holds `channels` interleaved channels at `rate`, computed in
/// the arithmetic mode `mode`.
#[must_use]
pub fn render_hash(
    rate: SampleRate,
    channels: usize,
    samples: Samples<'_>,
    mode: ArithmeticMode,
) -> RenderHash {
    let format = samples.format();
    let mut hasher = Sha256::new();
    hasher.update(TAG);
    hasher.update([format.precision().bits(), format.sample_bits()]);
    hasher.update(mode.bits().to_le_bytes());
    hasher.update(rate.hz().to_le_bytes());
    hasher.update(length(channels).to_le_bytes());
    hasher.update(length(samples.len()).to_le_bytes());
    match samples {
        Samples::Binary64(wide) => {
            for sample in wide {
                hasher.update(sample.to_bits().to_le_bytes());
            }
        }
        Samples::Binary64AsBinary32(narrow) | Samples::Binary32(narrow) => {
            for sample in narrow {
                hasher.update(sample.to_bits().to_le_bytes());
            }
        }
    }
    RenderHash(hasher.finalize().into())
}

#[cfg(test)]
mod tests {
    extern crate alloc;

    use alloc::string::ToString;
    use alloc::vec::Vec;

    use engine_core::{ArithmeticMode, MODE, RenderFormat, SampleRate, UnknownArithmeticOptions};
    use serde_json::Value;
    use sha2::{Digest, Sha256};

    use super::{Samples, TAG, render_hash};

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
                u8::from_str_radix(core::str::from_utf8(pair).expect("ASCII"), 16)
                    .expect("hexadecimal")
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

    #[test]
    fn reproduces_the_known_answer_vectors_of_the_specification() {
        // If one changes, every reference hash changes: the scheme needs a new tag.
        let json: Value = serde_json::from_str(VECTORS).expect("valid JSON");
        assert_eq!(text(&json, "scheme").as_bytes(), TAG, "same scheme");
        let vectors = json["vectors"].as_array().expect("a list of vectors");
        assert!(vectors.len() >= 7, "every vector is read");
        let mut formats = Vec::new();
        for vector in vectors {
            let name = text(vector, "name");
            let rate = SampleRate::new(u32::try_from(number(vector, "rate")).expect("u32"))
                .expect("a valid rate");
            let channels = usize::try_from(number(vector, "channels")).expect("usize");
            let mode = ArithmeticMode::from_bits(mode_bits(vector)).expect("a known mode");
            let bits = sample_bits(vector);
            let (wide, narrowed): (Vec<f64>, Vec<f32>) = match text(vector, "format") {
                "binary64" => (
                    bits.iter().copied().map(f64::from_bits).collect(),
                    Vec::new(),
                ),
                _ => (Vec::new(), narrow(&bits)),
            };
            let format = RenderFormat::ALL
                .into_iter()
                .find(|format| format.name() == text(vector, "format"))
                .expect("a render format");
            let samples = match format {
                RenderFormat::Binary64 => Samples::Binary64(&wide),
                RenderFormat::Binary64AsBinary32 => Samples::Binary64AsBinary32(&narrowed),
                RenderFormat::Binary32 => Samples::Binary32(&narrowed),
            };
            formats.push(samples.format());
            let expected = text(vector, "sha256");
            assert_eq!(
                render_hash(rate, channels, samples, mode).to_string(),
                expected,
                "{name}"
            );
            assert_eq!(
                Sha256::digest(hex_bytes(text(vector, "bytes"))).as_slice(),
                hex_bytes(expected),
                "{name}: the bytes of the vector give its hash"
            );
        }
        for format in RenderFormat::ALL {
            assert!(formats.contains(&format), "a vector of {format}");
        }
    }

    #[test]
    fn refuses_every_unknown_mode_of_the_specification() {
        let json: Value = serde_json::from_str(VECTORS).expect("valid JSON");
        let refused = json["refused_modes"].as_array().expect("a list of modes");
        assert!(refused.len() >= 3, "every refused mode is read");
        for vector in refused {
            let bits = mode_bits(vector);
            assert_eq!(
                ArithmeticMode::from_bits(bits),
                Err(UnknownArithmeticOptions(bits & !0b111)),
                "{}: refused, naming the unknown flags",
                text(vector, "name")
            );
        }
    }

    #[test]
    fn shows_64_lowercase_hex_digits() {
        let text = render_hash(RATE, 1, Samples::Binary64(&[]), MODE).to_string();
        assert_eq!(text.len(), 64, "32 bytes");
        assert!(
            text.bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "{text}"
        );
    }

    #[test]
    fn covers_every_bit_of_every_sample() {
        let hash = |samples: &[f64]| render_hash(RATE, 1, Samples::Binary64(samples), MODE);
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
}
