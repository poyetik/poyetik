//! Tests of the math layer.

use alloc::boxed::Box;
use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use super::to_binary32;
use crate::host::{Delivered, Raw};
use crate::{AlgorithmVersion, ObjectSpec, ParamSpec, ParamValues, Params};
use crate::{
    ArithmeticMode, ArithmeticOption, BLOCK_SIZE, Binary32, Binary64, Block, Create, Factory, MODE,
    Object, Precision, RenderFormat, Sample, SampleRate, UnknownArithmeticOptions,
};

/// The nearest binary32 to the binary64 `bits`, worked out in integers from IEEE 754 alone
/// (round to nearest, ties to even; NaN by the rule of [`to_binary32`]): a reference
/// independent of the conversion under test.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "a test reference on the fields of one binary64 value: every exponent and count is bounded by the format, far from overflow"
)]
fn reference(bits: u64) -> u32 {
    let sign = u32::try_from(bits >> 63_u32).expect("one bit") << 31_u32;
    let exponent = i64::try_from((bits >> 52_u32) & 0x7ff).expect("11 bits");
    let fraction = bits & 0x000f_ffff_ffff_ffff;
    if exponent == 0x7ff {
        return if fraction == 0 {
            sign | 0x7f80_0000
        } else {
            sign | 0x7fc0_0000 | u32::try_from((fraction >> 29_u32) & 0x003f_ffff).expect("22 bits")
        };
    }
    if exponent == 0 && fraction == 0 {
        return sign;
    }
    // The magnitude is m · 2^p, m a whole number below 2^53.
    let (mantissa, power) = if exponent == 0 {
        (fraction, -1074_i64)
    } else {
        (fraction | 1_u64 << 52_u32, exponent - 1075)
    };
    // Its leading power of two, and the spacing of binary32 values there: 2^(top - 23),
    // never below 2^-149, the spacing of subnormals.
    let top = 63 - i64::from(mantissa.leading_zeros()) + power;
    let quantum = (top - 23).max(-149);
    // The magnitude in units of that spacing, rounded to nearest, ties to even.
    let shift = quantum - power;
    let units = if shift <= 0 {
        mantissa << u32::try_from(-shift).expect("small")
    } else if shift >= 64 {
        0
    } else {
        let shift = u32::try_from(shift).expect("below 64");
        let (kept, rest, half) = (
            mantissa >> shift,
            mantissa & ((1_u64 << shift) - 1),
            1_u64 << (shift - 1),
        );
        if rest > half || (rest == half && kept & 1 == 1) {
            kept + 1
        } else {
            kept
        }
    };
    // `units · 2^quantum` encoded: a subnormal below 2^23 units at the smallest spacing,
    // else a normal (2^24 units after rounding up is 2^23 units of the next spacing).
    let (units, quantum) = if units == 1_u64 << 24_u32 {
        (1_u64 << 23_u32, quantum + 1)
    } else {
        (units, quantum)
    };
    if units < 1_u64 << 23_u32 {
        return sign | u32::try_from(units).expect("below 2^23");
    }
    let biased = quantum + 150;
    if biased >= 0xff {
        return sign | 0x7f80_0000;
    }
    sign | u32::try_from(biased).expect("8 bits") << 23_u32
        | u32::try_from(units - (1_u64 << 23_u32)).expect("23 bits")
}

#[test]
fn the_reference_rounds_like_ieee_754_on_worked_values() {
    // Values worked out by hand, so that the reference itself is checked.
    let cases: [(u64, u32); 9] = [
        (1.0_f64.to_bits(), 0x3f80_0000),
        (0.1_f64.to_bits(), 0x3dcc_cccd),
        // 1 + 2^-24, exactly halfway: ties to even, 1.
        (0x3ff0_0000_1000_0000, 0x3f80_0000),
        // 1 + 3 · 2^-24, halfway above an odd significand: up.
        (0x3ff0_0000_3000_0000, 0x3f80_0002),
        // 2^-149, the smallest subnormal binary32.
        (0x36a0_0000_0000_0000, 0x0000_0001),
        // 2^-150, halfway to zero: ties to even, +0.
        (0x3690_0000_0000_0000, 0x0000_0000),
        // The largest binary32, and the next binary64 above its halfway point: infinity.
        (0x47ef_ffff_e000_0000, 0x7f7f_ffff),
        (0x47ef_ffff_f000_0000, 0x7f80_0000),
        // A negative quiet NaN with a payload: its top 22 bits.
        (0xfff8_0000_2000_0001, 0xffc0_0001),
    ];
    for (bits, expected) in cases {
        assert_eq!(reference(bits), expected, "{bits:#018x}");
    }
}

#[test]
fn rounds_binary64_to_the_nearest_binary32() {
    let mut runner = splitmix::Draws::new();
    for _ in 0_u32..200_000_u32 {
        let bits = runner.next();
        let value = f64::from_bits(bits);
        assert_eq!(
            to_binary32(value).to_bits(),
            reference(bits),
            "{bits:#018x}"
        );
    }
    // Every binary32 exponent, with the halfway points around each: the boundaries of
    // rounding, overflow and the subnormal range.
    for exponent in 0_u64..0x800 {
        for fraction in [
            0_u64,
            1,
            0x0000_0000_1000_0000,
            0x0000_0000_0fff_ffff,
            0x0000_0000_1000_0001,
            0x0000_0000_3000_0000,
            0x000f_ffff_f000_0000,
            0x000f_ffff_ffff_ffff,
            0x0008_0000_0000_0000,
        ] {
            for sign in [0_u64, 1 << 63_u32] {
                let bits = sign | exponent << 52_u32 | fraction;
                assert_eq!(
                    to_binary32(f64::from_bits(bits)).to_bits(),
                    reference(bits),
                    "{bits:#018x}"
                );
            }
        }
    }
}

