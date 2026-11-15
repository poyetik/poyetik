//! Exact pixel coverage: the area of a pixel a shape covers, computed in integers, and
//! its exact conversion to a light intensity.

/// Subpixel units per pixel side.
///
/// Every coordinate is a whole number of these. A pixel is `SUBPIXELS × SUBPIXELS`
/// units, so its coverage is a whole number from 0 to `SUBPIXELS²` = 2²⁴, exactly
/// representable as an `f32`.
pub const SUBPIXELS: u32 = 4096;

/// 2⁻²⁴, the intensity of one subpixel unit of area: a power of two, so multiplying by
/// it is exact.
const UNIT_AREA: f32 = f32::from_bits(0x3380_0000);

/// The intensity of a pixel of which `coverage` subpixel units of area are covered, from 0
/// to 1, exact.
///
/// `coverage` is clamped to `SUBPIXELS²` (a full pixel). The result is exactly
/// `coverage / 2²⁴`: both halves of `coverage` convert to `f32` without rounding, their
/// sum stays below 2²⁴ + 1 so it is exact, and the scale is a power of two.
#[must_use]
pub fn coverage_to_intensity(coverage: u32) -> f32 {
    let full = SUBPIXELS.saturating_mul(SUBPIXELS);
    let area = coverage.min(full);
    let high = u16::try_from(area >> 16_u32).unwrap_or(u16::MAX);
    let low = u16::try_from(area & 0xffff).unwrap_or(u16::MAX);
    (f32::from(high) * 65_536.0_f32 + f32::from(low)) * UNIT_AREA
}

/// The length of the overlap of `[start, end)` with `[cell_start, cell_start + SUBPIXELS)`,
/// in subpixel units: from 0 to [`SUBPIXELS`].
#[must_use]
pub fn overlap(start: u64, end: u64, cell_start: u64) -> u32 {
    let cell_end = cell_start.saturating_add(u64::from(SUBPIXELS));
    let low = start.max(cell_start);
    let high = end.min(cell_end);
    u32::try_from(high.saturating_sub(low)).unwrap_or(SUBPIXELS)
}

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod tests;
