//! The contract every box implements.

#![cfg_attr(not(test), forbid(clippy::arithmetic_side_effects))]

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::fmt;
use core::num::NonZeroU32;
use core::ops::Deref;

use crate::host::Raw as _;
use crate::math::{Constructor, sealed};
use crate::{Binary32, Binary64, Block, Sample, SampleRate, flush};

/// A capability a box needs from the host platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Capability {
    /// Sends audio to the host output (speakers, plugin output). The plan sums the
    /// inlets of such boxes into the engine output channels.
    AudioOutput,
}

/// Description of an inlet or an outlet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortSpec {
    /// Short name.
    pub name: &'static str,
    /// What flows through the port.
    pub description: &'static str,
}

/// Description of a parameter: its range and default value.
///
/// Its values are binary32 floats, which box code never reads: they are private, and only
/// the engine and a host (through [`crate::host::ParamBounds`]) read them, so that a box
/// reaches a raw float through no parameter (`spec/engine-design.md`, decision 10). A box
/// receives its parameter values as [`Binary64`], from the plan. Reading one outside the
/// engine but through `host` does not compile:
///
/// ```compile_fail,E0616
/// fn default(param: &engine_core::ParamSpec) -> f32 {
///     param.default_value
/// }
/// ```
///
/// The same function through `host` compiles, so the field's privacy is the only error
/// above:
///
/// ```
/// use engine_core::host::ParamBounds;
///
/// fn default(param: &engine_core::ParamSpec) -> f32 {
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
    pub(crate) default_value: f32,
    /// Lowest accepted value.
    pub(crate) minimum: f32,
    /// Highest accepted value.
    pub(crate) maximum: f32,
}