/// Deterministic draws of 64-bit patterns, without a dependency: `SplitMix64` (Steele, Lea
/// and Flood, 2014), as `spec/measurements.md` defines it. The one copy of this crate's
/// tests (the reduction tests draw from it too); `audio-core` cannot use the battery's
/// (`audio_battery::splitmix64`), which depends on it.
pub(super) mod splitmix {
    /// The state of the generator.
    pub(in super::super) struct Draws(pub(in super::super) u64);

    impl Draws {
        /// A generator from a fixed seed.
        pub(in super::super) const fn new() -> Self {
            Self(0x5059_0006)
        }

        /// The next 64 bits.
        pub(in super::super) const fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut mixed = self.0;
            mixed = (mixed ^ (mixed >> 30_u32)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            mixed = (mixed ^ (mixed >> 27_u32)).wrapping_mul(0x94d0_49bb_1331_11eb);
            mixed ^ (mixed >> 31_u32)
        }
    }
}

#[test]
fn draws_splitmix64_from_its_published_values() {
    // SplitMix64's first outputs from the state 0, computed apart from this code.
    let mut draws = splitmix::Draws(0);
    assert_eq!(
        [draws.next(), draws.next()],
        [0xe220_a839_7b1d_cdaf, 0x6e78_9e6a_a1b9_65f4]
    );
}

#[test]
fn names_the_precisions_and_the_render_formats() {
    assert_eq!(
        Precision::default(),
        Precision::Binary64,
        "binary64 by default"
    );
    assert_eq!(
        Precision::ALL,
        [Precision::Binary64, Precision::Binary32],
        "both"
    );
    assert_eq!(
        Precision::ALL.map(Precision::bits),
        [64_u8, 32_u8],
        "their widths"
    );
    let described: Vec<(&str, u8, u8)> = RenderFormat::ALL
        .iter()
        .map(|format| {
            (
                format.name(),
                format.precision().bits(),
                format.sample_bits(),
            )
        })
        .collect();
    assert_eq!(
        described,
        [
            ("binary64", 64, 64),
            ("binary64-as-binary32", 64, 32),
            ("binary32", 32, 32)
        ],
        "every render format: its name, precision and sample width"
    );
    assert_eq!(
        format!("{}", RenderFormat::Binary64AsBinary32),
        "binary64-as-binary32",
        "shown by its name"
    );
}

#[test]
fn defines_the_arithmetic_options_by_their_flags() {
    assert_eq!(
        ArithmeticOption::ALL.map(ArithmeticOption::flag),
        [1_u64, 2, 4],
        "fused multiply-add, reassociable operations, platform math functions"
    );
    assert_eq!(ArithmeticMode::STRICT.bits(), 0, "all off");
    assert_eq!(MODE.bits(), BUILT_MODE, "the mode of this build");
}

/// The flags of the mode this build was made in, written apart from the code under test:
/// the strict mode, but in xtask's fused test build.
#[cfg(not(poyetik_math = "fused"))]
const BUILT_MODE: u64 = 0;

/// The flags of the mode this build was made in: fused multiply-add, in xtask's fused
/// test build.
#[cfg(poyetik_math = "fused")]
const BUILT_MODE: u64 = 1;

/// The proof xtask's fused test build reads in its output: this test exists, and passes,
/// only when the option is in force.
#[cfg(poyetik_math = "fused")]
#[test]
fn is_built_with_the_fused_multiply_add_option() {
    assert_eq!(MODE.bits(), 1, "fused multiply-add");
}

#[test]
fn reads_every_known_mode_and_refuses_every_unknown_flag() {
    for bits in 0_u64..8 {
        assert_eq!(
            ArithmeticMode::from_bits(bits).map(ArithmeticMode::bits),
            Ok(bits),
            "{bits:#b}"
        );
    }
    for bit in 3_u32..64 {
        for known in [0_u64, 7] {
            let unknown = 1_u64 << bit;
            assert_eq!(
                ArithmeticMode::from_bits(known | unknown).map_err(UnknownArithmeticOptions::bits),
                Err(unknown),
                "bit {bit}, with {known:#b}: the unknown bits only"
            );
        }
    }
    let refused = ArithmeticMode::from_bits(9).expect_err("flag 3 is unknown");
    assert_eq!(
        format!("{refused}"),
        "unknown arithmetic options: 0x0000000000000008",
        "explained, the known flag left out"
    );
}

#[test]
fn reports_unknown_options_as_an_error() {
    /// Compiles only for an error type.
    const fn error<E: core::error::Error>() {}
    error::<UnknownArithmeticOptions>();
}

/// Shorthand for a binary64 sample.
fn wide(value: f64) -> Binary64 {
    Binary64::from_raw(value)
}

/// Shorthand for a binary32 sample.
fn narrow(value: f32) -> Binary32 {
    Binary32::from_raw(value)
}

#[test]
fn computes_each_precision_with_the_pinned_libm() {
    // Known values first, so that the function is the right one, then the delegation.
    assert_eq!(wide(0.0).sin().bits(), 0, "sin 0");
    assert_eq!(narrow(-0.0).sin().bits(), 0x8000_0000, "sin -0");
    assert_eq!(
        wide(core::f64::consts::FRAC_PI_2).sin().raw().to_bits(),
        1.0_f64.to_bits(),
        "sin π/2"
    );
    assert_eq!(
        narrow(core::f32::consts::FRAC_PI_2).sin().raw().to_bits(),
        1.0_f32.to_bits(),
        "sinf π/2"
    );
    for value in [0.5_f64, 1.0_f64, 3.0_f64, 6.25_f64, -2.5_f64, 1e-300_f64] {
        assert_eq!(
            wide(value).sin().bits(),
            libm::sin(value).to_bits(),
            "{value}"
        );
        let rounded = to_binary32(value);
        assert_eq!(
            narrow(rounded).sin().bits(),
            u64::from(libm::sinf(rounded).to_bits()),
            "{rounded}"
        );
    }
    assert_ne!(
        narrow(1.0).sin().to_binary64().bits(),
        wide(1.0).sin().bits(),
        "each in its own precision"
    );
    for (value, floor) in [
        (2.5_f64, 2.0_f64),
        (-0.5_f64, -1.0_f64),
        (-0.0_f64, -0.0_f64),
        (7.0_f64, 7.0_f64),
    ] {
        assert_eq!(wide(value).floor().bits(), floor.to_bits(), "{value}");
        assert_eq!(
            narrow(to_binary32(value)).floor().bits(),
            u64::from(to_binary32(floor).to_bits()),
            "{value}"
        );
    }
}

