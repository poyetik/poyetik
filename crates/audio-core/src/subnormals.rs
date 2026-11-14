//! What the floating-point arithmetic of the calling thread does with subnormal numbers,
//! observed by computing some (`spec/engine-design.md`, decision 11).
//!
//! WebAssembly defines IEEE arithmetic, subnormals included, but a browser runs it on the
//! thread of its own audio, which it may put in a hardware flush mode: flush-to-zero
//! (results) or denormals-are-zero (operands). Whether that mode reaches the engine's
//! arithmetic cannot be read from WebAssembly, only observed; a native shell sets the
//! default mode itself, and observes it the same way to report a host whose mode could not
//! be set.

#![cfg_attr(not(test), forbid(clippy::arithmetic_side_effects))]

/// Whether subnormals survive in one format: as the operands of an operation, and as its
/// result. Both `true` is IEEE arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubnormalsKept {
    /// A subnormal operand is used as it is, not as zero (no denormals-are-zero).
    pub operands: bool,
    /// A subnormal result is given as it is, not as zero (no flush-to-zero).
    pub results: bool,
}

/// What the arithmetic of the calling thread does with subnormals, in each format, as
/// [`SubnormalArithmetic::observe`] saw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubnormalArithmetic {
    /// In binary32.
    pub binary32: SubnormalsKept,
    /// In binary64.
    pub binary64: SubnormalsKept,
}

/// The bits of `2^60` in binary64: a biased exponent of `1023 + 60`, no fraction.
const TWO_TO_60: u64 = 0x43b0_0000_0000_0000;

/// The bits of `2^-125` in binary32, normal: a biased exponent of `127 - 125`.
const TWO_TO_MINUS_125: u32 = 0x0100_0000;

/// The bits of `2^-128` in binary32, subnormal: `2^-149 · 2^21`.
const TWO_TO_MINUS_128: u32 = 0x0020_0000;

/// The bits of `2^-1014` in binary64, normal: a biased exponent of `1023 - 1014`.
const TWO_TO_MINUS_1014: u64 = 0x0090_0000_0000_0000;

/// The bits of `2^-1024` in binary64, subnormal: `2^-1074 · 2^50`.
const TWO_TO_MINUS_1024: u64 = 0x0004_0000_0000_0000;

impl SubnormalArithmetic {
    /// IEEE arithmetic: every subnormal kept, as operand and as result, in both formats.
    pub const IEEE: Self = Self {
        binary32: SubnormalsKept {
            operands: true,
            results: true,
        },
        binary64: SubnormalsKept {
            operands: true,
            results: true,
        },
    };

    /// Whether this is IEEE arithmetic: the engine's arithmetic, which it needs to give its
    /// reference bits.
    #[must_use]
    pub fn is_ieee(self) -> bool {
        self == Self::IEEE
    }

    /// Computes subnormals on the calling thread and reports which survive.
    ///
    /// The operand probe multiplies the smallest subnormal by `2^24` (binary32) or `2^60`
    /// (binary64): its exact result, `2^-125` or `2^-1014`, is normal, so only a flush of
    /// operands changes it. The result probe divides the smallest normal by 4: its operands
    /// are normal and its exact result, `2^-128` or `2^-1024`, is subnormal, so only a flush
    /// of results changes it. A probe counts as kept only when it gives exactly its IEEE
    /// bits. Every operand and result passes through `black_box`, so that nothing is
    /// computed when compiling: the processor computes it, in its mode at the call.
    #[must_use]
    pub fn observe() -> Self {
        use core::hint::black_box;
        let operand_32 = black_box(black_box(f32::from_bits(1)) * black_box(16_777_216.0_f32));
        let result_32 = black_box(black_box(f32::MIN_POSITIVE) / black_box(4.0_f32));
        let operand_64 =
            black_box(black_box(f64::from_bits(1)) * black_box(f64::from_bits(TWO_TO_60)));
        let result_64 = black_box(black_box(f64::MIN_POSITIVE) / black_box(4.0_f64));
        Self {
            binary32: SubnormalsKept {
                operands: operand_32.to_bits() == TWO_TO_MINUS_125,
                results: result_32.to_bits() == TWO_TO_MINUS_128,
            },
            binary64: SubnormalsKept {
                operands: operand_64.to_bits() == TWO_TO_MINUS_1014,
                results: result_64.to_bits() == TWO_TO_MINUS_1024,
            },
        }
    }
}

#[cfg(test)]
#[path = "subnormals_tests.rs"]
mod tests;
