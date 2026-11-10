//! Tests of the flush of subnormal samples to zero.

use super::{binary32 as flush, binary64};

/// The smallest normal binary32 value, 2^-126, written as a literal (not read from
/// the constants of the code under test).
const SMALLEST_NORMAL: f64 = 1.175_494_350_822_287_5e-38;

/// What [`flush`] must give, computed on the binary64 side: a binary32 value is exact
/// in binary64, where it is a normal number whatever its binary32 class, so the
/// comparison with 2^-126 is exact and no hardware flush mode can touch it.
fn expected(value: f32) -> u32 {
    let wide = f64::from(value);
    // A NaN compares false: it keeps its bits.
    if wide.abs() < SMALLEST_NORMAL {
        value.to_bits() & 0x8000_0000_u32
    } else {
        value.to_bits()
    }
}

#[test]
fn the_threshold_is_the_smallest_normal() {
    assert_eq!(
        SMALLEST_NORMAL.to_bits(),
        (1_023_u64 - 126) << 52_u32,
        "2^-126: a biased exponent of 1023 - 126, no fraction"
    );
    assert_eq!(
        SMALLEST_NORMAL.to_bits(),
        f64::from(f32::MIN_POSITIVE).to_bits(),
        "binary32's smallest normal"
    );
}

#[test]
fn flushes_every_subnormal_to_a_zero_of_its_sign() {
    // Every subnormal binary32 value, both signs: 2^24 values.
    for fraction in 1_u32..0x0080_0000_u32 {
        for sign in [0_u32, 0x8000_0000_u32] {
            let value = f32::from_bits(sign | fraction);
            assert_eq!(flush(value).to_bits(), sign, "{:#010x}", value.to_bits());
            assert_eq!(flush(value).to_bits(), expected(value), "{value:e}");
        }
    }
}

#[test]
fn keeps_every_other_class_bit_for_bit() {
    // Every exponent but the subnormal one, with fractions at both ends and between,
    // both signs: normals from the smallest, infinities, NaN payloads (quiet and
    // signaling).
    for exponent in 1_u32..=0xff_u32 {
        for fraction in [0_u32, 1, 0x0012_3457, 0x0040_0000, 0x007f_ffff] {
            for sign in [0_u32, 0x8000_0000_u32] {
                let bits = sign | exponent << 23_u32 | fraction;
                let value = f32::from_bits(bits);
                assert_eq!(flush(value).to_bits(), bits, "{bits:#010x}");
                assert_eq!(flush(value).to_bits(), expected(value), "{bits:#010x}");
            }
        }
    }
    assert_eq!(flush(0.0).to_bits(), 0_u32, "+0");
    assert_eq!(flush(-0.0).to_bits(), 0x8000_0000_u32, "-0");
    assert_eq!(
        flush(f32::MIN_POSITIVE).to_bits(),
        f32::MIN_POSITIVE.to_bits(),
        "the smallest normal"
    );
}

/// The smallest normal binary64 value, 2^-1022, written as a literal.
const SMALLEST_NORMAL_64: f64 = 2.225_073_858_507_201_4e-308;

#[test]
fn the_binary64_threshold_is_the_smallest_normal() {
    assert_eq!(
        SMALLEST_NORMAL_64.to_bits(),
        1_u64 << 52_u32,
        "2^-1022: a biased exponent of 1, no fraction"
    );
    assert_eq!(
        SMALLEST_NORMAL_64.to_bits(),
        f64::MIN_POSITIVE.to_bits(),
        "binary64's smallest normal"
    );
}

#[test]
fn flushes_binary64_subnormals_to_a_zero_of_their_sign() {
    // Fractions at both ends of the subnormal range and between, every bit position.
    let mut fractions: alloc::vec::Vec<u64> = (0_u32..52_u32).map(|bit| 1_u64 << bit).collect();
    fractions.extend([
        0x000f_ffff_ffff_ffff_u64,
        0x0001_2345_6789_abcd,
        3,
        0x0008_0000_0000_0001,
    ]);
    for fraction in fractions {
        for sign in [0_u64, 1_u64 << 63_u32] {
            let value = f64::from_bits(sign | fraction);
            assert!(value.abs() < SMALLEST_NORMAL_64, "{value:e} is subnormal");
            assert_eq!(binary64(value).to_bits(), sign, "{:#018x}", value.to_bits());
        }
    }
}

#[test]
fn keeps_every_other_binary64_class_bit_for_bit() {
    for exponent in [1_u64, 2, 0x3fe, 0x3ff, 0x400, 0x7fe, 0x7ff] {
        for fraction in [0_u64, 1, 0x0008_0000_0000_0000, 0x000f_ffff_ffff_ffff] {
            for sign in [0_u64, 1_u64 << 63_u32] {
                let bits = sign | exponent << 52_u32 | fraction;
                let value = f64::from_bits(bits);
                assert!(!value.is_subnormal(), "{bits:#018x}");
                assert_eq!(binary64(value).to_bits(), bits, "{bits:#018x}");
            }
        }
    }
    assert_eq!(binary64(0.0).to_bits(), 0_u64, "+0");
    assert_eq!(binary64(-0.0).to_bits(), 1_u64 << 63_u32, "-0");
    assert_eq!(
        binary64(SMALLEST_NORMAL_64).to_bits(),
        SMALLEST_NORMAL_64.to_bits(),
        "the smallest normal"
    );
}
