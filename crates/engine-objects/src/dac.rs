//! `dac~`: the audio output.

use alloc::boxed::Box;

use engine_core::{
    AlgorithmVersion, Binary64, Block, Capability, Create, Factory, Object, ObjectSpec, PortSpec,
    Sample, SampleRate,
};

/// Audio output: the engine sums the inlets of this box into its output channels.
/// The box itself does nothing, in either precision.
#[derive(Debug)]
struct Dac;

impl<S: Sample> Object<S> for Dac {
    fn process(&mut self, _inputs: &[Block<S>], _outputs: &mut [Block<S>]) {}
}

impl Create for Dac {
    fn create<S: Sample>(_rate: SampleRate, _params: &[Binary64]) -> Box<dyn Object<S>> {
        Box::new(Self)
    }
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
    create: Factory::new::<Dac>(),
    algorithm_version: AlgorithmVersion::FIRST,
};
