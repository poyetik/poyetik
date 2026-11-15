//! Errors raised while validating the values the visual library is given.

use alloc::string::String;
use core::fmt;

/// Why a value given to the visual library was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualError {
    /// The image rate is not a reduced fraction within the supported range.
    InvalidImageRate {
        /// Images per `denominator` seconds.
        numerator: u32,
        /// Seconds.
        denominator: u32,
    },
    /// The image size is outside the supported range.
    InvalidImageSize {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
    },
    /// The image has more pixels than [`crate::ImageSize::MAX_PIXELS`].
    ImageTooLarge {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
    },
    /// The machine cannot hold an image of this size: its memory is exhausted.
    OutOfMemory {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
    },
    /// A run of images ends past image 2⁶⁴ − 1.
    InvalidImageRange {
        /// The first image.
        first: u64,
        /// How many images.
        count: u64,
    },
    /// A duration is not a reduced, nonzero fraction of seconds.
    InvalidSeconds {
        /// Seconds, times `denominator`.
        numerator: u32,
        /// The divisor: the duration is `numerator / denominator` seconds.
        denominator: u32,
    },
}

impl fmt::Display for VisualError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::InvalidImageRate {
                numerator,
                denominator,
            } => write!(
                f,
                "invalid image rate {numerator}/{denominator}: a reduced fraction from 1 to {} images per second is required",
                crate::ImageRate::MAX_PER_SECOND
            ),
            Self::InvalidImageSize { width, height } => write!(
                f,
                "invalid image size {width}x{height}: each side must be from 1 to {} pixels",
                crate::ImageSize::MAX_SIDE
            ),
            Self::ImageTooLarge { width, height } => write!(
                f,
                "invalid image size {width}x{height}: an image holds at most {} pixels",
                crate::ImageSize::MAX_PIXELS
            ),
            Self::OutOfMemory { width, height } => write!(
                f,
                "image {width}x{height} does not fit in this machine's memory: {} bytes are needed",
                u64::from(width)
                    .saturating_mul(u64::from(height))
                    .saturating_mul(16)
            ),
            Self::InvalidImageRange { first, count } => write!(
                f,
                "invalid image range: {count} images from image {first} pass the largest index, {}",
                u64::MAX
            ),
            Self::InvalidSeconds {
                numerator,
                denominator,
            } => write!(
                f,
                "invalid duration {numerator}/{denominator} s: a reduced, nonzero fraction is required"
            ),
        }
    }
}

/// Its message says which value was rejected and why.
impl core::error::Error for VisualError {}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;

/// The error's message: what a caller reporting errors as text (the test kit's laws) gets
/// from `?`.
impl From<VisualError> for String {
    fn from(error: VisualError) -> Self {
        alloc::format!("{error}")
    }
}
