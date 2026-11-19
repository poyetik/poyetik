//! Values a host gives, encoded for display: the display encoding run on any values, so that
//! a host on another target compares its bytes with the native ones (the display corpus of
//! `conformance/visual/display.json`, which `visual-cli` writes).

use alloc::vec::Vec;

use visual_core::{DisplayImage, Image, ImageSize, pixel8};

/// Room for values to encode for display, and their bytes once encoded.
///
/// Both are allocated together, of one count of pixels, so that encoding never allocates
/// and never meets values and bytes of different counts: the values are written through
/// [`Encoding::values`], the only way to them, and encoded by [`Encoding::encode`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Encoding {
    /// The values, four per pixel (red, green, blue, alpha, premultiplied linear light).
    values: Vec<f32>,
    /// The bytes encoded from them, four per pixel ([`pixel8`]).
    bytes: Vec<u8>,
}

impl Encoding {
    /// No room yet: no value, no byte.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            values: Vec::new(),
            bytes: Vec::new(),
        }
    }

    /// Makes room for `pixels` pixels and returns their values to write, all 0; `None`,
    /// and no room left, for no pixel, for more than an image holds
    /// ([`ImageSize::MAX_PIXELS`]), or when the memory cannot hold them (16 bytes of values
    /// and 4 of bytes a pixel, at most 640 MiB).
    pub fn values(&mut self, pixels: u32) -> Option<&mut [f32]> {
        self.values_with(pixels, Vec::try_reserve_exact, Vec::try_reserve_exact)
    }

    /// [`Encoding::values`], reserving the values and the bytes with `values` and `bytes`:
    /// the allocator's own, or ones that refuse, in the tests (function pointers, so that the
    /// body the tests cover is the one shipped).
    fn values_with(
        &mut self,
        pixels: u32,
        values: fn(&mut Vec<f32>, usize) -> Result<(), alloc::collections::TryReserveError>,
        bytes: fn(&mut Vec<u8>, usize) -> Result<(), alloc::collections::TryReserveError>,
    ) -> Option<&mut [f32]> {
        // The old room is released first, so that a refusal leaves none.
        *self = Self::new();
        let count = u64::from(pixels);
        if count == 0 || count > ImageSize::MAX_PIXELS {
            return None;
        }
        // At most 2²⁵ pixels: their values and bytes fit a usize on every target, so that
        // neither conversion nor product saturates.
        let held = usize::try_from(count).unwrap_or(usize::MAX);
        let (value_count, byte_count) = (
            held.saturating_mul(Image::CHANNELS),
            held.saturating_mul(DisplayImage::CHANNELS),
        );
        let mut room = Self::new();
        values(&mut room.values, value_count).ok()?;
        bytes(&mut room.bytes, byte_count).ok()?;
        // Within the reservations: no other allocation.
        room.values.resize(value_count, 0.0);
        room.bytes.resize(byte_count, 0);
        *self = room;
        Some(&mut self.values)
    }

    /// Encodes the values written for display, each pixel by [`pixel8`] (8-bit sRGB, straight
    /// alpha), and returns the bytes, four per pixel; empty without room. Allocates nothing.
    pub fn encode(&mut self) -> &[u8] {
        let (pixels, _) = self.bytes.as_chunks_mut::<{ DisplayImage::CHANNELS }>();
        let (values, _) = self.values.as_chunks::<{ Image::CHANNELS }>();
        for (pixel, &[red, green, blue, alpha]) in pixels.iter_mut().zip(values) {
            *pixel = pixel8(red, green, blue, alpha);
        }
        &self.bytes
    }
}

#[cfg(test)]
#[path = "encoding_tests.rs"]
mod tests;
