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
use core::borrow::Borrow;
use core::fmt;
use core::iter;
use core::ops::{Add, AddAssign, BitOr, Div, Mul, Neg, Sub, SubAssign};

use crate::{BLOCK_SIZE, Block, Factory, Object, Params, SampleRate};

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
    /// Its order is also the format code of engine-wasm's C interface (the index), pinned
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

/// A constructor of box instances computing in `S`: how a [`Factory`] holds each
/// precision's, private to the engine.
pub(crate) type Constructor<S> = fn(SampleRate, Params<'_>) -> Box<dyn Object<S>>;

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
    use super::{Constructor, Factory, Sample};

    /// The sample types, and the floats a render is given in.
    pub trait Sealed {}
    impl Sealed for super::Binary64 {}
    impl Sealed for super::Binary32 {}
    impl Sealed for f64 {}
    impl Sealed for f32 {}

    /// How a sample type picks its constructor from a [`Factory`]: a supertrait of
    /// [`Sample`] no other crate can name, so that [`Factory::create`] stays the one way
    /// to create an instance.
    pub trait Construct {
        /// The constructor of `factory` for this type.
        fn constructor(factory: &Factory) -> Constructor<Self>
        where
            Self: Sample;
    }

    impl Construct for super::Binary64 {
        fn constructor(factory: &Factory) -> Constructor<Self> {
            factory.binary64
        }
    }

    impl Construct for super::Binary32 {
        fn constructor(factory: &Factory) -> Constructor<Self> {
            factory.binary32
        }
    }
}

/// A type the engine computes signals in: [`Binary64`] or [`Binary32`], nothing else. Its
/// operators are IEEE 754's, each correctly rounded; its math functions are the pinned
/// `libm`'s; all are the math layer's.
pub trait Sample:
    sealed::Sealed
    + sealed::Construct
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
    /// The sum of no terms: +0, or a block of +0.
    const NOTHING: Self;
    /// Adds `other` to itself, frame by frame for a block.
    fn add(&mut self, other: &Self);
}

impl<S: Sample> Summand for S {
    const NOTHING: Self = S::ZERO;

    #[expect(
        clippy::arithmetic_side_effects,
        reason = "IEEE 754 addition of the sample type, which neither overflows nor panics"
    )]
    fn add(&mut self, other: &Self) {
        *self += *other;
    }
}

impl<S: Sample> Summand for Block<S> {
    const NOTHING: Self = S::SILENCE;

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
/// `k mod 16` in increasing `k`; no term at all gives `NOTHING`; the tree then adds lane
/// `i + half` to lane `i` over the lanes holding a term.
fn reduce<L: Summand, T: Borrow<L>>(
    out: &mut L,
    rest: &mut [L; LANES - 1],
    terms: impl IntoIterator<Item = T>,
) {
    let mut terms = terms.into_iter();
    let Some(first) = terms.next() else {
        *out = L::NOTHING;
        return;
    };
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
/// with the build (the compiler may swap the operands of an addition); roadmap #24 decides
/// how sums treat NaN.
///
/// No other sum of samples is offered: the sample types implement neither `Sum` nor
/// `Product`, so an iterator of samples cannot be summed otherwise.
///
/// ```compile_fail,E0277
/// fn total(samples: &[engine_core::Binary64]) -> engine_core::Binary64 {
///     samples.iter().copied().sum()
/// }
/// ```
#[must_use]
pub fn ordered_sum<S: Sample>(terms: impl IntoIterator<Item = S>) -> S {
    let (mut out, mut rest) = (S::ZERO, [S::ZERO; LANES - 1]);
    reduce(&mut out, &mut rest, terms);
    out
}

/// The scratch lanes of the block form of the reduction order, 15 blocks, reserved once by
/// [`Lanes::new`] (a plan's build), so that summing blocks never allocates.
#[derive(Debug)]
pub(crate) struct Lanes<S: Sample>(Box<[Block<S>; LANES - 1]>);

impl<S: Sample> Lanes<S> {
    /// Lanes of silence, on the heap.
    pub(crate) fn new() -> Self {
        Self(Box::new([S::SILENCE; LANES - 1]))
    }
}

/// The sum of the blocks `terms` frame by frame, in the reduction order of
/// [`ordered_sum`] (the same core, `reduce`): the block itself for one (bit for bit),
/// silence (+0) for none. The plan sums the cables into an inlet and the output boxes on a
/// channel with it, using `lanes` as scratch.
pub(crate) fn sum_blocks<'terms, S: Sample>(
    out: &mut Block<S>,
    lanes: &mut Lanes<S>,
    terms: impl Iterator<Item = &'terms Block<S>>,
) {
    reduce(out, &mut lanes.0, terms);
}

/// The sum of the blocks `first` and `second` frame by frame, `first + second`: the
/// reduction order of [`ordered_sum`] for two terms (lane 0 holds `first`, lane 1 `second`,
/// and only the last step of the tree joins them), computed without scratch lanes. The plan
/// sums two cables into an inlet, or two output boxes on a channel, with it.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "IEEE 754 addition of the sample type, which neither overflows nor panics"
)]
pub(crate) fn add_two<S: Sample>(out: &mut Block<S>, first: &Block<S>, second: &Block<S>) {
    *out = *first;
    for (sum, term) in out.iter_mut().zip(second) {
        *sum += *term;
    }
}

/// What a host, never a box, needs of the engine's samples: their floats and their bits,
/// to give them to the platform, hash them, read them back and test them.
///
/// A box crate does not name this module: xtask's math layer gate refuses it, with `f32`
/// and `f64`, in a box crate's code (tests apart), so a box cannot leave the math layer.
pub mod host {
    use super::{Binary32, Binary64, Sample};

    /// How a render of samples `S` is given to its host, as floats of this type.
    ///
    /// A binary64 render as binary64 or rounded to the nearest binary32 (ties to even; a NaN
    /// by its bits), a binary32 render as binary32. A binary32 render cannot be given as
    /// binary64: no [`RenderFormat`](super::RenderFormat) is one. A host concern, here so
    /// that box code, which may not name this module, cannot take a sample's float out
    /// through it. It is not reachable from the crate root:
    ///
    /// ```compile_fail,E0432
    /// use engine_core::Delivered;
    /// ```
    pub trait Delivered<S: Sample>: super::sealed::Sealed + Copy {
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
            super::to_binary32(sample.0)
        }
    }

    /// The values of a parameter's description, as its box type writes them: for the
    /// engine's peers that check or show box types (the laws, an editor), never a box.
    pub trait ParamBounds {
        /// Value used when the patch does not set it.
        fn default_value(&self) -> f32;
        /// Lowest accepted value.
        fn minimum(&self) -> f32;
        /// Highest accepted value.
        fn maximum(&self) -> f32;
    }

    impl ParamBounds for crate::ParamSpec {
        fn default_value(&self) -> f32 {
            self.default_value
        }

        fn minimum(&self) -> f32 {
            self.minimum
        }

        fn maximum(&self) -> f32 {
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
