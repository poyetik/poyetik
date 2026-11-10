//! Tests of the reduction order (`spec/engine-design.md`, decision 28): the helper against a
//! reference written literally from the rule, on raw floats, apart from the helper.

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;
use core::ops::Add;

use super::{ordered_sum, sum_blocks};

use crate::host::Raw;
use crate::{BLOCK_SIZE, Binary64, Block, Sample};

/// A raw IEEE 754 float the reference computes with: `f64` or `f32`.
trait Float: Copy + Add<Output = Self> + core::fmt::Debug {
    /// Negative zero, where each lane starts.
    const NEGATIVE_ZERO: Self;
    /// Positive zero, the sum of no terms.
    const POSITIVE_ZERO: Self;
    /// Its bit pattern, widened to 64 bits.
    fn bits(self) -> u64;
    /// The value of the bit pattern `bits`, cut to the width of the type.
    fn from_bits(bits: u64) -> Self;
}

impl Float for f64 {
    const NEGATIVE_ZERO: Self = -0.0;
    const POSITIVE_ZERO: Self = 0.0;
    fn bits(self) -> u64 {
        self.to_bits()
    }
    fn from_bits(bits: u64) -> Self {
        Self::from_bits(bits)
    }
}

impl Float for f32 {
    const NEGATIVE_ZERO: Self = -0.0;
    const POSITIVE_ZERO: Self = 0.0;
    fn bits(self) -> u64 {
        u64::from(self.to_bits())
    }
    fn from_bits(bits: u64) -> Self {
        Self::from_bits(u32::try_from(bits & 0xffff_ffff).expect("32 bits"))
    }
}

/// The sum of `terms` written literally from decision 28: a sum of no terms is +0; term `k`
/// goes to lane `k mod 16`, each of the 16 lanes summing its terms in increasing `k` from
/// -0; then lane `i` with lane `i + 8`, then `i + 4`, `i + 2`, `i + 1`, for `i` from 0.
#[expect(
    clippy::integer_division_remainder_used,
    clippy::arithmetic_side_effects,
    reason = "the rule as the specification writes it, on indices below 16"
)]
fn reference<F: Float>(terms: &[F]) -> F {
    if terms.is_empty() {
        return F::POSITIVE_ZERO;
    }
    let mut lanes = [F::NEGATIVE_ZERO; 16];
    for (index, &term) in terms.iter().enumerate() {
        lanes[index % 16] = lanes[index % 16] + term;
    }
    for half in [8, 4, 2, 1] {
        for i in 0..half {
            lanes[i] = lanes[i] + lanes[i + half];
        }
    }
    lanes[0]
}

/// The same terms, summed from left to right: the order decision 28 rejects.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "IEEE 754 addition, which neither overflows nor panics"
)]
fn sequential<F: Float>(terms: &[F]) -> F {
    terms
        .iter()
        .copied()
        .reduce(|total, term| total + term)
        .unwrap_or(F::POSITIVE_ZERO)
}

/// The exponent field and the quiet bit of a float of the width of `F`.
fn fields<F: Float>() -> (u64, u64) {
    if F::NEGATIVE_ZERO.bits() == 1_u64 << 63_u32 {
        (0x7ff0_0000_0000_0000, 0x0008_0000_0000_0000)
    } else {
        (0x7f80_0000, 0x0040_0000)
    }
}

/// Whether `bits` is a NaN of the width of `F`: the exponent field all ones, the
/// fraction not zero.
fn is_nan<F: Float>(bits: u64) -> bool {
    let (exponent, quiet) = fields::<F>();
    let fraction = quiet.wrapping_mul(2).wrapping_sub(1);
    bits & exponent == exponent && bits & fraction != 0
}

/// Whether `summed`, the bits of a sum of `terms`, follows the rule: the bits of the
/// reference whenever it is not a NaN, else the payload rule of [`check`].
fn follows<F: Float>(terms: &[F], summed: u64) -> bool {
    let expected = reference(terms).bits();
    let sign = F::NEGATIVE_ZERO.bits();
    let (exponent, quiet) = fields::<F>();
    if !is_nan::<F>(expected) {
        summed == expected
    } else if let [only] = terms {
        summed == only.bits()
    } else {
        let default = exponent | quiet;
        let mut allowed: Vec<u64> = terms
            .iter()
            .map(|term| term.bits())
            .filter(|&bits| is_nan::<F>(bits))
            .map(|bits| (bits | quiet) & !sign)
            .collect();
        allowed.push(default);
        is_nan::<F>(summed) && allowed.contains(&(summed & !sign))
    }
}

