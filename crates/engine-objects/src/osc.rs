//! `osc~`: a sine oscillator.

use alloc::boxed::Box;

use engine_core::{
    Binary64, Block, Create, Factory, Object, ObjectSpec, ParamSpec, PortSpec, Sample, SampleRate,
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
/// (the policy there is decided with roadmap #12).
///
/// Subnormal outputs are flushed to zero (`spec/engine-design.md`, decision 11): at the
/// smallest frequencies, the first frames of a binary32 sine are below 2^-126 and come out
/// as +0. The phase, its only recursive state, is never subnormal
/// (`keeps_its_phase_zero_or_normal`).
#[derive(Debug)]
struct Osc {
    /// Current phase, in cycles, within `[0, 1)`.
    phase: Binary64,
    /// Phase increment per sample, in cycles.
    increment: Binary64,
}

impl<S: Sample> Object<S> for Osc {
    fn process(&mut self, _inputs: &[Block<S>], outputs: &mut [Block<S>]) {
        for output in outputs.iter_mut().take(1) {
            for sample in output.iter_mut() {
                *sample = S::from_binary64(Binary64::TAU * self.phase).sin().flush();
                self.phase += self.increment;
                if self.phase >= Binary64::ONE {
                    self.phase -= Binary64::ONE;
                }
            }
        }
    }
}

impl Osc {
    /// An oscillator at `frequency` hertz, starting at phase 0.
    fn new(rate: SampleRate, frequency: Binary64) -> Self {
        let cycles = frequency / Binary64::from(rate);
        Self {
            phase: Binary64::ZERO,
            // Whole cycles per sample do not change the output: drop them.
            increment: cycles - cycles.floor(),
        }
    }
}

impl Create for Osc {
    /// An oscillator from its frequency parameter.
    fn create<S: Sample>(rate: SampleRate, params: &[Binary64]) -> Box<dyn Object<S>> {
        let frequency = params.first().copied().unwrap_or(Binary64::new(440.0));
        Box::new(Self::new(rate, frequency))
    }
}

/// The `osc~` box type.
pub static OSC: ObjectSpec = ObjectSpec {
    name: "osc~",
    description: "Sine oscillator with a full-scale amplitude (-1 to 1), starting at phase 0.",
    inlets: &[],
    outlets: &[PortSpec {
        name: "out",
        description: "The sine wave",
    }],
    params: &[ParamSpec {
        name: "frequency",
        description: "Frequency, in hertz",
        default: 440.0,
        min: 0.0,
        max: 20_000.0,
    }],
    capabilities: &[],
    create: Factory::new::<Osc>(),
};

#[cfg(test)]
#[path = "osc_tests.rs"]
mod tests;
