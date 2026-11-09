//! Tests of the math layer.

use alloc::boxed::Box;
use alloc::format;
use alloc::vec::Vec;

use crate::{
    ArithmeticMode, ArithmeticOption, BLOCK_SIZE, Block, Create, Delivered, Factory, MODE, Object,
    Precision, RenderFormat, Sample, SampleRate, UnknownArithmeticOptions, to_binary32,
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
    let mut runner = proptest_like::Draws::new();
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
/// and Flood, 2014), as `spec/measurements.md` defines it.
mod proptest_like {
    /// The state of the generator.
    pub(super) struct Draws(u64);

    impl Draws {
        /// A generator from a fixed seed.
        pub(super) const fn new() -> Self {
            Self(0x5059_0006)
        }

        /// The next 64 bits.
        pub(super) const fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut mixed = self.0;
            mixed = (mixed ^ (mixed >> 30_u32)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            mixed = (mixed ^ (mixed >> 27_u32)).wrapping_mul(0x94d0_49bb_1331_11eb);
            mixed ^ (mixed >> 31_u32)
        }
    }
}

#[test]
fn names_the_precisions_and_the_render_formats() {
    assert_eq!(
        Precision::default(),
        Precision::Binary64,
        "64-bit by default"
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
    assert_eq!(MODE, ArithmeticMode::STRICT, "the only mode built");
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
                ArithmeticMode::from_bits(known | unknown),
                Err(UnknownArithmeticOptions(unknown)),
                "bit {bit}, with {known:#b}: the unknown bits only"
            );
        }
    }
    assert_eq!(
        format!("{}", UnknownArithmeticOptions(8)),
        "unknown arithmetic options: 0x0000000000000008",
        "explained"
    );
}

#[test]
fn computes_each_precision_with_the_pinned_libm() {
    // Known values first, so that the function is the right one, then the delegation.
    assert_eq!(Sample::sin(0.0_f64).to_bits(), 0, "sin 0");
    assert_eq!(Sample::sin(-0.0_f32).to_bits(), 0x8000_0000, "sin -0");
    assert_eq!(
        Sample::sin(core::f64::consts::FRAC_PI_2).to_bits(),
        1.0_f64.to_bits(),
        "sin π/2"
    );
    assert_eq!(
        Sample::sin(core::f32::consts::FRAC_PI_2).to_bits(),
        1.0_f32.to_bits(),
        "sinf π/2"
    );
    for value in [0.5_f64, 1.0_f64, 3.0_f64, 6.25_f64, -2.5_f64, 1e-300_f64] {
        assert_eq!(
            Sample::sin(value).to_bits(),
            libm::sin(value).to_bits(),
            "{value}"
        );
        let narrow = to_binary32(value);
        assert_eq!(
            Sample::sin(narrow).to_bits(),
            libm::sinf(narrow).to_bits(),
            "{narrow}"
        );
    }
    assert_ne!(
        f64::from(Sample::sin(1.0_f32)).to_bits(),
        Sample::sin(1.0_f64).to_bits(),
        "each in its own precision"
    );
    for (value, floor) in [
        (2.5_f64, 2.0_f64),
        (-0.5_f64, -1.0_f64),
        (-0.0_f64, -0.0_f64),
        (7.0_f64, 7.0_f64),
    ] {
        assert_eq!(Sample::floor(value).to_bits(), floor.to_bits(), "{value}");
        assert_eq!(
            Sample::floor(to_binary32(value)).to_bits(),
            to_binary32(floor).to_bits(),
            "{value}"
        );
    }
}

