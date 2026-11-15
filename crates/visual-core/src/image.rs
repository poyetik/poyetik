//! Images: their size, their color encoding and their pixels.

use alloc::vec;
use alloc::vec::Vec;

use crate::VisualError;

/// The size of an image in pixels, each side from 1 to [`ImageSize::MAX_SIDE`], and at most
/// [`ImageSize::MAX_PIXELS`] pixels in all.
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

    /// The most pixels an image holds, 2²⁵ (8K UHD, 7680 × 4320, and 16 384 × 2048 fit):
    /// its values then take 2²⁹ bytes (512 MiB), which every target allocates, those of 32
    /// bits included, where one allocation is at most `isize::MAX` bytes (2³¹ − 1) and the
    /// whole WebAssembly memory 4 GiB. So every target accepts the same sizes, and two
    /// images (one replacing the other) still fit in that memory.
    pub const MAX_PIXELS: u32 = 1 << 25;

    /// Creates an image size.
    ///
    /// # Errors
    ///
    /// Returns [`VisualError::InvalidImageSize`] when a side is 0 or longer than
    /// [`ImageSize::MAX_SIDE`], and [`VisualError::ImageTooLarge`] when the image has more
    /// than [`ImageSize::MAX_PIXELS`] pixels.
    pub fn new(width: u32, height: u32) -> Result<Self, VisualError> {
        let sides = 1..=Self::MAX_SIDE;
        let size = Self { width, height };
        if !(sides.contains(&width) && sides.contains(&height)) {
            Err(VisualError::InvalidImageSize { width, height })
        } else if size.pixels() > u64::from(Self::MAX_PIXELS) {
            Err(VisualError::ImageTooLarge { width, height })
        } else {
            Ok(size)
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

    /// The number of pixels: at most [`ImageSize::MAX_PIXELS`]. The product of two `u32`
    /// never saturates a `u64`.
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
