//! Canonical hash of a rendered signal.
//!
//! The hash covers the exact bit pattern of every sample, so two renders have the same
//! hash only if they are bit-for-bit identical. It is how conformance is proven across
//! targets: native, WASM, every browser and every SDK must reproduce the reference
//! hashes of `conformance/references.json`.
//!
//! The scheme is specified in `spec/render-hash.md`, with known-answer vectors in
//! `spec/render-hash-vectors.json`. In short, the hashed bytes, all little-endian: the
//! ASCII tag `render-hash/1`, the sample rate (`u32`), the channel count (`u64`), the
//! sample count (frames times channels, `u64`), then every sample's `f32` bit pattern
//! (`u32`), channels interleaved.

#![no_std]
#![forbid(unsafe_code)]

use core::fmt;

use engine_core::SampleRate;
use sha2::{Digest, Sha256};

/// Identifies the hashing scheme; changes whenever the hashed bytes change.
const TAG: &[u8] = b"render-hash/1";

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

/// Converts a length to `u64`: lossless on every supported target (`usize` is at most
/// 64 bits wide).
fn length(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

/// Hashes a render: `samples` holds `channels` interleaved channels at `rate`.
#[must_use]
pub fn render_hash(rate: SampleRate, channels: usize, samples: &[f32]) -> RenderHash {
    let mut hasher = Sha256::new();
    hasher.update(TAG);
    hasher.update(rate.hz().to_le_bytes());
    hasher.update(length(channels).to_le_bytes());
    hasher.update(length(samples.len()).to_le_bytes());
    for sample in samples {
        hasher.update(sample.to_bits().to_le_bytes());
    }
    RenderHash(hasher.finalize().into())
}

#[cfg(test)]
mod tests {
    extern crate alloc;

    use alloc::string::ToString;
    use alloc::vec::Vec;

    use engine_core::SampleRate;
    use serde_json::Value;
    use sha2::{Digest, Sha256};

    use super::{TAG, render_hash};

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

    #[test]
    fn reproduces_the_known_answer_vectors_of_the_specification() {
        // If one changes, every reference hash changes: the scheme needs a new tag.
        let json: Value = serde_json::from_str(VECTORS).expect("valid JSON");
        assert_eq!(text(&json, "scheme").as_bytes(), TAG, "same scheme");
        let vectors = json["vectors"].as_array().expect("a list of vectors");
        assert!(vectors.len() >= 3, "every vector is read");
        for vector in vectors {
            let name = text(vector, "name");
            let rate = SampleRate::new(u32::try_from(number(vector, "rate")).expect("u32"))
                .expect("a valid rate");
            let channels = usize::try_from(number(vector, "channels")).expect("usize");
            let samples: Vec<f32> = vector["samples"]
                .as_array()
                .expect("a list of samples")
                .iter()
                .map(|bits| {
                    f32::from_bits(
                        u32::from_str_radix(bits.as_str().expect("a string"), 16)
                            .expect("hexadecimal bits"),
                    )
                })
                .collect();
            let expected = text(vector, "sha256");
            assert_eq!(
                render_hash(rate, channels, &samples).to_string(),
                expected,
                "{name}"
            );
            assert_eq!(
                Sha256::digest(hex_bytes(text(vector, "bytes"))).as_slice(),
                hex_bytes(expected),
                "{name}: the bytes of the vector give its hash"
            );
        }
    }

    #[test]
    fn shows_64_lowercase_hex_digits() {
        let text = render_hash(RATE, 1, &[]).to_string();
        assert_eq!(text.len(), 64, "32 bytes");
        assert!(
            text.bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "{text}"
        );
    }

    #[test]
    fn covers_every_bit_of_every_sample() {
        let base = render_hash(RATE, 1, &[0.25, 0.5]);
        assert_ne!(
            base,
            render_hash(RATE, 1, &[0.25, f32::from_bits(0.5_f32.to_bits() + 1)]),
            "1 ulp"
        );
        assert_ne!(
            render_hash(RATE, 1, &[0.0]),
            render_hash(RATE, 1, &[-0.0]),
            "sign of zero"
        );
        assert_ne!(base, render_hash(RATE, 1, &[0.5, 0.25]), "order");
        assert_ne!(base, render_hash(RATE, 1, &[0.25]), "length");
    }

    #[test]
    fn covers_the_format() {
        let base = render_hash(RATE, 2, &[0.25, 0.5]);
        assert_ne!(base, render_hash(RATE, 1, &[0.25, 0.5]), "channel count");
        let rate = SampleRate::new(44_100).expect("valid rate");
        assert_ne!(base, render_hash(rate, 2, &[0.25, 0.5]), "sample rate");
        assert_eq!(base, render_hash(RATE, 2, &[0.25, 0.5]), "deterministic");
    }
}
