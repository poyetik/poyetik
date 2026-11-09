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
use core::ops::{Add, AddAssign, BitOr, Div, Mul, Neg, Sub, SubAssign};

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
    /// Computed in binary64, each sample then rounded to the nearest binary32 (ties to
    /// even; a NaN by its bits).
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
#[cfg(not(poyetik_math = "fused"))]
pub const MODE: ArithmeticMode = ArithmeticMode::STRICT;

/// The arithmetic mode of a test build with the fused multiply-add option, which proves the
/// option a hook of the math layer alone (xtask's fused build); no engine is built so.
#[cfg(poyetik_math = "fused")]
pub const MODE: ArithmeticMode = ArithmeticMode(1);

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
pub(crate) fn to_binary32(value: f64) -> f32 {
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
pub type Constructor<S> = fn(SampleRate, &[Binary64]) -> Box<dyn Object<S>>;

/// A binary64 value of the engine: a signal sample of a 64-bit render, and in both
/// precisions a parameter value and what boxes derive from it (increments, coefficients,
/// phases).
///
/// Its operators and comparisons are the math layer's, and nothing else can reach its
/// float: a box computes with it only through this layer, which a build-time arithmetic
/// option changes without a box changing (`spec/engine-design.md`, decision 10). A host
/// reads and writes its bits through [`host::Raw`].
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct Binary64(f64);

/// A binary32 value of the engine: a signal sample of a 32-bit render. As [`Binary64`], its
/// operators and comparisons are the math layer's.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct Binary32(f32);

impl Binary64 {
    /// 2π, the nearest binary64.
    pub const TAU: Self = Self(core::f64::consts::TAU);

    /// One.
    pub const ONE: Self = Self(1.0);

    /// The value `value`, which a box writes as a literal: a constant, never a computed
    /// float (a box has none to give).
    #[must_use]
    pub const fn new(value: f64) -> Self {
        Self(value)
    }
}

impl From<SampleRate> for Binary64 {
    /// The rate in hertz, exactly.
    fn from(rate: SampleRate) -> Self {
        Self(f64::from(rate.hz()))
    }
}

impl Add for Binary64 {
    type Output = Self;

    /// The IEEE 754 `+` of the two floats, correctly rounded: the strict mode's.
    #[inline]
    fn add(self, other: Self) -> Self {
        Self(self.0 + other.0)
    }
}

impl Sub for Binary64 {
    type Output = Self;

    /// The IEEE 754 `-` of the two floats, correctly rounded: the strict mode's.
    #[inline]
    fn sub(self, other: Self) -> Self {
        Self(self.0 - other.0)
    }
}

impl Mul for Binary64 {
    type Output = Self;

    /// The IEEE 754 `*` of the two floats, correctly rounded: the strict mode's.
    #[inline]
    fn mul(self, other: Self) -> Self {
        Self(self.0 * other.0)
    }
}

impl Div for Binary64 {
    type Output = Self;

    /// The IEEE 754 `/` of the two floats, correctly rounded: the strict mode's.
    #[inline]
    fn div(self, other: Self) -> Self {
        Self(self.0 / other.0)
    }
}

impl Neg for Binary64 {
    type Output = Self;

    /// The negation: the sign flipped, exact.
    #[inline]
    fn neg(self) -> Self {
        Self(-self.0)
    }
}

impl AddAssign for Binary64 {
    /// The IEEE 754 `+=`, correctly rounded: the strict mode's.
    #[inline]
    fn add_assign(&mut self, other: Self) {
        self.0 += other.0;
    }
}

impl SubAssign for Binary64 {
    /// The IEEE 754 `-=`, correctly rounded: the strict mode's.
    #[inline]
    fn sub_assign(&mut self, other: Self) {
        self.0 -= other.0;
    }
}

impl Add for Binary32 {
    type Output = Self;

    /// The IEEE 754 `+` of the two floats, correctly rounded: the strict mode's.
    #[inline]
    fn add(self, other: Self) -> Self {
        Self(self.0 + other.0)
    }
}

impl Sub for Binary32 {
    type Output = Self;

    /// The IEEE 754 `-` of the two floats, correctly rounded: the strict mode's.
    #[inline]
    fn sub(self, other: Self) -> Self {
        Self(self.0 - other.0)
    }
}

impl Mul for Binary32 {
    type Output = Self;

    /// The IEEE 754 `*` of the two floats, correctly rounded: the strict mode's.
    #[inline]
    fn mul(self, other: Self) -> Self {
        Self(self.0 * other.0)
    }
}

impl Div for Binary32 {
    type Output = Self;

    /// The IEEE 754 `/` of the two floats, correctly rounded: the strict mode's.
    #[inline]
    fn div(self, other: Self) -> Self {
        Self(self.0 / other.0)
    }
}

impl Neg for Binary32 {
    type Output = Self;

    /// The negation: the sign flipped, exact.
    #[inline]
    fn neg(self) -> Self {
        Self(-self.0)
    }
}

impl AddAssign for Binary32 {
    /// The IEEE 754 `+=`, correctly rounded: the strict mode's.
    #[inline]
    fn add_assign(&mut self, other: Self) {
        self.0 += other.0;
    }
}

impl SubAssign for Binary32 {
    /// The IEEE 754 `-=`, correctly rounded: the strict mode's.
    #[inline]
    fn sub_assign(&mut self, other: Self) {
        self.0 -= other.0;
    }
}

/// Hides [`Sample`] and [`Delivered`] from implementations outside this crate: the engine
/// computes in binary64 and binary32 only.
mod sealed {
    /// The sample types, and the floats a render is given in.
    pub trait Sealed {}
    impl Sealed for super::Binary64 {}
    impl Sealed for super::Binary32 {}
    impl Sealed for f64 {}
    impl Sealed for f32 {}
}

/// A type the engine computes signals in: [`Binary64`] or [`Binary32`], nothing else. Its
/// operators are IEEE 754's, each correctly rounded; its math functions are the pinned
/// `libm`'s; all are the math layer's.
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
    + SubAssign
    + 'static
{
    /// The precision this type computes in.
    const PRECISION: Precision;

    /// Positive zero.
    const ZERO: Self;

    /// A block of silence: positive zeros.
    const SILENCE: Block<Self> = [Self::ZERO; BLOCK_SIZE];

    /// The nearest value of this type to `value` (rounded to nearest for [`Binary32`], `value`
    /// itself for [`Binary64`]).
    fn from_binary64(value: Binary64) -> Self;

    /// Itself in binary64, exactly.
    fn to_binary64(self) -> Binary64;

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

    /// `self × mul + add`, the operation every box writes for a product then a sum: in the
    /// strict mode, the product rounded, then the sum rounded, exactly as `self * mul + add`.
    /// A fused multiply-add option (decision 10) changes this function alone.
    #[must_use]
    fn mul_then_add(self, mul: Self, add: Self) -> Self;

    /// The constructor of `factory` for this type.
    fn constructor(factory: &Factory) -> Constructor<Self>;
}

impl Sample for Binary64 {
    const PRECISION: Precision = Precision::Binary64;
    const ZERO: Self = Self(0.0);

    fn from_binary64(value: Binary64) -> Self {
        value
    }

    fn to_binary64(self) -> Binary64 {
        self
    }

    fn flush(self) -> Self {
        Self(crate::flush::binary64(self.0))
    }

    fn sin(self) -> Self {
        Self(libm::sin(self.0))
    }

    fn floor(self) -> Self {
        Self(libm::floor(self.0))
    }

    fn mul_then_add(self, mul: Self, add: Self) -> Self {
        // The strict mode: the product rounded, then the sum rounded.
        #[cfg(not(poyetik_math = "fused"))]
        let result = Self(self.0 * mul.0 + add.0);
        // The fused test build (xtask): one rounding, by the pinned `libm`'s `fma`.
        #[cfg(poyetik_math = "fused")]
        let result = Self(libm::fma(self.0, mul.0, add.0));
        result
    }

    fn constructor(factory: &Factory) -> Constructor<Self> {
        factory.binary64
    }
}

impl Sample for Binary32 {
    const PRECISION: Precision = Precision::Binary32;
    const ZERO: Self = Self(0.0);

    fn from_binary64(value: Binary64) -> Self {
        Self(to_binary32(value.0))
    }

    fn to_binary64(self) -> Binary64 {
        Binary64(f64::from(self.0))
    }

    fn flush(self) -> Self {
        Self(crate::flush::binary32(self.0))
    }

    fn sin(self) -> Self {
        Self(libm::sinf(self.0))
    }

    fn floor(self) -> Self {
        Self(libm::floorf(self.0))
    }

    fn mul_then_add(self, mul: Self, add: Self) -> Self {
        // The strict mode: the product rounded, then the sum rounded.
        #[cfg(not(poyetik_math = "fused"))]
        let result = Self(self.0 * mul.0 + add.0);
        // The fused test build (xtask): one rounding, by the pinned `libm`'s `fmaf`.
        #[cfg(poyetik_math = "fused")]
        let result = Self(libm::fmaf(self.0, mul.0, add.0));
        result
    }

    fn constructor(factory: &Factory) -> Constructor<Self> {
        factory.binary32
    }
}

/// How a render of samples `S` is given to its host, as floats of this type.
///
/// A binary64 render as binary64 or rounded to the nearest binary32 (ties to even; a NaN
/// by its bits), a binary32 render as binary32. A binary32 render cannot be given as binary64: no [`RenderFormat`]
/// is one.
pub trait Delivered<S: Sample>: sealed::Sealed + Copy {
    /// `sample`, given as this type.
    fn deliver(sample: S) -> Self;
}

impl Delivered<Binary64> for f64 {
    fn deliver(sample: Binary64) -> Self {
        sample.0
    }
}

impl Delivered<Binary32> for f32 {
    fn deliver(sample: Binary32) -> Self {
        sample.0
    }
}

impl Delivered<Binary64> for f32 {
    fn deliver(sample: Binary64) -> Self {
        to_binary32(sample.0)
    }
}

/// What a host, never a box, needs of the engine's samples: their floats and their bits,
/// to give them to the platform, hash them, read them back and test them.
///
/// A box crate does not name this module: xtask's math layer gate refuses it, with `f32`
/// and `f64`, in a box crate's code (tests apart), so a box cannot leave the math layer.
pub mod host {
    use super::{Binary32, Binary64, Sample};

    /// The float of a sample, in and out.
    pub trait Raw: Sample {
        /// Its float: `f64` or `f32`.
        type Float: Copy;

        /// The sample holding `value`.
        fn from_raw(value: Self::Float) -> Self;

        /// Its float.
        fn raw(self) -> Self::Float;

        /// Its bit pattern, widened to 64 bits: `-0.0` and NaN payloads compared exactly.
        fn bits(self) -> u64;
    }

    impl Raw for Binary64 {
        type Float = f64;

        fn from_raw(value: f64) -> Self {
            Self(value)
        }

        fn raw(self) -> f64 {
            self.0
        }

        fn bits(self) -> u64 {
            self.0.to_bits()
        }
    }

    impl Raw for Binary32 {
        type Float = f32;

        fn from_raw(value: f32) -> Self {
            Self(value)
        }

        fn raw(self) -> f32 {
            self.0
        }

        fn bits(self) -> u64 {
            u64::from(self.0.to_bits())
        }
    }
}

#[cfg(test)]
#[path = "math_tests.rs"]
mod tests;