#[test]
fn operates_and_compares_as_ieee_754() {
    // Each operator is the float's, correctly rounded: worked values, signed zeros.
    assert_eq!(
        (wide(0.1) + wide(0.2)).bits(),
        0x3fd3_3333_3333_3334,
        "0.1 + 0.2"
    );
    assert_eq!(
        (narrow(0.1) + narrow(0.2)).bits(),
        0x3e99_999a,
        "0.1 + 0.2 in binary32"
    );
    assert_eq!((wide(1.0) - wide(1.0)).bits(), 0, "+0");
    assert_eq!(
        (narrow(1.5) - narrow(2.0)).raw().to_bits(),
        (-0.5_f32).to_bits(),
        "-0.5"
    );
    assert_eq!(
        (wide(3.0) * wide(0.5)).raw().to_bits(),
        1.5_f64.to_bits(),
        "1.5"
    );
    assert_eq!((narrow(-2.0) * narrow(0.0)).bits(), 0x8000_0000, "-0");
    assert_eq!((wide(1.0) / wide(3.0)).bits(), 0x3fd5_5555_5555_5555, "1/3");
    assert_eq!(
        (narrow(1.0) / narrow(3.0)).bits(),
        0x3eaa_aaab,
        "1/3 in binary32"
    );
    assert_eq!((-wide(0.0)).bits(), 1 << 63_u32, "-(+0)");
    assert_eq!((-narrow(2.0)).raw().to_bits(), (-2.0_f32).to_bits(), "-2");
    let mut sum = wide(1.0);
    sum += wide(0.5);
    sum -= wide(0.25);
    assert_eq!(sum.raw().to_bits(), 1.25_f64.to_bits(), "+= and -=");
    let mut narrow_sum = narrow(1.0);
    narrow_sum += narrow(0.5);
    narrow_sum -= narrow(2.0);
    assert_eq!(
        narrow_sum.raw().to_bits(),
        (-0.5_f32).to_bits(),
        "+= and -= in binary32"
    );
    assert!(
        wide(1.0) < wide(2.0) && narrow(-1.0) < narrow(0.0),
        "ordered"
    );
    assert!(wide(f64::NAN) != wide(f64::NAN), "NaN is unordered");
    assert_eq!(wide(0.0), wide(-0.0), "zeros compare equal");
    assert_eq!(Binary64::TAU.raw().to_bits(), 0x4019_21fb_5444_2d18, "2π");
    assert_eq!(Binary64::ONE.raw().to_bits(), 1.0_f64.to_bits(), "1");
    assert_eq!(
        Binary64::new(0.25).raw().to_bits(),
        0.25_f64.to_bits(),
        "a literal"
    );
    assert_eq!(
        Binary64::from(SampleRate::new(44_100).expect("valid rate"))
            .raw()
            .to_bits(),
        44_100.0_f64.to_bits(),
        "a rate, exactly"
    );
}

/// `a × a - 1` for `a = 1 + 2^-27`, the product rounded then the sum: `a²` is
/// `1 + 2^-26 + 2^-54`, whose last term is a quarter of an ulp of 1, so the product rounds to
/// `1 + 2^-26` and the result is 2^-26.
const TWO_ROUNDINGS_64: u64 = 0x3e50_0000_0000_0000;

/// The same with one rounding (fused): 2^-26 + 2^-54, exact.
const ONE_ROUNDING_64: u64 = 0x3e50_0000_0100_0000;

/// The binary32 counterpart, `a = 1 + 2^-12`: `a²` is `1 + 2^-11 + 2^-24`, its last term
/// half an ulp of 1, rounding to the even `1 + 2^-11`: 2^-11 in two roundings.
const TWO_ROUNDINGS_32: u32 = 0x3a00_0000;

/// The fused binary32 result: 2^-11 + 2^-24, exact.
const ONE_ROUNDING_32: u32 = 0x3a00_0400;

/// The binary64 `mul_then_add` this build must give.
#[cfg(not(poyetik_math = "fused"))]
const MUL_THEN_ADD_64: u64 = TWO_ROUNDINGS_64;

/// The binary64 `mul_then_add` of the fused test build.
#[cfg(poyetik_math = "fused")]
const MUL_THEN_ADD_64: u64 = ONE_ROUNDING_64;

/// The binary32 `mul_then_add` this build must give.
#[cfg(not(poyetik_math = "fused"))]
const MUL_THEN_ADD_32: u32 = TWO_ROUNDINGS_32;

/// The binary32 `mul_then_add` of the fused test build.
#[cfg(poyetik_math = "fused")]
const MUL_THEN_ADD_32: u32 = ONE_ROUNDING_32;

