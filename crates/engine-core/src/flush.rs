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

/// The sign bit of a binary64 value.
const SIGN_64: u64 = 0x8000_0000_0000_0000;

/// The exponent bits of a binary64 value: all zero for a zero or a subnormal.
const EXPONENT_64: u64 = 0x7ff0_0000_0000_0000;

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
pub(crate) const fn binary32(value: f32) -> f32 {
    let bits = value.to_bits();
    if bits & EXPONENT == 0 {
        f32::from_bits(bits & SIGN)
    } else {
        value
    }
}

/// `value`, or a zero of its sign when it is subnormal: below the smallest normal binary64
/// value, 2^-1022, in magnitude. Decided from the bits alone, as [`binary32`].
#[must_use]
#[inline]
pub(crate) const fn binary64(value: f64) -> f64 {
    let bits = value.to_bits();
    if bits & EXPONENT_64 == 0 {
        f64::from_bits(bits & SIGN_64)
    } else {
        value
    }
}

#[cfg(test)]
#[path = "flush_tests.rs"]
mod tests;