#[test]
fn converts_and_flushes_in_each_precision() {
    assert_eq!(f64::from_f64(0.1).to_bits(), 0.1_f64.to_bits(), "exact");
    assert_eq!(f32::from_f64(0.1).to_bits(), 0x3dcc_cccd, "rounded");
    let tiny = f64::from_bits(1);
    assert_eq!(Sample::to_f64(tiny).to_bits(), 1, "binary64 as it is");
    assert_eq!(
        Sample::to_f64(f32::from_bits(1)).to_bits(),
        0x36a0_0000_0000_0000,
        "2^-149, exactly"
    );
    assert_eq!(
        Sample::to_binary32(0.1_f64).to_bits(),
        0x3dcc_cccd,
        "rounded"
    );
    let nan = f32::from_bits(0x7fa0_0001);
    assert_eq!(Sample::to_binary32(nan).to_bits(), 0x7fa0_0001, "as it is");
    let subnormal_64 = f64::from_bits(1);
    assert_eq!(
        Sample::flush(subnormal_64).to_bits(),
        0,
        "binary64 subnormal"
    );
    assert_eq!(
        Sample::flush(-f64::MIN_POSITIVE).to_bits(),
        (-f64::MIN_POSITIVE).to_bits(),
        "binary64 normal"
    );
    // 2^-1000 is normal in binary64 and below every binary32: a binary64 flush keeps it.
    let below_binary32 = f64::from_bits(0x0170_0000_0000_0000);
    assert_eq!(
        Sample::flush(below_binary32).to_bits(),
        below_binary32.to_bits(),
        "kept"
    );
    assert_eq!(
        Sample::flush(-f32::from_bits(1)).to_bits(),
        0x8000_0000,
        "binary32 subnormal"
    );
    assert_eq!(
        Sample::flush(f32::MIN_POSITIVE).to_bits(),
        f32::MIN_POSITIVE.to_bits(),
        "binary32 normal"
    );
    assert_eq!(f64::PRECISION, Precision::Binary64, "binary64");
    assert_eq!(f32::PRECISION, Precision::Binary32, "binary32");
    assert!(
        f64::SILENCE.iter().all(|sample| sample.to_bits() == 0),
        "+0"
    );
    assert!(
        f32::SILENCE.iter().all(|sample| sample.to_bits() == 0),
        "+0"
    );
}

#[test]
fn delivers_a_render_as_it_is_or_rounded_to_binary32() {
    let nan = f64::from_bits(0x7ff4_0000_0000_0001);
    assert_eq!(
        <f64 as Delivered<f64>>::deliver(nan).to_bits(),
        nan.to_bits(),
        "binary64 as it is"
    );
    let narrow_nan = f32::from_bits(0xffa0_0002);
    assert_eq!(
        <f32 as Delivered<f32>>::deliver(narrow_nan).to_bits(),
        0xffa0_0002,
        "binary32 as it is"
    );
    assert_eq!(
        <f32 as Delivered<f64>>::deliver(0.1_f64).to_bits(),
        0x3dcc_cccd,
        "binary64 rounded"
    );
    assert_eq!(
        <f32 as Delivered<f64>>::deliver(nan).to_bits(),
        0x7fe0_0000,
        "a NaN by its bits"
    );
}

/// A box that outputs the width of the precision it computes in, and how many parameters
/// it was given: what its factory gave it.
#[derive(Debug)]
struct Width(f64);

impl<S: Sample> Object<S> for Width {
    fn process(&mut self, _inputs: &[Block<S>], outputs: &mut [Block<S>]) {
        for output in outputs {
            *output = [S::from_f64(f64::from(S::PRECISION.bits()) + self.0); BLOCK_SIZE];
        }
    }
}

impl Create for Width {
    fn create<S: Sample>(rate: SampleRate, params: &[f64]) -> Box<dyn Object<S>> {
        let given = params.iter().sum::<f64>() + f64::from(rate.hz()) / 1e6_f64;
        Box::new(Self(given))
    }
}

#[test]
fn a_factory_creates_instances_in_either_precision() {
    let factory = Factory::new::<Width>();
    let rate = SampleRate::new(8_000).expect("valid rate");
    let mut out64 = [f64::SILENCE];
    factory
        .create::<f64>(rate, &[0.25_f64, 0.5_f64])
        .process(&[], &mut out64);
    assert_eq!(out64[0][0].to_bits(), 64.758_f64.to_bits(), "binary64");
    let mut out32 = [f32::SILENCE];
    factory
        .create::<f32>(rate, &[0.25_f64])
        .process(&[], &mut out32);
    assert_eq!(out32[0][0].to_bits(), 32.258_f32.to_bits(), "binary32");
    assert!(format!("{factory:?}").starts_with("Factory"), "{factory:?}");
}
