//! Tests of the generators: every value in its domain, and every edge reached.

use core::num::NonZeroU32;

use proptest::strategy::{Strategy, ValueTree};

use super::{
    NTSC, PER_SECOND, gcd, rate_terms, reduced, seconds_terms, side, small_size, tall_size,
    u32_edges, u64_edges, wide_size,
};
use crate::runner::runner;

/// `count` values drawn from `strategy`, from the fixed seed.
fn drawn<S: Strategy>(strategy: &S, count: usize) -> Vec<S::Value> {
    let mut source = runner(1);
    (0..count)
        .map(|_| strategy.new_tree(&mut source).expect("a value").current())
        .collect()
}

/// Whether `numerator / denominator` is reduced: computed apart from the generators' own
/// `gcd`, in 64 bits.
fn is_reduced(numerator: u32, denominator: u32) -> bool {
    let (mut left, mut right) = (u64::from(numerator), u64::from(denominator));
    while let Some(rest) = left.checked_rem(right) {
        (left, right) = (right, rest);
    }
    left == 1
}

#[test]
fn reduces_a_fraction_by_its_greatest_common_divisor() {
    let nonzero = |value| NonZeroU32::new(value).expect("nonzero");
    for (left, right, common) in [
        (12_u32, 18_u32, 6_u32),
        (18, 12, 6),
        (7, 3, 1),
        (0, 5, 5),
        (5, 5, 5),
        (u32::MAX, u32::MAX, u32::MAX),
        (24_000, 1001, 1),
    ] {
        assert_eq!(
            gcd(left, nonzero(right)).get(),
            common,
            "gcd({left}, {right})"
        );
    }
    assert_eq!(reduced(120, nonzero(2)), (60, 1), "120/2");
    assert_eq!(reduced(0, nonzero(7)), (0, 1), "0/7");
    assert_eq!(
        reduced(24_000, nonzero(1001)),
        (24_000, 1001),
        "already reduced"
    );
    assert_eq!(
        (PER_SECOND.get(), NTSC.get()),
        (1000, 1001),
        "the constants"
    );
}

#[test]
fn draws_every_edge_of_the_integers() {
    let small = drawn(&u32_edges(), 2000);
    for edge in [0, 1, 2, u32::MAX, u32::MAX - 1] {
        assert!(small.contains(&edge), "{edge}");
    }
    assert!(
        small.iter().any(|&value| (3..=100).contains(&value)),
        "small values"
    );
    assert!(
        small
            .iter()
            .any(|&value| value > 100 && value < u32::MAX - 1),
        "values far from the edges"
    );
    let wide = drawn(&u64_edges(), 2000);
    for edge in [0, 1, u64::MAX, u64::from(u32::MAX), (1 << 53_u32) + 1] {
        assert!(wide.contains(&edge), "{edge}");
    }
    assert!(
        wide.iter()
            .any(|&value| value > u64::MAX - 100 && value < u64::MAX),
        "near the largest"
    );
    assert!(
        wide.iter().any(|&value| (2..=100).contains(&value)),
        "small values"
    );
    assert!(
        wide.iter()
            .any(|&value| value > u64::from(u32::MAX) && value < u64::MAX - 100),
        "values far from the edges"
    );
}

#[test]
fn draws_valid_reduced_rates_up_to_both_ends() {
    let rates = drawn(&rate_terms(), 2000);
    for &(numerator, denominator) in &rates {
        assert!(denominator >= 1, "{numerator}/{denominator}");
        assert!(
            numerator >= denominator && u64::from(numerator) <= u64::from(denominator) * 1000,
            "{numerator}/{denominator}: from 1 to 1000 a second"
        );
        assert!(
            is_reduced(numerator, denominator),
            "{numerator}/{denominator}"
        );
    }
    for (rate, what) in [((1, 1), "one a second"), ((1000, 1), "the fastest")] {
        assert!(rates.contains(&rate), "{what}");
    }
    assert!(
        rates
            .iter()
            .any(|&(_, denominator)| denominator > 1_u32 << 30_u32),
        "32-bit denominators"
    );
    assert!(
        rates.iter().any(|&(numerator, _)| numerator == u32::MAX),
        "the largest numerator"
    );
}

#[test]
fn draws_valid_reduced_durations_with_32_bit_terms() {
    let durations = drawn(&seconds_terms(), 2000);
    for &(numerator, denominator) in &durations {
        assert!(
            numerator >= 1 && denominator >= 1,
            "{numerator}/{denominator}"
        );
        assert!(
            is_reduced(numerator, denominator),
            "{numerator}/{denominator}"
        );
    }
    for duration in [
        (1, 1),
        (u32::MAX, 1),
        (1, u32::MAX),
        (u32::MAX, u32::MAX - 1),
    ] {
        assert!(durations.contains(&duration), "{duration:?}");
    }
}

#[test]
fn draws_every_side_and_size_up_to_the_largest() {
    let sides = drawn(&side(), 1000);
    assert!(
        sides.iter().all(|&value| value <= 65_536),
        "at most the full height"
    );
    for edge in [0, 1, 65_535, 65_536] {
        assert!(sides.contains(&edge), "{edge}");
    }
    assert!(
        sides.iter().any(|&value| (2..65_535).contains(&value)),
        "between"
    );
    let small = drawn(&small_size(), 1000);
    assert!(
        small
            .iter()
            .all(|&(width, height)| (1..=8).contains(&width) && (1..=8).contains(&height)),
        "small"
    );
    assert!(
        small.contains(&(1, 1)) && small.contains(&(8, 8)),
        "both ends"
    );
    let wide = drawn(&wide_size(), 1000);
    assert!(
        wide.iter()
            .all(|&(width, height)| (1..=16_384).contains(&width) && (1..=3).contains(&height)),
        "wide"
    );
    assert!(
        wide.contains(&(16_384, 3)) && wide.iter().any(|&(width, _)| width < 16_384),
        "the widest and narrower"
    );
    let tall = drawn(&tall_size(), 1000);
    assert!(
        tall.iter()
            .all(|&(width, height)| (1..=3).contains(&width) && (1..=16_384).contains(&height)),
        "tall"
    );
    assert!(
        tall.contains(&(3, 16_384)) && tall.iter().any(|&(_, height)| height < 16_384),
        "the tallest and shorter"
    );
}
