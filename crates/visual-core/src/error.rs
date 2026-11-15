//! Errors raised while validating the values the visual library is given.

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
    /// A duration is not a reduced, nonzero fraction of seconds.
    InvalidSeconds {
        /// Seconds, times `denominator`.
        numerator: u32,
        /// Divides `numerator`.
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
                "invalid image rate {numerator}/{denominator}: a reduced fraction from 1 to 1000 images per second is required"
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

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
