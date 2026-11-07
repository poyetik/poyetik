//! `dac~`: the audio output.

use alloc::boxed::Box;

use engine_core::{Block, Capability, Object, ObjectSpec, PortSpec, SampleRate};

/// Audio output: the engine sums the inlets of this box into its output channels.
/// The box itself does nothing.
#[derive(Debug)]
struct Dac;

impl Object for Dac {
    fn process(&mut self, _inputs: &[Block], _outputs: &mut [Block]) {}
}

/// Creates the output box.
fn create(_rate: SampleRate, _params: &[f32]) -> Box<dyn Object> {
    Box::new(Dac)
}

/// The `dac~` box type.
pub static DAC: ObjectSpec = ObjectSpec {
    name: "dac~",
    description: "Audio output: left and right channels sent to the speakers or the host.",
    inlets: &[
        PortSpec {
            name: "left",
            description: "Left channel",
        },
        PortSpec {
            name: "right",
            description: "Right channel",
        },
    ],
    outlets: &[],
    params: &[],
    capabilities: &[Capability::AudioOutput],
    create,
};
