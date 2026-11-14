//! Tests of `rate.rs`.

use super::{ImageRate, Seconds, gcd};
use crate::VisualError;

#[test]
fn computes_the_greatest_common_divisor() {
    for (first, second, divisor) in [
        (12, 18, 6),
        (18, 12, 6),
        (7, 0, 7),
        (0, 7, 7),
        (24_000, 1001, 1),
    ] {
        assert_eq!(gcd(first, second), divisor, "gcd({first}, {second})");
    }
}

#[test]
fn accepts_reduced_rates_within_range() {
    for (numerator, denominator) in [(1, 1), (24_000, 1001), (30_000, 1001), (1000, 1), (25, 1)] {
        let rate = ImageRate::new(numerator, denominator);
        assert_eq!(
            rate.map(|accepted| (accepted.numerator(), accepted.denominator())),
            Ok((numerator, denominator)),
            "{numerator}/{denominator}"
        );
    }
    assert_eq!(ImageRate::new(60, 1), Ok(ImageRate::PER_SECOND_60), "60");
}

#[test]
fn rejects_unreduced_out_of_range_or_undefined_rates() {
    for (numerator, denominator) in [
        (120, 2),
        (0, 1),
        (1, 2),
        (999, 1000),
        (1001, 1),
        (2001, 2),
        (5, 0),
        (0, 0),
        (u32::MAX, 1),
    ] {
        assert_eq!(
            ImageRate::new(numerator, denominator),
            Err(VisualError::InvalidImageRate {
                numerator,
                denominator
            }),
            "{numerator}/{denominator}"
        );
    }
}

#[test]
fn accepts_the_edges_of_the_range() {
    assert!(ImageRate::new(1000, 1).is_ok(), "1000 per second");
    assert!(ImageRate::new(1999, 2).is_ok(), "just under 1000");
    assert!(
        ImageRate::new(1001, 1001).is_err(),
        "1 per second, unreduced"
    );
    assert!(ImageRate::new(1002, 1001).is_ok(), "just over 1 per second");
    assert!(
        ImageRate::new(u32::MAX, u32::MAX - 1).is_ok(),
        "largest terms, just over 1 per second"
    );
}

#[test]
fn accepts_only_reduced_nonzero_durations() {
    for (numerator, denominator) in [(1, 1), (1001, 1000), (u32::MAX, 1), (1, u32::MAX)] {
        let seconds = Seconds::new(numerator, denominator);
        assert_eq!(
            seconds.map(|accepted| (accepted.numerator(), accepted.denominator())),
            Ok((numerator, denominator)),
            "{numerator}/{denominator}"
        );
    }
    for (numerator, denominator) in [(0, 1), (1, 0), (0, 0), (2, 4), (6, 3)] {
        assert_eq!(
            Seconds::new(numerator, denominator),
            Err(VisualError::InvalidSeconds {
                numerator,
                denominator
            }),
            "{numerator}/{denominator}"
        );
    }
}