#[test]
fn multiplies_then_adds_in_the_mode_of_the_build() {
    // Inputs where one rounding and two differ, the two worked out by hand above.
    let near_one = wide(f64::from_bits(0x3ff0_0000_0200_0000)); // 1 + 2^-27
    assert_eq!(
        near_one.mul_then_add(near_one, wide(-1.0)).bits(),
        MUL_THEN_ADD_64,
        "binary64"
    );
    assert_eq!(
        (near_one * near_one + wide(-1.0)).bits(),
        TWO_ROUNDINGS_64,
        "the operators round twice in every build"
    );
    assert_ne!(
        TWO_ROUNDINGS_64, ONE_ROUNDING_64,
        "the inputs tell the modes apart"
    );
    let narrow_one = narrow(f32::from_bits(0x3f80_0800)); // 1 + 2^-12
    assert_eq!(
        narrow_one.mul_then_add(narrow_one, narrow(-1.0)).bits(),
        u64::from(MUL_THEN_ADD_32),
        "binary32"
    );
    assert_ne!(
        TWO_ROUNDINGS_32, ONE_ROUNDING_32,
        "the inputs tell the modes apart"
    );
    // Special values: signed zeros and infinities follow IEEE 754 in either mode.
    assert_eq!(
        wide(-0.0).mul_then_add(wide(1.0), wide(-0.0)).bits(),
        1 << 63_u32,
        "-0 × 1 + -0 = -0"
    );
    assert_eq!(
        narrow(f32::INFINITY)
            .mul_then_add(narrow(2.0), narrow(1.0))
            .raw(),
        f32::INFINITY,
        "infinity"
    );
}

#[test]
fn converts_and_flushes_in_each_precision() {
    assert_eq!(
        Binary64::from_binary64(wide(0.1)).bits(),
        0.1_f64.to_bits(),
        "exact"
    );
    assert_eq!(
        Binary32::from_binary64(wide(0.1)).bits(),
        0x3dcc_cccd,
        "rounded"
    );
    assert_eq!(
        narrow(f32::from_bits(1)).to_binary64().bits(),
        0x36a0_0000_0000_0000,
        "2^-149, exactly"
    );
    assert_eq!(
        wide(-0.5).to_binary64().bits(),
        (-0.5_f64).to_bits(),
        "itself"
    );
    // A binary32 NaN widened from its bits, the same on every target: its sign, the quiet
    // bit set, its 22 payload bits moved up by 29.
    for (narrow_bits, wide_bits, what) in [
        (
            0x7f80_0001_u32,
            0x7ff8_0000_2000_0000_u64,
            "a signaling NaN, quieted",
        ),
        (0xffc0_0005, 0xfff8_0000_a000_0000, "a negative quiet NaN"),
        (0x7fff_ffff, 0x7fff_ffff_e000_0000, "the largest payload"),
        (0x7fc0_0000, 0x7ff8_0000_0000_0000, "the default NaN"),
    ] {
        let widened = narrow(f32::from_bits(narrow_bits)).to_binary64();
        assert_eq!(widened.bits(), wide_bits, "{what}");
        assert_eq!(
            Binary32::from_binary64(widened).bits(),
            u64::from(narrow_bits | 0x0040_0000),
            "{what}: back to binary32, quieted"
        );
    }
    assert_eq!(
        narrow(f32::NEG_INFINITY).to_binary64().bits(),
        0xfff0_0000_0000_0000,
        "an infinity is a number"
    );
    assert_eq!(
        wide(f64::from_bits(1)).flush().bits(),
        0,
        "binary64 subnormal"
    );
    assert_eq!(
        wide(-f64::MIN_POSITIVE).flush().raw().to_bits(),
        (-f64::MIN_POSITIVE).to_bits(),
        "binary64 normal"
    );
    // 2^-1000 is normal in binary64 and below every binary32: a binary64 flush keeps it.
    let below_binary32 = f64::from_bits(0x0170_0000_0000_0000);
    assert_eq!(
        wide(below_binary32).flush().raw().to_bits(),
        below_binary32.to_bits(),
        "kept"
    );
    assert_eq!(
        narrow(-f32::from_bits(1)).flush().bits(),
        0x8000_0000,
        "binary32 subnormal"
    );
    assert_eq!(
        narrow(f32::MIN_POSITIVE).flush().raw().to_bits(),
        f32::MIN_POSITIVE.to_bits(),
        "binary32 normal"
    );
    assert_eq!(Binary64::PRECISION, Precision::Binary64, "binary64");
    assert_eq!(Binary32::PRECISION, Precision::Binary32, "binary32");
    assert!(
        Binary64::SILENCE.iter().all(|sample| sample.bits() == 0),
        "+0"
    );
    assert!(
        Binary32::SILENCE.iter().all(|sample| sample.bits() == 0),
        "+0"
    );
}

#[test]
fn delivers_a_render_as_it_is_or_rounded_to_binary32() {
    let nan = f64::from_bits(0x7ff4_0000_0000_0001);
    assert_eq!(
        <f64 as Delivered<Binary64>>::deliver(wide(nan)).to_bits(),
        nan.to_bits(),
        "binary64 as it is"
    );
    assert_eq!(
        <f32 as Delivered<Binary32>>::deliver(narrow(f32::from_bits(0xffa0_0002))).to_bits(),
        0xffa0_0002,
        "binary32 as it is"
    );
    assert_eq!(
        <f32 as Delivered<Binary64>>::deliver(wide(0.1)).to_bits(),
        0x3dcc_cccd,
        "binary64 rounded"
    );
    assert_eq!(
        <f32 as Delivered<Binary64>>::deliver(wide(nan)).to_bits(),
        0x7fe0_0000,
        "a NaN by its bits"
    );
}

