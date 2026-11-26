//! The contract every box implements.

#![cfg_attr(not(test), forbid(clippy::arithmetic_side_effects))]

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::any::Any;
use core::fmt;
use core::num::NonZeroU32;
use core::ops::Deref;

use crate::host::Raw as _;
use crate::math::Constructor;
use crate::output::silent_blocks;
use crate::{BLOCK_SIZE, Binary32, Binary64, Block, Sample, SampleRate, flush};

/// A capability a box needs from the host platform.
///
/// More will come (`#[non_exhaustive]`): a match outside this crate needs a wildcard arm.
///
/// ```compile_fail,E0004
/// fn output(capability: audio_core::Capability) -> bool {
///     match capability {
///         audio_core::Capability::AudioOutput => true,
///     }
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Capability {
    /// Sends audio to the host output (speakers, plugin output). The plan sums the
    /// inlets of such boxes into the engine output channels.
    AudioOutput,
}

/// Description of an inlet or an outlet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_structs,
    reason = "built by literal in every box crate while a built-in port is an audio signal alone: it gains its kind, through a constructor, with the first other kind a built-in box has (#12 step 19)"
)]
pub struct PortSpec {
    /// Short name.
    pub name: &'static str,
    /// What flows through the port.
    pub description: &'static str,
}

/// Description of a parameter: its range and default value.
///
/// Its values are binary64 floats, the precision of the values a box receives
/// (`spec/engine-design.md`, decision 10), which box code never reads: they are private, and
/// only the engine and a host (through [`crate::host::ParamBounds`]) read them, so that a box
/// reaches a raw float through no parameter. A box receives its parameter values as
/// [`Binary64`], from the plan. Reading one outside the engine but through `host` does not
/// compile:
///
/// ```compile_fail,E0616
/// fn default(param: &audio_core::ParamSpec) -> f64 {
///     param.default_value
/// }
/// ```
///
/// The same function through `host` compiles, so the field's privacy is the only error
/// above:
///
/// ```
/// use audio_core::host::ParamBounds;
///
/// fn default(param: &audio_core::ParamSpec) -> f64 {
///     param.default_value()
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamSpec {
    /// Name used in patches.
    pub(crate) name: &'static str,
    /// What the parameter controls.
    pub(crate) description: &'static str,
    /// Value used when the patch does not set it.
    pub(crate) default_value: f64,
    /// Lowest accepted value.
    pub(crate) minimum: f64,
    /// Highest accepted value.
    pub(crate) maximum: f64,
}

impl ParamSpec {
    /// The parameter `name`, described by `description`, of default value
    /// `default_value`, accepting `minimum` to `maximum` (inclusive).
    ///
    /// A `const fn`, so that a static spec is checked when it is compiled: [`crate::param!`]
    /// evaluates it in a `const` block, where a refused spec does not compile. A spec built
    /// at run time (a box type loaded then) gets the `Result`. The rule is the one a box
    /// type's description is read by (`host::Description`), one function for both, so that a box
    /// type passes it whoever wrote it.
    ///
    /// # Errors
    ///
    /// Returns [`ParamSpecError::NotFinite`] unless `minimum` and `maximum` are finite,
    /// [`ParamSpecError::Reversed`] unless `minimum` is at most `maximum`,
    /// [`ParamSpecError::Unreachable`] if either is below 2^-126 in magnitude but not zero (a
    /// value there is a zero once flushed, so no value would reach such a bound, and every
    /// comparison with the range is of numbers at least 2^-126 in magnitude or zeros, which
    /// no floating-point mode of a host changes), [`ParamSpecError::InfiniteWidth`] unless
    /// the range's width, `maximum` less `minimum`, is finite (a ramp across the range
    /// computes it), and [`ParamSpecError::DefaultOutsideRange`] unless `default_value`,
    /// flushed as a plan flushes it (below 2^-126 in magnitude, a zero of its sign), is
    /// within them, so that every parameter of every box type has a finite range holding its
    /// default, as a plan, the box laws and the fingerprint battery give it.
    pub const fn new(
        name: &'static str,
        description: &'static str,
        default_value: f64,
        minimum: f64,
        maximum: f64,
    ) -> Result<Self, ParamSpecError> {
        match range_rule(default_value, minimum, maximum) {
            Ok(()) => Ok(Self {
                name,
                description,
                default_value,
                minimum,
                maximum,
            }),
            Err(RangeFault::NotFinite(_)) => Err(ParamSpecError::NotFinite { name }),
            Err(RangeFault::Reversed) => Err(ParamSpecError::Reversed { name }),
            Err(RangeFault::Unreachable(_)) => Err(ParamSpecError::Unreachable { name }),
            Err(RangeFault::Width) => Err(ParamSpecError::InfiniteWidth { name }),
            Err(RangeFault::Default) => Err(ParamSpecError::DefaultOutsideRange { name }),
        }
    }

    /// Its name, used in patches.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// What it controls.
    #[must_use]
    pub const fn description(&self) -> &'static str {
        self.description
    }

    /// Whether `value`, flushed as a plan flushes it (below 2^-126 in magnitude, a zero of
    /// its sign, decided from its bits), is within range. Always `false` for NaN.
    #[must_use]
    pub const fn accepts(&self, value: f64) -> bool {
        self.holds(flush::parameter(value))
    }

    /// Whether `flushed`, a value already flushed as a plan flushes it, is within range.
    const fn holds(&self, flushed: f64) -> bool {
        self.minimum <= flushed && flushed <= self.maximum
    }

    /// `picked` if this parameter's range accepts it ([`ParamSpec::accepts`]), else the
    /// default, which no range refuses, flushed as a plan flushes a value: the one rule of
    /// [`ParamValues::within`] and [`DrivenValues::within`].
    const fn kept(&self, picked: f64) -> Binary64 {
        let value = if self.accepts(picked) {
            picked
        } else {
            self.default_value
        };
        Binary64::new(value).as_parameter()
    }
}

/// Why a parameter's range is refused by [`range_rule`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum RangeFault {
    /// This bound, the minimum or the maximum, is not finite.
    NotFinite(f64),
    /// The minimum is above the maximum.
    Reversed,
    /// This bound is below 2^-126 in magnitude but not zero: no value reaches it once
    /// flushed.
    Unreachable(f64),
    /// The width, the maximum less the minimum, is not finite in binary64.
    Width,
    /// The default, flushed, is not within the range: outside it, or not a number.
    Default,
}

/// The one rule of a parameter's range, for every box type, built in ([`ParamSpec::new`])
/// or described by a module (`spec/box-contract.md`): `minimum` and `maximum` finite, in
/// order, neither below 2^-126 in magnitude but zero (no value reaches such a bound once
/// flushed); the width, `maximum` less `minimum`, finite (a ramp across the range computes
/// it); `default_value`, flushed as a plan flushes it, within them. Decided by the faults in
/// that order (the minimum's before the maximum's), so that a range refused for two reasons
/// names the first.
pub(crate) const fn range_rule(
    default_value: f64,
    minimum: f64,
    maximum: f64,
) -> Result<(), RangeFault> {
    if !minimum.is_finite() {
        return Err(RangeFault::NotFinite(minimum));
    }
    if !maximum.is_finite() {
        return Err(RangeFault::NotFinite(maximum));
    }
    if minimum > maximum {
        return Err(RangeFault::Reversed);
    }
    if !reachable(minimum) {
        return Err(RangeFault::Unreachable(minimum));
    }
    if !reachable(maximum) {
        return Err(RangeFault::Unreachable(maximum));
    }
    if !(maximum - minimum).is_finite() {
        return Err(RangeFault::Width);
    }
    let flushed = flush::parameter(default_value);
    if minimum <= flushed && flushed <= maximum {
        Ok(())
    } else {
        Err(RangeFault::Default)
    }
}

