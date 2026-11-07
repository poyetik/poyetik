//! The contract every box implements.

use alloc::boxed::Box;

use crate::{Block, SampleRate};

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

/// Creates a box instance from a sample rate and its parameter values, given in the
/// order of [`ObjectSpec::params`] and already validated against their ranges.
pub type Factory = fn(SampleRate, &[f32]) -> Box<dyn Object>;

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
    /// Creates an instance.
    pub create: Factory,
}

/// A box instance, processing audio one block at a time.
pub trait Object: Send + core::fmt::Debug {
    /// Processes one block.
    ///
    /// `inputs` holds one block per inlet and `outputs` one block per outlet, in the
    /// order declared by the box's [`ObjectSpec`]. Must not allocate, lock or block.
    fn process(&mut self, inputs: &[Block], outputs: &mut [Block]);
}

#[cfg(test)]
mod tests {
    use super::ParamSpec;

    const P: ParamSpec = ParamSpec {
        name: "p",
        description: "p",
        default: 0.0,
        min: -1.0,
        max: 2.0,
    };

    #[test]
    fn accepts_values_within_range_inclusive() {
        for value in [-1.0_f32, 0.0, 2.0] {
            assert!(P.accepts(value), "{value} is accepted");
        }
    }

    #[test]
    fn rejects_values_outside_range_and_nan() {
        for value in [-1.5_f32, 2.5, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(!P.accepts(value), "{value} is rejected");
        }
    }
}