#[test]
fn delivers_no_subnormal_in_any_format() {
    // Decision 11: delivery is a flush point, decided after the rounding to binary32, each
    // subnormal given as a zero of its sign; the smallest normal value of each format kept.
    let binary64 =
        |bits: u64| <f64 as Delivered<Binary64>>::deliver(wide(f64::from_bits(bits))).to_bits();
    assert_eq!(
        [
            0x0000_0000_0000_0001,
            0x800f_ffff_ffff_ffff,
            0x0010_0000_0000_0000,
            0x8010_0000_0000_0000,
        ]
        .map(binary64),
        [
            0,
            0x8000_0000_0000_0000,
            0x0010_0000_0000_0000,
            0x8010_0000_0000_0000
        ],
        "binary64: subnormals flushed, the smallest normals kept"
    );
    let binary32 =
        |bits: u32| <f32 as Delivered<Binary32>>::deliver(narrow(f32::from_bits(bits))).to_bits();
    assert_eq!(
        [0x0000_0001, 0x807f_ffff, 0x0080_0000, 0x8080_0000].map(binary32),
        [0, 0x8000_0000, 0x0080_0000, 0x8080_0000],
        "binary32: subnormals flushed, the smallest normals kept"
    );
    let rounded =
        |bits: u64| <f32 as Delivered<Binary64>>::deliver(wide(f64::from_bits(bits))).to_bits();
    assert_eq!(
        [
            // The largest binary32 subnormal, exact in binary64, and 0.75 · 2^-126.
            0x380f_ffff_c000_0000,
            0xb808_0000_0000_0000,
            // Just below the midpoint between the largest binary32 subnormal and 2^-126:
            // rounds down to the subnormal, flushed.
            0x380f_ffff_dfff_ffff,
            // The midpoint, a tie to the even 2^-126, and the largest binary64 below
            // 2^-126: both round up to 2^-126, a normal value, kept.
            0x380f_ffff_e000_0000,
            0xb80f_ffff_ffff_ffff,
            // 2^-143, normal in binary64, a binary32 subnormal once rounded: flushed.
            0x3700_0000_0000_0000,
        ]
        .map(rounded),
        [0, 0x8000_0000, 0, 0x0080_0000, 0x8080_0000, 0],
        "binary64 given as binary32: flushed after the rounding"
    );
    // A sample known not to be subnormal in its precision (a channel that is no sum) is
    // given as it is in the same precision, so a subnormal there would pass (the box law
    // keeps it out); rounded to binary32, it is still flushed.
    assert_eq!(
        <f64 as Delivered<Binary64>>::deliver_unsummed(wide(f64::from_bits(1))).to_bits(),
        1,
        "binary64 as it is"
    );
    assert_eq!(
        <f32 as Delivered<Binary32>>::deliver_unsummed(narrow(f32::from_bits(1))).to_bits(),
        1,
        "binary32 as it is"
    );
    assert_eq!(
        <f32 as Delivered<Binary64>>::deliver_unsummed(wide(f64::from_bits(0xb808_0000_0000_0000)))
            .to_bits(),
        0x8000_0000,
        "binary64 given as binary32: flushed after the rounding"
    );
}

/// A box that outputs the width of the precision it computes in, and how many parameters
/// it was given: what its factory gave it.
#[derive(Debug)]
struct Width(f64);

impl<S: Sample, const N: usize> Object<S, N> for Width {
    fn process(&mut self, _inputs: &[Block<S, N>], outputs: &mut [Block<S, N>]) {
        let value = wide(f64::from(S::PRECISION.bits()) + self.0);
        for output in outputs {
            output.fill(S::from_binary64(value));
        }
    }
}

impl Create for Width {
    const PARAMS: &'static [ParamSpec] = &[];

    fn create<S: Sample, const N: usize>(
        rate: SampleRate,
        params: Params<'_>,
    ) -> Box<dyn Object<S, N>> {
        let sum = params.iter().map(|param| param.raw()).sum::<f64>();
        Box::new(Self(sum + f64::from(rate.hz()) / 1e6_f64))
    }
}

/// The box type of [`Width`], whose values these tests give unchecked.
static WIDTH: ObjectSpec = ObjectSpec::new::<Width>(
    "width~",
    "the width of its samples",
    &[],
    &[],
    &[],
    AlgorithmVersion::FIRST,
);

#[test]
fn a_factory_creates_instances_in_either_precision() {
    let factory = Factory::new::<Width>();
    let rate = SampleRate::new(8_000).expect("valid rate");
    let mut out64 = [Binary64::SILENCE];
    factory
        .create::<Binary64, BLOCK_SIZE>(
            rate,
            ParamValues::flushed(&WIDTH, vec![wide(0.25), wide(0.5)]).params(),
        )
        .expect("a test build holds every engine block")
        .process(&[], &mut out64);
    assert_eq!(out64[0][0].bits(), 64.758_f64.to_bits(), "binary64");
    let mut out32 = [Binary32::SILENCE];
    factory
        .create::<Binary32, BLOCK_SIZE>(
            rate,
            ParamValues::flushed(&WIDTH, vec![wide(0.25)]).params(),
        )
        .expect("a test build holds every engine block")
        .process(&[], &mut out32);
    assert_eq!(
        out32[0][0].bits(),
        u64::from(32.258_f32.to_bits()),
        "binary32"
    );
    assert!(format!("{factory:?}").starts_with("Factory"), "{factory:?}");
}

/// The interpolator of ITU-R BS.1770-4, Annex 2, as the standard prints it: twelve rows of
/// the four phases.
const BS1770_ROWS: [[f64; 4]; 12] = [
    [
        0.001_708_984_375_f64,
        -0.029_174_804_687_5_f64,
        -0.018_920_898_437_5_f64,
        -0.008_300_781_25_f64,
    ],
    [
        0.010_986_328_125_f64,
        0.029_296_875_f64,
        0.033_081_054_687_5_f64,
        0.014_892_578_125_f64,
    ],
    [
        -0.019_653_320_312_5_f64,
        -0.051_757_812_5_f64,
        -0.058_227_539_062_5_f64,
        -0.026_611_328_125_f64,
    ],
    [
        0.033_203_125_f64,
        0.089_111_328_125_f64,
        0.101_562_5_f64,
        0.047_607_421_875_f64,
    ],
    [
        -0.059_448_242_187_5_f64,
        -0.166_503_906_25_f64,
        -0.200_317_382_812_5_f64,
        -0.102_294_921_875_f64,
    ],
    [
        0.137_329_101_562_5_f64,
        0.465_087_890_625_f64,
        0.779_785_156_25_f64,
        0.972_167_968_75_f64,
    ],
    [
        0.972_167_968_75_f64,
        0.779_785_156_25_f64,
        0.465_087_890_625_f64,
        0.137_329_101_562_5_f64,
    ],
    [
        -0.102_294_921_875_f64,
        -0.200_317_382_812_5_f64,
        -0.166_503_906_25_f64,
        -0.059_448_242_187_5_f64,
    ],
    [
        0.047_607_421_875_f64,
        0.101_562_5_f64,
        0.089_111_328_125_f64,
        0.033_203_125_f64,
    ],
    [
        -0.026_611_328_125_f64,
        -0.058_227_539_062_5_f64,
        -0.051_757_812_5_f64,
        -0.019_653_320_312_5_f64,
    ],
    [
        0.014_892_578_125_f64,
        0.033_081_054_687_5_f64,
        0.029_296_875_f64,
        0.010_986_328_125_f64,
    ],
    [
        -0.008_300_781_25_f64,
        -0.018_920_898_437_5_f64,
        -0.029_174_804_687_5_f64,
        0.001_708_984_375_f64,
    ],
];

