//! The engine's math layer (`spec/engine-design.md`, decision 10): the sample type with its
//! operators, and the math functions, through which all box arithmetic goes.
//!
//! The engine computes in one of two **precisions**, chosen per render by the host:
//! IEEE 754 binary64 by default, binary32 as a setting. A box is written once, generic over
//! [`Sample`], and the plan instantiates it in the precision of the render; parameter values
//! and what boxes derive from them (increments, coefficients) are binary64 in both.
//!
//! The math functions are the pinned pure-Rust `libm`, the same bits on every target; no box
//! crate depends on `libm` outside its tests (xtask's math layer gate refuses it as a normal
//! or build dependency), and the methods of
//! `f32` and `f64` that compute them do not exist in the `no_std` crates boxes live in. So
//! a box reaches a math function only through this layer.
//!
//! The **arithmetic mode** of this build is [`MODE`], the strict default: every operation
//! correctly rounded, no fused multiply-add, `libm`. Other modes are build-time choices of
//! this layer's implementation that no build makes yet (decision 10); the render hash
//! records the mode (`spec/render-hash.md`).

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::borrow::Borrow;
use core::fmt;
use core::iter;
use core::ops::{Add, AddAssign, BitOr, Div, Mul, Neg, Sub, SubAssign};

use crate::{BLOCK_SIZE, Block, BoxInstance, Params, SampleRate};

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
/// A binary64 render is given as binary64, or rounded to binary32 for a host that takes
/// binary32 (the browser's audio thread, most device backends); a binary32 render is given
/// as binary32. A case has one reference hash per render format.
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
    ///
    /// Its order is also the format code of audio-wasm's C interface (the index), pinned
    /// there by literal tests: a new format is appended, never inserted.
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
///
/// Only [`ArithmeticMode::from_bits`] builds it, with the unknown bits alone: never zero,
/// never a known flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownArithmeticOptions(u64);

impl UnknownArithmeticOptions {
    /// The flags no option defines, alone: what a reader reports.
    #[must_use]
    pub const fn bits(self) -> u64 {
        self.0
    }
}

impl fmt::Display for UnknownArithmeticOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown arithmetic options: {:#018x}", self.0)
    }
}

impl core::error::Error for UnknownArithmeticOptions {}

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
pub const MODE: ArithmeticMode = ArithmeticMode(ArithmeticOption::FusedMultiplyAdd.flag());

/// The nearest binary32 value to `value`, the same bits on every target.
///
/// A number is rounded to nearest, ties to even (IEEE 754, which the conversion follows on
/// every target; a subnormal result is kept, under the default floating-point mode the
/// native shells set, and flushed by delivery, [`host::Delivered`]). A NaN, whose conversion IEEE 754 leaves open in its payload (and
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

