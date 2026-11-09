//! The engine's math layer (`spec/engine-design.md`, decision 10): the sample type with its
//! operators, and the math functions, through which all box arithmetic goes.
//!
//! The engine computes in one of two **precisions**, chosen per render by the host:
//! IEEE 754 binary64 by default, binary32 as a setting. A box is written once, generic over
//! [`Sample`], and the plan instantiates it in the precision of the render; parameter values
//! and what boxes derive from them (increments, coefficients) are binary64 in both.
//!
//! The math functions are the pinned pure-Rust `libm`, the same bits on every target; no box
//! crate depends on `libm` itself (xtask's dependency gate refuses it), and the methods of
//! `f32` and `f64` that compute them do not exist in the `no_std` crates boxes live in. So
//! a box reaches a math function only through this layer.
//!
//! The **arithmetic mode** of this build is [`MODE`], the strict default: every operation
//! correctly rounded, no fused multiply-add, `libm`. Other modes are build-time choices of
//! this layer's implementation that no build makes yet (decision 10); the render hash
//! records the mode (`spec/render-hash.md`).

use alloc::boxed::Box;
use core::fmt;
use core::ops::{Add, AddAssign, BitOr, Div, Mul, Neg, Sub};

use crate::{BLOCK_SIZE, Block, Factory, Object, SampleRate};

/// The precision a render computes its signals in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum Precision {
    /// IEEE 754 binary64: the default on every target.
    #[default]
    Binary64,
    /// IEEE 754 binary32: a host setting, for performance.
    Binary32,
}

impl Precision {
    /// Both precisions, the default first.
    pub const ALL: [Self; 2] = [Self::Binary64, Self::Binary32];

    /// The width of its samples, in bits: 64 or 32.
    #[must_use]
    pub const fn bits(self) -> u8 {
        match self {
            Self::Binary64 => 64,
            Self::Binary32 => 32,
        }
    }
}

/// What a render is: the precision that computed it and the format of its samples.
///
/// A 64-bit render is given as binary64, or rounded to binary32 for a host that takes
/// binary32 (the browser's audio thread, most device backends); a 32-bit render is given as
/// binary32. A case has one reference hash per render format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RenderFormat {
    /// Computed and given in binary64.
    Binary64,
    /// Computed in binary64, each sample then rounded to the nearest binary32
    /// ([`to_binary32`]).
    Binary64AsBinary32,
    /// Computed and given in binary32.
    Binary32,
}

impl RenderFormat {
    /// Every render format, in the order of `conformance/references.json`.
    pub const ALL: [Self; 3] = [Self::Binary64, Self::Binary64AsBinary32, Self::Binary32];

    /// The precision that computes it.
    #[must_use]
    pub const fn precision(self) -> Precision {
        match self {
            Self::Binary64 | Self::Binary64AsBinary32 => Precision::Binary64,
            Self::Binary32 => Precision::Binary32,
        }
    }

    /// The width of the samples it gives, in bits: 64 or 32.
    #[must_use]
    pub const fn sample_bits(self) -> u8 {
        match self {
            Self::Binary64 => 64,
            Self::Binary64AsBinary32 | Self::Binary32 => 32,
        }
    }

    /// Its name in `conformance/references.json` and in the names of exported files.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Binary64 => "binary64",
            Self::Binary64AsBinary32 => "binary64-as-binary32",
            Self::Binary32 => "binary32",
        }
    }
}

impl fmt::Display for RenderFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// One arithmetic option of the math layer: a build-time choice, one flag of the mode.
///
/// A flag of the [`ArithmeticMode`] (`spec/engine-design.md`, decision 10). None is built:
/// the options are named so that the render hash can record and read them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ArithmeticOption {
    /// Fused multiply-add: deterministic and provable.
    FusedMultiplyAdd,
    /// Reassociable operations: non-deterministic.
    Reassociable,
    /// Platform math functions instead of the pinned `libm`: non-deterministic.
    PlatformMath,
}

impl ArithmeticOption {
    /// Every option the arithmetic mode defines (version 1 of the set of options), by flag.
    pub const ALL: [Self; 3] = [
        Self::FusedMultiplyAdd,
        Self::Reassociable,
        Self::PlatformMath,
    ];

    /// Its flag: the bit of the arithmetic mode that turns it on.
    #[must_use]
    pub const fn flag(self) -> u64 {
        match self {
            Self::FusedMultiplyAdd => 1,
            Self::Reassociable => 2,
            Self::PlatformMath => 4,
        }
    }
}

/// The set of arithmetic options a render was computed with, recorded by the render hash:
/// one flag per [`ArithmeticOption`], every other bit refused. All off is the strict
/// default, [`ArithmeticMode::STRICT`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArithmeticMode(u64);

/// An arithmetic mode with flags that no [`ArithmeticOption`] defines: from a later
/// version of the set of options, or not a mode at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownArithmeticOptions(pub u64);

impl fmt::Display for UnknownArithmeticOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown arithmetic options: {:#018x}", self.0)
    }
}

impl ArithmeticMode {
    /// Every option off: each operation correctly rounded, no fused multiply-add, the
    /// pinned `libm`.
    pub const STRICT: Self = Self(0);

    /// The mode whose flags are `bits`.
    ///
    /// # Errors
    ///
    /// Returns [`UnknownArithmeticOptions`] with the bits no option defines.
    pub fn from_bits(bits: u64) -> Result<Self, UnknownArithmeticOptions> {
        // The union of the flags, as the trait's function: the flags are distinct bits, so
        // an operator here would read the same as `^` or `+`.
        let known = ArithmeticOption::ALL
            .map(ArithmeticOption::flag)
            .into_iter()
            .fold(0_u64, BitOr::bitor);
        if bits & !known == 0 {
            Ok(Self(bits))
        } else {
            Err(UnknownArithmeticOptions(bits & !known))
        }
    }

