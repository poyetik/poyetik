//! Tests of the reduction order (`spec/engine-design.md`, decision 28): the helper against a
//! reference written literally from the rule, on raw floats, apart from the helper.

use crate::tests::in_both_precisions;
use alloc::format;
use alloc::vec;
use alloc::vec::Vec;
use core::ops::Add;

use super::{Lanes, add_two, ordered_sum, sum_blocks};

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
/// the default NaN when no term is a NaN (an infinity minus an infinity): what Rust gives
/// natively, whichever operand of an addition the compiler puts first (the payload chosen
/// may change between a debug and a release build). Its sign is not compared: it may change
/// in the same way, and WebAssembly leaves it open, where any quiet NaN may come from a NaN
/// term that is not canonical: a run of these tests in WebAssembly would accept any quiet
/// NaN there (roadmap #24).
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

/// The deterministic generator of 64-bit draws of this crate's tests (`SplitMix64`), for
/// reproducible terms.
use super::tests::proptest_like::Draws;

impl Draws {
    /// A finite value of either sign whose magnitude lies in [2^-20, 2^21) (2^21 itself
    /// possible once rounded to binary32), so that sums of thousands of terms round at every
    /// step and never overflow.
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

/// The most roundings any term goes through in a sum of `count` terms in the reduction
/// order: the additions of its lane after its first term, at most `⌈count / 16⌉ - 1`, then one
/// per round of the tree that adds a lane to it, `⌈log2 min(count, 16)⌉`.
fn roundings(count: usize) -> u32 {
    let per_lane = count.div_ceil(16).saturating_sub(1);
    let lanes = count.clamp(1, 16);
    u32::try_from(per_lane)
        .expect("small")
        .saturating_add(lanes.next_power_of_two().trailing_zeros())
}

/// `value`, a finite float that is a multiple of 2^-72 (every value of [`Draws::finite`]
/// and every sum of them is: binary64 values from 2^-20 have an ulp of at least 2^-72,
/// binary32 ones of 2^-43; so is `3·2^-55`, whose ulp is smaller but whose lowest set bit is
/// 2^-55), as a whole number of 2^-72: exact.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "fields of one float: the exponent and shift stay far from overflow"
)]
fn fixed<F: Float>(value: F) -> i128 {
    let bits = value.bits();
    let wide = F::NEGATIVE_ZERO.bits() == 1_u64 << 63_u32;
    let (fraction_bits, offset, exponent_mask, sign_bit) = if wide {
        (52_u32, 1075_i64, 0x7ff_u64, 63_u32)
    } else {
        (23_u32, 150_i64, 0xff_u64, 31_u32)
    };
    let exponent = i64::try_from((bits >> fraction_bits) & exponent_mask).expect("small");
    let fraction = bits & ((1_u64 << fraction_bits) - 1);
    if exponent == 0 && fraction == 0 {
        return 0;
    }
    assert_ne!(exponent, 0, "no subnormal among these sums");
    let mantissa = i128::from(fraction | 1_u64 << fraction_bits);
    // value = mantissa · 2^(exponent - offset); in units of 2^-72: shift by
    // exponent - offset + 72.
    let shift = exponent - offset + 72;
    let magnitude = if shift >= 0 {
        mantissa << u32::try_from(shift).expect("non-negative")
    } else {
        let dropped = u32::try_from(-shift).expect("positive");
        assert_eq!(
            mantissa & ((1_i128 << dropped) - 1),
            0,
            "a multiple of 2^-72: {bits:#x}"
        );
        mantissa >> dropped
    };
    if bits >> sign_bit == 1 {
        -magnitude
    } else {
        magnitude
    }
}

