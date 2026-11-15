//! The square: a white square on opaque black, sweeping from the left edge to the right
//! edge once per cycle, with exact edge coverage.

use core::num::NonZeroU128;

use visual_core::{
    Image, ImageIndex, ImageRate, ImageSize, SUBPIXELS, Seconds, SubpixelPoint, Trace, VisualBox,
    coverage::overlap, coverage_to_intensity,
};

use crate::BoxError;

/// Where the square is in one image, in subpixel units.
///
/// [`visual_core::SUBPIXELS`] per pixel side: the half-open rectangle
/// `[left, right) × [top, bottom)`, exact. Every edge is at most
/// [`ImageSize::MAX_SIDE`] × [`SUBPIXELS`] = 2²⁶, so 32 bits hold it: what a renderer other
/// than the CPU reference (the GPU's, whose shading language has no wider integers) needs to
/// draw the same pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    /// Left edge (included).
    left: u32,
    /// Right edge (excluded).
    right: u32,
    /// Top edge (included).
    top: u32,
    /// Bottom edge (excluded).
    bottom: u32,
}

impl Placement {
    /// The left edge, included.
    #[must_use]
    pub const fn left(&self) -> u32 {
        self.left
    }

    /// The right edge, excluded.
    #[must_use]
    pub const fn right(&self) -> u32 {
        self.right
    }

    /// The top edge, included.
    #[must_use]
    pub const fn top(&self) -> u32 {
        self.top
    }

    /// The bottom edge, excluded.
    #[must_use]
    pub const fn bottom(&self) -> u32 {
        self.bottom
    }
}

/// A white square on opaque black, sweeping from left to right once per cycle.
///
/// Its side is a fraction of the image height, rounded down to a subpixel unit, and it is
/// vertically centered. Its left edge starts each cycle at the left edge of the image and
/// sweeps right, never quite reaching the position where its right edge meets the image's
/// right edge: the next cycle starts at the left edge again. A square wider than the image
/// does not move, and is clipped on the right.
///
/// Every edge falls on a subpixel unit, and each pixel's value is the exact area of the
/// pixel the square covers: antialiasing with no approximation. A pure function of the
/// image index, so any image can be rendered first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Square {
    /// The image rate, which ties image indices to seconds.
    rate: ImageRate,
    /// The side, in 65 536ths of the image height.
    side: u32,
    /// The duration of one sweep.
    cycle: Seconds,
}

impl Square {
    /// The side of a square as tall as the image: sides are in 65 536ths of the height.
    pub const FULL_SIDE: u32 = 65_536;

    /// Creates a square of side `side` / 65 536 of the image height, sweeping once per
    /// `cycle`, for images at `rate`.
    ///
    /// # Errors
    ///
    /// Returns [`BoxError::InvalidSide`] when `side` is above [`Square::FULL_SIDE`].
    pub const fn new(rate: ImageRate, side: u32, cycle: Seconds) -> Result<Self, BoxError> {
        if side > Self::FULL_SIDE {
            Err(BoxError::InvalidSide(side))
        } else {
            Ok(Self { rate, side, cycle })
        }
    }

    /// The image rate the square is made for: the rate its images are rendered and hashed
    /// at, held once, by the square.
    #[must_use]
    pub const fn rate(&self) -> ImageRate {
        self.rate
    }

    /// Where the square is in image `index` of `size`, exactly as it renders and traces it.
    #[must_use]
    pub fn placement(&self, index: ImageIndex, size: ImageSize) -> Placement {
        let unit = SUBPIXELS;
        // At most 16 384 × 4096 = 2²⁶ (ImageSize::MAX_SIDE): never saturates.
        let image_width = size.width().saturating_mul(unit);
        let image_height = size.height().saturating_mul(unit);
        // At most `image_height`, since `self.side` is at most 2¹⁶: the fallback is never
        // taken.
        let length =
            u32::try_from(u64::from(image_height).saturating_mul(u64::from(self.side)) >> 16_u32)
                .unwrap_or(image_height);
        let travel = image_width.saturating_sub(length);
        // Elapsed cycles: index × rate denominator × cycle denominator
        // / (rate numerator × cycle numerator); only its fractional part matters. The
        // product is at most (2⁶⁴ − 1)(2³² − 1)² < 2¹²⁸ by the widths of its terms: it never
        // saturates (reached by the extreme-terms test).
        let elapsed = u128::from(index.0)
            .saturating_mul(u128::from(self.rate.denominator()))
            .saturating_mul(u128::from(self.cycle.denominator()));
        // Nonzero by construction: both numerators are, and their product, below 2⁶⁴, never
        // saturates.
        let period = NonZeroU128::from(self.rate.nonzero_numerator())
            .saturating_mul(NonZeroU128::from(self.cycle.nonzero_numerator()));
        let within = elapsed % period;
        // `within × travel` stays below 2⁶⁴ × 2²⁶ (a travel of 16 384 × 4096 subpixels at
        // most): it never saturates either.
        let offset = within.saturating_mul(u128::from(travel)) / period;
        // `offset < travel`, since `within < period`: the fallback is never taken, and it
        // would keep the square inside the image.
        let left = u32::try_from(offset).unwrap_or(travel);
        let top = image_height.saturating_sub(length) >> 1_u32;
        Placement {
            left,
            right: left.saturating_add(length),
            top,
            bottom: top.saturating_add(length),
        }
    }
}

impl VisualBox for Square {
    fn render(&mut self, index: ImageIndex, output: &mut Image) {
        let size = output.size();
        let placed = self.placement(index, size);
        let unit = u64::from(SUBPIXELS);
        let mut row_start = 0_u64;
        for row in output.rows_mut() {
            let vertical = overlap(u64::from(placed.top), u64::from(placed.bottom), row_start);
            let mut column_start = 0_u64;
            for pixel in row.as_chunks_mut::<{ Image::CHANNELS }>().0 {
                let horizontal = overlap(
                    u64::from(placed.left),
                    u64::from(placed.right),
                    column_start,
                );
                let value = coverage_to_intensity(vertical.saturating_mul(horizontal));
                *pixel = [value, value, value, 1.0];
                column_start = column_start.saturating_add(unit);
            }
            row_start = row_start.saturating_add(unit);
        }
    }

    fn trace(&self, index: ImageIndex, point: SubpixelPoint) -> Trace {
        let size = point.size();
        let placed = self.placement(index, size);
        if (u64::from(placed.left)..u64::from(placed.right)).contains(&point.x())
            && (u64::from(placed.top)..u64::from(placed.bottom)).contains(&point.y())
        {
            Trace::Shape(0)
        } else {
            Trace::Background
        }
    }
}

#[cfg(test)]
#[path = "square_tests.rs"]
mod tests;
