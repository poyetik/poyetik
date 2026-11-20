//! Errors raised while validating the values the visual library is given.

use alloc::string::String;
use core::fmt;

/// What a refused allocation was to hold ([`VisualError::OutOfMemory`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Holding {
    /// An image's values: [`crate::Image::PIXEL_BYTES`] a pixel.
    Image,
    /// An image's encoding for display: [`crate::DisplayImage::PIXEL_BYTES`] a pixel.
    DisplayEncoding,
}

impl Holding {
    /// The bytes holding it takes for an image of `width` × `height` pixels.
    #[must_use]
    pub fn bytes(self, width: u32, height: u32) -> u64 {
        let per_pixel = match self {
            Self::Image => crate::Image::PIXEL_BYTES,
            Self::DisplayEncoding => crate::DisplayImage::PIXEL_BYTES,
        };
        u64::from(width)
            .saturating_mul(u64::from(height))
            .saturating_mul(per_pixel)
    }
}

/// Why a value given to the visual library was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
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
    /// The machine cannot hold an image of this size, or its encoding for display: its
    /// memory is exhausted. The bytes asked for follow from the size and what was to be
    /// held ([`Holding::bytes`]), so that no other count can be given with them.
    OutOfMemory {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
        /// What the memory was to hold.
        of: Holding,
    },
    /// A run of images ends past image 2⁶⁴ − 1.
    InvalidImageRange {
        /// The first image.
        first: u64,
        /// How many images.
        count: u64,
    },
    /// A display image is not of the size of the image it is to encode.
    DisplaySizeMismatch {
        /// The image's width in pixels.
        width: u32,
        /// The image's height in pixels.
        height: u32,
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
            Self::OutOfMemory { width, height, of } => write!(
                f,
                "{}image {width}x{height} does not fit in this machine's memory: {} bytes are needed",
                match of {
                    Holding::Image => "",
                    Holding::DisplayEncoding => "the display encoding of ",
                },
                of.bytes(width, height)
            ),
            Self::InvalidImageRange { first, count } => write!(
                f,
                "invalid image range: {count} images from image {first} pass the largest index, {}",
                u64::MAX
            ),
            Self::DisplaySizeMismatch { width, height } => write!(
                f,
                "the display image is not of the size of the image it encodes, {width}x{height}"
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
