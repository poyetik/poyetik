//! The square shown by a page: its parameters and the image it renders into.

#![forbid(unsafe_code)]

use visual_boxes::Square;
use visual_core::{Image, ImageIndex, ImageRange, ImageRate, ImageSize, Seconds, VisualBox};
use visual_hash::hash_sequence;

/// The image index `high × 2³² + low`, joined from the two halves a 32-bit caller passes:
/// from their bytes, so that no arithmetic can carry or overlap.
#[must_use]
pub const fn image_index(low: u32, high: u32) -> u64 {
    let [low0, low1, low2, low3] = low.to_le_bytes();
    let [high0, high1, high2, high3] = high.to_le_bytes();
    u64::from_le_bytes([low0, low1, low2, low3, high0, high1, high2, high3])
}

/// A square and the image it renders into, set up by the page.
#[derive(Debug)]
pub struct View {
    /// The square, its image and the image rate, once set up.
    shown: Option<(Square, Image, ImageRate)>,
    /// The bytes of the last image hash computed.
    hash: [u8; 32],
}

impl Default for View {
    fn default() -> Self {
        Self::new()
    }
}

impl View {
    /// No square yet.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            shown: None,
            hash: [0; 32],
        }
    }

    /// Sets the square up: images of `width` × `height` pixels at `rate` (a fraction of
    /// images per second), a side of `side` / 65 536 of the height, one sweep per `cycle`
    /// (a fraction of seconds). Returns whether every value was valid; otherwise the
    /// previous square, if any, is kept.
    pub fn setup(
        &mut self,
        width: u32,
        height: u32,
        rate: (u32, u32),
        side: u32,
        cycle: (u32, u32),
    ) -> bool {
        let built = ImageSize::new(width, height).ok().and_then(|size| {
            let rate = ImageRate::new(rate.0, rate.1).ok()?;
            let cycle = Seconds::new(cycle.0, cycle.1).ok()?;
            let square = Square::new(rate, side, cycle).ok()?;
            Some((square, Image::new(size), rate))
        });
        let valid = built.is_some();
        if built.is_some() {
            self.shown = built;
        }
        valid
    }

    /// The number of values of an image: width × height × 4; 0 when no square is set up.
    #[must_use]
    pub fn render_len(&self) -> usize {
        self.shown
            .as_ref()
            .map_or(0, |(_, image, _)| image.values().len())
    }

    /// Renders image `index` and returns its values (four per pixel, row by row from the
    /// top); empty when no square is set up.
    pub fn render(&mut self, index: u64) -> &[f32] {
        match &mut self.shown {
            Some((square, image, _)) => {
                square.render(ImageIndex(index), image);
                image.values()
            }
            None => &[],
        }
    }

    /// Renders `count` images from image `first` and returns their image hash
    /// (`spec/image-hash.md`), the 32 bytes of the SHA-256; all zero when no square is set
    /// up; `None` when the last of those images would be past image 2⁶⁴ − 1.
    pub fn hash(&mut self, first: u64, count: u64) -> Option<&[u8; 32]> {
        let range = ImageRange::new(ImageIndex(first), count).ok()?;
        if let Some((square, image, rate)) = &mut self.shown {
            self.hash = hash_sequence(square, *rate, range, image).bytes();
        }
        Some(&self.hash)
    }
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