#[test]
fn holds_the_true_peak_interpolator_of_the_standard_exact_in_both_precisions() {
    for (tap, row) in BS1770_ROWS.iter().enumerate() {
        for (phase, coefficient) in row.iter().enumerate() {
            let held = super::TRUE_PEAK_FILTER[phase][tap];
            assert_eq!(
                held.to_bits(),
                coefficient.to_bits(),
                "phase {phase}, tap {tap}"
            );
            // A multiple of 2^-13 below 1: exact in binary32 too.
            assert_eq!(
                f64::from(to_binary32(held)).to_bits(),
                held.to_bits(),
                "exact in binary32"
            );
            assert_eq!(
                (held * 8192.0_f64).fract().to_bits(),
                0,
                "a multiple of 2^-13"
            );
        }
    }
    assert_eq!(super::TRUE_PEAK_TAPS, 12, "48 taps in 4 phases");
}

/// The larger of two magnitudes, by comparison (`f64::max` is banned for its NaN rule).
fn larger(first: f64, second: f64) -> f64 {
    if second > first { second } else { first }
}

/// The interpolator in `S`, each phase reversed: the order of a window oldest first.
fn filter<S: Sample>() -> Vec<S> {
    super::TRUE_PEAK_FILTER
        .iter()
        .flat_map(|phase| phase.iter().rev())
        .map(|tap| S::from_binary64(super::Binary64(*tap)))
        .collect()
}

fn estimates_the_true_peak_of_an_impulse_at_each_tap<S: Sample>() {
    // One sample of ±1 at tap `k` (`k` frames before the newest, the window being oldest
    // first): each phase gives its coefficient `k`, and the two samples estimated
    // themselves, 5 and 6 frames back, give 1.
    for sign in [1.0_f64, -1.0_f64] {
        for tap in 0..super::TRUE_PEAK_TAPS {
            let mut window = [S::ZERO; super::TRUE_PEAK_TAPS];
            window[super::TRUE_PEAK_TAPS.saturating_sub(1).saturating_sub(tap)] =
                S::from_binary64(super::Binary64(sign));
            let expected = if tap == 5 || tap == 6 {
                1.0_f64
            } else {
                BS1770_ROWS[tap]
                    .iter()
                    .fold(0.0_f64, |peak, coefficient| larger(peak, coefficient.abs()))
            };
            assert_eq!(
                super::true_peak(&window, &filter::<S>(), super::TRUE_PEAK_TAPS),
                S::from_binary64(super::Binary64(expected)),
                "{sign} at tap {tap}"
            );
        }
    }
}

#[test]
fn estimates_the_true_peak_of_an_impulse_at_each_tap_in_each_precision() {
    estimates_the_true_peak_of_an_impulse_at_each_tap::<Binary64>();
    estimates_the_true_peak_of_an_impulse_at_each_tap::<Binary32>();
}

#[test]
fn estimates_the_true_peak_of_any_history_as_the_largest_phase() {
    // Against a reference summing each phase in plain order: the same value up to the
    // rounding of twelve products and sums, relative 2^-48 for samples of ±1.
    let mut draws = splitmix::Draws::new();
    for _ in 0_u16..1_000 {
        let history: [f64; super::TRUE_PEAK_TAPS] = core::array::from_fn(|_| {
            f64::from_bits((0x3ff_u64 << 52_u32) | (draws.next() >> 12_u32)) - 1.5_f64
        });
        let reference = super::TRUE_PEAK_FILTER
            .iter()
            .map(|phase| {
                phase
                    .iter()
                    .zip(history)
                    .map(|(tap, sample)| tap * sample)
                    .fold(0.0_f64, |sum, term| sum + term)
                    .abs()
            })
            .fold(larger(history[5].abs(), history[6].abs()), larger);
        let mut window = history.map(super::Binary64);
        window.reverse();
        let estimate = super::true_peak(&window, &filter::<Binary64>(), super::TRUE_PEAK_TAPS).0;
        assert!(
            (estimate - reference).abs() <= reference * f64::EPSILON * 32.0_f64,
            "{estimate} for {reference}"
        );
    }
}

#[test]
fn needs_no_reduction_up_to_the_ceiling_and_the_exact_share_above() {
    let ceiling = super::Binary64(0.75_f64);
    for peak in [-0.0_f64, 0.0_f64, 0.5_f64, 0.75_f64] {
        assert_eq!(
            super::reduction_needed(super::Binary64(peak), ceiling)
                .0
                .to_bits(),
            0.0_f64.to_bits(),
            "{peak}: exactly +0"
        );
    }
    assert_eq!(
        super::reduction_needed(super::Binary64(1.5_f64), ceiling),
        super::Binary64(0.5_f64),
        "half taken away"
    );
    assert_eq!(
        super::reduction_needed(super::Binary32(3.0_f32), super::Binary32(0.75_f32)),
        super::Binary32(0.75_f32),
        "in binary32"
    );
}

