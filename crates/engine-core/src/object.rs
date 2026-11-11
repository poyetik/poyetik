//! The contract every box implements.

#![cfg_attr(not(test), forbid(clippy::arithmetic_side_effects))]

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::fmt;
use core::num::NonZeroU32;
use core::ops::Deref;

use crate::{Binary32, Binary64, Block, Constructor, Sample, SampleRate};

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
    /// The parameter `name`, described by `description`, of default value `default`,
    /// accepting `min` to `max` (inclusive).
    #[must_use]
    pub const fn new(
        name: &'static str,
        description: &'static str,
        default: f32,
        min: f32,
        max: f32,
    ) -> Self {
        Self {
            name,
            description,
            default_value: default,
            minimum: min,
            maximum: max,
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

/// The parameter values of a box instance, in the order of [`ObjectSpec::params`].
///
/// In binary64 whatever the precision of the render (`spec/engine-design.md`, decision
/// 10), and never below 2^-126 in magnitude but zero: [`Params::new`], the only way to
/// build one, makes such a value a zero of its sign (decisions 5 and 11), as a plan reads
/// a patch's parameters. A box can rely on both, whoever created it: the plan, the box
/// laws or the fingerprint battery.
#[derive(Debug, Clone)]
pub struct Params(Vec<Binary64>);

impl Params {
    /// The parameter values `values`, in order, each nonzero one below 2^-126 in
    /// magnitude made a zero of its sign; every other value keeps its bits.
    #[must_use]
    pub fn new(values: impl IntoIterator<Item = Binary64>) -> Self {
        Self(values.into_iter().map(Binary64::as_parameter).collect())
    }
}

impl Deref for Params {
    type Target = [Binary64];

    /// The values, in order.
    fn deref(&self) -> &[Binary64] {
        &self.0
    }
}

/// A box type's way of creating its instances, written once for every precision: the
/// plan calls [`Create::create`] with the [`Sample`] type of the render.
pub trait Create {
    /// Creates an instance computing in `S`, from a sample rate and its parameter values.
    ///
    /// The values come as [`Params`] says (in order, in binary64, none below 2^-126 but
    /// zero); a plan also validates them against their ranges first.
    fn create<S: Sample>(rate: SampleRate, params: &Params) -> Box<dyn Object<S>>;
}

/// Creates the instances of a box type, in either precision: built only from one generic
/// [`Create::create`], so that a box type cannot behave differently by precision but
/// through the [`Sample`] it is given.
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
    /// [`Create::create`] receives them.
    #[must_use]
    pub fn create<S: Sample>(&self, rate: SampleRate, params: &Params) -> Box<dyn Object<S>> {
        S::constructor(self)(rate, params)
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

/// How a plan holds the instances of its boxes and calls them: the type of every instance
/// of a plan, created from its box type.
pub trait Instance<S: Sample>: Object<S> + Sized {
    /// The instance of the box type `spec` at `rate`, with `params` as [`Create::create`]
    /// receives them.
    fn create(spec: &'static ObjectSpec, rate: SampleRate, params: &Params) -> Self;
}

/// Every box type through its factory, each instance called through its virtual table.
impl<S: Sample> Instance<S> for Box<dyn Object<S>> {
    fn create(spec: &'static ObjectSpec, rate: SampleRate, params: &Params) -> Self {
        spec.create.create(rate, params)
    }
}

#[cfg(test)]
#[path = "object_tests.rs"]
mod tests;
