//! Images: their size, their color encoding and their pixels.

use alloc::vec;
use alloc::vec::Vec;

use crate::VisualError;

/// The size of an image in pixels, each side from 1 to [`ImageSize::MAX_SIDE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImageSize {
    /// Width in pixels.
    width: u32,
    /// Height in pixels.
    height: u32,
}

impl ImageSize {
    /// The longest supported side, in pixels (16K).
    pub const MAX_SIDE: u32 = 16_384;

    /// Creates an image size.
    ///
    /// # Errors
    ///
    /// Returns [`VisualError::InvalidImageSize`] when a side is 0 or longer than
    /// [`ImageSize::MAX_SIDE`].
    pub fn new(width: u32, height: u32) -> Result<Self, VisualError> {
        let side = 1..=Self::MAX_SIDE;
        if side.contains(&width) && side.contains(&height) {
            Ok(Self { width, height })
        } else {
            Err(VisualError::InvalidImageSize { width, height })
        }
    }

    /// Width in pixels.
    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }

    /// Height in pixels.
    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }

    /// The number of pixels: at most 2²⁸, so it fits every `usize` of 32 bits or more.
    #[must_use]
    pub fn pixels(self) -> u64 {
        u64::from(self.width).saturating_mul(u64::from(self.height))
    }
}

/// How the values of an image encode light.
///
/// One encoding for now (provisional, `spec/visual-design.md`): linear light (no transfer
/// function), the primaries and white point of ITU-R BT.709 (those of sRGB), full range,
/// red, green, blue and alpha as 32-bit floats, alpha premultiplied. The image hash gives
/// each encoding its code (`spec/image-hash.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorEncoding {
    /// Linear-light BT.709 RGBA, 32-bit floats, premultiplied alpha.
    LinearRec709Rgba32F,
}

/// An image: its size, its color encoding, and its pixels, row by row from the top,
/// each row from the left, each pixel red, green, blue, alpha.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    /// Its size.
    size: ImageSize,
    /// The four values of every pixel, in order.
    values: Vec<f32>,
}

impl Image {
    /// The values per pixel: red, green, blue, alpha.
    pub const CHANNELS: usize = 4;

    /// A transparent black image: every value 0. Allocates all its memory now, so that
    /// rendering into it never allocates.
    #[must_use]
    pub fn new(size: ImageSize) -> Self {
        let pixels = usize::try_from(size.pixels()).unwrap_or(usize::MAX);
        Self {
            size,
            values: vec![0.0; pixels.saturating_mul(Self::CHANNELS)],
        }
    }

    /// Its size.
    #[must_use]
    pub const fn size(&self) -> ImageSize {
        self.size
    }

    /// Its color encoding.
    #[must_use]
    pub const fn encoding(&self) -> ColorEncoding {
        ColorEncoding::LinearRec709Rgba32F
    }

    /// Its values, row by row from the top, four per pixel.
    #[must_use]
    pub fn values(&self) -> &[f32] {
        &self.values
    }

    /// Its rows from the top, each `width × 4` values, to write.
    pub fn rows_mut(&mut self) -> core::slice::ChunksExactMut<'_, f32> {
        let width = usize::try_from(self.size.width).unwrap_or(usize::MAX);
        self.values
            .chunks_exact_mut(width.saturating_mul(Self::CHANNELS))
    }
}

#[cfg(test)]
#[path = "image_tests.rs"]
mod tests;
