//! Generators of random inputs for the property tests of the visual library.
//!
//! Raw values only (integers and pairs of integers), never the library's types, so that
//! any crate of the library can use them in its own unit tests, where its types are its
//! own.
//!
//! Each draws its domain's extremes far more often than a uniform draw would, so that a
//! few hundred cases reach them: a property that holds only away from the edges fails here.

use core::num::NonZeroU32;

use proptest::prelude::{Just, Strategy, any, prop_oneof};

/// The most images per second, 1000: the fastest numerator is this times the denominator.
const PER_SECOND: NonZeroU32 = match NonZeroU32::new(1000) {
    Some(thousand) => thousand,
    None => NonZeroU32::MIN,
};

/// The denominator of the NTSC rates (`24000/1001`, `30000/1001`).
const NTSC: NonZeroU32 = match NonZeroU32::new(1001) {
    Some(ntsc) => ntsc,
    None => NonZeroU32::MIN,
};

/// The greatest common divisor of `left` and `right`, never 0 since `right` is not.
fn gcd(mut left: u32, mut right: NonZeroU32) -> NonZeroU32 {
    while let Some(rest) = NonZeroU32::new(left % right) {
        left = right.get();
        right = rest;
    }
    right
}

/// `numerator / denominator` reduced.
fn reduced(numerator: u32, denominator: NonZeroU32) -> (u32, u32) {
    let common = gcd(numerator, denominator);
    (numerator / common, denominator.get() / common)
}

/// Any `u32`, its edges (0, 1, 2, the largest and its neighbors) and small values often.
pub fn u32_edges() -> impl Strategy<Value = u32> {
    prop_oneof![
        Just(0_u32),
        Just(1_u32),
        Just(2_u32),
        Just(u32::MAX),
        Just(u32::MAX - 1),
        0_u32..=100,
        any::<u32>(),
    ]
}

/// Any `u64`, its edges (0, 1, the largest, the largest `u32` and `2⁵³ + 1`, the first
/// integer a 64-bit float rounds) and the values near both ends often.
pub fn u64_edges() -> impl Strategy<Value = u64> {
    prop_oneof![
        Just(0_u64),
        Just(1_u64),
        Just(u64::MAX),
        Just(u64::from(u32::MAX)),
        Just((1_u64 << 53_u32) + 1),
        0_u64..=100,
        (u64::MAX - 100)..=u64::MAX,
        any::<u64>(),
    ]
}

/// Any nonzero `u32`, its edges often.
fn nonzero_edges() -> impl Strategy<Value = NonZeroU32> {
    u32_edges().prop_filter_map("never 0", NonZeroU32::new)
}

/// The terms of a valid image rate, reduced, from 1 to 1000 images per second.
///
/// Denominators from 1 to the largest `u32`, numerators from the slowest rate to the fastest the
/// denominator allows, both ends often.
pub fn rate_terms() -> impl Strategy<Value = (u32, u32)> {
    prop_oneof![
        Just(NonZeroU32::MIN),
        Just(NTSC),
        Just(NonZeroU32::MAX),
        (1_u32..=2000).prop_filter_map("never 0", NonZeroU32::new),
        any::<NonZeroU32>(),
    ]
    .prop_flat_map(|denominator| {
        let fastest = denominator.saturating_mul(PER_SECOND).get();
        (
            prop_oneof![
                Just(denominator.get()),
                Just(fastest),
                denominator.get()..=fastest
            ],
            Just(denominator),
        )
    })
    .prop_map(|(numerator, denominator)| reduced(numerator, denominator))
}

/// The terms of a valid duration in seconds, reduced and nonzero: any `u32` terms, their
/// edges often.
pub fn seconds_terms() -> impl Strategy<Value = (u32, u32)> {
    (nonzero_edges(), nonzero_edges())
        .prop_map(|(numerator, denominator)| reduced(numerator.get(), denominator))
}

/// A valid side of a square, in 65 536ths of the image height: from none to the full
/// height, both ends and their neighbors often.
pub fn side() -> impl Strategy<Value = u32> {
    prop_oneof![
        Just(0_u32),
        Just(1_u32),
        Just(65_535_u32),
        Just(65_536_u32),
        0_u32..=65_536,
    ]
}

/// A small image size, each side from 1 to 8 pixels: every pixel of it can be checked
/// against a reference at every case.
pub fn small_size() -> impl Strategy<Value = (u32, u32)> {
    (1_u32..=8, 1_u32..=8)
}

/// A wide image size: a row of up to the longest side, 16 384 pixels, at most 3 rows, the
/// widest often (where the motion's travel is longest).
pub fn wide_size() -> impl Strategy<Value = (u32, u32)> {
    (prop_oneof![Just(16_384_u32), 1_u32..=16_384], 1_u32..=3)
}

/// A tall image size: a column of up to the longest side, 16 384 pixels, at most 3
/// columns, the tallest often (where the square's side is longest).
pub fn tall_size() -> impl Strategy<Value = (u32, u32)> {
    (1_u32..=3, prop_oneof![Just(16_384_u32), 1_u32..=16_384])
}

#[cfg(test)]
#[path = "generate_tests.rs"]
mod tests;