/// A constructor of box instances computing in `S`: how a [`crate::Factory`] holds each
/// precision's, private to the engine. None when given another number of values than the
/// box type has parameters ([`Params::array`]).
pub(crate) type Constructor<S, const N: usize> =
    fn(SampleRate, Params<'_>) -> Option<Box<dyn BoxInstance<S, N>>>;

/// A binary64 value of the engine: a signal sample of a binary64 render, and in both
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

/// A binary32 value of the engine: a signal sample of a binary32 render. As [`Binary64`], its
/// operators and comparisons are the math layer's.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct Binary32(f32);

impl Binary64 {
    /// 2π, the nearest binary64.
    pub const TAU: Self = Self(core::f64::consts::TAU);

    /// One.
    pub const ONE: Self = Self(1.0);

    /// This value as a parameter: a zero of its sign below 2^-126 in magnitude
    /// ([`crate::flush::parameter`]).
    #[must_use]
    pub(crate) const fn as_parameter(self) -> Self {
        Self(crate::flush::parameter(self.0))
    }

    /// The value `value`, which a box writes as a literal: a constant, never a computed
    /// float. Box code computes no float outside its tests (`float_arithmetic` is forbidden
    /// at the box crate's root, a line xtask's math layer gate requires), so only a `const`
    /// item can give it a value computed by the compiler (`spec/engine-design.md`,
    /// decision 10).
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

/// Hides [`Sample`] and [`host::Delivered`] from implementations outside this crate: the engine
/// computes in binary64 and binary32 only.
pub(crate) mod sealed {
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

    /// Itself in binary64: exactly for every number; a NaN from its bits, the same on every
    /// target (its sign, the quiet bit set, its payload), since IEEE 754 and WebAssembly leave
    /// the payload of a converted NaN open.
    fn to_binary64(self) -> Binary64;

    /// The value of this type nearest to `value` on the side of zero: never larger in
    /// magnitude than `value` (rounded toward zero; `value` itself for [`Binary64`], the next
    /// binary32 toward zero from the nearest when the nearest is farther from zero for
    /// [`Binary32`], the largest finite one for a number beyond the format; a NaN as
    /// [`Sample::from_binary64`] gives it). A bound that must hold in this type once it
    /// holds in binary64 is rounded so: the limiter's ceiling and its gain.
    fn toward_zero(value: Binary64) -> Self;

    /// Itself, or a zero of its sign when it is subnormal (`spec/engine-design.md`,
    /// decision 11): decided from its bits, so that no hardware mode can change it.
    #[must_use]
    fn flush(self) -> Self;

    /// Whether it is a number: neither NaN nor infinite, decided from its bits.
    fn is_finite(self) -> bool;

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

    fn toward_zero(value: Binary64) -> Self {
        value
    }

    fn flush(self) -> Self {
        Self(crate::flush::binary64(self.0))
    }

    fn is_finite(self) -> bool {
        self.0.is_finite()
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
}

impl Sample for Binary32 {
    const PRECISION: Precision = Precision::Binary32;
    const ZERO: Self = Self(0.0);

    fn from_binary64(value: Binary64) -> Self {
        Self(to_binary32(value.0))
    }

    fn to_binary64(self) -> Binary64 {
        if self.0.is_nan() {
            let bits = u64::from(self.0.to_bits());
            let sign = (bits >> 31_u32) << 63_u32;
            let payload = (bits & 0x003f_ffff) << 29_u32;
            // Disjoint fields (the sign, the exponent and quiet bit, the payload), joined by
            // the trait's function: an operator here would read the same as `^` or `+`.
            Binary64(f64::from_bits(
                sign.bitor(0x7ff8_0000_0000_0000).bitor(payload),
            ))
        } else {
            Binary64(f64::from(self.0))
        }
    }

    fn toward_zero(value: Binary64) -> Self {
        let nearest = to_binary32(value.0);
        // A nearest farther from zero than `value` (never a zero nor a NaN, which compare
        // as nearer) steps one binary32 toward zero: its magnitude's bits less one, whatever
        // its sign, from an infinity to the largest finite value.
        if f64::from(nearest).abs() > value.0.abs() {
            Self(f32::from_bits(nearest.to_bits().saturating_sub(1)))
        } else {
            Self(nearest)
        }
    }

    fn flush(self) -> Self {
        Self(crate::flush::binary32(self.0))
    }

    fn is_finite(self) -> bool {
        self.0.is_finite()
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
}

/// The number of lanes of the reduction order: a constant of the specification
/// (`spec/engine-design.md`, decision 28), never the vector width of a machine.
const LANES: usize = 16;

/// How the reduction order combines its lanes: lane `i` with lane `i + 8`, then `i + 4`,
/// `i + 2` and `i + 1`, for `i` from 0 (decision 28).
const TREE: [usize; 4] = [8, 4, 2, 1];

// `TREE` halves `LANES` round by round down to 1: the two constants state one fact, which
// the compiler checks (the split points of `reduce`, `half - 1`, rely on it).
const _: () = {
    let [first, second, third, last] = TREE;
    assert!(
        first == LANES.div_euclid(2)
            && second == first.div_euclid(2)
            && third == second.div_euclid(2)
            && last == third.div_euclid(2)
            && last == 1,
        "TREE halves LANES down to one lane"
    );
};

/// What the reduction order sums: a sample, or a block of samples frame by frame.
trait Summand: Copy {
    /// Adds `other` to itself, frame by frame for a block.
    fn add(&mut self, other: &Self);
}

impl<S: Sample> Summand for S {
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "IEEE 754 addition of the sample type, which neither overflows nor panics"
    )]
    fn add(&mut self, other: &Self) {
        *self += *other;
    }
}

impl<S: Sample, const N: usize> Summand for Block<S, N> {
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "IEEE 754 addition of the sample type, which neither overflows nor panics"
    )]
    fn add(&mut self, other: &Self) {
        for (sum, term) in self.iter_mut().zip(other) {
            *sum += *term;
        }
    }
}

/// The reduction order of decision 28, once, for a sample and for a block alike: lane 0 is
/// `out`, lanes 1 to 15 are `rest`. A block's frames never meet, so frame `f` of a block
/// sum is the sample sum of the frames `f` of its terms, by construction.
///
/// Term `k` starts lane `k` (copied, never added to -0) for `k` below 16, then adds to lane
/// `k mod 16` in increasing `k`, `first` being term 0 and `terms` the others; the tree then
/// adds lane `i + half` to lane `i` over the lanes holding a term. The sum of no term at
/// all, +0, is its callers'.
fn reduce<L: Summand, T: Borrow<L>>(
    out: &mut L,
    rest: &mut [L; LANES - 1],
    first: T,
    terms: impl IntoIterator<Item = T>,
) {
    let mut terms = terms.into_iter();
    *out = *first.borrow();
    let mut used = 1_usize;
    // `zip` asks the lanes first, so a term is never taken without a lane for it.
    for (lane, term) in rest.iter_mut().zip(terms.by_ref()) {
        *lane = *term.borrow();
        used = used.saturating_add(1);
    }
    // Every lane holds a term: each next term adds to lane 0, the ones after it to lanes 1
    // to 15, in turn.
    while let Some(next) = terms.next() {
        out.add(next.borrow());
        for (lane, term) in rest.iter_mut().zip(terms.by_ref()) {
            lane.add(term.borrow());
        }
    }
    for half in TREE {
        // Lanes 0 to `half - 1` are `out` and `rest[..half - 1]`; lanes `half` on are
        // `rest[half - 1..]`, of which the first `used - half` hold a term.
        let (low, high) = rest.split_at_mut(half.saturating_sub(1));
        for (lane, other) in iter::once(&mut *out)
            .chain(low.iter_mut())
            .zip(high.iter().take(used.saturating_sub(half)))
        {
            lane.add(other);
        }
    }
}

