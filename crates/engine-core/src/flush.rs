//! Subnormal numbers flushed to zero, in software (`spec/engine-design.md`, decision 11).
//!
//! WebAssembly has no hardware flush-to-zero, and a native host may turn one on: the
//! engine flushes at defined points itself (every box output and every recursive state,
//! each frame; every parameter when a plan is built), so that every target computes the
//! same bits, and recursive tails never slow down on subnormal arithmetic.

/// The sign bit of a binary32 value.
const SIGN: u32 = 0x8000_0000;

/// The exponent bits of a binary32 value: all zero for a zero or a subnormal.
const EXPONENT: u32 = 0x7f80_0000;

/// `value`, or a zero of its sign when it is subnormal.
///
/// A subnormal is nonzero and below the smallest normal binary32 value, 2^-126, in
/// magnitude. Every other value keeps its bits: zeros, normals, infinities and NaN
/// payloads.
///
/// Decided from the bits alone, with integer operations: a hardware mode that treats
/// subnormal operands as zero (denormals-are-zero) cannot change the decision, as it could
/// a floating-point comparison.
#[must_use]
#[inline]
pub const fn flush(value: f32) -> f32 {
    let bits = value.to_bits();
    if bits & EXPONENT == 0 {
        f32::from_bits(bits & SIGN)
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::flush;

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
}
