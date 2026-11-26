//! Tests of `rate.rs`.

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString as _};
use alloc::vec::Vec;

use super::{ImageIndex, ImageRange, ImageRate, Seconds, gcd};
use crate::VisualError;

#[test]
fn gives_its_numerators_as_divisors_needing_no_check() {
    let rate = ImageRate::new(24_000, 1001).expect("a valid rate");
    assert_eq!(rate.nonzero_numerator().get(), 24_000, "the rate's");
    let cycle = Seconds::new(1001, 1000).expect("a valid duration");
    assert_eq!(cycle.nonzero_numerator().get(), 1001, "the duration's");
}
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
    // The last: a denominator whose thousandfold overflows `u32`, about one image a second.
    for (numerator, denominator) in [
        (1, 1),
        (24_000, 1001),
        (30_000, 1001),
        (1000, 1),
        (25, 1),
        (u32::MAX, u32::MAX - 1),
    ] {
        let rate = ImageRate::new(numerator, denominator);
        assert_eq!(
            rate.map(|accepted| (accepted.numerator(), accepted.denominator())),
            Ok((numerator, denominator)),
            "{numerator}/{denominator}"
        );
    }
    assert_eq!(
        ImageRate::new(60, 1).map(|rate| (rate.numerator(), rate.denominator())),
        Ok((60, 1)),
        "60"
    );
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
    for (numerator, denominator, what) in [
        (1000, 1, "1000 per second"),
        (1999, 2, "just under 1000"),
        (1002, 1001, "just over 1 per second"),
        (
            u32::MAX,
            u32::MAX - 1,
            "largest terms, just over 1 per second",
        ),
    ] {
        assert_eq!(
            ImageRate::new(numerator, denominator)
                .map(|rate| (rate.numerator(), rate.denominator())),
            Ok((numerator, denominator)),
            "{what}"
        );
    }
    assert_eq!(
        ImageRate::new(1001, 1001),
        Err(VisualError::InvalidImageRate {
            numerator: 1001,
            denominator: 1001
        }),
        "1 per second, unreduced"
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

/// Whether `numerator / denominator` is reduced, by Euclid's algorithm in 64 bits: apart
/// from the code under test.
fn coprime(numerator: u32, denominator: u32) -> bool {
    let (mut left, mut right) = (u64::from(numerator), u64::from(denominator));
    while let Some(rest) = left.checked_rem(right) {
        (left, right) = (right, rest);
    }
    left == 1
}

#[test]
fn accepts_exactly_the_values_its_rules_allow() {
    // Each constructor against its rule written apart, at random terms, edges often: what
    // it accepts, it keeps as given; what it refuses, it names.
    use visual_testkit::generate::{u32_edges, u64_edges};
    use visual_testkit::proptest::test_runner::TestCaseError;
    use visual_testkit::runner::runner;
    let check = |holds: bool, what: String| {
        if holds {
            Ok(())
        } else {
            Err(TestCaseError::fail(what))
        }
    };
    let rates = runner(4096).run(&(u32_edges(), u32_edges()), |(numerator, denominator)| {
        let valid = denominator != 0
            && coprime(numerator, denominator)
            && numerator >= denominator
            && u64::from(numerator) <= u64::from(denominator) * 1000;
        let made = ImageRate::new(numerator, denominator);
        check(
            made.map(|rate| (rate.numerator(), rate.denominator()))
                == if valid {
                    Ok((numerator, denominator))
                } else {
                    Err(VisualError::InvalidImageRate {
                        numerator,
                        denominator,
                    })
                },
            format!("rate {numerator}/{denominator}: {made:?}"),
        )
    });
    assert_eq!(rates.map_err(|error| error.to_string()), Ok(()), "rates");
    let durations = runner(4096).run(&(u32_edges(), u32_edges()), |(numerator, denominator)| {
        let valid = numerator != 0 && denominator != 0 && coprime(numerator, denominator);
        let made = Seconds::new(numerator, denominator);
        check(
            made.map(|seconds| (seconds.numerator(), seconds.denominator()))
                == if valid {
                    Ok((numerator, denominator))
                } else {
                    Err(VisualError::InvalidSeconds {
                        numerator,
                        denominator,
                    })
                },
            format!("{numerator}/{denominator} s: {made:?}"),
        )
    });
    assert_eq!(
        durations.map_err(|error| error.to_string()),
        Ok(()),
        "durations"
    );
    let ranges = runner(4096).run(&(u64_edges(), u64_edges()), |(first, count)| {
        let valid = count == 0 || first.checked_add(count - 1).is_some();
        let made = ImageRange::new(ImageIndex(first), count);
        let expected = if valid {
            Ok((first, count))
        } else {
            Err(VisualError::InvalidImageRange { first, count })
        };
        check(
            made.map(|range| (range.first().0, range.count())) == expected,
            format!("{count} images from {first}: {made:?}"),
        )
    });
    assert_eq!(ranges.map_err(|error| error.to_string()), Ok(()), "ranges");
}