/// Whether a value can be `bound` once flushed: a zero, or at least 2^-126 in magnitude,
/// told by its bits, which [`flush::parameter`] leaves as they are.
const fn reachable(bound: f64) -> bool {
    flush::parameter(bound).to_bits() == bound.to_bits()
}

/// A [`ParamSpec`] checked when the program is compiled.
///
/// [`ParamSpec::new`] evaluated in a `const` block, so that a parameter whose range is not
/// finite, or whose default lies outside it, does not compile, the compiler's error saying
/// which of the two ([`ParamSpecError`]). The way to write the parameters of a static box
/// type:
///
/// ```
/// static FREQUENCY: audio_core::ParamSpec =
///     audio_core::param!("frequency", "in hertz", 440.0, 0.0, 20_000.0);
/// ```
///
/// A default above the maximum, below the minimum, not a number, or below 2^-126 in
/// magnitude where the range starts above zero (flushed, it is zero), does not compile:
///
/// ```compile_fail,E0080
/// static LOUD: audio_core::ParamSpec = audio_core::param!("gain", "", 1.5, 0.0, 1.0);
/// ```
///
/// ```compile_fail,E0080
/// static LOW: audio_core::ParamSpec = audio_core::param!("gain", "", -0.5, 0.0, 1.0);
/// ```
///
/// ```compile_fail,E0080
/// static NAN: audio_core::ParamSpec = audio_core::param!("gain", "", f64::NAN, 0.0, 1.0);
/// ```
///
/// ```compile_fail,E0080
/// static TINY: audio_core::ParamSpec =
///     audio_core::param!("t", "", 1e-40, 1.1754943508222875e-38, 1.0);
/// ```
///
/// Nor does a range that is not finite, one whose minimum is above its maximum, or one with
/// a bound below 2^-126 in magnitude but zero, which no value reaches (flushed, a value
/// there is zero):
///
/// ```compile_fail,E0080
/// static SUBNORMAL: audio_core::ParamSpec = audio_core::param!("t", "", 0.5, 1e-40, 1.0);
/// ```
///
/// The same range from zero compiles, so its subnormal minimum is the only error above:
///
/// ```
/// static ZERO: audio_core::ParamSpec = audio_core::param!("t", "", 0.5, 0.0, 1.0);
/// ```
///
/// ```compile_fail,E0080
/// static OPEN: audio_core::ParamSpec =
///     audio_core::param!("gain", "", 0.5, 0.0, f64::INFINITY);
/// ```
///
/// Nor does a range whose width, its maximum less its minimum, is not finite in binary64,
/// which a ramp across it would compute:
///
/// ```compile_fail,E0080
/// static WIDE: audio_core::ParamSpec =
///     audio_core::param!("gain", "", 0.0, -1.0e308, 1.0e308);
/// ```
///
/// ```compile_fail,E0080
/// static EMPTY: audio_core::ParamSpec = audio_core::param!("gain", "", 0.5, 1.0, 0.0);
/// ```
#[macro_export]
macro_rules! param {
    ($name:expr, $description:expr, $default_value:expr, $minimum:expr, $maximum:expr $(,)?) => {
        const {
            match $crate::ParamSpec::new($name, $description, $default_value, $minimum, $maximum) {
                ::core::result::Result::Ok(spec) => spec,
                ::core::result::Result::Err(refused) => ::core::panic!("{}", refused.reason()),
            }
        }
    };
}

/// The parameter values of a box instance, owned, with the box type they were checked
/// against: one per entry of its [`ObjectSpec::params`], in its order, each within its
/// parameter's range.
///
/// In binary64 whatever the precision of the render (`spec/engine-design.md`, decision
/// 10), and never below 2^-126 in magnitude but zero: such a value is a zero of its sign
/// (decisions 5 and 11), the flush a plan applies to a patch's parameters. Outside the
/// engine, only [`ParamValues::of`] and [`ParamValues::defaults`] build one, from the box
/// type's spec, which it keeps ([`ParamValues::spec`]): an instance is created from the
/// values alone ([`Dispatch::create`]), of the box type they were checked against, and given
/// only values of its own box type afterwards ([`BoxInstance::receive`], [`Typed`]), so a
/// box can rely on all of it, whoever created it: the plan, the box laws, the fingerprint
/// battery or a host. No unchecked constructor is public:
///
/// ```compile_fail,E0599
/// let _values = audio_core::ParamValues::new([audio_core::Binary64::new(1e9)]);
/// ```
///
/// Laid out in C order with the spec first: the layout measured cheapest to load a plan
/// with, and the smallest module (`spec/benchmarks.md`, "Measured design choices", "A box's
/// parameters, with their box type"); the order of its fields changes no value.
#[derive(Debug, Clone)]
#[repr(C)]
pub struct ParamValues {
    /// The box type the values were checked against.
    spec: &'static ObjectSpec,
    /// The values, one per parameter of `spec`, in order.
    values: Vec<Binary64>,
}

/// Why a parameter is refused ([`ParamSpec::new`]).
///
/// One variant per rule of a range, as a module's description names its refusals
/// ([`crate::host::Fault`]), each with one text, which the compile error of [`crate::param!`]
/// and the error's own display share ([`ParamSpecError::reason`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParamSpecError {
    /// A parameter whose minimum or maximum is not finite.
    NotFinite {
        /// The parameter.
        name: &'static str,
    },
    /// A parameter whose minimum is above its maximum.
    Reversed {
        /// The parameter.
        name: &'static str,
    },
    /// A parameter whose minimum or maximum is below 2^-126 in magnitude but not zero, which
    /// no value reaches once flushed.
    Unreachable {
        /// The parameter.
        name: &'static str,
    },
    /// A parameter whose width, its maximum less its minimum, is not finite in binary64.
    InfiniteWidth {
        /// The parameter.
        name: &'static str,
    },
    /// A parameter whose default, flushed, is not within its range: outside it, or not a
    /// number.
    DefaultOutsideRange {
        /// The parameter.
        name: &'static str,
    },
}

impl ParamSpecError {
    /// Why the parameter is refused, in words, without its name.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::NotFinite { .. } => "its minimum or its maximum is not finite",
            Self::Reversed { .. } => "its minimum is above its maximum",
            Self::Unreachable { .. } => {
                "its minimum or its maximum is below 2^-126 in magnitude but not zero: no value \
                 reaches it"
            }
            Self::InfiniteWidth { .. } => {
                "its width, its maximum less its minimum, is not finite in binary64"
            }
            Self::DefaultOutsideRange { .. } => "its default, flushed, is not within its range",
        }
    }

    /// The parameter refused.
    const fn name(self) -> &'static str {
        match self {
            Self::NotFinite { name }
            | Self::Reversed { name }
            | Self::Unreachable { name }
            | Self::InfiniteWidth { name }
            | Self::DefaultOutsideRange { name } => name,
        }
    }
}

impl fmt::Display for ParamSpecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "parameter \"{}\": {}", self.name(), self.reason())
    }
}

/// An error of its own, with no cause: a host passes it on as any other.
impl core::error::Error for ParamSpecError {}

