//! The square a host shows: its parameters, the image it renders into and that image
//! encoded for display.

use core::fmt;

use visual_boxes::{BoxError, Square};
use visual_core::{
    DisplayImage, Image, ImageIndex, ImageRange, ImageRate, ImageSize, Seconds, VisualBox,
    VisualError,
};
use visual_hash::hash_sequence;

/// A WGSL shader of the library, which a GPU host compiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ShaderKind {
    /// The square's ([`visual_boxes::SQUARE_WGSL`]).
    Square,
    /// The display encoding's ([`visual_core::DISPLAY_WGSL`]).
    Display,
}

impl ShaderKind {
    /// The shader of code `code`, as a C caller names it: 0 the square's, 1 the display
    /// encoding's; `None` for any other code.
    #[must_use]
    pub const fn from_code(code: u32) -> Option<Self> {
        match code {
            0 => Some(Self::Square),
            1 => Some(Self::Display),
            _ => None,
        }
    }

    /// Its WGSL source.
    #[must_use]
    pub const fn source(self) -> &'static str {
        match self {
            Self::Square => visual_boxes::SQUARE_WGSL,
            Self::Display => visual_core::DISPLAY_WGSL,
        }
    }
}

/// The WGSL source of the shader of code `code` ([`ShaderKind::from_code`]), for a C caller;
/// empty for a code that names none.
#[must_use]
pub const fn shader(code: u32) -> &'static str {
    match ShaderKind::from_code(code) {
        Some(kind) => kind.source(),
        None => "",
    }
}

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
#[non_exhaustive]
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

/// A square set up, the image it renders into and that image encoded for display, all of
/// one size.
///
/// Both images are allocated at setup, even for a host that renders and encodes on its GPU
/// and reads neither: 20 bytes a pixel (16 for the image's four 32-bit values, 4 for its
/// display bytes), 640 MiB at the largest size ([`ImageSize::MAX_PIXELS`]). They are what
/// the host draws with when no GPU draws for it, and allocating them at setup is what lets the
/// setup, not a later render, refuse a size the memory cannot hold. Whether a GPU host should
/// hold them is the resources and GPU memory decision of roadmap #35 step 2.
#[derive(Debug)]
struct Shown {
    /// The square, which holds the image rate.
    square: Square,
    /// The image it renders into, in linear light.
    image: Image,
    /// The same image encoded for display (`spec/visual-design.md`, "Display encoding").
    display: DisplayImage,
}

impl Shown {
    /// `square` with an image and a display image of `size`.
    ///
    /// # Errors
    ///
    /// Returns [`VisualError::OutOfMemory`] when the memory cannot hold them.
    fn try_new(square: Square, size: ImageSize) -> Result<Self, VisualError> {
        let image = Image::try_new(size)?;
        let display = DisplayImage::try_new(size)?;
        Ok(Self {
            square,
            image,
            display,
        })
    }
}

/// A square and the image it renders into, set up by the page.
#[derive(Debug)]
pub struct View {
    /// The square, its image and its display image, once set up.
    shown: Option<Shown>,
    /// The bytes of the last image hash computed.
    hash: [u8; 32],
    /// The last placement asked for: left, right, top, bottom, in subpixel units.
    placed: [u32; 4],
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
            placed: [0; 4],
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
    /// square, if any, is set up again (at its size; its images rendered again by the next
    /// call), or none if the memory cannot even hold it again, every call then refused
    /// until a setup succeeds.
    pub fn setup(
        &mut self,
        width: u32,
        height: u32,
        rate: (u32, u32),
        side: u32,
        cycle: (u32, u32),
    ) -> Result<(), SetupError> {
        // The images replaced are released before the next ones are allocated, so that the
        // two sets are never held at once; a refusal, a value or images the memory cannot
        // hold, sets the previous square up again, at its size, in the memory just freed.
        let previous = self
            .shown
            .take()
            .map(|shown| (shown.square, shown.image.size()));
        let checked =
            Self::checked(width, height, rate, side, cycle).and_then(|(square, image_size)| {
                Shown::try_new(square, image_size).map_err(SetupError::Size)
            });
        match checked {
            Ok(shown) => {
                self.shown = Some(shown);
                self.message = Message::EMPTY;
                Ok(())
            }
            Err(error) => {
                self.shown = previous.and_then(|(square, size)| Shown::try_new(square, size).ok());
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
            .map_or(0, |shown| shown.image.values().len())
    }

    /// The number of bytes of an image encoded for display: width × height × 4; 0 when no
    /// square is set up.
    #[must_use]
    pub fn display_len(&self) -> usize {
        self.shown
            .as_ref()
            .map_or(0, |shown| shown.display.bytes().len())
    }

    /// Renders image `index` and returns its values (four per pixel, row by row from the
    /// top); empty when no square is set up.
    pub fn render(&mut self, index: u64) -> &[f32] {
        match &mut self.shown {
            Some(shown) => {
                shown.square.render(ImageIndex(index), &mut shown.image);
                shown.image.values()
            }
            None => &[],
        }
    }

    /// Renders image `index` and returns it encoded for display: four bytes per pixel,
    /// 8-bit sRGB color then straight alpha, row by row from the top, as a canvas holds
    /// them; empty when no square is set up.
    pub fn display(&mut self, index: u64) -> &[u8] {
        match &mut self.shown {
            Some(shown) => {
                shown.square.render(ImageIndex(index), &mut shown.image);
                // Of one size by construction (`Shown::try_new`): the encoding never
                // refuses them, and the bytes are those of this image.
                match shown.display.encode(&shown.image) {
                    Ok(()) | Err(_) => shown.display.bytes(),
                }
            }
            None => &[],
        }
    }

    /// Where the square is in image `index`, as a GPU renders it ([`Square::placement`]): its
    /// left, right, top and bottom edges in subpixel units, exact; `None` when no square is
    /// set up.
    pub fn placement(&mut self, index: u64) -> Option<&[u32; 4]> {
        let shown = self.shown.as_ref()?;
        let placed = shown
            .square
            .placement(ImageIndex(index), shown.image.size());
        self.placed = [placed.left(), placed.right(), placed.top(), placed.bottom()];
        Some(&self.placed)
    }

    /// Renders `count` images from image `first` and returns their image hash
    /// (`spec/image-hash.md`), the 32 bytes of the SHA-256; `None` when no square is set
    /// up, or when the last of those images would be past image 2⁶⁴ − 1: never a value
    /// that could pass for a hash.
    pub fn hash(&mut self, first: u64, count: u64) -> Option<&[u8; 32]> {
        let range = ImageRange::new(ImageIndex(first), count).ok()?;
        let shown = self.shown.as_mut()?;
        let rate = shown.square.rate();
        self.hash = hash_sequence(&mut shown.square, rate, range, &mut shown.image).bytes();
        Some(&self.hash)
    }
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
