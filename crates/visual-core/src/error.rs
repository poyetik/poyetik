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
    /// The side of a square is larger than the image height.
    InvalidSide(u32),
    /// The duration of a cycle is not a reduced, nonzero fraction of seconds.
    InvalidCycle {
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
            Self::InvalidSide(side) => write!(
                f,
                "invalid square side {side}: at most {} (the image height)",
                crate::Square::FULL_SIDE
            ),
            Self::InvalidCycle {
                numerator,
                denominator,
            } => write!(
                f,
                "invalid cycle {numerator}/{denominator} s: a reduced, nonzero fraction is required"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate alloc;

    use alloc::string::ToString;

    use super::VisualError;

    #[test]
    fn names_the_rejected_values() {
        let cases = [
            (
                VisualError::InvalidImageRate {
                    numerator: 2,
                    denominator: 4,
                },
                "invalid image rate 2/4: a reduced fraction from 1 to 1000 images per second is required",
            ),
            (
                VisualError::InvalidImageSize {
                    width: 0,
                    height: 7,
                },
                "invalid image size 0x7: each side must be from 1 to 16384 pixels",
            ),
            (
                VisualError::InvalidSide(65_537),
                "invalid square side 65537: at most 65536 (the image height)",
            ),
            (
                VisualError::InvalidCycle {
                    numerator: 0,
                    denominator: 3,
                },
                "invalid cycle 0/3 s: a reduced, nonzero fraction is required",
            ),
        ];
        for (error, message) in cases {
            assert_eq!(error.to_string(), message, "{error:?}");
        }
    }
}