#[test]
fn decays_approaches_shares_and_complements_reductions() {
    assert_eq!(
        super::decayed(super::Binary64(0.5_f64), super::Binary64(0.5_f64)),
        super::Binary64(0.25_f64),
        "kept"
    );
    assert_eq!(
        super::decayed(super::Binary64(f64::MIN_POSITIVE), super::Binary64(0.5_f64))
            .0
            .to_bits(),
        0,
        "flushed to zero, never stalling"
    );
    assert_eq!(
        super::decayed(super::Binary32(f32::MIN_POSITIVE), super::Binary32(0.5_f32))
            .0
            .to_bits(),
        0,
        "in binary32 too"
    );
    assert_eq!(
        super::approached(
            super::Binary64(0.25_f64),
            super::Binary64(0.75_f64),
            super::Binary64(0.5_f64)
        ),
        super::Binary64(0.5_f64),
        "halfway"
    );
    assert_eq!(
        super::approached(
            super::Binary64(0.75_f64),
            super::Binary64(0.25_f64),
            super::Binary64(0.25_f64)
        ),
        super::Binary64(0.625_f64),
        "downward too"
    );
    assert_eq!(
        super::share(super::Binary64(0.5_f64), super::Binary64(0.25_f64)),
        super::Binary64(0.125_f64),
        "a share"
    );
    assert_eq!(
        super::complement(super::Binary64(0.25_f64)),
        super::Binary64(0.75_f64),
        "one minus"
    );
    assert_eq!(
        super::larger_reduction(super::Binary64(0.25_f64), super::Binary64(0.5_f64)),
        super::Binary64(0.5_f64),
        "the larger, second"
    );
    assert_eq!(
        super::larger_reduction(super::Binary64(0.5_f64), super::Binary64(0.25_f64)),
        super::Binary64(0.5_f64),
        "the larger, first"
    );
    assert_eq!(
        super::reduced(super::Binary64(-0.5_f64), super::Binary64(0.25_f64)),
        super::Binary64(-0.375_f64),
        "a sample reduced"
    );
    assert_eq!(
        super::reduced(super::Binary64(f64::MIN_POSITIVE), super::Binary64(0.5_f64))
            .0
            .to_bits(),
        0,
        "a normal sample reduced to a subnormal is flushed"
    );
}

#[test]
fn slides_the_hann_sums_as_the_exact_sums_of_the_window() {
    let mut draws = splitmix::Draws::new();
    for length in [1_usize, 2, 7, 72, 300] {
        let phasor: Vec<(Binary64, Binary64)> = (1..=length)
            .map(|steps| {
                let (cos, sin) = super::limiter::turned(steps, length);
                (cos, sin)
            })
            .collect();
        let turn = phasor[0];
        let values: Vec<Binary64> = (0..length * 5)
            .map(|_| {
                super::Binary64(
                    f64::from_bits((0x3ff_u64 << 52_u32) | (draws.next() >> 12_u32)) - 1.0_f64,
                )
            })
            .collect();
        let mut sums = super::HannSums::<Binary64>::ZERO;
        for (index, value) in values.iter().enumerate() {
            let oldest = index
                .checked_sub(length)
                .map_or(super::Binary64(0.0_f64), |earlier| values[earlier]);
            sums = sums.advanced(*value, oldest, turn);
            let window = (0..length).map(|back| {
                index
                    .checked_sub(back)
                    .map_or(super::Binary64(0.0_f64), |at| values[at])
            });
            let exact = super::HannSums::of(window, &phasor);
            for (slid, summed) in [
                (sums.sum, exact.sum),
                (sums.real, exact.real),
                (sums.imaginary, exact.imaginary),
            ] {
                assert!(
                    (slid.0 - summed.0).abs() <= 1e-12_f64,
                    "{length}: {} and {}",
                    slid.0,
                    summed.0
                );
            }
        }
        // A window of ones: the Hann weights sum to `length + 1`, so the mean is one.
        let ones = super::HannSums::of(
            core::iter::repeat_n(super::Binary64(1.0_f64), length),
            &phasor,
        );
        let mean = ones.smoothed(super::limiter::scale(length)).0;
        assert!(
            (mean - 1.0_f64).abs() <= 1e-12_f64,
            "{length}: the mean of ones is {mean}"
        );
    }
}

#[test]
fn computes_the_limiter_coefficients_from_its_settings() {
    use super::limiter::{amplitude, fast_release, frames, keep, scale, step};
    let rate = SampleRate::new(48_000).expect("a rate");
    assert_eq!(
        frames(super::Binary64(1.5_f64), rate),
        72,
        "1.5 ms at 48 kHz"
    );
    assert_eq!(
        frames(super::Binary64(0.01_f64), rate),
        1,
        "at least one frame"
    );
    assert_eq!(
        frames(super::Binary64(1.0_f64 / 96.0_f64), rate),
        1,
        "half a frame rounds up"
    );
    assert_eq!(
        frames(super::Binary64(f64::NAN), rate),
        usize::MAX,
        "no count saturates"
    );
    // e^-1, computed apart; libm's exp is within an ulp of it.
    assert!(
        (keep(super::Binary64(1000.0_f64 / 48_000.0_f64), rate).0 - 0.367_879_441_171_442_33_f64)
            .abs()
            <= f64::EPSILON,
        "one frame's constant keeps 1/e"
    );
    // e^-0.5, computed apart.
    assert!(
        (keep(super::Binary64(2000.0_f64 / 48_000.0_f64), rate).0 - 0.606_530_659_712_633_4_f64)
            .abs()
            <= f64::EPSILON,
        "two frames' constant keeps e^-1/2"
    );
    assert_eq!(
        step(super::Binary64(0.25_f64)),
        super::Binary64(0.75_f64),
        "the gap closed"
    );
    assert_eq!(
        amplitude(super::Binary64(0.0_f64)),
        super::Binary64(1.0_f64),
        "0 dB"
    );
    assert!(
        (amplitude(super::Binary64(-20.0_f64)).0 - 0.1_f64).abs() < 1e-15_f64,
        "-20 dB"
    );
    assert!(
        (amplitude(super::Binary64(-1.0_f64)).0 - 0.891_250_938_133_745_6_f64).abs() < 1e-15_f64,
        "-1 dBTP"
    );
    assert_eq!(
        fast_release(
            super::Binary64(100.0_f64),
            super::Binary64(0.0_f64),
            super::Binary64(10.0_f64)
        ),
        super::Binary64(100.0_f64),
        "fixed"
    );
    assert_eq!(
        fast_release(
            super::Binary64(100.0_f64),
            super::Binary64(1.0_f64),
            super::Binary64(10.0_f64)
        ),
        super::Binary64(10.0_f64),
        "a tenth"
    );
    assert_eq!(scale(3), super::Binary64(0.25_f64), "one over four");
}

