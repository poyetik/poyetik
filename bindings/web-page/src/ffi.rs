//! The coordinating layer's C interface, exported by the page module beside the visual
//! library's.
//!
//! [`page_abi_version`] first: a module with another version is a different interface.
//! Then [`page_image_at`] answers which image of a sequence the page shows at a frame of
//! the audio clock. Both take and return integers only: no memory is shared, no state kept,
//! so a call can never leave the instance in another state. JavaScript passes and receives
//! a 64-bit integer as a `BigInt`.
//!
//! The only `unsafe` here is `#[unsafe(no_mangle)]`, which gives each function a fixed
//! export name.

#![expect(
    unsafe_code,
    reason = "the exported C interface: `#[unsafe(no_mangle)]` gives each function its export name"
)]

use audio_core::SampleRate;
use visual_core::ImageRate;

#[cfg(test)]
#[path = "ffi_tests.rs"]
mod tests;

/// Identity and version of this interface: `0x5050` ("PP") in the high 16 bits, the
/// version in the low 16 bits.
pub const PAGE_ABI_VERSION: u32 = 0x5050_0001;

/// What [`page_image_at`] answers for a sample rate or an image rate it refuses: never an
/// image, since an image is at most a frame's eighth (`coordinator::image_at`).
pub const REFUSED: u64 = u64::MAX;

/// The version of this interface, [`PAGE_ABI_VERSION`].
#[unsafe(no_mangle)]
pub const extern "C" fn page_abi_version() -> u32 {
    PAGE_ABI_VERSION
}

/// The image a sequence shows at frame `frame` of the audio clock.
///
/// The sequence runs at `numerator / denominator` images a second and the clock at
/// `sample_rate` hertz (`coordinator::image_at`); [`REFUSED`] when the sample rate or the
/// image rate is not a valid one.
#[unsafe(no_mangle)]
pub extern "C" fn page_image_at(
    frame: u64,
    sample_rate: u32,
    numerator: u32,
    denominator: u32,
) -> u64 {
    match (
        SampleRate::new(sample_rate),
        ImageRate::new(numerator, denominator),
    ) {
        (Ok(sample_rate), Ok(rate)) => coordinator::image_at(frame, sample_rate, rate).0,
        (Err(_), _) | (_, Err(_)) => REFUSED,
    }
}