/// Whether every sample of `block` is finite, decided once per block: the sum of every
/// sample times zero is zero (of either sign) when every sample is finite, and NaN as soon
/// as one is not (∞ × 0 and NaN × 0 are NaN, and NaN absorbs every sum), so that a block
/// costs a product and a sum per sample and one decision. Each product then sum is
/// [`Sample::mul_then_add`], exact in every arithmetic mode (a product by zero is exact, so
/// fusing it rounds nothing).
///
/// Never inlined: its one copy per sample type costs fewer instructions per sample than
/// the copy the compiler would inline into the engine's binary32 render, the one caller of
/// its binary32 copy (a binary64 render given as binary32 checks each sample as it rounds it,
/// in `narrowed_finite`; `spec/benchmarks.md`, "The check fused with the rounding").
#[inline(never)]
pub(crate) fn all_finite<S: Sample, const N: usize>(block: &Block<S, N>) -> bool {
    // Eight at a time, each eight unrolled (a fixed length): their loads take constant
    // offsets, and the loop runs eight times less; the frames after the last eight, as
    // many as the engine block leaves (a constant of the build), follow one by one.
    let (eights, rest) = block.as_chunks::<8>();
    let whole = eights.iter().fold(S::ZERO, |sum, eight| {
        eight
            .iter()
            .fold(sum, |partial, sample| sample.mul_then_add(S::ZERO, partial))
    });
    rest.iter()
        .fold(whole, |partial, sample| {
            sample.mul_then_add(S::ZERO, partial)
        })
        .is_finite()
}

/// The sum of `terms`, in the reduction order (`spec/engine-design.md`, decision 28).
///
/// Every sum of more than two terms of the engine and of boxes goes through it, so that
/// vectorizing it later, at any width dividing 16, never changes a bit.
///
/// The terms are numbered `k = 0 … n-1` in the order `terms` gives them, which the caller
/// fixes (cables in their fixed order, taps by index). Term `k` goes to lane `k mod 16`;
/// each of the 16 lanes sums its terms in increasing `k`, from -0; the lanes are then
/// combined by a fixed pairwise tree, lane `i` with lane `i + 8`, then `i + 4`, `i + 2`,
/// `i + 1`. A sum of no terms is +0.
///
/// Adding -0 changes no value (`x + -0` is `x`, -0 included), so a lane starts from its
/// first term and an empty lane is left out of the tree: the same bits as the rule for
/// every result but a NaN, which stays a NaN. So one term is returned bit for bit (a NaN
/// payload included), and two give their sum. The payload and sign of a NaN result are not
/// reproducible: natively the payload of one of the NaN terms (quieted) or the default NaN,
/// in WebAssembly any quiet NaN when a NaN term is not canonical, and either may change
/// with the build (the compiler may swap the operands of an addition); #24 step 2 decides
/// how sums treat NaN.
///
/// No other sum of samples is offered: the sample types implement neither `Sum` nor
/// `Product`, so an iterator of samples cannot be summed otherwise.
///
/// ```compile_fail,E0277
/// fn total(samples: &[audio_core::Binary64]) -> audio_core::Binary64 {
///     samples.iter().copied().sum()
/// }
/// ```
#[must_use]
pub fn ordered_sum<S: Sample>(terms: impl IntoIterator<Item = S>) -> S {
    let mut terms = terms.into_iter();
    let Some(first) = terms.next() else {
        return S::ZERO;
    };
    // `reduce` writes `first` into `out` itself (its term 0): the value `out` starts with is
    // never read, any sample would do.
    let (mut out, mut rest) = (first, [S::ZERO; LANES - 1]);
    reduce(&mut out, &mut rest, first, terms);
    out
}

/// The scratch lanes of the block form of the reduction order, 15 blocks, reserved once by
/// [`Lanes::new`] (a plan's build), so that summing blocks never allocates: one buffer on
/// the heap, read as its blocks, so that no block is ever built on the stack, whatever the
/// engine block.
#[derive(Debug)]
pub(crate) struct Lanes<S: Sample, const N: usize>(Vec<S>);

impl<S: Sample, const N: usize> Lanes<S, N> {
    /// Lanes of silence, on the heap, and one block of silence after them: the term a sum
    /// reads for a source it cannot find.
    pub(crate) fn new() -> Self {
        Self(alloc::vec![S::ZERO; LANES.saturating_mul(N)])
    }

    /// The lanes, as their 15 blocks, and the block of silence: always, the buffer holding
    /// exactly them (`new`), so that the reduction's tree runs over a fixed number of lanes.
    fn parts(&mut self) -> Option<(&mut [Block<S, N>; LANES - 1], &Block<S, N>)> {
        let (silence, lanes) = self.0.as_chunks_mut::<N>().0.split_last_mut()?;
        Some((lanes.try_into().ok()?, &*silence))
    }
}

/// The sum of the blocks `terms` frame by frame, in the reduction order of
/// [`ordered_sum`] (the same core, `reduce`): the block itself for one (bit for bit),
/// silence (+0) for none. The plan sums the cables into an inlet and the output boxes on a
/// channel with it, using `lanes` as scratch; a term it cannot find (none) is silence in
/// its place, the later terms keeping theirs.
pub(crate) fn sum_blocks<'terms, S: Sample + 'terms, const N: usize>(
    out: &mut Block<S, N>,
    lanes: &mut Lanes<S, N>,
    terms: impl Iterator<Item = Option<&'terms Block<S, N>>>,
) {
    // The lanes are always found (`Lanes::parts`): the sum always runs.
    for ((rest, silence), every) in lanes.parts().into_iter().zip(iter::once(terms)) {
        let mut every = every.map(|term| term.unwrap_or(silence));
        match every.next() {
            Some(first) => reduce(out, rest, first, every),
            // Filled in place, so that no block of zeros is held as a constant, whatever
            // the engine block.
            None => out.fill(S::ZERO),
        }
    }
}