/// Checks the helper against the reference on `terms`: the same bits whenever the reference
/// is not a NaN. A NaN result follows the payload rule: it is a NaN; of one term, that term
/// bit for bit; of more, quiet, with the payload (and quiet bit) of one of the NaN terms, or
/// the default NaN when no term is a NaN (an infinity minus an infinity). Its sign is not
/// compared: WebAssembly leaves it open (roadmap #24).
fn check<S: Raw>(terms: &[S::Float]) -> Result<(), alloc::string::String>
where
    S::Float: Float,
{
    let samples: Vec<S> = terms.iter().map(|&term| S::from_raw(term)).collect();
    let summed = sum_of(&samples).bits();
    let expected = reference(terms).bits();
    if follows(terms, summed) {
        Ok(())
    } else {
        Err(format!(
            "{} terms {:x?}: {summed:#x}, expected {expected:#x}",
            terms.len(),
            terms.iter().map(|term| term.bits()).collect::<Vec<u64>>()
        ))
    }
}

/// The helper on a slice of samples: every test sums through this one call, so that the
/// helper has one instantiation per sample type in the tests, each covered whole.
fn sum_of<S: Sample>(terms: &[S]) -> S {
    ordered_sum(terms.iter().copied())
}

/// The counts every check runs at: none, one, two, around each multiple of 16 up to 64, and
/// large counts, so that every lane holds one, two, three or many terms, or none.
const COUNTS: [usize; 24] = [
    0, 1, 2, 3, 4, 7, 8, 9, 15, 16, 17, 31, 32, 33, 47, 48, 49, 63, 64, 65, 255, 256, 257, 4099,
];

/// A deterministic generator of 64-bit draws (`SplitMix64`), for reproducible terms.
struct Draws(u64);

impl Draws {
    /// The next 64 bits.
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut mixed = self.0;
        mixed = (mixed ^ (mixed >> 30_u32)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        mixed = (mixed ^ (mixed >> 27_u32)).wrapping_mul(0x94d0_49bb_1331_11eb);
        mixed ^ (mixed >> 31_u32)
    }

    /// A finite value of either sign whose magnitude lies between 2^-20 and 2^20, so that
    /// sums of thousands of terms round at every step and never overflow.
    fn finite<F: Float>(&mut self) -> F {
        let bits = self.next();
        let wide = f64::from_bits(
            (bits & 0x800f_ffff_ffff_ffff)
                | ((bits >> 52_u32).wrapping_rem(41).wrapping_add(1003) << 52_u32),
        );
        F::from_bits(if F::NEGATIVE_ZERO.bits() == 1_u64 << 63_u32 {
            wide.to_bits()
        } else {
            u64::from(super::to_binary32(wide).to_bits())
        })
    }
}

/// Every count, with finite terms: the helper and the reference agree bit for bit, and
/// from three terms on the order is revealed (the sequential sum differs on some draw).
fn sums_finite_terms_in_the_specified_order<S: Raw>()
where
    S::Float: Float,
{
    let mut draws = Draws(0x5059_0007);
    for count in COUNTS {
        let mut revealed = false;
        for _ in 0_u32..64 {
            let terms: Vec<S::Float> = (0..count).map(|_| draws.finite()).collect();
            assert_eq!(check::<S>(&terms), Ok(()), "{count} finite terms");
            revealed |= reference(&terms).bits() != sequential(&terms).bits();
        }
        assert_eq!(revealed, count >= 3, "{count} terms reveal the order");
    }
}

/// Special values among the terms, at every count: zeros of both signs, subnormals, the
/// largest finite values, infinities and NaNs with payloads, quiet and signaling.
fn sums_special_values_in_the_specified_order<S: Raw>()
where
    S::Float: Float,
{
    let specials = specials::<S::Float>();
    let mut draws = Draws(0x5059_0028);
    let pick = |source: &mut Draws| {
        let index = usize::try_from(source.next().wrapping_rem(12)).expect("below 12");
        S::Float::from_bits(*specials.get(index).expect("below 12"))
    };
    for count in COUNTS {
        for _ in 0_u32..64 {
            let terms: Vec<S::Float> = (0..count).map(|_| pick(&mut draws)).collect();
            assert_eq!(check::<S>(&terms), Ok(()), "{count} special terms");
            // A few specials among finite terms, as a mix meets them.
            let mut mixed: Vec<S::Float> = (0..count).map(|_| draws.finite()).collect();
            for term in mixed.iter_mut().take(2) {
                *term = pick(&mut draws);
            }
            assert_eq!(check::<S>(&mixed), Ok(()), "{count} mixed terms");
        }
    }
}

