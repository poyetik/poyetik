//! Exact pixel coverage: the area of a pixel a shape covers, computed in integers, and
//! its exact conversion to a light intensity.

/// Subpixel units per pixel side.
///
/// Every coordinate is a whole number of these. A pixel is `SUBPIXELS × SUBPIXELS` units, so its coverage is a whole number from 0 to
/// `SUBPIXELS²` = 2²⁴, exactly representable as an `f32`.
pub const SUBPIXELS: u32 = 4096;

/// 2⁻²⁴, the intensity of one subpixel unit of area: a power of two, so multiplying by
/// it is exact.
const UNIT_AREA: f32 = f32::from_bits(0x3380_0000);

/// The intensity of a pixel `coverage` units of area are covered of, from 0 to 1, exact.
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
mod tests {
    use super::{SUBPIXELS, UNIT_AREA, coverage_to_intensity, overlap};

    #[test]
    fn the_unit_area_is_two_to_the_minus_24() {
        let mut expected = 1.0_f32;
        for _ in 0..24_u32 {
            expected /= 2.0;
        }
        assert_eq!(UNIT_AREA.to_bits(), expected.to_bits(), "2^-24");
    }

    #[test]
    fn converts_every_coverage_exactly() {
        // Every value from 0 to 2^24 is checked against an independent exact
        // computation: an integer below 2^24 converts exactly through f64.
        let full = SUBPIXELS * SUBPIXELS;
        let mut coverage = 0_u32;
        while coverage <= full {
            let exact = f64::from(coverage) / f64::from(full);
            let intensity = coverage_to_intensity(coverage);
            assert_eq!(
                f64::from(intensity).to_bits(),
                exact.to_bits(),
                "coverage {coverage}"
            );
            coverage += 1;
        }
    }

    #[test]
    fn spans_zero_to_one() {
        assert_eq!(coverage_to_intensity(0).to_bits(), 0.0_f32.to_bits(), "0");
        assert_eq!(
            coverage_to_intensity(SUBPIXELS * SUBPIXELS).to_bits(),
            1.0_f32.to_bits(),
            "full pixel"
        );
        assert_eq!(
            coverage_to_intensity(1).to_bits(),
            UNIT_AREA.to_bits(),
            "one unit"
        );
        assert_eq!(
            coverage_to_intensity(0x0001_0000).to_bits(),
            (1.0_f32 / 256.0).to_bits(),
            "high half only"
        );
    }

    #[test]
    fn clamps_more_than_a_pixel_to_one() {
        for coverage in [SUBPIXELS * SUBPIXELS + 1, u32::MAX] {
            assert_eq!(
                coverage_to_intensity(coverage).to_bits(),
                1.0_f32.to_bits(),
                "{coverage}"
            );
        }
    }

    #[test]
    fn measures_overlaps_with_a_cell() {
        let side = u64::from(SUBPIXELS);
        for (start, end, cell, length) in [
            (0, side, 0, SUBPIXELS),
            (0, side, side, 0),
            (side / 2, side * 3, 0, SUBPIXELS / 2),
            (side / 2, side * 3, side, SUBPIXELS),
            (side / 2, side * 3, side * 2, SUBPIXELS),
            (side / 2, side * 3, side * 3, 0),
            (10, 20, 0, 10),
            (side - 1, side + 1, 0, 1),
            (side - 1, side + 1, side, 1),
            (5, 5, 0, 0),
            (7, 3, 0, 0),
        ] {
            assert_eq!(
                overlap(start, end, cell),
                length,
                "[{start}, {end}) on the cell at {cell}"
            );
        }
    }

    #[test]
    fn measures_overlaps_at_the_end_of_the_range() {
        assert_eq!(
            overlap(u64::MAX - 5, u64::MAX, u64::MAX - 10),
            5,
            "a cell whose end saturates"
        );
    }
}
