//! `dac~`: the audio output.

use audio_core::{
    AlgorithmVersion, Block, Capability, Create, Driven, Object, ObjectSpec, ParamSpec, PortSpec,
    Sample, SampleRate, Values,
};

/// Audio output: the engine sums the inlets of this box into its output channels.
/// The box itself does nothing, in either precision.
#[derive(Debug)]
pub(crate) struct Dac;

impl<S: Sample, const N: usize> Object<S, N> for Dac {
    fn process(&mut self, _inputs: &[Block<S, N>], _outputs: &mut [Block<S, N>]) {}
}

impl Create<0> for Dac {
    const BOX_MODE: audio_core::BoxMode = audio_core::BoxMode::LiveCapable;
    const PARAMS: &'static [ParamSpec; 0] = &[];

    fn create<S: Sample, const N: usize>(
        _rate: SampleRate,
        _: Values<0>,
    ) -> impl Driven<S, N, 0> + 'static {
        Self
    }
}

/// The `dac~` box type.
pub static DAC: ObjectSpec = ObjectSpec::new::<Dac, 0>(
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