/// The sum of the blocks `first` and `second` frame by frame, `first + second`: the
/// reduction order of [`ordered_sum`] for two terms (lane 0 holds `first`, lane 1 `second`,
/// and only the last step of the tree joins them), computed without scratch lanes. The plan
/// sums two cables into an inlet, or two output boxes on a channel, with it.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "IEEE 754 addition of the sample type, which neither overflows nor panics"
)]
pub(crate) fn add_two<S: Sample, const N: usize>(
    out: &mut Block<S, N>,
    first: &Block<S, N>,
    second: &Block<S, N>,
) {
    *out = *first;
    for (sum, term) in out.iter_mut().zip(second) {
        *sum += *term;
    }
}

/// Taps of each phase of the true-peak interpolator ([`TRUE_PEAK_FILTER`]).
pub const TRUE_PEAK_TAPS: usize = 12;

/// The true-peak interpolator of ITU-R BS.1770-4, Annex 2, as the standard publishes it.
///
/// 4x oversampling by a 48-tap FIR in 4 phases of [`TRUE_PEAK_TAPS`] taps. Tap `k` of a phase
/// multiplies the sample `k` frames before the newest. Every coefficient is a multiple of
/// 2^-13 below 1 in magnitude, so it is exact in binary32 as in binary64. Public for the
/// limiter's measurements, which derive its worst-case under-read from it.
pub const TRUE_PEAK_FILTER: [[f64; TRUE_PEAK_TAPS]; 4] = [
    [
        0.001_708_984_375,
        0.010_986_328_125,
        -0.019_653_320_312_5,
        0.033_203_125,
        -0.059_448_242_187_5,
        0.137_329_101_562_5,
        0.972_167_968_75,
        -0.102_294_921_875,
        0.047_607_421_875,
        -0.026_611_328_125,
        0.014_892_578_125,
        -0.008_300_781_25,
    ],
    [
        -0.029_174_804_687_5,
        0.029_296_875,
        -0.051_757_812_5,
        0.089_111_328_125,
        -0.166_503_906_25,
        0.465_087_890_625,
        0.779_785_156_25,
        -0.200_317_382_812_5,
        0.101_562_5,
        -0.058_227_539_062_5,
        0.033_081_054_687_5,
        -0.018_920_898_437_5,
    ],
    [
        -0.018_920_898_437_5,
        0.033_081_054_687_5,
        -0.058_227_539_062_5,
        0.101_562_5,
        -0.200_317_382_812_5,
        0.779_785_156_25,
        0.465_087_890_625,
        -0.166_503_906_25,
        0.089_111_328_125,
        -0.051_757_812_5,
        0.029_296_875,
        -0.029_174_804_687_5,
    ],
    [
        -0.008_300_781_25,
        0.014_892_578_125,
        -0.026_611_328_125,
        0.047_607_421_875,
        -0.102_294_921_875,
        0.972_167_968_75,
        0.137_329_101_562_5,
        -0.059_448_242_187_5,
        0.033_203_125,
        -0.019_653_320_312_5,
        0.010_986_328_125,
        0.001_708_984_375,
    ],
];

/// The magnitude of `value`: itself, or its negation when it is below zero (a zero keeps
/// its sign, which no comparison sees).
#[expect(
    clippy::arithmetic_side_effects,
    reason = "IEEE 754 negation of the sample type, exact"
)]
fn magnitude<S: Sample>(value: S) -> S {
    if value < S::ZERO { -value } else { value }
}

/// The larger of `first` and `second`, `first` when they compare equal: the larger of two
/// samples, two peaks among them.
pub(crate) fn larger<S: Sample>(first: S, second: S) -> S {
    if second > first { second } else { first }
}

/// The true-peak estimate of the interval at the middle of `window`, the last `taps`
/// samples of a channel, oldest first: the largest magnitude of its two middle samples
/// (`taps / 2 - 1` and `taps / 2`, the interval's ends) and of each phase of `filter`,
/// phase by phase, each holding `taps` coefficients in the order of `window` (its last
/// multiplying the newest sample), summed in the reduction order ([`ordered_sum`]).
///
/// Every sample of `window` is finite: the output stage replaces a non-finite one first.
/// A phase's sum can still overflow; it is then an infinite peak.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "IEEE 754 products of the sample type, which neither overflow into a panic nor panic"
)]
pub(crate) fn true_peak<S: Sample>(window: &[S], filter: &[S], taps: usize) -> S {
    let middle = taps / 2;
    let ends = window
        .get(middle.saturating_sub(1)..=middle)
        .unwrap_or_default()
        .iter()
        .fold(S::ZERO, |peak, sample| larger(peak, magnitude(*sample)));
    filter.chunks_exact(taps).fold(ends, |peak, phase| {
        let value = ordered_sum(phase.iter().zip(window).map(|(tap, sample)| *tap * *sample));
        // A sum that overflowed is beyond the format: an infinity, or NaN from both, which a
        // comparison would drop. Either counts as an infinite peak.
        if value.is_finite() {
            larger(peak, magnitude(value))
        } else {
            S::from_binary64(Binary64(f64::INFINITY))
        }
    })
}

