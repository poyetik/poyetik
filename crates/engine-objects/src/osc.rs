//! `osc~`: a sine oscillator.

use alloc::boxed::Box;
use core::f64::consts::TAU;

use engine_core::{Block, Object, ObjectSpec, ParamSpec, PortSpec, SampleRate};

use crate::sample::to_sample;

/// Sine oscillator. The phase is accumulated in `f64` and wrapped to `[0, 1)`; the
/// sine comes from `libm`, so the output is identical on every target.
///
/// The increment is reduced to `[0, 1)` cycle per sample, so that one subtraction always
/// wraps the phase, at any frequency and sample rate. Above Nyquist the output aliases
/// (the policy there is decided with roadmap #12).
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

impl Osc {
    /// An oscillator at `frequency` hertz, starting at phase 0.
    fn new(rate: SampleRate, frequency: f64) -> Self {
        let cycles = frequency / f64::from(rate.hz());
        Self {
            phase: 0.0,
            // Whole cycles per sample do not change the output: drop them.
            increment: cycles - libm::floor(cycles),
        }
    }
}

/// Creates an oscillator from its frequency parameter.
fn create(rate: SampleRate, params: &[f32]) -> Box<dyn Object> {
    let frequency = params.first().copied().map_or(440.0_f64, f64::from);
    Box::new(Osc::new(rate, frequency))
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

    /// The largest distance a correct oscillator has from the exact sine after `frames`
    /// samples, derived term by term (the oracle uses the same bound, `sineErrorBound`):
    ///
    /// - output rounding: half an f32 ulp below 1, 2^-25;
    /// - phase accumulation: at most 2^-52 cycle per sample, hence 2π · 2^-52 radians,
    ///   in each regime of `cycles = frequency / rate`:
    ///   - below 1 cycle per sample, the increment is `cycles` rounded once (at most
    ///     2^-54, half an ulp below 1) and each addition to a phase below 2 rounds by at
    ///     most 2^-53; the wrap (`- 1.0`) is exact;
    ///   - from 1 to 4 cycles per sample, `cycles` is rounded once (at most 2^-52, half an
    ///     ulp below 4) and removing its whole cycles is exact, so the increment is a
    ///     multiple of 2^-52 below 1, and so is the phase: every addition is exact;
    ///   - from 4 cycles on, rounding `cycles` costs up to 2^-51: the bound would no
    ///     longer hold. The parameter range keeps `cycles` below 4 at every supported
    ///     rate, checked by `stays_below_four_cycles_per_sample`;
    /// - evaluating the sine: 2^-48.
    fn bound(frames: u32) -> f64 {
        libm::exp2(-25.0) + TAU * f64::from(frames) * libm::exp2(-52.0) + libm::exp2(-48.0)
    }

    /// The largest error of [`exact`] itself: its argument, 2π times a correctly rounded
    /// quotient below 1, is off by at most 3 · 2^-53 · 2π < 2^-48.4, and `libm::sin` adds
    /// less than an ulp, 2^-53. Comparisons leave room for it inside the bound: a sample
    /// within `bound(frames) - REFERENCE_ERROR` of the reference is within
    /// `bound(frames)` of the exact sine.
    const REFERENCE_ERROR: f64 = 3.552_713_678_800_501e-15; // 2^-48

    /// The exact sine at `frame` of an oscillator at `frequency` Hz starting at phase 0,
    /// both rates whole: the phase is reduced in integers, `(frequency · frame) mod rate`,
    /// so the reference never accumulates an error, whatever the frame.
    fn exact(frequency: u64, rate: u64, frame: u64) -> f64 {
        let cycle = frequency
            .checked_mul(frame)
            .and_then(|turns| turns.checked_rem(rate))
            .expect("no overflow, a rate above zero");
        let to_f64 = |value: u64| f64::from(u32::try_from(value).expect("below 2^32"));
        libm::sin(TAU * to_f64(cycle) / to_f64(rate))
    }

    /// Checks `samples`, rendered from frame 0 at `frequency` Hz and `rate` Hz, against
    /// [`exact`] within `bound(frames)`, leaving room for the reference's own error.
    fn assert_exact(samples: &[f32], frequency: u64, rate: u64) {
        let frames = u32::try_from(samples.len()).expect("a short render");
        for (frame, &sample) in (0_u64..).zip(samples) {
            let expected = exact(frequency, rate, frame);
            assert!(
                (f64::from(sample) - expected).abs() + REFERENCE_ERROR <= bound(frames),
                "{frequency} Hz at {rate} Hz, sample {frame}: {sample} instead of {expected}"
            );
        }
    }

    /// Renders `frames` frames of `osc~` at `frequency` Hz and `rate` Hz.
    fn render_at(frequency: f32, rate: u32, frames: usize) -> Vec<f32> {
        let mut osc = (OSC.create)(SampleRate::new(rate).expect("valid rate"), &[frequency]);
        let mut samples = Vec::new();
        while samples.len() < frames {
            let mut out: [Block; 1] = [SILENCE];
            osc.process(&[], &mut out);
            samples.extend_from_slice(&out[0]);
        }
        samples.truncate(frames);
        samples
    }

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
        assert_exact(&samples, 440, 48_000);
        assert_eq!(samples[0].to_bits(), 0.0_f32.to_bits(), "starts at phase 0");
    }

    #[test]
    fn has_a_full_scale_amplitude() {
        let samples = render(440.0, 40);
        let peak = samples
            .iter()
            .fold(0.0_f32, |max, sample| max.max(sample.abs()));
        assert!(peak > 0.999_f32, "peak {peak} reaches full scale");
        assert!(peak <= 1.0_f32, "peak {peak} never exceeds full scale");
    }

    #[test]
    fn rises_through_zero_once_per_period() {
        let samples = render(440.0, 750);
        let rising = samples
            .windows(2)
            .filter(|pair| pair[0] < 0.0 && pair[1] >= 0.0)
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
            .filter(|pair| pair[0] < 0.0 && pair[1] >= 0.0)
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
    fn keeps_its_phase_within_one_cycle_at_every_rate() {
        // Every rate of the matrix, with frequencies at the limits, below, at and above
        // Nyquist: one subtraction must always wrap the phase.
        for rate in engine_testkit::matrix::sample_rates() {
            let nyquist = f64::from(rate.hz()) / 2.0_f64;
            for frequency in [
                0.0_f64,
                1.0_f64,
                440.0_f64,
                19_000.0_f64,
                20_000.0_f64,
                nyquist,
                nyquist + 1.0_f64,
            ] {
                let mut osc = super::Osc::new(rate, frequency);
                let mut out: [Block; 1] = [SILENCE];
                for _ in 0_u16..200_u16 {
                    engine_core::Object::process(&mut osc, &[], &mut out);
                    assert!(
                        (0.0_f64..1.0_f64).contains(&osc.phase),
                        "phase {} at {frequency} Hz and {} Hz",
                        osc.phase,
                        rate.hz()
                    );
                }
            }
        }
    }

    #[test]
    fn stays_exact_above_the_sample_rate() {
        // 10 001 Hz at 8 kHz: 1.25 cycles per sample, over 10 s.
        assert_exact(&render_at(10_001.0, 8_000, 80_000), 10_001, 8_000);
    }

    #[test]
    fn stays_exact_between_two_and_four_cycles_per_sample() {
        // 19 984 Hz at 8 kHz: 2.498 cycles per sample, the regime where the increment
        // rounds by up to 2^-52 (here 0.992 · 2^-52, among the worst), over 10 s.
        assert_exact(&render_at(19_984.0, 8_000, 80_000), 19_984, 8_000);
    }

    #[test]
    fn stays_below_four_cycles_per_sample() {
        // The precondition of `bound`: from 4 cycles per sample on, the increment rounds
        // by up to 2^-51 and the bound no longer holds.
        let highest = f64::from(OSC.params[0].max) / f64::from(SampleRate::MIN);
        assert!(highest < 4.0_f64, "{highest} cycles per sample");
    }

    #[test]
    fn defaults_to_440_hz_without_parameter() {
        let rate = SampleRate::new(48_000).expect("valid rate");
        let mut with_default = (OSC.create)(rate, &[]);
        let mut with_440 = (OSC.create)(rate, &[440.0]);
        let (mut default_out, mut explicit_out): ([Block; 1], [Block; 1]) = ([SILENCE], [SILENCE]);
        with_default.process(&[], &mut default_out);
        with_440.process(&[], &mut explicit_out);
        assert_eq!(default_out, explicit_out, "same output");
    }
}
