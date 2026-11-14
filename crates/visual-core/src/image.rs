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
mod tests {
    use super::{ColorEncoding, Image, ImageSize};
    use crate::VisualError;

    #[test]
    fn accepts_sizes_within_range() {
        for (width, height) in [
            (1, 1),
            (1920, 1080),
            (16_384, 1),
            (1, 16_384),
            (16_384, 16_384),
        ] {
            let size = ImageSize::new(width, height).expect("a valid size");
            assert_eq!(
                (size.width(), size.height()),
                (width, height),
                "{width}x{height}"
            );
            assert_eq!(
                size.pixels(),
                u64::from(width) * u64::from(height),
                "{width}x{height}"
            );
        }
    }

    #[test]
    fn rejects_sizes_outside_range() {
        for (width, height) in [
            (0, 1),
            (1, 0),
            (16_385, 1),
            (1, 16_385),
            (u32::MAX, u32::MAX),
        ] {
            assert_eq!(
                ImageSize::new(width, height),
                Err(VisualError::InvalidImageSize { width, height }),
                "{width}x{height}"
            );
        }
    }

    #[test]
    fn starts_transparent_black() {
        let size = ImageSize::new(3, 2).expect("a valid size");
        let image = Image::new(size);
        assert_eq!(image.size(), size, "its size");
        assert_eq!(
            image.encoding(),
            ColorEncoding::LinearRec709Rgba32F,
            "encoding"
        );
        assert_eq!(image.values().len(), 24, "4 values per pixel");
        assert!(
            image.values().iter().all(|value| value.to_bits() == 0),
            "every value is +0"
        );
    }

    #[test]
    fn gives_its_rows_from_the_top() {
        let mut image = Image::new(ImageSize::new(3, 2).expect("a valid size"));
        let mut rows = 0_u8;
        for (index, row) in image.rows_mut().enumerate() {
            assert_eq!(row.len(), 12, "3 pixels of 4 values");
            row.fill(f32::from(u8::try_from(index).expect("small")) + 1.0);
            rows += 1;
        }
        assert_eq!(rows, 2, "two rows");
        let (first, second) = image.values().split_at(12);
        assert!(
            first
                .iter()
                .all(|value| value.to_bits() == 1.0_f32.to_bits()),
            "top row first"
        );
        assert!(
            second
                .iter()
                .all(|value| value.to_bits() == 2.0_f32.to_bits()),
            "then the next"
        );
    }
}
