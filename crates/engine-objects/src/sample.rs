//! Conversion of computed values to output samples.

/// Rounds an `f64` to the nearest `f32` sample.
///
/// This IEEE 754 conversion (round to nearest, ties to even) is exact and identical on
/// every target. The standard library offers no `From` for it, hence the cast.
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    reason = "f64 to f32 rounding is the intended, IEEE 754-defined conversion; no From exists"
)]
pub(super) const fn to_sample(value: f64) -> f32 {
    value as f32
}

#[cfg(test)]
mod tests {
    use super::to_sample;

    #[test]
    fn rounds_to_the_nearest_f32() {
        assert_eq!(
            to_sample(0.5).to_bits(),
            0.5_f32.to_bits(),
            "exact values are kept"
        );
        assert_eq!(
            to_sample(-1.0).to_bits(),
            (-1.0_f32).to_bits(),
            "sign is kept"
        );
        assert_eq!(
            to_sample(0.1).to_bits(),
            0.1_f32.to_bits(),
            "0.1 rounds to the same f32 as the literal"
        );
    }
}