/// The gain reduction a true peak `peak` needs to stay at `ceiling` (positive), plus
/// `margin`: the share of the signal to take away, `(peak - ceiling) / peak + margin`
/// above the ceiling, and exactly +0 at or below it, the share by one formula,
/// `(peak - ceiling) / max(peak, ceiling)`, whose divisor is never below the ceiling; and
/// all of it, exactly `1 + margin`, for an infinite peak (an estimate that overflowed,
/// [`true_peak`]), whose share the formula would make NaN. The margin covers the rounding
/// of the limiter's control path (`spec/measurements.md`, "The true-peak limiter").
#[expect(
    clippy::arithmetic_side_effects,
    reason = "IEEE 754 difference, quotient and sum of a peak, the positive ceiling and the margin, the divisor at least the ceiling"
)]
pub(crate) fn reduction_needed<S: Sample>(peak: S, ceiling: S, margin: S) -> S {
    let share = (peak - ceiling) / larger(peak, ceiling);
    let share = if share.is_finite() {
        share
    } else {
        S::from_binary64(Binary64::ONE)
    };
    if share > S::ZERO {
        share + margin
    } else {
        S::ZERO
    }
}

/// The least value the limiter's control path keeps, `T = 2^-100`: a release stage, a
/// share, or a reduction applied below it is zero ([`kept`]). Every positive need is at least
/// the margin, `64 × 2^-53` or more (`limiter::margin`), so `T` lies a factor `2^53` below the
/// smallest of them, below every reduction that covers a need, and more than `2^900` above
/// the subnormal numbers, so that no product or sum the control path computes from values
/// zero or at least `T` is subnormal (`spec/measurements.md`, "The true-peak limiter",
/// step 7).
pub(crate) const LEAST_KEPT: Binary64 = Binary64(7.888_609_052_210_118e-31);

/// `value`, a value of the limiter's control path, or zero, `+0`, below [`LEAST_KEPT`]: so
/// that the control path never reaches the subnormal numbers, which a host flushing them
/// would compute otherwise (decision 11), and a release ends at exactly zero.
pub(crate) fn kept<S: Sample>(value: S) -> S {
    if value < S::from_binary64(LEAST_KEPT) {
        S::ZERO
    } else {
        value
    }
}

/// The reduction `reduction` kept for one more frame: `reduction × keep` (`keep` below 1),
/// zero below [`LEAST_KEPT`], so that a release decays to exactly zero.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "IEEE 754 product of the sample type, between zero and one"
)]
pub(crate) fn decayed<S: Sample>(reduction: S, keep: S) -> S {
    kept(reduction * keep)
}

/// `reduction` moved toward `target` by the share `step` of their difference (a one-pole
/// approach, `step` between zero and one), zero below [`LEAST_KEPT`].
#[expect(
    clippy::arithmetic_side_effects,
    reason = "IEEE 754 difference, product and sum of samples between zero and one"
)]
pub(crate) fn approached<S: Sample>(reduction: S, target: S, step: S) -> S {
    kept((target - reduction).mul_then_add(step, reduction))
}

/// The larger of two reductions.
pub(crate) fn larger_reduction<S: Sample>(first: S, second: S) -> S {
    larger(first, second)
}

/// `value × by`, zero below [`LEAST_KEPT`]: the reduction the crossfade between raw and
/// protected applies, so that it is zero or at least `T` on every host (decision 11).
#[expect(
    clippy::arithmetic_side_effects,
    reason = "IEEE 754 product of finite reductions"
)]
pub(crate) fn crossfaded<S: Sample>(value: S, by: S) -> S {
    kept(value * by)
}

/// The sliding sums of a Hann window over the last frames of a reduction: the plain sum,
/// and the sum weighted by the unit phasor turning one step of the window per frame (its
/// real and imaginary parts). The Hann-weighted sum is half their difference ([`Self::smoothed`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct HannSums<S> {
    /// The plain sum.
    pub(crate) sum: S,
    /// The real part of the turning sum.
    pub(crate) real: S,
    /// Its imaginary part.
    pub(crate) imaginary: S,
}

impl<S: Sample> HannSums<S> {
    /// Sums of a window of zeros.
    pub(crate) const ZERO: Self = Self {
        sum: S::ZERO,
        real: S::ZERO,
        imaginary: S::ZERO,
    };

    /// The sums one frame later: `newest` enters the window and `oldest` leaves it;
    /// `turn` (`cos`, `sin`) is the phasor's step. With `c = cos + i sin`, the turning sum
    /// `C` becomes `c × (C + newest) - oldest`, since `c` turned a whole window is 1.
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "IEEE 754 sums and products of finite samples bounded by the window's length"
    )]
    #[must_use]
    pub(crate) fn advanced(self, newest: S, oldest: S, turn: (S, S)) -> Self {
        let (cos, sin) = turn;
        let real = self.real + newest;
        Self {
            sum: self.sum + newest - oldest,
            real: real * cos - self.imaginary * sin - oldest,
            imaginary: real * sin + self.imaginary * cos,
        }
    }

    /// The sums with `value` added at the position whose phasor is `turn` (`cos`, `sin` of
    /// the phasor turned `k + 1` steps for the position `k`, 0 the newest): the plain sum
    /// plus `value`, the turning sum's parts plus `value × cos` and `value × sin`. Summing a
    /// window's values one by one, each at the position it holds when the sum is complete,
    /// gives its exact sums in the order they are added, which a recurrence over frames
    /// fixes (one term per frame, never a sum taken at once).
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "IEEE 754 products and sums of finite samples"
    )]
    #[must_use]
    pub(crate) fn added(self, value: S, turn: (S, S)) -> Self {
        let (cos, sin) = turn;
        Self {
            sum: self.sum + value,
            real: self.real + value * cos,
            imaginary: self.imaginary + value * sin,
        }
    }

    /// The Hann-weighted mean of the window: `(sum - real) × scale`, where `scale` is one
    /// over the window's length plus one (the weights `1 - cos(2π (k + 1) / (L + 1))`, for
    /// `k` from 0 to `L - 1`, sum to `L + 1`).
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "IEEE 754 difference and product of finite samples"
    )]
    pub(crate) fn smoothed(self, scale: S) -> S {
        (self.sum - self.real) * scale
    }
}

