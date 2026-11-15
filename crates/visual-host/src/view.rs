//! The square a host shows: its parameters and the image it renders into.

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

/// The most bytes a refusal's message takes: about twice the longest a [`SetupError`] says
/// (every variant with values of ten digits, about 125 bytes), which a test checks.
const MESSAGE_BYTES: usize = 256;

/// The explanation of a refused setup, held in the view itself, so that refusing allocates
/// nothing: a message on the heap between two images would keep the next large image from
/// reusing the space of the last one.
#[derive(Debug, Clone, Copy)]
struct Message {
    /// Its UTF-8 bytes, the first `len` of them.
    bytes: [u8; MESSAGE_BYTES],
    /// How many bytes it holds.
    len: usize,
}

impl Message {
    /// No message.
    const EMPTY: Self = Self {
        bytes: [0; MESSAGE_BYTES],
        len: 0,
    };

    /// The message of `error`, whole when it fits ([`MESSAGE_BYTES`]), as it always does.
    fn of(error: SetupError) -> Self {
        let mut message = Self::EMPTY;
        match fmt::write(&mut message, format_args!("{error}")) {
            Ok(()) | Err(fmt::Error) => message,
        }
    }

    /// The text written, whole strings only, so always UTF-8.
    fn text(&self) -> &str {
        self.bytes
            .get(..self.len)
            .and_then(|written| core::str::from_utf8(written).ok())
            .unwrap_or_default()
    }
}

impl fmt::Write for Message {
    /// Appends `piece` whole, or refuses it (and keeps what was written) when it does not fit.
    fn write_str(&mut self, piece: &str) -> fmt::Result {
        let end = self
            .len
            .checked_add(piece.len())
            .filter(|end| *end <= MESSAGE_BYTES)
            .ok_or(fmt::Error)?;
        self.bytes
            .get_mut(self.len..end)
            .ok_or(fmt::Error)?
            .copy_from_slice(piece.as_bytes());
        self.len = end;
        Ok(())
    }
}

/// A square and the image it renders into, set up by the page.
#[derive(Debug)]
pub struct View {
    /// The square (which holds the image rate) and its image, once set up.
    shown: Option<(Square, Image)>,
    /// The bytes of the last image hash computed.
    hash: [u8; 32],
    /// Why the last setup was refused; empty after a setup that succeeded.
    message: Message,
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
            message: Message::EMPTY,
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
        let checked =
            Self::checked(width, height, rate, side, cycle).and_then(|(square, image_size)| {
                // Every value is valid: the image replaced is released before the next one
                // is allocated, so that the two are never held at once; an image the
                // memory cannot hold leaves nothing shown.
                self.shown = None;
                Image::try_new(image_size)
                    .map(|image| (square, image))
                    .map_err(SetupError::Size)
            });
        match checked {
            Ok(shown) => {
                self.shown = Some(shown);
                self.message = Message::EMPTY;
                Ok(())
            }
            Err(error) => {
                self.message = Message::of(error);
                Err(error)
            }
        }
    }

    /// The square (holding its image rate) and the size of a setup, checked.
    fn checked(
        width: u32,
        height: u32,
        rate: (u32, u32),
        side: u32,
        cycle: (u32, u32),
    ) -> Result<(Square, ImageSize), SetupError> {
        let image_size = ImageSize::new(width, height).map_err(SetupError::Size)?;
        let checked_rate = ImageRate::new(rate.0, rate.1).map_err(SetupError::Rate)?;
        let sweep = Seconds::new(cycle.0, cycle.1).map_err(SetupError::Cycle)?;
        let square = Square::new(checked_rate, side, sweep).map_err(SetupError::Side)?;
        Ok((square, image_size))
    }

    /// Why the last setup was refused, in one line; empty after a setup that succeeded, or
    /// before any.
    #[must_use]
    pub fn message(&self) -> &str {
        self.message.text()
    }

    /// The number of values of an image: width × height × 4; 0 when no square is set up.
    #[must_use]
    pub fn render_len(&self) -> usize {
        self.shown
            .as_ref()
            .map_or(0, |(_, image)| image.values().len())
    }

    /// Renders image `index` and returns its values (four per pixel, row by row from the
    /// top); empty when no square is set up.
    pub fn render(&mut self, index: u64) -> &[f32] {
        match &mut self.shown {
            Some((square, image)) => {
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
        let (square, image) = self.shown.as_mut()?;
        let rate = square.rate();
        self.hash = hash_sequence(square, rate, range, image).bytes();
        Some(&self.hash)
    }
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
