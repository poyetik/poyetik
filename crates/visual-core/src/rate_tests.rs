//! Tests of `rate.rs`.

extern crate alloc;

use alloc::vec::Vec;

use super::{ImageIndex, ImageRange, ImageRate, Seconds, gcd};
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

#[test]
fn ranges_up_to_the_largest_index() {
    for (first, count, indices) in [
        (5, 3, [5, 6, 7].as_slice()),
        (5, 0, [].as_slice()),
        (0, 1, [0].as_slice()),
        (u64::MAX, 1, [u64::MAX].as_slice()),
        (
            u64::MAX - 4,
            5,
            [
                u64::MAX - 4,
                u64::MAX - 3,
                u64::MAX - 2,
                u64::MAX - 1,
                u64::MAX,
            ]
            .as_slice(),
        ),
        (u64::MAX, 0, [].as_slice()),
    ] {
        let range = ImageRange::new(ImageIndex(first), count).expect("a valid range");
        assert_eq!(
            (range.first(), range.count()),
            (ImageIndex(first), count),
            "{count} from {first}"
        );
        let listed: Vec<u64> = range.indices().map(|index| index.0).collect();
        assert_eq!(listed, indices, "{count} from {first}");
    }
}

#[test]
fn refuses_a_range_past_the_largest_index() {
    for (first, count) in [
        (u64::MAX, 2),
        (u64::MAX - 3, 5),
        (2, u64::MAX),
        (u64::MAX, u64::MAX),
    ] {
        assert_eq!(
            ImageRange::new(ImageIndex(first), count),
            Err(VisualError::InvalidImageRange { first, count }),
            "{count} from {first}"
        );
    }
    for first in [0, 1] {
        assert_eq!(
            ImageRange::new(ImageIndex(first), u64::MAX).map(ImageRange::count),
            Ok(u64::MAX),
            "from {first}: up to the largest index, none past"
        );
    }
}