/// Why parameter values are not those of a box type: values given one per parameter
/// ([`ParamValues::of`]), or blocks of values given one per parameter
/// ([`DrivenValues::of`]).
///
/// A number is written as Rust's `Debug` writes it, the shortest form that reads back
/// exactly, with an exponent past 16 digits, as a module's description writes its refusals.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum ParamError {
    /// Not one value per parameter of the box type ([`ParamValues::of`]).
    Count {
        /// The box type.
        kind: &'static str,
        /// How many parameters it has.
        expected: usize,
        /// How many values were given.
        given: usize,
    },
    /// Not one block of values per parameter of the box type ([`DrivenValues::of`]).
    Blocks {
        /// The box type.
        kind: &'static str,
        /// How many parameters it has.
        expected: usize,
        /// How many blocks were given.
        given: usize,
    },
    /// A value its parameter's range does not hold once flushed, or not a number
    /// ([`ParamValues::of`]).
    OutOfRange {
        /// The box type.
        kind: &'static str,
        /// The parameter.
        name: &'static str,
        /// The bits of the binary64 value refused, as it was given.
        bits: u64,
        /// The parameter's lowest accepted value.
        minimum: f64,
        /// The parameter's highest accepted value.
        maximum: f64,
    },
    /// A value of a block its parameter's range does not hold once flushed, or not a number
    /// ([`DrivenValues::of`]): the first, in the order of the parameters, then of the frames.
    DrivenOutOfRange {
        /// The box type.
        kind: &'static str,
        /// The parameter.
        name: &'static str,
        /// The frame of the block, from 0.
        frame: usize,
        /// The bits of the binary64 value refused, as it was given.
        bits: u64,
        /// The parameter's lowest accepted value.
        minimum: f64,
        /// The parameter's highest accepted value.
        maximum: f64,
    },
}

impl fmt::Display for ParamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Count {
                kind,
                expected,
                given,
            } => write!(
                f,
                "box type \"{kind}\" takes {expected} parameter value(s), not {given}"
            ),
            Self::Blocks {
                kind,
                expected,
                given,
            } => write!(
                f,
                "box type \"{kind}\" takes {expected} block(s) of values, one per parameter, not {given}"
            ),
            Self::OutOfRange {
                kind,
                name,
                bits,
                minimum,
                maximum,
            } => write!(
                f,
                "parameter \"{name}\" of box type \"{kind}\": {:?} is not within [{minimum:?}, {maximum:?}]",
                f64::from_bits(bits)
            ),
            Self::DrivenOutOfRange {
                kind,
                name,
                frame,
                bits,
                minimum,
                maximum,
            } => write!(
                f,
                "parameter \"{name}\" of box type \"{kind}\", frame {frame}: {:?} is not within [{minimum:?}, {maximum:?}]",
                f64::from_bits(bits)
            ),
        }
    }
}

/// An error of its own, with no cause: a host passes it on as any other.
impl core::error::Error for ParamError {}

impl ParamValues {
    /// The parameter values `values` of the box type `spec`: one per parameter, in order,
    /// each nonzero one below 2^-126 in magnitude made a zero of its sign, then checked
    /// against its parameter's range ([`ParamSpec::accepts`], as a plan checks a patch's
    /// value); every other value keeps its bits.
    ///
    /// # Errors
    ///
    /// Returns [`ParamError::Count`] unless there is one value per parameter, and
    /// [`ParamError::OutOfRange`] for the first value its range does not hold, or NaN, as
    /// it was given.
    pub fn of(spec: &'static ObjectSpec, values: &[Binary64]) -> Result<Self, ParamError> {
        if values.len() != spec.params.len() {
            return Err(ParamError::Count {
                kind: spec.name,
                expected: spec.params.len(),
                given: values.len(),
            });
        }
        values
            .iter()
            .zip(spec.params)
            .map(|(&value, param)| {
                let flushed = value.as_parameter();
                if param.holds(flushed.raw()) {
                    Ok(flushed)
                } else {
                    Err(ParamError::OutOfRange {
                        kind: spec.name,
                        name: param.name,
                        bits: value.raw().to_bits(),
                        minimum: param.minimum,
                        maximum: param.maximum,
                    })
                }
            })
            .collect::<Result<_, _>>()
            .map(|values| Self { spec, values })
    }

    /// The default values of the box type `spec`, each flushed as a plan flushes it: within
    /// its range, as [`ParamSpec::new`] checks.
    #[must_use]
    pub fn defaults(spec: &'static ObjectSpec) -> Self {
        Self {
            spec,
            values: spec
                .params
                .iter()
                .map(|param| Binary64::new(param.default_value).as_parameter())
                .collect(),
        }
    }

    /// The parameter values of the box type `spec` that `pick` picks, called once per
    /// parameter, in order: each value its parameter's range accepts ([`ParamSpec::accepts`]), else the
    /// parameter's default, which no range refuses, each flushed as a plan flushes it. So
    /// that a caller trying values of every range (the box laws, the fingerprint battery)
    /// is given values of the box type whatever it picks, with no error to handle: a value
    /// picked outside a range, or not a number, is its default, tried anyway.
    #[must_use]
    pub fn within(spec: &'static ObjectSpec, mut pick: impl FnMut(&ParamSpec) -> f64) -> Self {
        Self {
            spec,
            values: spec
                .params
                .iter()
                .map(|param| param.kept(pick(param)))
                .collect(),
        }
    }

    /// The parameter values `values` of the box type `spec`, which the plan has checked
    /// against their ranges and flushed as [`ParamValues::of`] would: taken as they are, so
    /// that a patch's parameters are read with no second flush (the plan tests check what a
    /// box is given).
    pub(crate) const fn flushed(spec: &'static ObjectSpec, values: Vec<Binary64>) -> Self {
        Self { spec, values }
    }

    /// The box type the values were checked against, whose instance they create.
    #[must_use]
    pub const fn spec(&self) -> &'static ObjectSpec {
        self.spec
    }

    /// The values, as a box is given them.
    #[must_use]
    pub fn params(&self) -> Params<'_> {
        Params(&self.values)
    }
}

/// The values that drive the parameters of a box type over one block.
///
/// A block of values per parameter, frame `f` of the `k`-th the value of the `k`-th
/// parameter at frame `f`, each within its parameter's range, never below 2^-126 in
/// magnitude but zero.
///
/// What [`BoxInstance::run_driven`] takes, so that a box's driven body receives only values
/// of its box type's ranges ([`Driven::process_driven`]): outside the engine, only
/// [`DrivenValues::of`] and [`DrivenValues::within`] build one, from the box type's spec,
/// which it keeps, and an instance refuses those of another box type ([`Typed`]). No
/// unchecked constructor is public:
///
/// ```compile_fail,E0599
/// let _values = audio_core::DrivenValues::<64>::new(vec![[audio_core::Binary64::new(1e9); 64]]);
/// ```
///
/// Built off the audio thread: [`DrivenValues::of`] takes the blocks it is given, allocating
/// nothing more, and an instance is given it by reference.
#[derive(Debug, Clone)]
pub struct DrivenValues<const N: usize = BLOCK_SIZE> {
    /// The box type the values were checked against.
    spec: &'static ObjectSpec,
    /// A block of values per parameter of `spec`, in order.
    blocks: Vec<Block<Binary64, N>>,
}

impl<const N: usize> DrivenValues<N> {
    /// The values `blocks` driving the parameters of the box type `spec`: one block per
    /// parameter, in order, each value below 2^-126 in magnitude made a zero of its sign,
    /// then checked against its parameter's range ([`ParamSpec::accepts`], as
    /// [`ParamValues::of`] checks a value); every other value keeps its bits.
    ///
    /// # Errors
    ///
    /// Returns [`ParamError::Blocks`] unless there is one block per parameter, and
    /// [`ParamError::DrivenOutOfRange`] for the first value its range does not hold, or NaN,
    /// as it was given, with its frame, in the order of the parameters, then of the frames.
    pub fn of(
        spec: &'static ObjectSpec,
        mut blocks: Vec<Block<Binary64, N>>,
    ) -> Result<Self, ParamError> {
        if blocks.len() != spec.params.len() {
            return Err(ParamError::Blocks {
                kind: spec.name,
                expected: spec.params.len(),
                given: blocks.len(),
            });
        }
        for (block, param) in blocks.iter_mut().zip(spec.params) {
            for (frame, value) in block.iter_mut().enumerate() {
                let flushed = value.as_parameter();
                if !param.holds(flushed.raw()) {
                    return Err(ParamError::DrivenOutOfRange {
                        kind: spec.name,
                        name: param.name,
                        frame,
                        bits: value.raw().to_bits(),
                        minimum: param.minimum,
                        maximum: param.maximum,
                    });
                }
                *value = flushed;
            }
        }
        Ok(Self { spec, blocks })
    }