impl ParamSpec {
    /// The parameter `name`, described by `description`, of default value
    /// `default_value`, accepting `minimum` to `maximum` (inclusive).
    ///
    /// A `const fn`, so that a static spec is checked when it is compiled: [`crate::param!`]
    /// evaluates it in a `const` block, where a refused spec does not compile. A spec built
    /// at run time (a box type loaded then) gets the `Result`.
    ///
    /// # Errors
    ///
    /// Returns [`ParamError::Range`] unless `minimum` and `maximum` are finite and `minimum`
    /// is at most `maximum`, and [`ParamError::Default`] unless `default_value`, flushed as a
    /// plan flushes it (a subnormal is a zero of its sign), is within them, so that every
    /// parameter of every box type has a finite range holding its default, as a plan, the
    /// box laws and the fingerprint battery give it.
    pub const fn new(
        name: &'static str,
        description: &'static str,
        default_value: f32,
        minimum: f32,
        maximum: f32,
    ) -> Result<Self, ParamError> {
        if !(minimum.is_finite() && maximum.is_finite() && minimum <= maximum) {
            return Err(ParamError::Range { name });
        }
        let flushed = flush::binary32(default_value);
        if minimum <= flushed && flushed <= maximum {
            Ok(Self {
                name,
                description,
                default_value,
                minimum,
                maximum,
            })
        } else {
            Err(ParamError::Default { name })
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

    /// Whether `value` is within range. Always `false` for NaN.
    #[must_use]
    pub fn accepts(&self, value: f32) -> bool {
        self.minimum <= value && value <= self.maximum
    }
}

/// A [`ParamSpec`] checked when the program is compiled.
///
/// [`ParamSpec::new`] evaluated in a `const` block, so that a parameter whose range is not
/// finite, or whose default lies outside it, does not compile. The way to write the
/// parameters of a static box type:
///
/// ```
/// static FREQUENCY: engine_core::ParamSpec =
///     engine_core::param!("frequency", "in hertz", 440.0, 0.0, 20_000.0);
/// ```
///
/// A default above the maximum, below the minimum, not a number, or below 2^-126 in
/// magnitude where the range starts above zero (flushed, it is zero), does not compile:
///
/// ```compile_fail,E0080
/// static LOUD: engine_core::ParamSpec = engine_core::param!("gain", "", 1.5, 0.0, 1.0);
/// ```
///
/// ```compile_fail,E0080
/// static LOW: engine_core::ParamSpec = engine_core::param!("gain", "", -0.5, 0.0, 1.0);
/// ```
///
/// ```compile_fail,E0080
/// static NAN: engine_core::ParamSpec = engine_core::param!("gain", "", f32::NAN, 0.0, 1.0);
/// ```
///
/// ```compile_fail,E0080
/// static TINY: engine_core::ParamSpec = engine_core::param!("t", "", 1e-40, 1e-41, 1.0);
/// ```
///
/// Nor does a range that is not finite, nor one whose minimum is above its maximum:
///
/// ```compile_fail,E0080
/// static OPEN: engine_core::ParamSpec =
///     engine_core::param!("gain", "", 0.5, 0.0, f32::INFINITY);
/// ```
///
/// ```compile_fail,E0080
/// static EMPTY: engine_core::ParamSpec = engine_core::param!("gain", "", 0.5, 1.0, 0.0);
/// ```
#[macro_export]
macro_rules! param {
    ($name:expr, $description:expr, $default_value:expr, $minimum:expr, $maximum:expr $(,)?) => {
        const {
            match $crate::ParamSpec::new($name, $description, $default_value, $minimum, $maximum) {
                ::core::result::Result::Ok(spec) => spec,
                ::core::result::Result::Err(_) => ::core::panic!(
                    "a parameter's range is not finite, or its default lies outside it \
                     (`engine_core::ParamSpec::new` says which)"
                ),
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
/// values alone ([`Dispatch::create`]), of the box type they were checked against, so a
/// box can rely on all of it, whoever created it: the plan, the box laws, the fingerprint
/// battery or a host. No unchecked constructor is public:
///
/// ```compile_fail,E0599
/// let _values = engine_core::ParamValues::new([engine_core::Binary64::new(1e9)]);
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

/// Why parameter values are not those of a box type ([`ParamValues::of`]), or a parameter's
/// description is refused ([`ParamSpec::new`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamError {
    /// Not one value per parameter of the box type.
    Count {
        /// The box type.
        kind: &'static str,
        /// How many parameters it has.
        expected: usize,
        /// How many values were given.
        given: usize,
    },
    /// A value outside its parameter's range, or not a number, once flushed.
    OutOfRange {
        /// The box type.
        kind: &'static str,
        /// The parameter.
        name: &'static str,
        /// The bits of the binary64 value refused.
        bits: u64,
    },
    /// A parameter whose minimum or maximum is not finite, or whose minimum is above its
    /// maximum ([`ParamSpec::new`]).
    Range {
        /// The parameter.
        name: &'static str,
    },
    /// A parameter whose default, flushed, is outside its range or not a number
    /// ([`ParamSpec::new`]).
    Default {
        /// The parameter.
        name: &'static str,
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
            Self::OutOfRange { kind, name, bits } => write!(
                f,
                "parameter \"{name}\" of box type \"{kind}\": {} is outside its range",
                f64::from_bits(bits)
            ),
            Self::Range { name } => write!(
                f,
                "parameter \"{name}\": its range is not finite, or its minimum is above its maximum"
            ),
            Self::Default { name } => {
                write!(f, "parameter \"{name}\": its default is outside its range")
            }
        }
    }
}

/// An error of its own, with no cause: a host passes it on as any other.
impl core::error::Error for ParamError {}

impl ParamValues {
    /// The parameter values `values` of the box type `spec`: one per parameter, in order,
    /// each nonzero one below 2^-126 in magnitude made a zero of its sign, then checked
    /// against its parameter's range widened to binary64 (exactly, as a plan checks a
    /// patch's value against it in binary32); every other value keeps its bits. So it also
    /// accepts binary64 values no binary32 parameter holds, which a plan never gives while
    /// patch parameters are binary32 (`spec/engine-design.md`, decision 10).
    ///
    /// # Errors
    ///
    /// Returns [`ParamError::Count`] unless there is one value per parameter, and
    /// [`ParamError::OutOfRange`] for the first value outside its range or NaN.
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
                let wide = flushed.raw();
                if f64::from(param.minimum) <= wide && wide <= f64::from(param.maximum) {
                    Ok(flushed)
                } else {
                    Err(ParamError::OutOfRange {
                        kind: spec.name,
                        name: param.name,
                        bits: wide.to_bits(),
                    })
                }
            })
            .collect::<Result<_, _>>()
            .map(|values| Self { spec, values })
    }

    /// The default values of the box type `spec`, each a binary32 value widened exactly and
    /// flushed as a plan flushes it: within its range, as [`ParamSpec::new`] checks.
    #[must_use]
    pub fn defaults(spec: &'static ObjectSpec) -> Self {
        Self {
            spec,
            values: spec
                .params
                .iter()
                .map(|param| Binary64::new(f64::from(param.default_value)).as_parameter())
                .collect(),
        }
    }

    /// The parameter values `values` of the box type `spec`, which the plan has checked
    /// against their ranges and
    /// flushed as [`ParamValues::of`] would (each a binary32 value flushed by
    /// [`crate::flush::binary32`], widened exactly): taken as they are, so that a patch's
    /// parameters are read with no second flush (the plan tests check what a box is given,
    /// and the flush tests that [`ParamValues::of`] gives a binary32 value the same).
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

/// The parameter values a box is created with, borrowed from [`ParamValues`].
///
/// They keep its promise: one per parameter of the box type, in order, each within its
/// range, in binary64, none below 2^-126 but zero. Passed by value, as cheap as a slice
/// (two words: a reference to the owned values made loading a plan dearer,
/// `spec/benchmarks.md`), and read as one.
#[derive(Debug, Clone, Copy)]
pub struct Params<'values>(&'values [Binary64]);

impl Deref for Params<'_> {
    type Target = [Binary64];

