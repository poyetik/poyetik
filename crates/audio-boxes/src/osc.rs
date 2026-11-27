//! `osc~`: a sine oscillator.

use audio_core::{
    AlgorithmVersion, Binary64, Block, Create, Driven, Driving, Object, ObjectSpec, ParamSpec,
    PortSpec, Sample, SampleRate, Values, param,
};

/// Sine oscillator, written once for both precisions. The phase is accumulated in binary64
/// and wrapped to `[0, 1)`, in both: an interim accumulator, until the exact integer phase
/// of decision 9 of `spec/engine-design.md` (roadmap #12 step 19); the increment derives
/// from the parameter in binary64 (decision 10); the sine is computed in the precision of
/// the render, `S`, from the angle `2π · phase` rounded to it, with the math layer's `libm`,
/// so the output is identical on every target. All its arithmetic is the math layer's.
///
/// The increment is reduced to `[0, 1)` cycle per sample, so that one subtraction always
/// wraps the phase, at any frequency and sample rate. Above Nyquist the output aliases
/// (the policy there is decided with #12 step 41).
///
/// Subnormal outputs are flushed to zero (`spec/engine-design.md`, decision 11): at the
/// smallest frequencies, the first frames of a binary32 sine are below 2^-126 and come out
/// as +0. The phase, its only recursive state, is never subnormal
/// (`keeps_its_phase_zero_or_normal`).
#[derive(Debug)]
pub(crate) struct Osc {
    /// Current phase, in cycles, within `[0, 1)`.
    phase: Binary64,
    /// Phase increment per sample, in cycles.
    increment: Binary64,
    /// The sample rate, in hertz, which the increment is derived with.
    rate: Binary64,
}

impl<S: Sample, const N: usize> Object<S, N> for Osc {
    fn process(&mut self, _inputs: &[Block<S, N>], outputs: &mut [Block<S, N>]) {
        for output in outputs.iter_mut().take(1) {
            for sample in output.iter_mut() {
                *sample = self.frame();
            }
        }
    }
}

impl<S: Sample, const N: usize> Driven<S, N, 1> for Osc {
    fn set(&mut self, values: Values<1>) {
        let [frequency] = values.get();
        self.tune(frequency);
    }

    fn process_driven(
        &mut self,
        _inputs: &[Block<S, N>],
        outputs: &mut [Block<S, N>],
        driving: Driving<'_, N, 1>,
    ) {
        let [frequencies] = driving.blocks();
        for output in outputs.iter_mut().take(1) {
            for (sample, &frequency) in output.iter_mut().zip(frequencies) {
                self.tune(frequency);
                *sample = self.frame();
            }
        }
    }
}

impl Osc {
    /// An oscillator at `frequency` hertz, starting at phase 0.
    fn new(rate: SampleRate, frequency: Binary64) -> Self {
        let mut osc = Self {
            phase: Binary64::ZERO,
            increment: Binary64::ZERO,
            rate: Binary64::from(rate),
        };
        osc.tune(frequency);
        osc
    }

    /// Takes `frequency` hertz from the next frame on, the phase staying where it is.
    #[inline]
    fn tune(&mut self, frequency: Binary64) {
        let cycles = frequency / self.rate;
        // Whole cycles per sample do not change the output: drop them.
        self.increment = cycles - cycles.floor();
    }

    /// The next sample: the one arithmetic of both bodies. A frame gives the sine of the
    /// phase it starts at, then moves the phase by the increment, so that the frequency
    /// set at a frame (`tune`) moves the phase from the next frame on and no sample
    /// depends on the frequency given at its own frame.
    #[inline]
    fn frame<S: Sample>(&mut self) -> S {
        let sample = S::from_binary64(Binary64::TAU * self.phase).sin().flush();
        self.phase += self.increment;
        if self.phase >= Binary64::ONE {
            self.phase -= Binary64::ONE;
        }
        sample
    }

    /// An oscillator from its frequency parameter, as [`Create::create`] receives it: the
    /// one constructor of both its factory and [`crate::BuiltIn`].
    ///
    /// The frequency cannot be missing: both are given one value per parameter of this
    /// type, as an array.
    #[inline]
    pub(crate) fn of(rate: SampleRate, values: Values<1>) -> Self {
        let [frequency] = values.get();
        Self::new(rate, frequency)
    }
}

impl Create<1> for Osc {
    const BOX_MODE: audio_core::BoxMode = audio_core::BoxMode::LiveCapable;
    const PARAMS: &'static [ParamSpec; 1] = &[param!(
        "frequency",
        "Frequency, in hertz",
        440.0,
        0.0,
        20_000.0,
    )];

    /// An oscillator from its frequency parameter.
    fn create<S: Sample, const N: usize>(
        rate: SampleRate,
        params: Values<1>,
    ) -> impl Driven<S, N, 1> + 'static {
        Self::of(rate, params)
    }
}

/// The `osc~` box type.
pub static OSC: ObjectSpec = ObjectSpec::new::<Osc, 1>(
    "osc~",
    "Sine oscillator with a full-scale amplitude (-1 to 1), starting at phase 0.",
    &[],
    &[PortSpec {
        name: "out",
        description: "The sine wave",
    }],
    &[],
    AlgorithmVersion::FIRST,
);

#[cfg(test)]
#[path = "osc_tests.rs"]
mod tests;