#[test]
fn rounds_to_counts_exactly_or_refuses() {
    for (raw, count) in [
        (0.0_f64, Some(0)),
        (-0.0_f64, Some(0)),
        (-0.4_f64, Some(0)),
        (0.49_f64, Some(0)),
        (0.5_f64, Some(1)),
        (1.5_f64, Some(2)),
        (2.5_f64, Some(3)),
        (1_920.0_f64, Some(1_920)),
        (2_147_483_648.0_f64, Some(2_147_483_648)),
        (4_294_967_295.0_f64, Some(u32::MAX)),
        (4_294_967_295.4_f64, Some(u32::MAX)),
        (4_294_967_295.5_f64, None),
        (-0.5_f64, None),
        (f64::NAN, None),
        (f64::INFINITY, None),
    ] {
        assert_eq!(super::Binary64(raw).rounded_count(), count, "{raw}");
    }
    assert_eq!(Binary64::from_count(0), super::Binary64(0.0_f64), "zero");
    assert_eq!(
        Binary64::from_count(1_920),
        super::Binary64(1_920.0_f64),
        "a count"
    );
    assert_eq!(
        Binary64::from_count(usize::MAX),
        super::Binary64(f64::from(u32::MAX)),
        "saturates"
    );
}

#[test]
fn computes_the_bessel_function_of_the_kaiser_window() {
    use super::limiter::bessel_i0;
    // Known values of I0, computed apart.
    for (x, value) in [
        (0.0_f64, 1.0_f64),
        (1.0_f64, 1.266_065_877_752_008_4_f64),
        (2.5_f64, 3.289_839_144_050_123_f64),
        (5.0_f64, 27.239_871_823_604_442_f64),
    ] {
        let computed = bessel_i0(super::Binary64(x)).0;
        assert!(
            (computed - value).abs() <= value * 4.0_f64 * f64::EPSILON,
            "I0({x}) = {computed}"
        );
    }
}

#[test]
fn keeps_the_sign_of_zeros_as_documented() {
    let (plus, minus) = (super::Binary64(0.0_f64), super::Binary64(-0.0_f64));
    assert_eq!(
        super::magnitude(plus).0.to_bits(),
        0.0_f64.to_bits(),
        "+0 stays +0"
    );
    assert_eq!(
        super::magnitude(minus).0.to_bits(),
        (-0.0_f64).to_bits(),
        "-0 keeps its sign"
    );
    assert_eq!(
        super::magnitude(super::Binary64(-2.0_f64)),
        super::Binary64(2.0_f64),
        "negated"
    );
    assert_eq!(
        super::larger(plus, minus).0.to_bits(),
        0.0_f64.to_bits(),
        "the first when equal"
    );
    assert_eq!(
        super::larger(minus, plus).0.to_bits(),
        (-0.0_f64).to_bits(),
        "the first when equal"
    );
}

/// The Kaiser-windowed sinc interpolator, written apart from the math layer's: point `m`
/// of `phases × taps`, `u = (m - c) / phases` input frames from the centre `c`, is
/// `sin(π u) / (π u) × I0(β √(1 - ((m - c)/c)²)) / I0(β)`, I0 summed over sixty terms; tap `k`
/// of phase `p` is point `k × phases + p`, each phase then divided by its sum.
fn reference_interpolator(phases: usize, taps: usize, beta: f64) -> Vec<f64> {
    let bessel = |x: f64| {
        // Sixty terms: far past convergence for arguments up to 10.
        let (mut term, mut sum) = (1.0_f64, 1.0_f64);
        for order in 1_u32..=60 {
            let ratio = x / 2.0_f64 / f64::from(order);
            term *= ratio * ratio;
            sum += term;
        }
        sum
    };
    let count = |value: usize| f64::from(u32::try_from(value).expect("small"));
    let centre = (count(phases.saturating_mul(taps)) - 1.0_f64) / 2.0_f64;
    let mut all = Vec::new();
    for phase in 0..phases {
        let row: Vec<f64> = (0..taps)
            .map(|tap| {
                let offset = count(tap.saturating_mul(phases).saturating_add(phase)) - centre;
                let angle = core::f64::consts::PI * offset / count(phases);
                let ratio = offset / centre;
                libm::sin(angle) / angle * bessel(beta * libm::sqrt(1.0_f64 - ratio * ratio))
                    / bessel(beta)
            })
            .collect();
        let gain = row.iter().fold(0.0_f64, |sum, tap| sum + tap);
        all.extend(row.iter().map(|tap| tap / gain));
    }
    all
}

#[test]
fn computes_the_windowed_interpolators_as_their_formula() {
    for (phases, taps, beta) in [
        (4_usize, 32_usize, 2.5_f64),
        (8, 128, 5.0_f64),
        (2, 6, 1.0_f64),
    ] {
        let computed = super::limiter::interpolator(phases, taps, super::Binary64(beta));
        let reference = reference_interpolator(phases, taps, beta);
        assert_eq!(computed.len(), reference.len(), "{phases} × {taps}");
        for (index, (one, other)) in computed.iter().zip(&reference).enumerate() {
            assert!(
                (one.0 - other).abs() <= 1e-14_f64,
                "{phases} × {taps}, point {index}: {} and {other}",
                one.0
            );
        }
    }
}