    /// The values of the box type `spec` that `pick` picks, called with each parameter's
    /// place and the parameter, then each frame, parameter by parameter in order, frame by
    /// frame within each: as [`ParamValues::within`] takes a value, the value if its
    /// parameter's range accepts it, else the parameter's default, each flushed. So that a
    /// caller trying values of every range (the box laws) drives a box with values of its
    /// box type whatever it picks, with no error to handle. Made on the heap, at any engine
    /// block: an array of the largest would not fit a thread's stack.
    #[must_use]
    pub fn within(
        spec: &'static ObjectSpec,
        mut pick: impl FnMut(usize, &ParamSpec, usize) -> f64,
    ) -> Self {
        let mut blocks = silent_blocks::<Binary64, N>(spec.params.len());
        for (index, (block, param)) in blocks.iter_mut().zip(spec.params).enumerate() {
            for (frame, value) in block.iter_mut().enumerate() {
                *value = param.kept(pick(index, param, frame));
            }
        }
        Self { spec, blocks }
    }

    /// The box type the values were checked against.
    #[must_use]
    pub const fn spec(&self) -> &'static ObjectSpec {
        self.spec
    }
}

/// The parameter values a box is created with, borrowed from [`ParamValues`].
///
/// They keep its promise: one per parameter of the box type, in order, each within its
/// range, in binary64, none below 2^-126 in magnitude but zero. Passed by value, as cheap
/// as a slice (two words: a reference to the owned values made loading a plan dearer,
/// `spec/benchmarks.md`), and read as one.
#[derive(Debug, Clone, Copy)]
pub struct Params<'values>(&'values [Binary64]);

impl Params<'_> {
    /// The values as an array, one per parameter of a box type of `K`, which the engine hands
    /// a box ([`Create`], [`Typed`]) for it to take apart by pattern (`[frequency]`); none when
    /// there are not `K`. The values of a box type are as many as its parameters
    /// ([`ParamValues`]), so it is never none where `K` is that number, as [`Create`] requires
    /// of a box type.
    #[must_use]
    pub fn array<const K: usize>(self) -> Option<[Binary64; K]> {
        self.0.try_into().ok()
    }
}

impl Deref for Params<'_> {
    type Target = [Binary64];

    /// The values, in order.
    fn deref(&self) -> &[Binary64] {
        self.0
    }
}

/// A box type's parameters and its way of creating its instances, written once for every
/// precision: the plan calls [`Create::create`] with the [`Sample`] type of the render.
///
/// Both belong to one implementation, for `K` parameters: a spec takes its parameters and
/// its factory from the same one ([`ObjectSpec::new`]), so that a box is created only from
/// values checked against its own parameters, and receives exactly as many: a box taking
/// its values apart names each, with no value to invent for a missing one:
///
/// ```
/// use audio_core::{Binary64, Block, BoxMode, Create, Driven, Object, ParamSpec, Sample, SampleRate};
///
/// #[derive(Debug)]
/// struct Gain(Binary64);
///
/// impl<S: Sample, const N: usize> Object<S, N> for Gain {
///     fn process(&mut self, _inputs: &[Block<S, N>], _outputs: &mut [Block<S, N>]) {}
/// }
///
/// impl<S: Sample, const N: usize> Driven<S, N, 1> for Gain {
///     fn set(&mut self, [gain]: [Binary64; 1]) {
///         self.0 = gain;
///     }
///
///     fn process_driven(
///         &mut self,
///         _inputs: &[Block<S, N>],
///         _outputs: &mut [Block<S, N>],
///         [_gains]: &[Block<Binary64, N>; 1],
///     ) {
///     }
/// }
///
/// impl Create<1> for Gain {
///     const BOX_MODE: BoxMode = BoxMode::LiveCapable;
///     const PARAMS: &'static [ParamSpec; 1] =
///         &[audio_core::param!("gain", "how loud", 0.5, 0.0, 1.0)];
///
///     fn create<S: Sample, const N: usize>(
///         _rate: SampleRate,
///         [gain]: [Binary64; 1],
///     ) -> impl Driven<S, N, 1> + 'static {
///         Self(gain)
///     }
/// }
/// ```
///
/// The same box taking a second value it has no parameter for does not compile, nor one
/// taking none, nor one describing two parameters and taking one:
///
/// ```compile_fail,E0527
/// # use audio_core::{Binary64, Block, BoxMode, Create, Driven, Object, ParamSpec, Sample, SampleRate};
/// # #[derive(Debug)]
/// # struct Gain(Binary64);
/// # impl<S: Sample, const N: usize> Object<S, N> for Gain {
/// #     fn process(&mut self, _inputs: &[Block<S, N>], _outputs: &mut [Block<S, N>]) {}
/// # }
/// # impl<S: Sample, const N: usize> Driven<S, N, 1> for Gain {
/// #     fn set(&mut self, [gain]: [Binary64; 1]) {
/// #         self.0 = gain;
/// #     }
/// #     fn process_driven(
/// #         &mut self,
/// #         _inputs: &[Block<S, N>],
/// #         _outputs: &mut [Block<S, N>],
/// #         [_gains]: &[Block<Binary64, N>; 1],
/// #     ) {
/// #     }
/// # }
/// impl Create<1> for Gain {
///     const BOX_MODE: BoxMode = BoxMode::LiveCapable;
///     const PARAMS: &'static [ParamSpec; 1] =
///         &[audio_core::param!("gain", "how loud", 0.5, 0.0, 1.0)];
///
///     fn create<S: Sample, const N: usize>(
///         _rate: SampleRate,
///         [gain, _pan]: [Binary64; 1],
///     ) -> impl Driven<S, N, 1> + 'static {
///         Self(gain)
///     }
/// }
/// ```
///
/// ```compile_fail,E0053
/// # use audio_core::{Binary64, Block, BoxMode, Create, Driven, Object, ParamSpec, Sample, SampleRate};
/// # #[derive(Debug)]
/// # struct Gain(Binary64);
/// # impl<S: Sample, const N: usize> Object<S, N> for Gain {
/// #     fn process(&mut self, _inputs: &[Block<S, N>], _outputs: &mut [Block<S, N>]) {}
/// # }
/// # impl<S: Sample, const N: usize> Driven<S, N, 1> for Gain {
/// #     fn set(&mut self, [gain]: [Binary64; 1]) {
/// #         self.0 = gain;
/// #     }
/// #     fn process_driven(
/// #         &mut self,
/// #         _inputs: &[Block<S, N>],
/// #         _outputs: &mut [Block<S, N>],
/// #         [_gains]: &[Block<Binary64, N>; 1],
/// #     ) {
/// #     }
/// # }
/// impl Create<1> for Gain {
///     const BOX_MODE: BoxMode = BoxMode::LiveCapable;
///     const PARAMS: &'static [ParamSpec; 1] =
///         &[audio_core::param!("gain", "how loud", 0.5, 0.0, 1.0)];
///
///     fn create<S: Sample, const N: usize>(
///         _rate: SampleRate,
///         []: [Binary64; 0],
///     ) -> impl Driven<S, N, 1> + 'static {
///         Self(Binary64::new(0.5))
///     }
/// }
/// ```
///
/// ```compile_fail,E0308
/// # use audio_core::{Binary64, Block, BoxMode, Create, Driven, Object, ParamSpec, Sample, SampleRate};
/// # #[derive(Debug)]
/// # struct Gain(Binary64);
/// # impl<S: Sample, const N: usize> Object<S, N> for Gain {
/// #     fn process(&mut self, _inputs: &[Block<S, N>], _outputs: &mut [Block<S, N>]) {}
/// # }
/// # impl<S: Sample, const N: usize> Driven<S, N, 1> for Gain {
/// #     fn set(&mut self, [gain]: [Binary64; 1]) {
/// #         self.0 = gain;
/// #     }
/// #     fn process_driven(
/// #         &mut self,
/// #         _inputs: &[Block<S, N>],
/// #         _outputs: &mut [Block<S, N>],
/// #         [_gains]: &[Block<Binary64, N>; 1],
/// #     ) {
/// #     }
/// # }
/// impl Create<1> for Gain {
///     const BOX_MODE: BoxMode = BoxMode::LiveCapable;
///     const PARAMS: &'static [ParamSpec; 1] = &[
///         audio_core::param!("gain", "how loud", 0.5, 0.0, 1.0),
///         audio_core::param!("pan", "where", 0.0, -1.0, 1.0),
///     ];
///
///     fn create<S: Sample, const N: usize>(
///         _rate: SampleRate,
///         [gain]: [Binary64; 1],
///     ) -> impl Driven<S, N, 1> + 'static {
///         Self(gain)
///     }
/// }
/// ```
///
/// A box with a parameter and no body for the blocks where it is driven ([`Driven`]) does
/// not compile either:
///
/// ```compile_fail,E0277
/// use audio_core::{Binary64, Block, BoxMode, Create, Driven, Object, ParamSpec, Sample, SampleRate};
///
/// #[derive(Debug)]
/// struct Gain(Binary64);
///
/// impl<S: Sample, const N: usize> Object<S, N> for Gain {
///     fn process(&mut self, _inputs: &[Block<S, N>], _outputs: &mut [Block<S, N>]) {}
/// }
///
/// impl Create<1> for Gain {
///     const BOX_MODE: BoxMode = BoxMode::LiveCapable;
///     const PARAMS: &'static [ParamSpec; 1] =
///         &[audio_core::param!("gain", "how loud", 0.5, 0.0, 1.0)];
///
///     fn create<S: Sample, const N: usize>(
///         _rate: SampleRate,
///         [gain]: [Binary64; 1],
///     ) -> impl Driven<S, N, 1> + 'static {
///         Self(gain)
///     }
/// }
/// ```
pub trait Create<const K: usize> {
    /// The parameters of the box type, in order: those of every spec built from this
    /// implementation, against which every value [`Create::create`] receives was checked.
    const PARAMS: &'static [ParamSpec; K];

