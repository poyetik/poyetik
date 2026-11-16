//! The coordinating layer above the two standalone libraries (`spec/engine-design.md`,
//! decisions 1, 17, 32 and 33): what spans sound and image, which neither library may hold.
//!
//! Its first form holds one thing: which image a visual sequence shows at each frame of
//! the audio clock, exactly, so that a page draws the image of the instant it hears. The
//! clock choice, transport controls, scenes and cues come with #35 step 3.

#![no_std]
#![forbid(unsafe_code)]
#![doc(test(attr(forbid(unsafe_code))))]

use audio_core::SampleRate;
use visual_core::{ImageIndex, ImageRate};

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;

/// The image of a sequence at `rate` shown at frame `frame` of an audio clock at
/// `sample_rate`: the last image whose instant is not after the frame's.
///
/// Image `n` is shown at `n × denominator / numerator` seconds and frame `frame` sounds at
/// `frame / sample_rate` seconds, so the image is
/// `⌊frame × numerator / (denominator × sample_rate)⌋`, computed exactly in 128-bit
/// integers: at most 2⁶⁴ × 1000 for the product (a rate holds at most 1000 images a
/// second), and at most `frame / 8` for the image (a sample rate is at least 8000 Hz),
/// so it never overflows and always fits an index.
#[must_use]
pub fn image_at(frame: u64, sample_rate: SampleRate, rate: ImageRate) -> ImageIndex {
    let elapsed = u128::from(frame).saturating_mul(u128::from(rate.numerator()));
    let per_image = u128::from(rate.denominator()).saturating_mul(u128::from(sample_rate.hz()));
    // Never zero: both factors are positive by construction; never above `frame`.
    let image = elapsed.checked_div(per_image).unwrap_or_default();
    ImageIndex(u64::try_from(image).unwrap_or(u64::MAX))
}