/// The special values of the width of `F`: zeros of both signs, the smallest subnormals of
/// both signs, the largest finite values, infinities, a quiet and a signaling NaN with
/// payloads, and 1 and -1.
fn specials<F: Float>() -> [u64; 12] {
    let wide = F::NEGATIVE_ZERO.bits() == 1_u64 << 63_u32;
    if wide {
        [
            0,
            0x8000_0000_0000_0000,
            1,
            0x8000_0000_0000_0001,
            0x7fef_ffff_ffff_ffff,
            0xffef_ffff_ffff_ffff,
            0x7ff0_0000_0000_0000,
            0xfff0_0000_0000_0000,
            0x7ff8_0000_0000_0005,
            0xfff0_0000_0000_0009,
            0x3ff0_0000_0000_0000,
            0xbff0_0000_0000_0000,
        ]
    } else {
        [
            0,
            0x8000_0000,
            1,
            0x8000_0001,
            0x7f7f_ffff,
            0xff7f_ffff,
            0x7f80_0000,
            0xff80_0000,
            0x7fc0_0005,
            0xff80_0009,
            0x3f80_0000,
            0xbf80_0000,
        ]
    }
}

/// The edges written out: no term is +0, one term is itself bit for bit (a signaling NaN
/// and -0 included), zeros keep their sign as IEEE 754 adds them, infinities of both signs
/// give a NaN, and three terms follow the rule where the sequential sum does not.
fn sums_the_edges_as_written<S: Raw>()
where
    S::Float: Float,
{
    let sum = |terms: &[u64]| {
        let samples: Vec<S> = terms
            .iter()
            .map(|&bits| S::from_raw(S::Float::from_bits(bits)))
            .collect();
        sum_of(&samples).bits()
    };
    let wide = S::Float::NEGATIVE_ZERO.bits() == 1_u64 << 63_u32;
    let (negative_zero, infinity, signaling, one) = if wide {
        (
            0x8000_0000_0000_0000,
            0x7ff0_0000_0000_0000,
            0x7ff0_0000_0000_0001,
            0x3ff0_0000_0000_0000,
        )
    } else {
        (0x8000_0000, 0x7f80_0000, 0x7f80_0001, 0x3f80_0000)
    };
    assert_eq!(sum(&[]), 0, "no term: +0");
    assert_eq!(sum(&[negative_zero]), negative_zero, "-0 alone");
    assert_eq!(
        sum(&[signaling]),
        signaling,
        "a signaling NaN alone, untouched"
    );
    for count in [2, 16, 17, 33] {
        assert_eq!(
            sum(&vec![negative_zero; count]),
            negative_zero,
            "{count} times -0"
        );
        let mut zeros = vec![negative_zero; count];
        if let Some(last) = zeros.last_mut() {
            *last = 0;
        }
        assert_eq!(sum(&zeros), 0, "{count} zeros, one of them +0");
    }
    assert!(
        is_nan::<S::Float>(sum(&[infinity, infinity | negative_zero])),
        "+inf + -inf"
    );
    assert_eq!(sum(&[infinity, one, one]), infinity, "+inf + 1 + 1");
    // 1, -1 and 2^-60: (1 + 2^-60) + -1 = 0 in the reduction order, while the sequential
    // (1 + -1) + 2^-60 is 2^-60.
    let tiny = S::from_binary64(Binary64::new(8.673_617_379_884_035e-19));
    let terms = [
        S::from_binary64(Binary64::ONE),
        S::from_binary64(Binary64::new(-1.0)),
        tiny,
    ];
    assert_eq!(sum_of(&terms).bits(), 0, "the third term goes first");
    assert_eq!(
        sequential(&terms.map(Raw::raw)).bits(),
        tiny.bits(),
        "where the sequential sum keeps it"
    );
}