    /// The execution modes the box type may run in (`spec/engine-design.md`, decision 20):
    /// every box type states it, with no default.
    const BOX_MODE: crate::BoxMode;

    /// Creates an instance computing in `S` in blocks of `N` frames, from a sample rate and
    /// its parameter values.
    ///
    /// The values come as [`Params`] says, as an array: one per parameter of [`Create::PARAMS`], in
    /// order, each within its range, in binary64, none below 2^-126 in magnitude but zero. The engine
    /// calls it only through the box type's [`Factory`], from values checked against a spec
    /// of this type ([`Dispatch::create`]), whose parameters are this type's by construction
    /// ([`ObjectSpec::new`]).
    fn create<S: Sample, const N: usize>(
        rate: SampleRate,
        params: [Binary64; K],
    ) -> impl Driven<S, N, K> + 'static;
}

/// The instance of the box type `T` from as many values as it has parameters, the one
/// constructor a [`Factory`] holds of it per precision and engine block, tied to the box
/// type `values` were checked against ([`Typed`]); none from another number of values,
/// which no [`ParamValues`] of that box type holds.
fn created<T: Create<K>, const K: usize, S: Sample, const N: usize>(
    rate: SampleRate,
    values: &ParamValues,
) -> Option<Box<dyn BoxInstance<S, N>>> {
    let params = values.params().array::<K>()?;
    Some(Box::new(Typed::<_, K>::new(
        values.spec,
        T::create::<S, N>(rate, params),
    )))
}

/// Creates the instances of a box type, in either precision: built only from one generic
/// [`Create::create`], so that a box type cannot behave differently by precision but
/// through the [`Sample`] it is given.
///
/// How it picks the constructor of a precision is the engine's own: [`Dispatch::create`],
/// from values that carry their box type, is the one way to create an instance, and
/// neither the factory's own creation, which takes values without their box type, nor the
/// per-precision constructor, nor its type, is public:
///
/// ```compile_fail,E0624
/// use audio_core::{Binary64, Factory, BoxInstance, ParamValues, SampleRate};
///
/// fn create(
///     factory: &Factory,
///     rate: SampleRate,
///     values: &ParamValues,
/// ) -> Option<Box<dyn BoxInstance<Binary64>>> {
///     factory.create(rate, values)
/// }
/// ```
///
/// ```compile_fail,E0599
/// use audio_core::{Binary64, Factory, Sample};
///
/// fn pick(factory: &Factory) {
///     let _constructor = Binary64::constructor(factory);
/// }
/// ```
///
/// ```compile_fail,E0432
/// use audio_core::Constructor;
/// ```
///
/// Nor is its constructor, so a factory exists only inside the spec of its box type, with
/// that type's parameters ([`ObjectSpec::new`]), and is neither cloned nor copied out of it:
///
/// ```compile_fail,E0624
/// use audio_core::{
///     Binary64, Block, Create, Driven, Factory, Object, ParamSpec, Sample, SampleRate,
/// };
///
/// #[derive(Debug)]
/// struct Silent;
///
/// impl<S: Sample, const N: usize> Object<S, N> for Silent {
///     fn process(&mut self, _inputs: &[Block<S, N>], _outputs: &mut [Block<S, N>]) {}
/// }
///
/// impl Create<0> for Silent {
///     const BOX_MODE: audio_core::BoxMode = audio_core::BoxMode::LiveCapable;
///     const PARAMS: &'static [ParamSpec; 0] = &[];
///
///     fn create<S: Sample, const N: usize>(
///         _rate: SampleRate,
///         []: [Binary64; 0],
///     ) -> impl Driven<S, N, 0> + 'static {
///         Self
///     }
/// }
///
/// fn factory() -> Factory {
///     Factory::new::<Silent, 0>()
/// }
/// ```
///
/// (The same box type builds its spec, the control: [`ObjectSpec`]'s examples.)
#[derive(Debug)]
pub struct Factory {
    /// Creates an instance, for each precision and each engine block of
    /// [`crate::ENGINE_BLOCKS`].
    pub(crate) constructors: Constructors,
}

