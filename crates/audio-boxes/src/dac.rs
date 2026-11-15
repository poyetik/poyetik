//! `dac~`: the audio output.

use alloc::boxed::Box;

use audio_core::{
    AlgorithmVersion, Block, Capability, Create, Object, ObjectSpec, ParamSpec, Params, PortSpec,
    Sample, SampleRate,
};

/// Audio output: the engine sums the inlets of this box into its output channels.
/// The box itself does nothing, in either precision.
#[derive(Debug)]
pub(crate) struct Dac;

impl<S: Sample, const N: usize> Object<S, N> for Dac {
    fn process(&mut self, _inputs: &[Block<S, N>], _outputs: &mut [Block<S, N>]) {}
}

impl Create for Dac {
    const MODE: audio_core::BoxMode = audio_core::BoxMode::LiveCapable;
    const PARAMS: &'static [ParamSpec] = &[];

    fn create<S: Sample, const N: usize>(
        _rate: SampleRate,
        _params: Params<'_>,
    ) -> Box<dyn Object<S, N>> {
        Box::new(Self)
    }
}

/// The `dac~` box type.
pub static DAC: ObjectSpec = ObjectSpec::new::<Dac>(
    "dac~",
    "Audio output: left and right channels sent to the speakers or the host.",
    &[
        PortSpec {
            name: "left",
            description: "Left channel",
        },
        PortSpec {
            name: "right",
            description: "Right channel",
        },
    ],
    &[],
    &[Capability::AudioOutput],
    AlgorithmVersion::FIRST,
);
