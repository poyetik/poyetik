//! The contract every box implements.

use alloc::boxed::Box;

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
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamSpec {
    /// Name used in patches.
    pub name: &'static str,
    /// What the parameter controls.
    pub description: &'static str,
    /// Value used when the patch does not set it.
    pub default: f32,
    /// Lowest accepted value.
    pub min: f32,
    /// Highest accepted value.
    pub max: f32,
}

impl ParamSpec {
    /// Whether `value` is within range. Always `false` for NaN.
    #[must_use]
    pub fn accepts(&self, value: f32) -> bool {
        self.min <= value && value <= self.max
    }
}

/// A box type's way of creating its instances, written once for every precision: the
/// plan calls [`Create::create`] with the [`Sample`] type of the render.
pub trait Create {
    /// Creates an instance computing in `S`, from a sample rate and its parameter values.
    ///
    /// The values come in the order of [`ObjectSpec::params`], already validated against
    /// their ranges, never subnormal (a plan flushes a subnormal parameter to a zero of its
    /// sign, [`Sample::flush`]), and given in binary64 whatever the precision of the render
    /// (`spec/engine-design.md`, decision 10).
    fn create<S: Sample>(rate: SampleRate, params: &[Binary64]) -> Box<dyn Object<S>>;
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
    pub fn create<S: Sample>(&self, rate: SampleRate, params: &[Binary64]) -> Box<dyn Object<S>> {
        S::constructor(self)(rate, params)
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
}

/// A box instance, processing audio one block at a time, in the samples `S` of its render.
pub trait Object<S: Sample>: Send + core::fmt::Debug {
    /// Processes one block.
    ///
    /// `inputs` holds one block per inlet and `outputs` one block per outlet, in the
    /// order declared by the box's [`ObjectSpec`]. Must not allocate or free memory, lock
    /// or block.
    fn process(&mut self, inputs: &[Block<S>], outputs: &mut [Block<S>]);
}

#[cfg(test)]
mod tests {
    use super::ParamSpec;

    const PARAM: ParamSpec = ParamSpec {
        name: "p",
        description: "p",
        default: 0.0,
        min: -1.0,
        max: 2.0,
    };

    #[test]
    fn accepts_values_within_range_inclusive() {
        for value in [-1.0_f32, 0.0, 2.0] {
            assert!(PARAM.accepts(value), "{value} is accepted");
        }
    }

    #[test]
    fn rejects_values_outside_range_and_nan() {
        for value in [-1.5_f32, 2.5, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(!PARAM.accepts(value), "{value} is rejected");
        }
    }
}