/// One constructor per precision and engine block of [`crate::ENGINE_BLOCKS`], each the
/// box type's one generic [`Create::create`], through [`created`] (`binary64_64`: binary64,
/// blocks of 64).
macro_rules! constructors {
    ($($binary64:ident, $binary32:ident, $frames:literal;)*) => {
        /// The constructors of a [`Factory`]: one per precision and engine block offered.
        #[derive(Debug)]
        pub(crate) struct Constructors {
            $(
                #[doc = concat!("Binary64 instances in blocks of ", $frames, " frames, if built.")]
                pub(crate) $binary64: Option<Constructor<Binary64, $frames>>,
                #[doc = concat!("Binary32 instances in blocks of ", $frames, " frames, if built.")]
                pub(crate) $binary32: Option<Constructor<Binary32, $frames>>,
            )*
        }

        impl Constructors {
            /// Those of the box type `T`, for the engine blocks this build holds
            /// ([`crate::NAMED_ENGINE_BLOCK`]): none of another size is compiled.
            const fn of<T: Create<K>, const K: usize>() -> Self {
                Self {
                    $(
                        $binary64: crate::block::held::<Constructor<Binary64, $frames>>(
                            crate::block::holds(crate::NAMED_ENGINE_BLOCK, $frames),
                            created::<T, K, Binary64, $frames>,
                        ),
                        $binary32: crate::block::held::<Constructor<Binary32, $frames>>(
                            crate::block::holds(crate::NAMED_ENGINE_BLOCK, $frames),
                            created::<T, K, Binary32, $frames>,
                        ),
                    )*
                }
            }

            /// The constructor computing in `S` in blocks of `N` frames, if this build holds
            /// it: found by its type among them all, so that any sample type and engine block
            /// may ask, an engine block of none of them finding none. A plan's build asks it
            /// once per box created through its virtual table, never its processing
            /// (`spec/engine-design.md`, decision 33).
            fn find<S: Sample, const N: usize>(&self) -> Option<Constructor<S, N>> {
                let all: &[&dyn Any] = &[$(&self.$binary64, &self.$binary32),*];
                all.iter()
                    .find_map(|held| held.downcast_ref::<Option<Constructor<S, N>>>())
                    .copied()
                    .flatten()
            }
        }

        /// The engine blocks a build of the patcher offers (`spec/engine-design.md`,
        /// decision 2), in frames, ascending: those the constructors of every factory cover,
        /// written once, where they are made.
        ///
        /// The web engine ships one module per size, the desktop patcher holds them all. An
        /// exported native target compiles the size it is exported with, any from 1 to
        /// [`crate::MAX_ENGINE_BLOCK`]: the engine's own box types (`audio_boxes::BuiltIn`)
        /// run at any of them, while a box type created through its virtual table exists
        /// only at the sizes of this list the build holds ([`crate::NAMED_ENGINE_BLOCK`]),
        /// and is refused at any other ([`crate::CoreError::Unbuilt`]).
        pub const ENGINE_BLOCKS: [usize; [$({
            let frames: usize = $frames;
            frames
        }),*]
            .len()] = [$({
            let frames: usize = $frames;
            frames
        }),*];
    };
}

constructors! {
    binary64_16, binary32_16, 16;
    binary64_32, binary32_32, 32;
    binary64_64, binary32_64, 64;
    binary64_128, binary32_128, 128;
    binary64_256, binary32_256, 256;
    binary64_512, binary32_512, 512;
    binary64_1024, binary32_1024, 1024;
}

impl Factory {
    /// The factory of the box type `T`: crate-private, so that it is built only by the spec
    /// of `T` ([`ObjectSpec::new`]), with `T`'s parameters.
    #[must_use]
    pub(crate) const fn new<T: Create<K>, const K: usize>() -> Self {
        Self {
            constructors: Constructors::of::<T, K>(),
        }
    }

    /// Creates an instance computing in `S` in blocks of `N` frames, at `rate`, with
    /// `values` as [`Create::create`] receives them, tied to their box type ([`Typed`]);
    /// none when this build does not hold that engine block ([`crate::NAMED_ENGINE_BLOCK`]),
    /// or when `values` are not as many as its box type's parameters. Crate-private, since
    /// `values` may be of another box type than this factory's; [`Dispatch::create`] takes
    /// the factory of the box type the values carry.
    #[must_use]
    pub(crate) fn create<S: Sample, const N: usize>(
        &self,
        rate: SampleRate,
        values: &ParamValues,
    ) -> Option<Box<dyn BoxInstance<S, N>>> {
        self.constructors
            .find::<S, N>()
            .and_then(|constructor| constructor(rate, values))
    }
}

/// The version of the engine's render rules (`spec/engine-design.md`, decision 29).
///
/// The render rules are the bits of a render that no box type owns (the plan's sums and
/// their order, when events take effect, flush points, the output stage, delivery). Any
/// change of them that can change a bit of a render takes a new one; version 1 is the
/// rules of the reduction order (decision 28), version 2 adds delivery as a flush point
/// (decision 11), version 3 the output stage (decision 12: a non-finite sample replaced by
/// zero, and the true-peak limiter of a protected stage). The conformance references
/// record it, and bless refuses a changed reference under the same one.
pub const RENDER_RULES_VERSION: u32 = 3;

/// The version of a box type's algorithm (`spec/engine-design.md`, decision 29).
///
/// Any change of a box type that can change a bit of its output takes a new one, and the
/// conformance references record the version of every box type they were blessed with,
/// with its fingerprint: `audio-cli check` refuses a new version without new references,
/// and bits changed at a recorded version where its fingerprint or a reference sees them;
/// `audio-cli bless` refuses to record either.
///
/// Versions count from 1 ([`AlgorithmVersion::FIRST`]); a box type declares its version in
/// its [`ObjectSpec`], so a box type without one does not compile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AlgorithmVersion(NonZeroU32);

impl AlgorithmVersion {
    /// Version 1, the first algorithm of a box type.
    pub const FIRST: Self = Self(NonZeroU32::MIN);

    /// The version after this one, the only way to write a version above the first: a box
    /// type whose bits change declares `AlgorithmVersion::FIRST.next()`, in a `const`. The
    /// largest version stays the largest, never wrapping to an earlier one.
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }

    /// Its number, from 1.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

impl fmt::Display for AlgorithmVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Static description of a box type: everything the editor, the AI assistant, the
/// documentation and the plan builder need to know about it.
///
/// Built only by [`ObjectSpec::new`], from the type implementing [`Create`], which gives
/// both its parameters and its factory: no spec pairs a factory with parameters other
/// than its own type's, so a box is created only from values checked against its own
/// parameters. A box type is its spec, a static: neither cloned nor copied. A box type
/// whose creation and parameters are written once:
///
/// ```
/// use audio_core::{
///     AlgorithmVersion, Binary64, Block, Create, Driven, Object, ObjectSpec, ParamSpec, Sample,
///     SampleRate,
/// };
///
/// #[derive(Debug)]
/// struct Silent;
///
/// impl<S: Sample, const N: usize> Object<S, N> for Silent {
///     fn process(&mut self, _inputs: &[Block<S, N>], _outputs: &mut [Block<S, N>]) {}
/// }
///
/// impl Create<0> for Silent {
///     const BOX_MODE: audio_core::BoxMode = audio_core::BoxMode::LiveCapable;
///     const PARAMS: &'static [ParamSpec; 0] = &[];
///
///     fn create<S: Sample, const N: usize>(
///         _rate: SampleRate,
///         []: [Binary64; 0],
///     ) -> impl Driven<S, N, 0> + 'static {
///         Self
///     }
/// }
///
/// fn copy() -> ObjectSpec {
///     ObjectSpec::new::<Silent, 0>("copy~", "silence", &[], &[], &[], AlgorithmVersion::FIRST)
/// }
/// ```
///
/// Its fields are private, so another box type's parameters cannot be paired with its
/// factory by a struct update:
///
/// ```compile_fail,E0451
/// use audio_core::{
///     AlgorithmVersion, Binary64, Block, Create, Driven, Object, ObjectSpec, ParamSpec, Sample,
///     SampleRate,
/// };
///
/// #[derive(Debug)]
/// struct Silent;
///
/// impl<S: Sample, const N: usize> Object<S, N> for Silent {
///     fn process(&mut self, _inputs: &[Block<S, N>], _outputs: &mut [Block<S, N>]) {}
/// }
///
/// impl Create<0> for Silent {
///     const BOX_MODE: audio_core::BoxMode = audio_core::BoxMode::LiveCapable;
///     const PARAMS: &'static [ParamSpec; 0] = &[];
///
///     fn create<S: Sample, const N: usize>(
///         _rate: SampleRate,
///         []: [Binary64; 0],
///     ) -> impl Driven<S, N, 0> + 'static {
///         Self
///     }
/// }
///
/// fn copy() -> ObjectSpec {
///     ObjectSpec {
///         name: "copy~",
///         ..ObjectSpec::new::<Silent, 0>(
///             "silent~",
///             "silence",
///             &[],
///             &[],
///             &[],
///             AlgorithmVersion::FIRST,
///         )
///     }
/// }
/// ```
#[derive(Debug)]
pub struct ObjectSpec {
    /// Name used in patches, for example `osc~`.
    name: &'static str,
    /// What the box does.
    description: &'static str,
    /// Inlets, in order.
    inlets: &'static [PortSpec],
    /// Outlets, in order.
    outlets: &'static [PortSpec],
    /// Parameters, in order: those of the type its factory creates ([`Create::PARAMS`]).
    params: &'static [ParamSpec],
    /// The execution modes the box type may run in ([`Create::BOX_MODE`]).
    mode: crate::BoxMode,
    /// Capabilities the box needs from the host platform.
    capabilities: &'static [Capability],
    /// Creates its instances, in either precision.
    create: Factory,
    /// The version of its algorithm (decision 29): a new one for any change that can
    /// change a bit of its output.
    algorithm_version: AlgorithmVersion,
}