    /// The values, in order.
    fn deref(&self) -> &[Binary64] {
        self.0
    }
}

/// A box type's way of creating its instances, written once for every precision: the
/// plan calls [`Create::create`] with the [`Sample`] type of the render.
pub trait Create {
    /// Creates an instance computing in `S`, from a sample rate and its parameter values.
    ///
    /// The values come as [`Params`] says: one per parameter, in order, each within its
    /// range, in binary64, none below 2^-126 but zero. The engine calls it only through the
    /// box type's [`Factory`], from values checked against the box type's own spec
    /// ([`Dispatch::create`]); a box type keeps the type implementing it private, so that no
    /// other caller can give it the values of another box type (the engine's own box types
    /// do).
    fn create<S: Sample>(rate: SampleRate, params: Params<'_>) -> Box<dyn Object<S>>;
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
/// use engine_core::{Binary64, Factory, Object, ParamValues, SampleRate};
///
/// fn create(factory: &Factory, rate: SampleRate, values: &ParamValues) -> Box<dyn Object<Binary64>> {
///     factory.create(rate, values.params())
/// }
/// ```
///
/// ```compile_fail,E0599
/// use engine_core::{Binary64, Factory, Sample};
///
/// fn pick(factory: &Factory) {
///     let _constructor = Binary64::constructor(factory);
/// }
/// ```
///
/// ```compile_fail,E0432
/// use engine_core::Constructor;
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Factory {
    /// Creates a binary64 instance.
    pub(crate) binary64: Constructor<Binary64>,
    /// Creates a binary32 instance.
    pub(crate) binary32: Constructor<Binary32>,
}

impl Factory {
    /// The factory of the box type `T`.
    #[must_use]
    pub const fn new<T: Create>() -> Self {
        Self {
            binary64: T::create::<Binary64>,
            binary32: T::create::<Binary32>,
        }
    }

    /// Creates an instance computing in `S`, at `rate`, with `params` as
    /// [`Create::create`] receives them: crate-private, since `params` do not say their box
    /// type; [`Dispatch::create`] reads it from the values.
    #[must_use]
    pub(crate) fn create<S: Sample>(
        &self,
        rate: SampleRate,
        params: Params<'_>,
    ) -> Box<dyn Object<S>> {
        <S as sealed::Construct>::constructor(self)(rate, params)
    }
}

/// The version of the engine's render rules (`spec/engine-design.md`, decision 29).
///
/// The render rules are the bits of a render that no box type owns (the plan's sums and
/// their order, when events take effect, flush points, the output stage, delivery). Any
/// change of them that can change a bit of a render takes a new one; version 1 is the
/// rules of the reduction order
/// (decision 28). The conformance references record it, and bless refuses a changed
/// reference under the same one.
pub const RENDER_RULES_VERSION: u32 = 1;

/// The version of a box type's algorithm (`spec/engine-design.md`, decision 29).
///
/// Any change of a box type that can change a bit of its output takes a new one, and the
/// conformance references record the version of every box type they were blessed with,
/// with its fingerprint: `engine-cli check` refuses a new version without new references,
/// and bits changed at a recorded version where its fingerprint or a reference sees them;
/// `engine-cli bless` refuses to record either.
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
#[derive(Debug, Clone, Copy)]
pub struct ObjectSpec {
    /// Name used in patches, for example `osc~`.
    pub name: &'static str,
    /// What the box does.
    pub description: &'static str,
    /// Inlets, in order.
    pub inlets: &'static [PortSpec],
    /// Outlets, in order.
    pub outlets: &'static [PortSpec],
    /// Parameters, in order.
    pub params: &'static [ParamSpec],
    /// Capabilities the box needs from the host platform.
    pub capabilities: &'static [Capability],
    /// Creates its instances, in either precision.
    pub create: Factory,
    /// The version of its algorithm (decision 29): a new one for any change that can
    /// change a bit of its output.
    pub algorithm_version: AlgorithmVersion,
}

/// A box instance, processing audio one block at a time, in the samples `S` of its render.
pub trait Object<S: Sample>: Send + fmt::Debug {
    /// Processes one block.
    ///
    /// `inputs` holds one block per inlet and `outputs` one block per outlet, in the
    /// order declared by the box's [`ObjectSpec`]. Must not allocate or free memory, lock
    /// or block.
    fn process(&mut self, inputs: &[Block<S>], outputs: &mut [Block<S>]);
}

/// A boxed instance processes as the instance it holds.
impl<S: Sample, T: Object<S> + ?Sized> Object<S> for Box<T> {
    fn process(&mut self, inputs: &[Block<S>], outputs: &mut [Block<S>]) {
        (**self).process(inputs, outputs);
    }
}

/// What the instance type of a plan implements: how the plan holds the instances of its
/// boxes and calls them, each created from its box type.
///
/// An instance is created from its parameter values alone, which carry the box type they
/// were checked against, so that no caller can give a box type the values of another:
///
/// ```compile_fail,E0061
/// use engine_core::{Binary64, Dispatch, Object, ObjectSpec, ParamValues, SampleRate};
///
/// fn create(spec: &'static ObjectSpec, rate: SampleRate, values: &ParamValues) -> Box<dyn Object<Binary64>> {
///     <Box<dyn Object<Binary64>> as Dispatch<Binary64>>::create(spec, rate, values.params())
/// }
/// ```
///
/// The same function from the values alone compiles, so the separate spec is the only
/// error above:
///
/// ```
/// use engine_core::{Binary64, Dispatch, Object, ParamValues, SampleRate};
///
/// fn create(rate: SampleRate, values: &ParamValues) -> Box<dyn Object<Binary64>> {
///     <Box<dyn Object<Binary64>> as Dispatch<Binary64>>::create(rate, values)
/// }
/// ```
pub trait Dispatch<S: Sample>: Object<S> + Sized {
    /// The instance at `rate` of the box type `values` were checked against
    /// ([`ParamValues::spec`]), created with them as [`Create::create`] receives them.
    fn create(rate: SampleRate, values: &ParamValues) -> Self;
}

/// Every box type through its factory, each instance called through its virtual table.
impl<S: Sample> Dispatch<S> for Box<dyn Object<S>> {
    fn create(rate: SampleRate, values: &ParamValues) -> Self {
        values.spec.create.create(rate, values.params())
    }
}

#[cfg(test)]
#[path = "object_tests.rs"]
mod tests;
