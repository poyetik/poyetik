//! `osc~`: a sine oscillator.

use alloc::boxed::Box;
use core::f64::consts::TAU;

use engine_core::{Block, Object, ObjectSpec, ParamSpec, PortSpec, SampleRate};

use crate::sample::to_sample;

/// Sine oscillator. The phase is accumulated in `f64` and wrapped to `[0, 1)`; the
/// sine comes from `libm`, so the output is identical on every target.
#[derive(Debug)]
struct Osc {
    /// Current phase, in cycles, within `[0, 1)`.
    phase: f64,
    /// Phase increment per sample, in cycles.
    increment: f64,
}

impl Object for Osc {
    fn process(&mut self, _inputs: &[Block], outputs: &mut [Block]) {
        for output in outputs.iter_mut().take(1) {
            for sample in output.iter_mut() {
                *sample = to_sample(libm::sin(TAU * self.phase));
                self.phase += self.increment;
                if self.phase >= 1.0_f64 {
                    self.phase -= 1.0_f64;
                }
            }
        }
    }
}

/// Creates an oscillator from its frequency parameter.
fn create(rate: SampleRate, params: &[f32]) -> Box<dyn Object> {
    let frequency = params.first().copied().map_or(440.0_f64, f64::from);
    Box::new(Osc {
        phase: 0.0,
        increment: frequency / f64::from(rate.hz()),
    })
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
    create,
};

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;
    use core::f64::consts::TAU;

    use engine_core::{BLOCK_SIZE, Block, SILENCE, SampleRate};

    use super::OSC;

    /// Renders `blocks` blocks of `osc~` at 48 kHz.
    fn render(frequency: f32, blocks: usize) -> Vec<f32> {
        let rate = SampleRate::new(48_000).expect("valid rate");
        let mut osc = (OSC.create)(rate, &[frequency]);
        let mut samples = Vec::new();
        for _ in 0..blocks {
            let mut out: [Block; 1] = [SILENCE];
            osc.process(&[], &mut out);
            samples.extend_from_slice(&out[0]);
        }
        samples
    }

    #[test]
    fn produces_a_440_hz_sine() {
        let samples = render(440.0, 40);
        for (n, &sample) in samples.iter().enumerate() {
            let index = f64::from(u32::try_from(n).expect("small index"));
            let expected = libm::sin(TAU * 440.0_f64 * index / 48_000.0_f64);
            assert!(
                (f64::from(sample) - expected).abs() < 1e-6_f64,
                "sample {n}: {sample} instead of {expected}"
            );
        }
        assert_eq!(samples[0].to_bits(), 0.0_f32.to_bits(), "starts at phase 0");
    }

    #[test]
    fn has_a_full_scale_amplitude() {
        let samples = render(440.0, 40);
        let peak = samples.iter().fold(0.0_f32, |max, s| max.max(s.abs()));
        assert!(peak > 0.999_f32, "peak {peak} reaches full scale");
        assert!(peak <= 1.0_f32, "peak {peak} never exceeds full scale");
    }

    #[test]
    fn rises_through_zero_once_per_period() {
        let samples = render(440.0, 750);
        let rising = samples
            .windows(2)
            .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
            .count();
        // 750 blocks of 64 frames = 1 second: 440 periods, the first starting at 0.
        assert_eq!(
            rising, 439,
            "one rising zero crossing per period after the first"
        );
    }

    #[test]
    fn follows_its_frequency_parameter() {
        let samples = render(1_000.0, 750);
        let rising = samples
            .windows(2)
            .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
            .count();
        assert_eq!(rising, 999, "1 kHz");
        assert_eq!(render(0.0, 2), vec![0.0; 2 * BLOCK_SIZE], "0 Hz is silence");
    }

    #[test]
    fn writes_only_its_single_outlet() {
        let rate = SampleRate::new(48_000).expect("valid rate");
        let mut osc = (OSC.create)(rate, &[440.0]);
        let mut out: [Block; 2] = [SILENCE, [7.0; BLOCK_SIZE]];
        osc.process(&[], &mut out);
        assert_eq!(
            out[1], [7.0; BLOCK_SIZE],
            "extra buffers are left untouched"
        );
        let mut none: [Block; 0] = [];
        osc.process(&[], &mut none);
    }

    #[test]
    fn keeps_its_phase_within_one_cycle() {
        // A 19 kHz oscillator wraps its phase on almost every sample.
        let mut osc = super::Osc {
            phase: 0.0,
            increment: 19_000.0_f64 / 48_000.0_f64,
        };
        let mut out: [Block; 1] = [SILENCE];
        for _ in 0_u16..1_000_u16 {
            engine_core::Object::process(&mut osc, &[], &mut out);
            assert!(osc.phase >= 0.0_f64, "phase {} is not negative", osc.phase);
            assert!(
                osc.phase < 1.0_f64,
                "phase {} stays below one cycle",
                osc.phase
            );
        }
    }

    #[test]
    fn defaults_to_440_hz_without_parameter() {
        let rate = SampleRate::new(48_000).expect("valid rate");
        let mut with_default = (OSC.create)(rate, &[]);
        let mut with_440 = (OSC.create)(rate, &[440.0]);
        let (mut a, mut b): ([Block; 1], [Block; 1]) = ([SILENCE], [SILENCE]);
        with_default.process(&[], &mut a);
        with_440.process(&[], &mut b);
        assert_eq!(a, b, "same output");
    }
}