impl ObjectSpec {
    /// The box type `name`, described by `description`, with the ports `inlets` and
    /// `outlets`, the `capabilities` it needs and the version of its algorithm
    /// `algorithm_version`, whose parameters and instances are those of `T`
    /// ([`Create::PARAMS`], [`Create::create`]).
    ///
    /// A `const fn`, so that a box type is a static. Its description, by the rules of a
    /// module's (`spec/box-contract.md`, section 4: texts that show, names that are unique and
    /// hold no character a message escapes), is checked by the box laws
    /// (`audio_testkit::check_description`).
    #[must_use]
    pub const fn new<T: Create<K>, const K: usize>(
        name: &'static str,
        description: &'static str,
        inlets: &'static [PortSpec],
        outlets: &'static [PortSpec],
        capabilities: &'static [Capability],
        algorithm_version: AlgorithmVersion,
    ) -> Self {
        Self {
            name,
            description,
            inlets,
            outlets,
            params: T::PARAMS,
            mode: T::BOX_MODE,
            capabilities,
            create: Factory::new::<T, K>(),
            algorithm_version,
        }
    }

    /// Name used in patches, for example `osc~`.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// The execution modes the box type may run in, as it declares them ([`Create::BOX_MODE`]).
    #[must_use]
    pub const fn mode(&self) -> crate::BoxMode {
        self.mode
    }

    /// What the box does.
    #[must_use]
    pub const fn description(&self) -> &'static str {
        self.description
    }

    /// Inlets, in order.
    #[must_use]
    pub const fn inlets(&self) -> &'static [PortSpec] {
        self.inlets
    }

    /// Outlets, in order.
    #[must_use]
    pub const fn outlets(&self) -> &'static [PortSpec] {
        self.outlets
    }

    /// Parameters, in order: those of the type that creates its instances.
    #[must_use]
    pub const fn params(&self) -> &'static [ParamSpec] {
        self.params
    }

    /// Capabilities the box needs from the host platform.
    #[must_use]
    pub const fn capabilities(&self) -> &'static [Capability] {
        self.capabilities
    }

    /// The version of its algorithm (decision 29): a new one for any change that can
    /// change a bit of its output.
    #[must_use]
    pub const fn algorithm_version(&self) -> AlgorithmVersion {
        self.algorithm_version
    }
}

/// A box instance, processing audio one block of `N` frames at a time (the engine block,
/// [`BLOCK_SIZE`] by default), in the samples `S` of its render.
pub trait Object<S: Sample, const N: usize = BLOCK_SIZE>: Send + fmt::Debug {
    /// Processes one block.
    ///
    /// `inputs` holds one block per inlet and `outputs` one block per outlet, in the
    /// order declared by the box's [`ObjectSpec`]. Must not allocate or free memory, lock
    /// or block.
    ///
    /// A box writes every sample of every outlet at every block: what `outputs` holds on
    /// entry is unspecified (`spec/engine-design.md`, decision 25), so that no box's bits
    /// depend on how the engine keeps its buffers; a box that needs its last output keeps it
    /// as its own state. The box laws give the outputs two different values on entry and
    /// require the same bits, and a NaN on entry through a long run of silence, which an
    /// output left unwritten keeps (`audio_testkit::check_laws`).
    fn process(&mut self, inputs: &[Block<S, N>], outputs: &mut [Block<S, N>]);
}

/// A boxed instance processes as the instance it holds.
impl<S: Sample, const N: usize, T: Object<S, N> + ?Sized> Object<S, N> for Box<T> {
    fn process(&mut self, inputs: &[Block<S, N>], outputs: &mut [Block<S, N>]) {
        (**self).process(inputs, outputs);
    }
}

/// What a box with `K` parameters writes beside [`Object::process`]: its second body, for the
/// blocks where a parameter changes, and a way to take new values between two blocks.
///
/// [`Object::process`] renders the blocks where every parameter keeps one value; a block where
/// one changes is rendered by [`Driven::process_driven`], which is given every parameter as a
/// block of values, one per frame. The two must agree: a render does not
/// depend on which body rendered a block (`audio_testkit::check_driven_laws`, over every box
/// type of a registry). [`Create::create`] returns an instance of it, so a box with
/// parameters and no second body does not compile; a box without parameters writes nothing,
/// being driven by none.
///
/// The arithmetic of a frame is best written once, as a private function both bodies call,
/// so that they cannot differ in it.
pub trait Driven<S: Sample, const N: usize, const K: usize>: Object<S, N> {
    /// Gives the parameters the values `params`, from the next frame on, the state of the box
    /// being otherwise what it was: a box derives from them what it needs, as at its creation.
    /// Called between two blocks; must not allocate or free memory, lock or block.
    ///
    /// After a driven block whose last values stay, it is called before the next
    /// [`Object::process`], so a box need not remember the values it was driven by: by the
    /// driven laws (`audio_testkit::check_driven_laws`), and by a plan from #12 step 41.
    fn set(&mut self, params: [Binary64; K]);

    /// Processes one block as [`Object::process`] does, frame `f` of it with the values
    /// `params[..][f]`, as if [`Driven::set`] had given them before that frame: under the same
    /// rules (every sample of every outlet written, no heap, no lock).
    ///
    /// Each value is within its parameter's range, finite, none below 2^-126 in magnitude
    /// but zero: an instance takes them as [`DrivenValues`], which checks them
    /// ([`BoxInstance::run_driven`]; a plan calls it from #12 step 41).
    fn process_driven(
        &mut self,
        inputs: &[Block<S, N>],
        outputs: &mut [Block<S, N>],
        params: &[Block<Binary64, N>; K],
    );
}

/// A box without parameters has none to be driven by: it renders every block by
/// [`Object::process`], and has nothing to write.
impl<S: Sample, const N: usize, T: Object<S, N>> Driven<S, N, 0> for T {
    fn set(&mut self, []: [Binary64; 0]) {}

    fn process_driven(
        &mut self,
        inputs: &[Block<S, N>],
        outputs: &mut [Block<S, N>],
        []: &[Block<Binary64, N>; 0],
    ) {
        self.process(inputs, outputs);
    }
}

