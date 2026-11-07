//! Canonical hash of a rendered signal.
//!
//! The hash covers the exact bit pattern of every sample, so two renders have the same
//! hash only if they are bit-for-bit identical. It is how conformance is proven across
//! targets: native, WASM, every browser and every SDK must reproduce the reference
//! hashes of `conformance/references.json`.
//!
//! Hashed bytes, all little-endian: the ASCII tag `render-hash/1`, the sample rate
//! (`u32`), the channel count (`u64`), the sample count (`u64`), then every sample's
//! `f32` bit pattern (`u32`), channels interleaved.

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

    use engine_core::SampleRate;

    use super::render_hash;

    const RATE: SampleRate = SampleRate::HZ_48000;

    #[test]
    fn is_stable() {
        // Fixed value, cross-checked with an independent implementation (Python's
        // hashlib over the documented bytes). If it changes, every reference hash changes.
        assert_eq!(
            render_hash(RATE, 2, &[0.0, 0.5, -0.5, 1.0]).to_string(),
            "9d1756287e39b44f34500c890770292a9c31c69410b00dc87b670967cfce6397",
            "reference value of the scheme"
        );
    }

    #[test]
    fn shows_64_lowercase_hex_digits() {
        let text = render_hash(RATE, 1, &[]).to_string();
        assert_eq!(text.len(), 64, "32 bytes");
        assert!(
            text.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
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
