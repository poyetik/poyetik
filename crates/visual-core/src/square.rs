//! The square: a white square on opaque black, sweeping from the left edge to the right
//! edge once per cycle, with exact edge coverage.

use crate::coverage::overlap;
use crate::rate::gcd;
use crate::{
    Image, ImageIndex, ImageRate, SUBPIXELS, SubpixelPoint, Trace, VisualBox, VisualError,
    coverage_to_intensity,
};

/// The duration of one sweep: `numerator / denominator` seconds, an exact, reduced,
/// nonzero fraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Cycle {
    /// Seconds, times `denominator`.
    numerator: u32,
    /// Divides `numerator`.
    denominator: u32,
}

impl Cycle {
    /// Creates a cycle of `numerator / denominator` seconds.
    ///
    /// # Errors
    ///
    /// Returns [`VisualError::InvalidCycle`] unless both terms are nonzero and the
    /// fraction is reduced.
    pub fn new(numerator: u32, denominator: u32) -> Result<Self, VisualError> {
        if numerator == 0
            || denominator == 0
            || gcd(u64::from(numerator), u64::from(denominator)) != 1
        {
            Err(VisualError::InvalidCycle {
                numerator,
                denominator,
            })
        } else {
            Ok(Self {
                numerator,
                denominator,
            })
        }
    }
}

/// Where the square is in one image, in subpixel units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Placement {
    /// Left edge (included).
    left: u64,
    /// Right edge (excluded).
    right: u64,
    /// Top edge (included).
    top: u64,
    /// Bottom edge (excluded).
    bottom: u64,
}

/// A white square on opaque black, sweeping from left to right once per cycle.
///
/// Its side is a fraction of the image height, it is vertically centered, and its left
/// edge sweeping from the left edge of the image (at the start of
/// each cycle) towards the position where its right edge meets the right edge of the
/// image (at the end of the cycle), then jumping back.
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
    cycle: Cycle,
}

impl Square {
    /// The side of a square as tall as the image: sides are in 65 536ths of the height.
    pub const FULL_SIDE: u32 = 65_536;

    /// Creates a square of side `side` / 65 536 of the image height, sweeping once per
    /// `cycle`, for images at `rate`.
    ///
    /// # Errors
    ///
    /// Returns [`VisualError::InvalidSide`] when `side` is above
    /// [`Square::FULL_SIDE`].
    pub const fn new(rate: ImageRate, side: u32, cycle: Cycle) -> Result<Self, VisualError> {
        if side > Self::FULL_SIDE {
            Err(VisualError::InvalidSide(side))
        } else {
            Ok(Self { rate, side, cycle })
        }
    }

    /// Where the square is in image `index`, for an output of `width` × `height` pixels.
    fn place(&self, index: ImageIndex, width: u32, height: u32) -> Placement {
        let unit = u64::from(SUBPIXELS);
        let image_width = u64::from(width).saturating_mul(unit);
        let image_height = u64::from(height).saturating_mul(unit);
        let side = image_height.saturating_mul(u64::from(self.side)) >> 16_u32;
        let travel = image_width.saturating_sub(side);
        // Elapsed cycles: index × rate denominator × cycle denominator
        // / (rate numerator × cycle numerator); only its fractional part matters.
        let elapsed = u128::from(index.0)
            .saturating_mul(u128::from(self.rate.denominator()))
            .saturating_mul(u128::from(self.cycle.denominator));
        let period = u128::from(self.rate.numerator())
            .saturating_mul(u128::from(self.cycle.numerator))
            .max(1);
        let within = elapsed.checked_rem(period).unwrap_or(0);
        let offset = within
            .saturating_mul(u128::from(travel))
            .checked_div(period)
            .unwrap_or(0);
        let left = u64::try_from(offset).unwrap_or(travel);
        let top = image_height.saturating_sub(side) >> 1_u32;
        Placement {
            left,
            right: left.saturating_add(side),
            top,
            bottom: top.saturating_add(side),
        }
    }
}

impl VisualBox for Square {
    fn render(&mut self, index: ImageIndex, output: &mut Image) {
        let size = output.size();
        let placed = self.place(index, size.width(), size.height());
        let unit = u64::from(SUBPIXELS);
        let mut row_start = 0_u64;
        for row in output.rows_mut() {
            let vertical = overlap(placed.top, placed.bottom, row_start);
            let mut column_start = 0_u64;
            for pixel in row.as_chunks_mut::<{ Image::CHANNELS }>().0 {
                let horizontal = overlap(placed.left, placed.right, column_start);
                let value = coverage_to_intensity(vertical.saturating_mul(horizontal));
                *pixel = [value, value, value, 1.0];
                column_start = column_start.saturating_add(unit);
            }
            row_start = row_start.saturating_add(unit);
        }
    }

    fn trace(&self, index: ImageIndex, output: &Image, point: SubpixelPoint) -> Trace {
        let size = output.size();
        let placed = self.place(index, size.width(), size.height());
        if (placed.left..placed.right).contains(&point.x)
            && (placed.top..placed.bottom).contains(&point.y)
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