/// A box instance as the engine holds it, its box type's number of parameters forgotten.
///
/// The parameters come as the values a plan checked, which carry their box type, and an
/// instance takes only those of its own.
pub trait BoxInstance<S: Sample, const N: usize = BLOCK_SIZE>: Object<S, N> {
    /// Gives the parameters the values `values` ([`Driven::set`]); false, with nothing done,
    /// when they were checked against another box type than the instance's, whose ranges
    /// may refuse them ([`ParamValues::spec`]).
    #[must_use]
    fn receive(&mut self, values: &ParamValues) -> bool;

    /// Processes one block where a parameter changes ([`Driven::process_driven`]); false,
    /// with nothing written, when `values` were checked against another box type than the
    /// instance's ([`DrivenValues::spec`]).
    #[must_use]
    fn run_driven(
        &mut self,
        inputs: &[Block<S, N>],
        outputs: &mut [Block<S, N>],
        values: &DrivenValues<N>,
    ) -> bool;
}

/// An instance of `K` parameters of the type `I`, tied to its box type ([`BoxInstance`]).
///
/// It hands the box only values checked against that box type, and answers `false` to any
/// other, at each call.
///
/// The box type is told by the address of its spec, as a plan holds it: a spec is a
/// `static`, and values carry the reference they were checked against, so the values a plan
/// checked against the spec it created a box from are taken, and those of another spec are
/// refused, as many as they may be. One pointer compared per call, once per block at most.
#[derive(Debug)]
pub struct Typed<I, const K: usize> {
    /// The box type of the instance.
    box_type: BoxType,
    /// The instance.
    instance: I,
}

impl<I, const K: usize> Typed<I, K> {
    /// `instance`, an instance of the box type `spec` created from values checked against
    /// it, as the engine holds it.
    ///
    /// `K` is that box type's number of parameters, as [`Create`] gives it. Every box has a
    /// body for none ([`Driven`]), so a `K` of 0 compiles for a box with parameters, an
    /// instance that takes no values of its own box type: the driven laws refuse it.
    #[must_use]
    pub const fn new(spec: &'static ObjectSpec, instance: I) -> Self {
        Self {
            box_type: BoxType(spec),
            instance,
        }
    }

    /// Whether `spec` is the instance's box type.
    #[inline]
    fn takes(&self, spec: &'static ObjectSpec) -> bool {
        core::ptr::eq(spec, self.box_type.0)
    }
}

/// A box type, shown by its name: its spec's other fields, which a derived form would show,
/// would put the formatting of every one of them in a module that shows none
/// (`spec/benchmarks.md`, the engine module's bytes).
#[derive(Clone, Copy)]
struct BoxType(&'static ObjectSpec);

impl fmt::Debug for BoxType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.0.name, formatter)
    }
}

impl<S: Sample, const N: usize, I: Object<S, N>, const K: usize> Object<S, N> for Typed<I, K> {
    #[inline]
    fn process(&mut self, inputs: &[Block<S, N>], outputs: &mut [Block<S, N>]) {
        self.instance.process(inputs, outputs);
    }
}

impl<S: Sample, const N: usize, I: Driven<S, N, K>, const K: usize> BoxInstance<S, N>
    for Typed<I, K>
{
    #[inline]
    fn receive(&mut self, values: &ParamValues) -> bool {
        match values.params().array::<K>() {
            Some(params) if self.takes(values.spec) => {
                Driven::<S, N, K>::set(&mut self.instance, params);
                true
            }
            _ => false,
        }
    }

    #[inline]
    fn run_driven(
        &mut self,
        inputs: &[Block<S, N>],
        outputs: &mut [Block<S, N>],
        values: &DrivenValues<N>,
    ) -> bool {
        match <&[Block<Binary64, N>; K]>::try_from(values.blocks.as_slice()) {
            Ok(blocks) if self.takes(values.spec) => {
                Driven::<S, N, K>::process_driven(&mut self.instance, inputs, outputs, blocks);
                true
            }
            _ => false,
        }
    }
}

/// A boxed instance is given its parameters as the instance it holds.
impl<S: Sample, const N: usize, T: BoxInstance<S, N> + ?Sized> BoxInstance<S, N> for Box<T> {
    fn receive(&mut self, values: &ParamValues) -> bool {
        (**self).receive(values)
    }

    fn run_driven(
        &mut self,
        inputs: &[Block<S, N>],
        outputs: &mut [Block<S, N>],
        values: &DrivenValues<N>,
    ) -> bool {
        (**self).run_driven(inputs, outputs, values)
    }
}

/// What the instance type of a plan implements: how the plan holds the instances of its
/// boxes and calls them, each created from its box type.
///
/// An instance is created from its parameter values alone, which carry the box type they
/// were checked against, so that no caller can give a box type the values of another:
///
/// ```compile_fail,E0061
/// use audio_core::{Binary64, Dispatch, BoxInstance, ObjectSpec, ParamValues, SampleRate};
///
/// fn create(
///     spec: &'static ObjectSpec,
///     rate: SampleRate,
///     values: &ParamValues,
/// ) -> Option<Box<dyn BoxInstance<Binary64>>> {
///     <Box<dyn BoxInstance<Binary64>> as Dispatch<Binary64>>::create(spec, rate, values.params())
/// }
/// ```
///
/// The same function from the values alone compiles, so the separate spec is the only
/// error above:
///
/// ```
/// use audio_core::{Binary64, Dispatch, BoxInstance, ParamValues, SampleRate};
///
/// fn create(rate: SampleRate, values: &ParamValues) -> Option<Box<dyn BoxInstance<Binary64>>> {
///     <Box<dyn BoxInstance<Binary64>> as Dispatch<Binary64>>::create(rate, values)
/// }
/// ```
pub trait Dispatch<S: Sample, const N: usize = BLOCK_SIZE>: BoxInstance<S, N> + Sized {
    /// The instance at `rate` of the box type `values` were checked against
    /// ([`ParamValues::spec`]), created with them as [`Create::create`] receives them; none
    /// when this instance type cannot create that box type in blocks of `N` frames in this
    /// build (a box type through its virtual table, in a build naming another engine block,
    /// [`crate::NAMED_ENGINE_BLOCK`]).
    fn create(rate: SampleRate, values: &ParamValues) -> Option<Self>;
}

/// Every box type through its factory, each instance called through its virtual table, at
/// any engine block: an instance at the sizes of [`crate::ENGINE_BLOCKS`] the build holds,
/// those a factory holds a constructor for, none at any other. Outside 1 to
/// [`crate::MAX_ENGINE_BLOCK`] frames (decision 2), it does not compile, 0 and one more
/// alike:
///
/// ```compile_fail,E0080
/// use audio_core::{Binary64, Dispatch, BoxInstance};
/// let create: fn(_, &_) -> Option<_> = <Box<dyn BoxInstance<Binary64, 0>> as Dispatch<Binary64, 0>>::create;
/// let _ = std::hint::black_box(create);
/// ```
///
/// ```compile_fail,E0080
/// use audio_core::{Binary64, Dispatch, BoxInstance};
/// let create: fn(_, &_) -> Option<_> = <Box<dyn BoxInstance<Binary64, 1_048_577>> as Dispatch<Binary64, 1_048_577>>::create;
/// let _ = std::hint::black_box(create);
/// ```
impl<S: Sample, const N: usize> Dispatch<S, N> for Box<dyn BoxInstance<S, N>> {
    fn create(rate: SampleRate, values: &ParamValues) -> Option<Self> {
        crate::block::engine_block::<N>();
        values.spec.create.create(rate, values)
    }
}

#[cfg(test)]
#[path = "object_tests.rs"]
mod tests;