/// The accuracy of the reduction order (decision 28), against the exact sum: at every count,
/// on finite terms between 2^-20 and 2^21 (a finite sum: the bound does not hold when it
/// overflows), `|sum - exact| ≤ γ_h · Σ|x_k|`, where `h` is
/// [`roundings`] and `γ_h = h·u / (1 - h·u)` with `u` the unit roundoff of the precision
/// (2^-53, 2^-24): the standard bound of a sum in which each term goes through at most `h`
/// roundings (Higham, *Accuracy and Stability of Numerical Algorithms*, 4.2). The exact sum
/// and the error are whole numbers of 2^-72 in `i128` (4099 terms below 2^21 need 105 bits);
/// the bound is evaluated in binary64, five operations each within 2^-53 of its value, so it
/// is compared with a margin of 2^-49 of itself. It guards the accuracy of the rule (a
/// change of the rule toward a numerically poorer one, decision 28 and the reference
/// together), which the comparison with the literal reference cannot see. Random draws of
/// both signs stay far inside the worst case (a sequential sum of them passes too), so the
/// guard is an adversarial sum: 4099 terms, 1 then 4098 times `3u/4`, each of which a
/// running sum near 1 rounds away. The rule keeps it within its bound, and the sequential
/// sum, whose `n - 1` roundings each lose almost a whole term, is shown to break it: a
/// rule as poor as the sequential one would fail here. The draws' errors are also shown not
/// to be zero: the test does see rounding.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "binary64 arithmetic of the bound and i128 sums of at most 4099 terms below 2^93"
)]
fn sums_within_the_accuracy_bound_of_the_order<S: Raw>()
where
    S::Float: Float,
{
    let wide = S::Float::NEGATIVE_ZERO.bits() == 1_u64 << 63_u32;
    let unit = libm::exp2(if wide { -53.0_f64 } else { -24.0_f64 });
    let margin = 1.0_f64 + libm::exp2(-49.0_f64);
    #[expect(
        clippy::cast_precision_loss,
        clippy::as_conversions,
        reason = "an i128 below 2^106, within 2^-53 of itself"
    )]
    let float = |value: i128| value as f64;
    let bound = |count: usize, magnitudes: i128| {
        let roundings_of = f64::from(roundings(count));
        roundings_of * unit / (1.0_f64 - roundings_of * unit) * float(magnitudes) * margin
    };
    let error = |summed: S::Float, exact: i128| float((fixed(summed) - exact).abs());
    let mut draws = Draws(0x5059_0031);
    let mut rounded = false;
    for count in COUNTS {
        for _ in 0_u32..16 {
            let terms: Vec<S::Float> = (0..count).map(|_| draws.finite()).collect();
            let exact: i128 = terms.iter().map(|&term| fixed(term)).sum();
            let magnitudes: i128 = terms.iter().map(|&term| fixed(term).abs()).sum();
            let samples: Vec<S> = terms.iter().map(|&term| S::from_raw(term)).collect();
            let summed = sum_of(&samples).raw();
            assert!(
                error(summed, exact) <= bound(count, magnitudes),
                "{count} terms: error {} beyond {}",
                error(summed, exact),
                bound(count, magnitudes)
            );
            rounded |= error(summed, exact) > 0.0_f64;
        }
    }
    assert!(rounded, "some sum rounds");
    // 1, then 4098 times 3u/4 (3·2^-55 in binary64, 3·2^-26 in binary32): multiples of 2^-72.
    let (one, three_quarters_of_u) = if wide {
        (0x3ff0_0000_0000_0000_u64, 0x3c98_0000_0000_0000_u64)
    } else {
        (0x3f80_0000_u64, 0x3340_0000_u64)
    };
    let adversarial: Vec<S::Float> = core::iter::once(one)
        .chain(core::iter::repeat_n(three_quarters_of_u, 4098))
        .map(S::Float::from_bits)
        .collect();
    let exact: i128 = adversarial.iter().map(|&term| fixed(term)).sum();
    let magnitudes: i128 = adversarial.iter().map(|&term| fixed(term).abs()).sum();
    assert_eq!(
        exact,
        fixed(S::Float::from_bits(one)) + 4098 * fixed(S::Float::from_bits(three_quarters_of_u)),
        "the terms are exact in units of 2^-72"
    );
    let samples: Vec<S> = adversarial.iter().map(|&term| S::from_raw(term)).collect();
    let summed = sum_of(&samples).raw();
    assert!(
        error(summed, exact) <= bound(4099, magnitudes),
        "the adversarial sum: error {} beyond {}",
        error(summed, exact),
        bound(4099, magnitudes)
    );
    let poorer = sequential(&adversarial);
    assert!(
        error(poorer, exact) > 4.0_f64 * bound(4099, magnitudes),
        "the sequential sum breaks the bound: error {} within 4 times {}",
        error(poorer, exact),
        bound(4099, magnitudes)
    );
}