/// The block form: silence for no block, the block itself for one (a NaN payload and -0
/// kept), and the rule frame by frame for more, at every count, each frame of each block
/// drawn on its own (a finite value, or one of the special values one time in four), each
/// frame checked against the reference as the helper is.
fn sums_blocks_frame_by_frame<S: Raw>()
where
    S::Float: Float,
{
    let specials = specials::<S::Float>();
    let mut draws = Draws(0x5059_0064);
    let draw = |source: &mut Draws| {
        let pick = source.next();
        if pick.trailing_zeros() >= 2 {
            let index = usize::try_from((pick >> 2_u32).wrapping_rem(12)).expect("below 12");
            S::from_raw(S::Float::from_bits(*specials.get(index).expect("below 12")))
        } else {
            S::from_raw(source.finite())
        }
    };
    for count in COUNTS {
        let blocks: Vec<Block<S>> = (0..count)
            .map(|_| core::array::from_fn(|_| draw(&mut draws)))
            .collect();
        let mut out = [S::from_raw(draws.finite()); BLOCK_SIZE];
        sum_blocks(&mut out, &blocks.iter());
        for (frame, sample) in out.iter().enumerate() {
            let terms: Vec<S::Float> = blocks
                .iter()
                .map(|block| block.get(frame).expect("a frame").raw())
                .collect();
            assert!(
                follows(&terms, sample.bits()),
                "{count} blocks, frame {frame}: {:#x}, expected {:#x}",
                sample.bits(),
                reference(&terms).bits()
            );
        }
    }
    let wide = S::Float::NEGATIVE_ZERO.bits() == 1_u64 << 63_u32;
    let signaling = if wide {
        0x7ff0_0000_0000_0001
    } else {
        0x7f80_0001
    };
    let mut odd = [S::from_raw(S::Float::NEGATIVE_ZERO); BLOCK_SIZE];
    if let Some(first) = odd.first_mut() {
        *first = S::from_raw(S::Float::from_bits(signaling));
    }
    let mut out = [S::ZERO; BLOCK_SIZE];
    sum_blocks(&mut out, &core::slice::from_ref(&odd).iter());
    assert_eq!(
        out.map(Raw::bits),
        odd.map(Raw::bits),
        "one block, bit for bit"
    );
    sum_blocks(&mut out, &core::slice::from_ref(&odd)[..0].iter());
    assert_eq!(out.map(Raw::bits), [0; BLOCK_SIZE], "no block: +0");
}

/// The reference itself, checked by hand: exact sums, the sum of no term and of -0 alone,
/// three terms where the order shows, and seventeen where term 16 must join lane 0 before
/// the tree.
#[test]
fn the_reference_follows_the_rule_on_worked_sums() {
    // 2^0 to 2^16 sum exactly, in any order, to 2^17 - 1.
    let powers: Vec<f64> = (0..17_u32)
        .map(|power| f64::from(1_u32.checked_shl(power).expect("below 2^17")))
        .collect();
    assert_eq!(
        reference(&powers).to_bits(),
        131_071.0_f64.to_bits(),
        "exact"
    );
    assert_eq!(reference::<f64>(&[]).to_bits(), 0, "no term: +0");
    assert_eq!(
        reference(&[-0.0_f64]).to_bits(),
        (-0.0_f64).to_bits(),
        "-0 alone"
    );
    // 2^-60: 1 + 2^-60 rounds to 1 in both precisions.
    let tiny = 8.673_617_379_884_035e-19_f64;
    // Lanes 0, 1, 2 hold 1, -1, 2^-60; the tree adds lane 2 to lane 0 first: (1 + 2^-60) - 1.
    assert_eq!(
        reference(&[1.0_f64, -1.0_f64, tiny]).to_bits(),
        0,
        "(1 + tiny) - 1"
    );
    assert_eq!(
        sequential(&[1.0_f64, -1.0_f64, tiny]).to_bits(),
        tiny.to_bits(),
        "(1 - 1) + tiny"
    );
    // Terms 0, 8 and 16 are 1, -1 and 2^-60, the others +0: lane 0 sums 1 and 2^-60 (1),
    // then lane 8 (-1) joins it: +0. Thirty-two lanes would keep 2^-60.
    let mut late = vec![0.0_f64; 17];
    for (index, value) in [(0_usize, 1.0_f64), (8, -1.0_f64), (16, tiny)] {
        *late.get_mut(index).expect("17 terms") = value;
    }
    assert_eq!(reference(&late).to_bits(), 0, "17 terms");
    assert!(is_nan::<f64>(f64::NAN.to_bits()), "a NaN");
    assert!(!is_nan::<f64>(f64::INFINITY.to_bits()), "not an infinity");
    assert!(
        is_nan::<f32>(u64::from(f32::NAN.to_bits())),
        "a binary32 NaN"
    );
    assert!(
        !is_nan::<f32>(u64::from(f32::INFINITY.to_bits())),
        "not a binary32 infinity"
    );
}

/// Runs each generic test in both precisions, each instantiation a test of its own.
macro_rules! in_both_precisions {
    ($($test:ident),* $(,)?) => {
        /// The tests in binary64.
        mod binary64 {
            $(
                #[test]
                fn $test() {
                    super::$test::<crate::Binary64>();
                }
            )*
        }
        /// The tests in binary32.
        mod binary32 {
            $(
                #[test]
                fn $test() {
                    super::$test::<crate::Binary32>();
                }
            )*
        }
    };
}

in_both_precisions!(
    sums_finite_terms_in_the_specified_order,
    sums_special_values_in_the_specified_order,
    sums_the_edges_as_written,
    sums_blocks_frame_by_frame,
);