    /// Its flags.
    #[must_use]
    pub const fn bits(self) -> u64 {
        self.0
    }
}

/// The arithmetic mode of this build of the math layer: the strict default, the only one
/// built.
pub const MODE: ArithmeticMode = ArithmeticMode::STRICT;

/// The nearest binary32 value to `value`, the same bits on every target.
///
/// A number is rounded to nearest, ties to even (IEEE 754, which the conversion follows on
/// every target; a subnormal result is kept, under the default floating-point mode the
/// native shells set). A NaN, whose conversion IEEE 754 leaves open in its payload (and
/// WebAssembly too), is converted from its bits: its sign, the quiet bit set, and the top 22
/// bits of its payload.
#[must_use]
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    reason = "f64 to f32 rounding is the intended, IEEE 754-defined conversion; no From exists"
)]
pub fn to_binary32(value: f64) -> f32 {
    if value.is_nan() {
        let bits = value.to_bits();
        let sign = if bits >> 63_u32 == 0 {
            0
        } else {
            0x8000_0000_u32
        };
        let payload = ((bits >> 29_u32) & 0x003f_ffff) as u32;
        // Disjoint fields (the sign, the exponent and quiet bit, the payload), joined by the
        // trait's function: an operator here would read the same as `^` or `+`.
        f32::from_bits(sign.bitor(0x7fc0_0000).bitor(payload))
    } else {
        value as f32
    }
}

/// A constructor of box instances computing in `S`.
pub type Constructor<S> = fn(SampleRate, &[f64]) -> Box<dyn Object<S>>;

/// Hides [`Sample`] from implementations outside this crate: the engine computes in
/// binary64 and binary32 only.
mod sealed {
    /// The sample types.
    pub trait Sealed {}
    impl Sealed for f32 {}
    impl Sealed for f64 {}
}

/// A type the engine computes signals in: `f64` or `f32`, nothing else. Its operators are
/// IEEE 754's, each correctly rounded; its math functions are the pinned `libm`'s.
///
/// Also the type of parameter values and what boxes derive from them, as `f64`, whatever the
/// precision of the render.
pub trait Sample:
    sealed::Sealed
    + Copy
    + fmt::Debug
    + Send
    + Sync
    + PartialEq
    + PartialOrd
    + Add<Output = Self>
    + Sub<Output = Self>
    + Mul<Output = Self>
    + Div<Output = Self>
    + Neg<Output = Self>
    + AddAssign
    + 'static
{
    /// The precision this type computes in.
    const PRECISION: Precision;

    /// Positive zero.
    const ZERO: Self;

    /// A block of silence: positive zeros.
    const SILENCE: Block<Self> = [Self::ZERO; BLOCK_SIZE];

    /// The nearest value of this type to `value` ([`to_binary32`] for `f32`, `value` itself
    /// for `f64`).
    fn from_f64(value: f64) -> Self;

    /// Itself in binary64, exactly.
    fn to_f64(self) -> f64;

    /// Its nearest binary32 value ([`to_binary32`] for `f64`, itself for `f32`): what a host
    /// that takes binary32 is given.
    fn to_binary32(self) -> f32;

    /// Itself, or a zero of its sign when it is subnormal (`spec/engine-design.md`,
    /// decision 11): decided from its bits, so that no hardware mode can change it.
    #[must_use]
    fn flush(self) -> Self;

    /// The sine of `self` radians (`libm`).
    #[must_use]
    fn sin(self) -> Self;

    /// The largest integer value not above `self` (`libm`).
    #[must_use]
    fn floor(self) -> Self;

    /// The constructor of `factory` for this type.
    fn constructor(factory: &Factory) -> Constructor<Self>;
}

impl Sample for f64 {
    const PRECISION: Precision = Precision::Binary64;
    const ZERO: Self = 0.0;

    fn from_f64(value: f64) -> Self {
        value
    }

    fn to_f64(self) -> f64 {
        self
    }

    fn to_binary32(self) -> f32 {
        to_binary32(self)
    }

    fn flush(self) -> Self {
        crate::flush::binary64(self)
    }

    fn sin(self) -> Self {
        libm::sin(self)
    }

    fn floor(self) -> Self {
        libm::floor(self)
    }

    fn constructor(factory: &Factory) -> Constructor<Self> {
        factory.binary64
    }
}

impl Sample for f32 {
    const PRECISION: Precision = Precision::Binary32;
    const ZERO: Self = 0.0;

    fn from_f64(value: f64) -> Self {
        to_binary32(value)
    }

    fn to_f64(self) -> f64 {
        f64::from(self)
    }

    fn to_binary32(self) -> f32 {
        self
    }

    fn flush(self) -> Self {
        crate::flush::binary32(self)
    }

    fn sin(self) -> Self {
        libm::sinf(self)
    }

    fn floor(self) -> Self {
        libm::floorf(self)
    }

    fn constructor(factory: &Factory) -> Constructor<Self> {
        factory.binary32
    }
}

/// How a render of samples `S` is given to its host as samples of this type.
///
/// As they are, or a binary64 sample rounded to binary32 ([`Sample::to_binary32`]). A
/// binary32 render cannot be given as binary64: no [`RenderFormat`] is one.
pub trait Delivered<S: Sample>: sealed::Sealed + Copy {
    /// `sample`, given as this type.
    fn deliver(sample: S) -> Self;
}

impl<S: Sample> Delivered<S> for S {
    fn deliver(sample: S) -> Self {
        sample
    }
}

impl Delivered<f64> for f32 {
    fn deliver(sample: f64) -> Self {
        sample.to_binary32()
    }
}

#[cfg(test)]
#[path = "math_tests.rs"]
mod tests;
