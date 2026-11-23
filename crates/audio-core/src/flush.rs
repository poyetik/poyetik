//! Subnormal numbers flushed to zero, in software (`spec/engine-design.md`, decision 11).
//!
//! WebAssembly has no hardware flush-to-zero, and a native host may turn one on: the
//! engine flushes at defined points itself (every box output and every recursive state,
//! each frame; every parameter when a plan is built; every sample delivered to a host),
//! so that every target computes the same bits, no host mode changes a delivered one,
//! and recursive tails never slow down on subnormal arithmetic.

#![cfg_attr(not(test), forbid(clippy::arithmetic_side_effects))]

/// The sign bit of a binary32 value.
const SIGN_32: u32 = 0x8000_0000;

/// The exponent bits of a binary32 value: all zero for a zero or a subnormal.
const EXPONENT_32: u32 = 0x7f80_0000;

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
    if bits & EXPONENT_32 == 0 {
        f32::from_bits(bits & SIGN_32)
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

/// The binary64 exponent bits of 2^-126, the smallest normal binary32 value.
const SMALLEST_NORMAL_32_IN_64: u64 = 0x3810_0000_0000_0000;

/// A parameter value, `value`, or a zero of its sign when its magnitude is nonzero and
/// below 2^-126, the smallest normal binary32 value (`spec/engine-design.md`, decisions 5
/// and 11): what a binary32 value flushed by [`binary32`] gives once widened, and the rule
/// binary64 parameters follow, before any rounding to binary32. Decided from the bits
/// alone, as [`binary32`].
#[must_use]
#[inline]
pub(crate) const fn parameter(value: f64) -> f64 {
    let bits = value.to_bits();
    if bits & EXPONENT_64 < SMALLEST_NORMAL_32_IN_64 {
        f64::from_bits(bits & SIGN_64)
    } else {
        value
    }
}

/// The bits of the canonical binary32 NaN: positive, quiet, without a payload.
const CANONICAL_NAN_32: u32 = 0x7fc0_0000;

/// The bits of the canonical binary64 NaN: positive, quiet, without a payload.
const CANONICAL_NAN_64: u64 = 0x7ff8_0000_0000_0000;

/// The binary32 value of bits `bits`, or the canonical NaN when it is one, as bits
/// (`spec/box-contract.md`, section 3). Decided from the bits alone, as [`binary32`].
#[must_use]
#[inline]
pub(crate) const fn canonical_bits_32(bits: u32) -> u32 {
    if bits & !SIGN_32 > EXPONENT_32 {
        CANONICAL_NAN_32
    } else {
        bits
    }
}

/// The binary64 value of bits `bits`, or the canonical NaN when it is one, as bits. Decided
/// from the bits alone, as [`binary32`].
#[must_use]
#[inline]
pub(crate) const fn canonical_bits_64(bits: u64) -> u64 {
    if bits & !SIGN_64 > EXPONENT_64 {
        CANONICAL_NAN_64
    } else {
        bits
    }
}

#[cfg(test)]
#[path = "flush_tests.rs"]
mod tests;