/// The gain of a reduction: `1 - reduction`, rounded, and never below +0 (a reduction
/// above one, a need's margin past all of the signal, takes all of it).
#[expect(
    clippy::arithmetic_side_effects,
    reason = "IEEE 754 difference of finite reductions"
)]
pub(crate) fn gain<S: Sample>(reduction: S) -> S {
    larger(S::ZERO, S::from_binary64(Binary64::ONE) - reduction)
}

/// `sample` times `gain`, flushed, since a small normal sample reduced may become subnormal
/// and delivery gives none (decision 11). The caller passes the sample itself through when
/// there is no reduction, so that it keeps its bits.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "IEEE 754 product of finite samples"
)]
pub(crate) fn with_gain<S: Sample>(sample: S, gain: S) -> S {
    (sample * gain).flush()
}

/// `1 - value`, for a share between zero and one.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "IEEE 754 difference of finite samples between zero and one"
)]
pub(crate) fn complement<S: Sample>(value: S) -> S {
    S::from_binary64(Binary64::ONE) - value
}

/// The limiter's coefficients, computed in binary64 when it is built or a setting changes,
/// never per sample.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "IEEE 754 binary64 arithmetic on settings within their checked ranges and on rates and frame counts far from overflow"
)]
pub(crate) mod limiter {
    use alloc::vec::Vec;

    use super::Binary64;
    use crate::SampleRate;

    /// Frames in `milliseconds` at `rate`, rounded to the nearest, at least one.
    pub(crate) fn frames(milliseconds: Binary64, rate: SampleRate) -> usize {
        let count = (milliseconds * Binary64::from(rate) / Binary64(1000.0))
            .rounded_count()
            .and_then(|count| usize::try_from(count).ok())
            .unwrap_or(usize::MAX);
        count.max(1)
    }

    /// The share a one-pole with a time constant of `milliseconds` at `rate` keeps per
    /// frame: `exp(-1 / frames)`.
    pub(crate) fn keep(milliseconds: Binary64, rate: SampleRate) -> Binary64 {
        let frames = milliseconds * Binary64::from(rate) / Binary64(1000.0);
        (-(Binary64::ONE / frames)).exp()
    }

    /// `1 - keep`, the share of the gap a one-pole closes per frame.
    pub(crate) fn step(keep: Binary64) -> Binary64 {
        Binary64::ONE - keep
    }

    /// The amplitude of `decibels`: `10^(decibels / 20)`, as `exp(decibels × ln 10 / 20)`.
    pub(crate) fn amplitude(decibels: Binary64) -> Binary64 {
        (decibels * Binary64(core::f64::consts::LN_10 / 20.0)).exp()
    }

    /// The fast release time: the release and its `divisor`-th mixed by the adaptivity,
    /// `release × (1 - adaptivity) + release / divisor × adaptivity`, exactly the release at
    /// an adaptivity of 0 and exactly its `divisor`-th at 1.
    pub(crate) fn fast_release(
        release: Binary64,
        adaptivity: Binary64,
        divisor: Binary64,
    ) -> Binary64 {
        release * (Binary64::ONE - adaptivity) + release / divisor * adaptivity
    }

    /// The phasor turned `steps` steps of a window of `length` frames, plus one: the cosine
    /// and sine of `2π steps / (length + 1)`.
    pub(crate) fn turned(steps: usize, length: usize) -> (Binary64, Binary64) {
        let angle = Binary64::TAU * Binary64::from_count(steps)
            / Binary64::from_count(length.saturating_add(1));
        (angle.cos(), angle.sine())
    }

    /// The frames of a limiter's rest under a release whose slow stage closes `share` of the
    /// gap per frame: `71 / share` rounded to the nearest, plus 2, about 71 time constants of
    /// the release, within which every release stage of every window decays to zero once no
    /// window holds a need (`spec/measurements.md`, "The true-peak limiter", step 6, which
    /// derives it with the slack it leaves).
    pub(crate) fn rest_frames(share: Binary64) -> u64 {
        (Binary64(71.0) / share)
            .rounded_count()
            .map_or(u64::MAX, |count| u64::from(count).saturating_add(2))
    }

    /// One over `length + 1`.
    pub(crate) fn scale(length: usize) -> Binary64 {
        Binary64::ONE / Binary64::from_count(length.saturating_add(1))
    }

    /// The margin per frame of the lookahead: 64 units of 2^-53, `2^-47`.
    const MARGIN_PER_FRAME: Binary64 = Binary64(1.0 / 140_737_488_355_328.0);