#[test]
fn counts_the_roundings_of_each_regime() {
    // The counts decision 28 states with its bound: 2, 16, 64 and 4099 terms.
    assert_eq!(
        [0, 1, 2, 3, 16, 17, 64, 4099].map(roundings),
        [0, 0, 1, 2, 4, 5, 7, 260],
        "per lane, then the tree"
    );
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
    // One set of scratch lanes for every sum, as a plan keeps one: nothing left in them by a
    // sum may reach the next. The counts run downward, then upward: every count but the
    // largest follows a larger sum, which left nonzero values in more lanes than it uses.
    let mut lanes = Lanes::new();
    let draw = |source: &mut Draws| {
        let pick = source.next();
        if pick.trailing_zeros() >= 2 {
            let index = usize::try_from((pick >> 2_u32).wrapping_rem(12)).expect("below 12");
            S::from_raw(S::Float::from_bits(*specials.get(index).expect("below 12")))
        } else {
            S::from_raw(source.finite())
        }
    };
    for count in COUNTS.iter().rev().chain(&COUNTS).copied() {
        let blocks: Vec<Block<S>> = (0..count)
            .map(|_| core::array::from_fn(|_| draw(&mut draws)))
            .collect();
        let mut out = [S::from_raw(draws.finite()); BLOCK_SIZE];
        sum_blocks(&mut out, &mut lanes, blocks.iter());
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
    sum_blocks(&mut out, &mut lanes, core::slice::from_ref(&odd).iter());
    assert_eq!(
        out.map(Raw::bits),
        odd.map(Raw::bits),
        "one block, bit for bit"
    );
    sum_blocks(
        &mut out,
        &mut lanes,
        core::slice::from_ref(&odd)[..0].iter(),
    );
    assert_eq!(out.map(Raw::bits), [0; BLOCK_SIZE], "no block: +0");
}

/// The two-term form, which the plan uses without lanes: frame by frame the rule for two
/// terms (the first plus the second), every pair of special values included (each of the
/// twelve against each, in both orders), then drawn pairs, against the reference and against
/// the block form of the rule.
fn adds_two_blocks_by_the_rule_of_two_terms<S: Raw>()
where
    S::Float: Float,
{
    let specials = specials::<S::Float>().map(|bits| S::from_raw(S::Float::from_bits(bits)));
    let mut pairs: Vec<(S, S)> = specials
        .iter()
        .flat_map(|&first| specials.iter().map(move |&second| (first, second)))
        .collect();
    let mut draws = Draws(0x5059_0002);
    pairs.extend(
        (0..2 * BLOCK_SIZE).map(|_| (S::from_raw(draws.finite()), S::from_raw(draws.finite()))),
    );
    let mut lanes = Lanes::new();
    for chunk in pairs.chunks(BLOCK_SIZE) {
        let mut first = [S::ZERO; BLOCK_SIZE];
        let mut second = [S::ZERO; BLOCK_SIZE];
        for ((one, other), &(left, right)) in first.iter_mut().zip(second.iter_mut()).zip(chunk) {
            *one = left;
            *other = right;
        }
        let mut out = [S::from_raw(draws.finite()); BLOCK_SIZE];
        add_two(&mut out, &first, &second);
        let mut summed = [S::ZERO; BLOCK_SIZE];
        sum_blocks(&mut summed, &mut lanes, [&first, &second].into_iter());
        for (frame, ((sample, sum), (one, other))) in out
            .iter()
            .zip(&summed)
            .zip(first.iter().zip(&second))
            .enumerate()
        {
            assert!(
                follows(&[one.raw(), other.raw()], sample.bits()),
                "frame {frame}: {:#x} + {:#x} gave {:#x}",
                one.bits(),
                other.bits(),
                sample.bits()
            );
            assert!(
                sample.bits() == sum.bits()
                    || (is_nan::<S::Float>(sample.bits()) && is_nan::<S::Float>(sum.bits())),
                "frame {frame}: {:#x}, the block form {:#x}",
                sample.bits(),
                sum.bits()
            );
        }
    }
    assert_eq!(
        pairs.len(),
        12 * 12 + 2 * BLOCK_SIZE,
        "every pair of specials, and draws"
    );
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

in_both_precisions!(
    sums_finite_terms_in_the_specified_order,
    sums_special_values_in_the_specified_order,
    sums_the_edges_as_written,
    sums_blocks_frame_by_frame,
    adds_two_blocks_by_the_rule_of_two_terms,
    sums_within_the_accuracy_bound_of_the_order,
);
