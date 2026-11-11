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
//! sample's bit pattern (`u32` or `u64`), channels interleaved. Samples that are not
//! whole frames are refused, never hashed.

#![no_std]
#![forbid(unsafe_code)]
#![doc(test(attr(forbid(unsafe_code))))]

use core::fmt;

use engine_core::{ArithmeticMode, RenderFormat, SampleRate};
use sha2::{Digest, Sha256};

/// The name and version of the hashing scheme, `render-hash/2`: a new version whenever the
/// hashed bytes change, recorded beside the references blessed under it.
pub const SCHEME: &str = "render-hash/2";

/// Identifies the hashing scheme at the start of the hashed bytes: [`SCHEME`].
const TAG: &[u8] = SCHEME.as_bytes();

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

/// Samples that are not whole frames of the channel count: not a render, so not hashed
/// (`spec/render-hash.md`, Input).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotWholeFrames {
    /// The number of samples given.
    samples: u64,
    /// The channel count given.
    channels: u64,
}

impl NotWholeFrames {
    /// The number of samples given.
    #[must_use]
    pub const fn samples(self) -> u64 {
        self.samples
    }

    /// The channel count given.
    #[must_use]
    pub const fn channels(self) -> u64 {
        self.channels
    }
}

impl fmt::Display for NotWholeFrames {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} samples are not whole frames of {} channels",
            self.samples, self.channels
        )
    }
}

impl core::error::Error for NotWholeFrames {}

/// Hashes a render: `samples` holds `channels` interleaved channels at `rate`, computed in
/// the arithmetic mode `mode`.
///
/// # Errors
///
/// Returns [`NotWholeFrames`] when the samples are not whole frames of `channels` channels
/// (any sample at all without a channel): a hash of them would identify no render.
pub fn render_hash(
    rate: SampleRate,
    channels: usize,
    samples: Samples<'_>,
    mode: ArithmeticMode,
) -> Result<RenderHash, NotWholeFrames> {
    // Without a channel there is no remainder: only no sample at all is whole frames.
    let whole = samples
        .len()
        .checked_rem(channels)
        .map_or(samples.is_empty(), |rest| rest == 0);
    if !whole {
        return Err(NotWholeFrames {
            samples: length(samples.len()),
            channels: length(channels),
        });
    }
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
    Ok(RenderHash(hasher.finalize().into()))
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