    /// The margin the limiter adds to every need above the ceiling, for a window of at most
    /// `frames` frames: `64 × frames × 2^-53`, exactly (a count times a power of two). It
    /// exceeds every rounding of the control path between a need and the gain it yields,
    /// `(43 frames + 3) × 2^-53` (`spec/measurements.md`, "The true-peak limiter").
    pub(crate) fn margin(frames: usize) -> Binary64 {
        Binary64::from_count(frames) * MARGIN_PER_FRAME
    }

    /// Terms of the series of [`bessel_i0`]: enough for every argument up to 10, the terms
    /// left out then below 2^-60 of the sum. Each term is the one before times
    /// `(x/2)² / k²`, at most `25 / 49² < 1/96` past the 48th at `x ≤ 10`, so what is left
    /// out is under `96/95` times the first term left out: 3.0·10^-61 of the sum at `x = 10`
    /// (exact fractions), 9.9·10^-89 at 5 (`computes_the_bessel_function_of_the_kaiser_window`
    /// checks the sum at 10).
    const BESSEL_TERMS: u32 = 48;

    /// The modified Bessel function of the first kind and order zero, `I0(x)`, by its series
    /// `Σ ((x/2)^k / k!)^2`, summed term by term in increasing `k`: the Kaiser window's.
    pub(crate) fn bessel_i0(x: Binary64) -> Binary64 {
        let half = x / Binary64(2.0);
        let mut term = Binary64::ONE;
        let mut sum = Binary64::ONE;
        for order in 1..=BESSEL_TERMS {
            let ratio = half / Binary64(f64::from(order));
            term = term * ratio * ratio;
            sum += term;
        }
        sum
    }

    /// The interpolator of `phases` phases of `taps` taps for oversampling by `phases`: a
    /// sinc cut at half the input rate, under a Kaiser window of `beta` spanning its
    /// `phases × taps` points (`I0(β √(1 - r²))`, its scale `1 / I0(β)` cancelled by what
    /// follows), each phase then scaled to a unit gain at DC. Phase by phase,
    /// tap `k` multiplying the sample `k` frames before the newest (as
    /// [`super::TRUE_PEAK_FILTER`]). `phases × taps` even, so that no point lies at the
    /// sinc's centre.
    pub(crate) fn interpolator(phases: usize, taps: usize, beta: Binary64) -> Vec<Binary64> {
        let points = phases.saturating_mul(taps);
        let centre = Binary64::from_count(points.saturating_sub(1)) / Binary64(2.0);
        let point = |index: usize| {
            let offset = Binary64::from_count(index) - centre;
            let input_frames = offset / Binary64::from_count(phases);
            let angle = Binary64(core::f64::consts::PI) * input_frames;
            let ratio = offset / centre;
            let width = Binary64::ONE - ratio * ratio;
            // The window's scale, 1 / I0(β), is left out: the phases are scaled to a unit
            // gain at DC below, which would cancel it.
            angle.sine() / angle * bessel_i0(beta * width.square_root())
        };
        let mut coefficients = Vec::with_capacity(points);
        for phase in 0..phases {
            let raw: Vec<Binary64> = (0..taps)
                .map(|tap| point(tap.saturating_mul(phases).saturating_add(phase)))
                .collect();
            let gain = crate::math::ordered_sum(raw.iter().copied());
            coefficients.extend(raw.into_iter().map(|coefficient| coefficient / gain));
        }
        coefficients
    }
}

impl Binary64 {
    /// e raised to `self` (`libm`): the limiter's coefficients, computed when it is built
    /// or a setting changes, never per sample.
    #[must_use]
    pub(crate) fn exp(self) -> Self {
        Self(libm::exp(self.0))
    }

    /// The cosine of `self` radians (`libm`).
    #[must_use]
    pub(crate) fn cos(self) -> Self {
        Self(libm::cos(self.0))
    }

    /// The sine of `self` radians (`libm`).
    #[must_use]
    pub(crate) fn sine(self) -> Self {
        Self(libm::sin(self.0))
    }

    /// The square root of `self`, correctly rounded (IEEE 754).
    #[must_use]
    pub(crate) fn square_root(self) -> Self {
        Self(libm::sqrt(self.0))
    }

    /// `count`, exactly up to 2^53 (counts of frames are far below), saturating at
    /// `u32::MAX`.
    #[must_use]
    pub(crate) fn from_count(count: usize) -> Self {
        Self(u32::try_from(count).map_or_else(|_| f64::from(u32::MAX), f64::from))
    }

    /// `self` rounded to the nearest whole number (halves away from zero, `libm`), as a
    /// count: `None` when it is not a number from 0 to `u32::MAX` once rounded. Read from
    /// the bits of the rounded value, exactly, with no cast.
    #[must_use]
    pub(crate) fn rounded_count(self) -> Option<u32> {
        let rounded = libm::round(self.0);
        if !(rounded >= 0.0_f64 && rounded <= f64::from(u32::MAX)) {
            return None;
        }
        if rounded < 1.0_f64 {
            return Some(0);
        }
        // A whole number from 1 to 2^32 - 1: 1.f × 2^e with e from 0 to 31, so its value is
        // the 53 bits of 1.f shifted right by 52 - e.
        let bits = rounded.to_bits();
        let exponent = (bits >> 52_u32) & 0x7ff;
        // The implicit bit set, the sign and exponent cleared.
        let significand = (bits | 0x0010_0000_0000_0000) & 0x001f_ffff_ffff_ffff;
        let shift = 1075_u64.checked_sub(exponent)?;
        u32::try_from(significand.checked_shr(u32::try_from(shift).ok()?)?).ok()
    }
}

