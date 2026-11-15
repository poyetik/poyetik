//! The square shown by a page: its parameters and the image it renders into.

#![forbid(unsafe_code)]

use core::fmt;

use visual_boxes::{BoxError, Square};
use visual_core::{
    Image, ImageIndex, ImageRange, ImageRate, ImageSize, Seconds, VisualBox, VisualError,
};
use visual_hash::hash_sequence;

/// The image index `high × 2³² + low`, joined from the two halves a 32-bit caller passes:
/// from their bytes, so that no arithmetic can carry or overlap.
#[must_use]
pub const fn image_index(low: u32, high: u32) -> u64 {
    let [low0, low1, low2, low3] = low.to_le_bytes();
    let [high0, high1, high2, high3] = high.to_le_bytes();
    u64::from_le_bytes([low0, low1, low2, low3, high0, high1, high2, high3])
}

/// Why a square could not be set up: the argument refused, with the library's reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupError {
    /// The width and height.
    Size(VisualError),
    /// The image rate.
    Rate(VisualError),
    /// The cycle.
    Cycle(VisualError),
    /// The side of the square.
    Side(BoxError),
}

impl fmt::Display for SetupError {
    /// Names the argument refused as the caller wrote it (`width and height`, `rate`,
    /// `cycle`, `side`), then why.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Size(error) => write!(f, "width and height: {error}"),
            Self::Rate(error) => write!(f, "rate: {error}"),
            Self::Cycle(error) => write!(f, "cycle: {error}"),
            Self::Side(error) => write!(f, "side: {error}"),
        }
    }
}

/// Its message names the argument refused; its source is the library's error.
impl core::error::Error for SetupError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Size(error) | Self::Rate(error) | Self::Cycle(error) => Some(error),
            Self::Side(error) => Some(error),
        }
    }
}

/// A square and the image it renders into, set up by the page.
#[derive(Debug)]
pub struct View {
    /// The square, its image and the image rate, once set up.
    shown: Option<(Square, Image, ImageRate)>,
    /// The bytes of the last image hash computed.
    hash: [u8; 32],
    /// Why the last setup was refused; empty after a setup that succeeded.
    message: String,
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
            message: String::new(),
        }
    }

    /// Sets the square up: images of `width` × `height` pixels at `rate` (a fraction of
    /// images per second), a side of `side` / 65 536 of the height, one sweep per `cycle`
    /// (a fraction of seconds).
    ///
    /// # Errors
    ///
    /// Returns the first value refused, as [`View::message`] then says too; the previous
    /// square, if any, is kept.
    pub fn setup(
        &mut self,
        width: u32,
        height: u32,
        rate: (u32, u32),
        side: u32,
        cycle: (u32, u32),
    ) -> Result<(), SetupError> {
        let checked = Self::checked(width, height, rate, side, cycle);
        match checked {
            Ok((square, image_size, images)) => {
                // Every value is valid: the image replaced is released before the next one
                // is allocated, so that the two are never held at once.
                self.shown = None;
                self.shown = Some((square, Image::new(image_size), images));
                self.message.clear();
                Ok(())
            }
            Err(error) => {
                self.message = error.to_string();
                Err(error)
            }
        }
    }

    /// The square, the size and the image rate of a setup, checked.
    fn checked(
        width: u32,
        height: u32,
        rate: (u32, u32),
        side: u32,
        cycle: (u32, u32),
    ) -> Result<(Square, ImageSize, ImageRate), SetupError> {
        let image_size = ImageSize::new(width, height).map_err(SetupError::Size)?;
        let images = ImageRate::new(rate.0, rate.1).map_err(SetupError::Rate)?;
        let sweep = Seconds::new(cycle.0, cycle.1).map_err(SetupError::Cycle)?;
        let square = Square::new(images, side, sweep).map_err(SetupError::Side)?;
        Ok((square, image_size, images))
    }

    /// Why the last setup was refused, in one line; empty after a setup that succeeded, or
    /// before any.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
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
    /// (`spec/image-hash.md`), the 32 bytes of the SHA-256; `None` when no square is set
    /// up, or when the last of those images would be past image 2⁶⁴ − 1: never a value
    /// that could pass for a hash.
    pub fn hash(&mut self, first: u64, count: u64) -> Option<&[u8; 32]> {
        let range = ImageRange::new(ImageIndex(first), count).ok()?;
        let (square, image, rate) = self.shown.as_mut()?;
        self.hash = hash_sequence(square, *rate, range, image).bytes();
        Some(&self.hash)
    }
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