/// What a host, never a box, needs of the engine's samples: their floats and their bits,
/// to give them to the platform, hash them, read them back and test them.
///
/// A box crate does not name this module: xtask's math layer gate refuses it, with `f32`
/// and `f64`, in a box crate's code (tests apart), so a box cannot leave the math layer.
pub mod host {
    use super::{Binary32, Binary64, Sample};

    pub use crate::abi::{
        BUFFER_ALIGNMENT, BufferRun, CONTRACT_VERSION, ContractVersion, CopyError, Crossing,
        Header, HeaderError, RunError, SUPPORTED_VERSIONS, VersionError,
    };
    pub use crate::description::{
        DESCRIPTION_BYTES_MAX, DescribedParam, DescribedPort, Description, DescriptionError, Fault,
        Field, PORTS_MAX, PortKind, RESERVED_PORT_KINDS, Response,
    };

    /// How a render of samples `S` is given to its host, as floats of this type.
    ///
    /// A binary64 render as binary64 or rounded to the nearest binary32 (ties to even; a NaN
    /// by its bits), a binary32 render as binary32. Delivery is a flush point
    /// (`spec/engine-design.md`, decision 11): a sample that is subnormal in the delivered
    /// type, after the rounding, is given as a zero of its sign, so that a host's
    /// flush-to-zero or denormals-are-zero mode, which would read it as zero, changes no bit
    /// by reading it. Such a mode in force on the thread that rounds can still change one on
    /// aarch64, at the rounding to binary32, before this flush (decision 11). A binary32
    /// render cannot be given as binary64: no [`RenderFormat`](super::RenderFormat) is one.
    /// A host concern, here so that box code, which may not name this module, cannot take a
    /// sample's float out through it. It is not reachable from the crate root:
    ///
    /// ```compile_fail,E0432
    /// use audio_core::Delivered;
    /// ```
    pub trait Delivered<S: Sample>: super::sealed::Sealed + Copy {
        /// Whether this type is narrower than `S`, so that giving a sample rounds it: the
        /// output stage then rounds its limiter's ceiling toward zero in this type, so that
        /// no sample rounds above it (decision 12).
        const NARROWS: bool = false;

        /// `sample`, given as this type: rounded if this type is narrower, then flushed.
        fn deliver(sample: S) -> Self;

        /// `sample`, known not to be subnormal in its own precision (a channel that is not a
        /// sum, [`crate::Plan::channel_is_sum`]), given as this type: what [`deliver`] gives
        /// for it, without the flush where it cannot change a bit. In the same precision
        /// the sample is given as it is (flushing a value that is not subnormal leaves it
        /// unchanged); rounded to binary32, it is flushed as [`deliver`] does, since the
        /// rounding can make any value below 2^-126 subnormal.
        ///
        /// [`deliver`]: Delivered::deliver
        fn deliver_unsummed(sample: S) -> Self;

        /// Replaces by zero each sample of `given` (samples given from ones the output stage
        /// left finite) that is not finite, calling `replaced` with its index in `given`: only
        /// a type narrower than `S` can make one so, rounding a binary64 value beyond the
        /// largest binary32 one to infinity, and the stage then replaces it as it is given
        /// (decision 12). A type as wide as `S` never does: it does nothing, and costs
        /// nothing.
        fn replace_overflowed<'given>(
            _given: impl Iterator<Item = &'given mut Self>,
            _replaced: impl FnMut(usize),
        ) where
            Self: 'given,
        {
        }
    }

    impl Delivered<Binary64> for f64 {
        fn deliver(sample: Binary64) -> Self {
            crate::flush::binary64(sample.0)
        }

        fn deliver_unsummed(sample: Binary64) -> Self {
            sample.0
        }
    }

    impl Delivered<Binary32> for f32 {
        fn deliver(sample: Binary32) -> Self {
            crate::flush::binary32(sample.0)
        }

        fn deliver_unsummed(sample: Binary32) -> Self {
            sample.0
        }
    }

    impl Delivered<Binary64> for f32 {
        const NARROWS: bool = true;

        fn deliver(sample: Binary64) -> Self {
            crate::flush::binary32(super::to_binary32(sample.0))
        }

        fn deliver_unsummed(sample: Binary64) -> Self {
            Self::deliver(sample)
        }

        fn replace_overflowed<'given>(
            given: impl Iterator<Item = &'given mut Self>,
            mut replaced: impl FnMut(usize),
        ) where
            Self: 'given,
        {
            for (index, sample) in given.enumerate() {
                if !sample.is_finite() {
                    *sample = Self::deliver(Binary64::ZERO);
                    replaced(index);
                }
            }
        }
    }

    /// The values of a parameter's description, as its box type writes them: for the
    /// engine's peers that check or show box types (the laws, an editor), never a box.
    pub trait ParamBounds {
        /// Value used when the patch does not set it.
        fn default_value(&self) -> f64;
        /// Lowest accepted value.
        fn minimum(&self) -> f64;
        /// Highest accepted value.
        fn maximum(&self) -> f64;
    }

    impl ParamBounds for crate::ParamSpec {
        fn default_value(&self) -> f64 {
            self.default_value
        }

        fn minimum(&self) -> f64 {
            self.minimum
        }

        fn maximum(&self) -> f64 {
            self.maximum
        }
    }

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

#[cfg(test)]
#[path = "reduction_tests.rs"]
mod reduction_tests;
