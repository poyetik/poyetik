//! Tests of `osc~`.

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;
use core::f64::consts::TAU;

use alloc::boxed::Box;
use audio_core::host::{Delivered, ParamBounds, Raw};

use audio_core::{
    BLOCK_SIZE, Binary32, Binary64, Block, BoxInstance, Dispatch, Object, ParamValues,
    RenderFormat, Sample, SampleRate,
};

/// The nearest binary32 to `value`, as the engine gives a binary64 render to a host that
/// takes binary32.
fn to_binary32(value: f64) -> f32 {
    <f32 as Delivered<Binary64>>::deliver(Binary64::from_raw(value))
}

/// `value` as the engine's binary64 parameter.
fn param(value: f64) -> Binary64 {
    Binary64::from_raw(value)
}

use super::OSC;

/// The terms of [`bound`] that depend on the render format: how the sine of the angle is
/// computed and given (`spec/measurements.md`, "Thresholds of `osc~`"):
///
/// - binary64: the sine computed in binary64, half an ulp below 1, 2^-54 (the error of
///   `libm::sin` beyond its rounding counts in the evaluation term of [`bound`]);
/// - binary64 given as binary32: the binary64 sine rounded to binary32, half an ulp below
///   1, 2^-25 (a value rounding below 2^-126 is a binary32 subnormal, flushed to zero at
///   delivery, an error below 2^-126);
/// - binary32: the angle `2π · phase`, below 2π < 8, rounded to binary32, by at most
///   half an ulp of [4, 8), 2^-22 (`computes_its_angle_below_eight`), which moves the
///   sine by at most as much; `libm::sinf` computes in binary64 within 2^-34.1 of the
///   sine of that angle (FreeBSD's kernels: 2^-34.1 for the cosine, 2^-37.5 relative for
///   the sine, after a reduction by multiples of π/2 in binary64), bounded by 2^-33, then
///   rounds to binary32, half an ulp below 1, 2^-25; a value below 2^-126 is flushed to
///   zero, an error below 2^-126. The shared formula of a binary32 sine
///   (`sine_error_bounds`, `spec/measurements.md`) also counts the rounding of a gain
///   after the sine, half an ulp of the output, 2^-25 at full scale: `osc~` has no gain,
///   so it meets that bound with that much to spare.
fn output_term(format: RenderFormat) -> f64 {
    match format {
        RenderFormat::Binary64 => libm::exp2(-54.0),
        RenderFormat::Binary64AsBinary32 => libm::exp2(-25.0),
        RenderFormat::Binary32 => {
            libm::exp2(-25.0) + libm::exp2(-22.0) + libm::exp2(-25.0) + libm::exp2(-33.0)
        }
    }
}

/// The largest distance a correct oscillator has from the exact sine after `frames`
/// samples in `format`, derived term by term (the oracle uses the same bound,
/// `sineErrorBound`):
///
/// - computing and giving the sine: [`output_term`], by render format;
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
///
///   each from phase 0 at one frequency: a frequency changed while it runs keeps the
///   phase (`tune`), whose additions then round in a regime of their own, bounded with
///   #12 step 41;
/// - evaluating the sine: 2^-48 (the angle `2π · phase` in binary64, within 2^-50.3,
///   and `libm::sin`'s error, within an ulp).
fn bound(format: RenderFormat, frames: u32) -> f64 {
    output_term(format) + TAU * f64::from(frames) * libm::exp2(-52.0) + libm::exp2(-48.0)
}

/// The largest error of [`exact`] itself: its argument, 2π times a correctly rounded
/// quotient below 1, is off by at most 3 · 2^-53 · 2π < 2^-48.4, and `libm::sin` adds
/// less than an ulp, 2^-53. Comparisons leave room for it inside the bound: a sample
/// within `bound(frames) - REFERENCE_ERROR` of the reference is within
/// `bound(frames)` of the exact sine.
const REFERENCE_ERROR: f64 = 3.552_713_678_800_501e-15; // 2^-48

/// `value` as an `f64`, exactly: it must be below 2^53.
fn exact_f64(value: u128) -> f64 {
    assert!(
        value < 9_007_199_254_740_992_u128,
        "{value} is not exact in f64"
    );
    let high = value
        .checked_shr(32_u32)
        .and_then(|high| u32::try_from(high).ok())
        .expect("below 2^21");
    let low = u32::try_from(value & 0xffff_ffff_u128).expect("32 bits");
    f64::from(high) * 4_294_967_296.0_f64 + f64::from(low)
}

/// A positive binary32 value as `m · 2^p` with `m` odd: `(m, p)`.
fn dyadic(value: f32) -> (u128, i32) {
    let bits = value.to_bits();
    assert_eq!(
        bits.checked_shr(31_u32),
        Some(0_u32),
        "a positive frequency"
    );
    let biased = bits.checked_shr(23_u32).expect("a 32-bit shift");
    let fraction = bits & 0x007f_ffff_u32;
    let (mantissa, exponent) = if biased == 0_u32 {
        (fraction, -149_i32)
    } else {
        let unbiased = i32::try_from(biased)
            .ok()
            .and_then(|biased| biased.checked_sub(150_i32))
            .expect("8 bits");
        (fraction | 0x0080_0000_u32, unbiased)
    };
    assert_ne!(mantissa, 0_u32, "a value above zero");
    let zeros = mantissa.trailing_zeros();
    let odd = mantissa.checked_shr(zeros).expect("below 32");
    let power = i32::try_from(zeros)
        .ok()
        .and_then(|zeros| exponent.checked_add(zeros))
        .expect("a binary32 exponent");
    (u128::from(odd), power)
}

/// The exact sine at `frame` of an oscillator at `frequency` Hz (any binary32 value
/// of the parameter's range) starting at phase 0, at `rate` Hz.
///
/// With `frequency = m · 2^p` (`m` odd), the phase at `frame` is `m · 2^p · frame /
/// rate` cycles, reduced exactly in `u128`: for `p ≥ 0`, `(m · 2^p · frame) mod
/// rate`, over `rate`; for `p = -k < 0`, `(m · frame) mod (rate · 2^k)`, over
/// `rate · 2^k`; so the reference never accumulates an error, whatever the frame.
/// When `rate · 2^k` does not fit in `u128` (the smallest frequencies: the minimum
/// normal and below), `m · frame` is below 2^53, far below the modulus: nothing to
/// reduce. Either way the remainder is below 2^53, exact in `f64`, and the phase is
/// that remainder divided by `rate` (one correctly rounded division) times `2^-k`
/// (exact): a correctly rounded quotient below 1.
fn exact(frequency: f32, rate: u32, frame: u64) -> f64 {
    if frequency.to_bits() == 0_u32 {
        return 0.0_f64;
    }
    let (odd, power) = dyadic(frequency);
    let (remainder, down) = u32::try_from(power).map_or_else(
        |_| {
            let down = power.unsigned_abs();
            let turns = odd.checked_mul(u128::from(frame)).expect("m below 2^24");
            let modulus = 1_u128
                .checked_shl(down)
                .and_then(|scale| scale.checked_mul(u128::from(rate)));
            let reduced = modulus.map_or(Some(turns), |modulus| turns.checked_rem(modulus));
            (reduced.expect("a modulus above zero"), down)
        },
        |up| {
            let whole = odd
                .checked_shl(up)
                .and_then(|step| step.checked_mul(u128::from(frame)))
                .and_then(|turns| turns.checked_rem(u128::from(rate)))
                .expect("a frequency below 2^24 Hz, a rate above zero");
            (whole, 0_u32)
        },
    );
    // 2^-down, exactly: down is at most 149, so 2^-down is a normal f64.
    let scale = 1_023_u64
        .checked_sub(u64::from(down))
        .and_then(|biased| biased.checked_shl(52_u32))
        .map(f64::from_bits)
        .expect("below 1023");
    libm::sin(TAU * (exact_f64(remainder) / f64::from(rate) * scale))
}

/// Checks `samples` of `format`, rendered from frame 0 at `frequency` Hz and `rate` Hz,
/// against [`exact`] within `bound(format, frames)`, leaving room for the reference's
/// own error.
fn assert_exact(samples: &[f64], format: RenderFormat, frequency: f32, rate: u32) {
    let frames = u32::try_from(samples.len()).expect("a short render");
    for (frame, &sample) in (0_u64..).zip(samples) {
        let expected = exact(frequency, rate, frame);
        assert!(
            (sample - expected).abs() + REFERENCE_ERROR <= bound(format, frames),
            "{format}: {frequency} Hz at {rate} Hz, sample {frame}: {sample} instead of {expected}"
        );
    }
}

/// Renders `frames` frames of `osc~`'s oscillator computing in `S`, at `frequency` Hz and
/// `rate` Hz: built directly, so that its tests reach frequencies beyond the range of its
/// parameter (up to Nyquist at every rate), which no plan gives it.
fn render_in<S: Raw>(frequency: f32, rate: u32, frames: usize) -> Vec<S::Float> {
    let mut osc = super::Osc::new(
        SampleRate::new(rate).expect("valid rate"),
        param(f64::from(frequency)),
    );
    let mut samples = Vec::new();
    while samples.len() < frames {
        let mut out: [Block<S>; 1] = [S::SILENCE];
        osc.process(&[], &mut out);
        samples.extend(out[0].iter().map(|sample| sample.raw()));
    }
    samples.truncate(frames);
    samples
}

/// Renders `frames` frames of `osc~` in `format` at `frequency` Hz and `rate` Hz, each
/// sample widened to binary64 (exactly).
fn render_at(format: RenderFormat, frequency: f32, rate: u32, frames: usize) -> Vec<f64> {
    match format {
        RenderFormat::Binary64 => render_in::<Binary64>(frequency, rate, frames),
        RenderFormat::Binary64AsBinary32 => render_in::<Binary64>(frequency, rate, frames)
            .into_iter()
            .map(|sample| f64::from(to_binary32(sample)))
            .collect(),
        RenderFormat::Binary32 => render_in::<Binary32>(frequency, rate, frames)
            .into_iter()
            .map(f64::from)
            .collect(),
    }
}

/// Renders `blocks` blocks of `osc~` in `format` at 48 kHz.
fn render(format: RenderFormat, frequency: f32, blocks: usize) -> Vec<f64> {
    render_at(format, frequency, 48_000, blocks.saturating_mul(BLOCK_SIZE))
}

#[test]
fn produces_a_440_hz_sine() {
    for format in RenderFormat::ALL {
        let samples = render(format, 440.0, 40);
        assert_exact(&samples, format, 440.0, 48_000);
        assert_eq!(samples[0].to_bits(), 0_u64, "{format}: starts at phase 0");
    }
}

#[test]
fn has_a_full_scale_amplitude() {
    for format in RenderFormat::ALL {
        let samples = render(format, 440.0, 40);
        let peak = samples.iter().fold(0.0_f64, |max, sample| {
            if sample.abs() > max {
                sample.abs()
            } else {
                max
            }
        });
        assert!(peak > 0.999_f64, "{format}: peak {peak} reaches full scale");
        assert!(
            peak <= 1.0_f64,
            "{format}: peak {peak} never exceeds full scale"
        );
    }
}

#[test]
fn rises_through_zero_once_per_period() {
    for format in RenderFormat::ALL {
        let samples = render(format, 440.0, 750);
        let rising = samples
            .windows(2)
            .filter(|pair| pair[0] < 0.0_f64 && pair[1] >= 0.0_f64)
            .count();
        // 750 blocks of 64 frames = 1 second: 440 periods, the first starting at 0.
        assert_eq!(
            rising, 439,
            "{format}: one rising zero crossing per period after the first"
        );
    }
}

#[test]
fn follows_its_frequency_parameter() {
    for format in RenderFormat::ALL {
        let samples = render(format, 1_000.0, 750);
        let rising = samples
            .windows(2)
            .filter(|pair| pair[0] < 0.0_f64 && pair[1] >= 0.0_f64)
            .count();
        assert_eq!(rising, 999, "{format}: 1 kHz");
        assert_eq!(
            render(format, 0.0, 2),
            vec![0.0_f64; 2 * BLOCK_SIZE],
            "{format}: 0 Hz is silence"
        );
    }
}

/// Renders one block into a first outlet and an extra buffer, which must stay as it was.
fn leaves_extra_buffers<S: Sample>() {
    let rate = SampleRate::new(48_000).expect("valid rate");
    let mut osc =
        <Box<dyn BoxInstance<S>> as Dispatch<S>>::create(rate, &ParamValues::defaults(&OSC))
            .expect("a test build creates every box type");
    let seven = [S::from_binary64(param(7.0)); BLOCK_SIZE];
    let mut out: [Block<S>; 2] = [S::SILENCE, seven];
    osc.process(&[], &mut out);
    assert_eq!(out[1], seven, "extra buffers are left untouched");
    let mut none: [Block<S>; 0] = [];
    osc.process(&[], &mut none);
}

#[test]
fn writes_only_its_single_outlet() {
    leaves_extra_buffers::<Binary64>();
    leaves_extra_buffers::<Binary32>();
}

#[test]
fn keeps_its_phase_within_one_cycle_at_every_rate() {
    // Every rate of the matrix, with frequencies at the limits, below, at and above
    // Nyquist: one subtraction must always wrap the phase.
    for rate in audio_testkit::matrix::sample_rates() {
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
            let mut osc = super::Osc::new(rate, param(frequency));
            let mut out: [Block<Binary64>; 1] = [Binary64::SILENCE];
            for _ in 0_u16..200_u16 {
                Object::process(&mut osc, &[], &mut out);
                assert!(
                    (0.0_f64..1.0_f64).contains(&osc.phase.raw()),
                    "phase {} at {frequency} Hz and {} Hz",
                    osc.phase.raw(),
                    rate.hz()
                );
            }
        }
    }
}

#[test]
fn stays_exact_above_the_sample_rate() {
    // 10 001 Hz at 8 kHz: 1.25 cycles per sample, over 10 s.
    for format in RenderFormat::ALL {
        let samples = render_at(format, 10_001.0, 8_000, 80_000);
        assert_exact(&samples, format, 10_001.0, 8_000);
    }
}

#[test]
fn stays_exact_between_two_and_four_cycles_per_sample() {
    // 19 984 Hz at 8 kHz: 2.498 cycles per sample, the regime where the increment
    // rounds by up to 2^-52 (here 0.992 · 2^-52, among the worst), over 10 s.
    for format in RenderFormat::ALL {
        let samples = render_at(format, 19_984.0, 8_000, 80_000);
        assert_exact(&samples, format, 19_984.0, 8_000);
    }
}

/// 16,000.0087890625 Hz, 16,000 + 9/1024: a binary32 frequency that is not whole.
const FRACTIONAL: f32 = 16_384_009.0_f32 / 1_024.0_f32;

#[test]
fn stays_exact_at_every_rate_and_frequency() {
    // Every rate of the matrix, crossed with the ends of the range, the smallest
    // normal frequency, binary32 frequencies that are not whole (16,000.0087890625 Hz
    // rounds its increment by 0.992 · 2^-52 at 8 kHz, among the worst) and the
    // regimes of `bound`: a rate- or frequency-specific defect cannot pass.
    for format in RenderFormat::ALL {
        for rate in audio_testkit::matrix::sample_rates() {
            for frequency in [
                0.0_f32,
                f32::MIN_POSITIVE,
                1.0,
                440.0,
                440.25,
                FRACTIONAL,
                19_984.0,
                20_000.0,
            ] {
                assert_exact(
                    &render_at(format, frequency, rate.hz(), 4_096),
                    format,
                    frequency,
                    rate.hz(),
                );
            }
        }
    }
}

#[test]
fn reduces_binary32_frequencies_exactly() {
    // The reference itself, against values worked by hand: 440.25 Hz is 1761 · 2^-2,
    // so at 96 kHz frame 96,000 is 440.25 whole cycles (phase 0.25, a sine of 1), and
    // frame 1 is 1761 / 384,000 cycle.
    assert_eq!(dyadic(440.25), (1_761_u128, -2_i32), "1761 · 2^-2");
    assert_eq!(dyadic(20_000.0), (625_u128, 5_i32), "625 · 2^5");
    assert_eq!(dyadic(1.0), (1_u128, 0_i32), "1");
    assert_eq!(
        dyadic(FRACTIONAL),
        (16_384_009_u128, -10_i32),
        "16,384,009 · 2^-10"
    );
    assert_eq!(dyadic(f32::MIN_POSITIVE), (1_u128, -126_i32), "2^-126");
    assert_eq!(dyadic(f32::from_bits(1)), (1_u128, -149_i32), "2^-149");
    assert_eq!(
        exact(440.25, 96_000, 96_000).to_bits(),
        1.0_f64.to_bits(),
        "a quarter cycle"
    );
    assert_eq!(
        exact(440.25, 96_000, 1).to_bits(),
        libm::sin(TAU * (1_761.0 / 384_000.0)).to_bits(),
        "one frame"
    );
    assert_eq!(
        exact(20_000.0, 8_000, 3).to_bits(),
        libm::sin(TAU * 0.5).to_bits(),
        "7.5 cycles"
    );
    assert_eq!(exact(0.0, 8_000, 5).to_bits(), 0, "silence");
    // 2^-126 Hz at 8 kHz, frame 2: 2^-125 / 8000 cycle, where rate · 2^126 does not
    // fit in u128.
    assert_eq!(
        exact(f32::MIN_POSITIVE, 8_000, 2).to_bits(),
        libm::sin(TAU * (2.0 / 8_000.0 * libm::exp2(-126.0))).to_bits(),
        "the smallest normal frequency"
    );
}

#[test]
fn flushes_its_subnormal_outputs() {
    // 2^-126 Hz, the smallest normal frequency, at every rate: the exact sine stays
    // below the smallest normal binary32 value for the first `rate / 2π` frames or so
    // (about 1,273 at 8 kHz), where the output must be +0, never a subnormal; then it
    // is normal and within the bound of the exact sine (`stays_exact_at_every_rate_and_
    // frequency`). An exact value just below 2^-126 could round up to it and break the
    // strict split below; no frame of these renders comes that close, which the split
    // itself checks.
    //
    // In binary64 the same sine is normal from its second frame on (2^-126 Hz moves the
    // angle by more than 2^-145 per frame, far above 2^-1022): nothing to flush. Given as
    // binary32, its first frames are rounded to binary32 subnormals, as the delivery
    // rounds every sample (decision 10 of `spec/engine-design.md`), not flushed.
    let smallest_normal = f64::from(f32::MIN_POSITIVE);
    for rate in audio_testkit::matrix::sample_rates() {
        let frames = usize::try_from(rate.hz() / 4).expect("a short render");
        let wide = render_in::<Binary64>(f32::MIN_POSITIVE, rate.hz(), frames);
        assert_eq!(wide[0].to_bits(), 0_u64, "+0 at phase 0");
        assert!(
            wide.iter().skip(1).all(|sample| sample.is_normal()),
            "binary64 at {} Hz: normal",
            rate.hz()
        );
        let samples = render_in::<Binary32>(f32::MIN_POSITIVE, rate.hz(), frames);
        let mut zeros = 0_u32;
        for (frame, &sample) in (0_u64..).zip(&samples) {
            let what = format!("frame {frame} at {} Hz", rate.hz());
            assert!(!sample.is_subnormal(), "{what}: {sample:e}");
            let exact = exact(f32::MIN_POSITIVE, rate.hz(), frame);
            if exact < smallest_normal {
                assert_eq!(sample.to_bits(), 0_u32, "{what}: +0");
                zeros += 1;
            } else {
                assert!(sample.is_normal(), "{what}: {sample:e}");
            }
        }
        // Below 2^-126 while 2π · frame · 2^-126 / rate < 2^-126: frame < rate / 2π.
        let crossing = f64::from(rate.hz()) / TAU;
        assert!(
            (f64::from(zeros) - crossing).abs() < 2.0_f64,
            "{zeros} zeros at {} Hz instead of about {crossing}",
            rate.hz()
        );
    }
}

#[test]
fn keeps_its_phase_zero_or_normal() {
    // The only recursive state of `osc~` is its binary64 phase: it is 0, a sum of
    // positive increments, or such a sum less 1 (exact, a multiple of 2^-52). So it is
    // either 0 or at least the smallest increment, the smallest positive binary32
    // frequency at the highest rate (about 2^-167.6), a normal binary64 value: it never
    // needs a flush.
    let fastest = SampleRate::new(SampleRate::MAX).expect("valid rate");
    let smallest = super::Osc::new(fastest, param(f64::from(f32::from_bits(1))));
    let increment = smallest.increment.raw();
    assert!(increment.is_normal(), "{increment:e}");
    for frequency in [0.0, f32::from_bits(1), f32::MIN_POSITIVE, 1.0, 20_000.0] {
        for rate in audio_testkit::matrix::sample_rates() {
            let mut osc = super::Osc::new(rate, param(f64::from(frequency)));
            let mut out: [Block<Binary32>; 1] = [Binary32::SILENCE];
            for _ in 0_u16..64_u16 {
                Object::process(&mut osc, &[], &mut out);
                assert!(
                    matches!(
                        osc.phase.raw().classify(),
                        core::num::FpCategory::Zero | core::num::FpCategory::Normal
                    ),
                    "phase {:e} at {frequency:e} Hz and {} Hz",
                    osc.phase.raw(),
                    rate.hz()
                );
            }
        }
    }
}

#[test]
fn stays_below_four_cycles_per_sample() {
    // The precondition of `bound`: from 4 cycles per sample on, the increment rounds
    // by up to 2^-51 and the bound no longer holds.
    let highest = OSC.params()[0].maximum() / f64::from(SampleRate::MIN);
    assert!(highest < 4.0_f64, "{highest} cycles per sample");
}

/// Checks that an oscillator computing in `S` given the defaults of its box type plays 440 Hz.
fn defaults_to_440_hz_in<S: Sample>() {
    let rate = SampleRate::new(48_000).expect("valid rate");
    let mut with_default =
        <Box<dyn BoxInstance<S>> as Dispatch<S>>::create(rate, &ParamValues::defaults(&OSC))
            .expect("a test build creates every box type");
    let explicit = ParamValues::of(&OSC, &[param(440.0)]).expect("440 Hz within range");
    let mut with_440 = <Box<dyn BoxInstance<S>> as Dispatch<S>>::create(rate, &explicit)
        .expect("a test build creates every box type");
    let (mut default_out, mut explicit_out) = ([S::SILENCE], [S::SILENCE]);
    with_default.process(&[], &mut default_out);
    with_440.process(&[], &mut explicit_out);
    assert_eq!(default_out, explicit_out, "same output");
}

#[test]
fn defaults_to_440_hz() {
    defaults_to_440_hz_in::<Binary64>();
    defaults_to_440_hz_in::<Binary32>();
}

#[test]
fn computes_its_angle_below_eight() {
    // The precondition of the binary32 term of `output_term`: the angle `2π · phase`,
    // with the phase below 1 (`keeps_its_phase_within_one_cycle_at_every_rate`), is below
    // 2π, so within [0, 8), where binary32 values are at most 2^-21 apart.
    let largest = to_binary32(TAU);
    let below_eight = f32::from_bits(8.0_f32.to_bits() - 1);
    assert!(largest <= below_eight, "{largest}");
    assert_eq!(
        f64::from(below_eight),
        8.0_f64 - libm::exp2(-21.0),
        "the spacing of [4, 8)"
    );
}

/// The known-answer vectors of the measurements, whose `sine_error_bounds` the oracle's
/// `sineErrorBound` reads too: one derivation, checked in both languages.
const VECTORS: &str = include_str!("../../../spec/measurement-vectors.json");

#[test]
fn bounds_binary32_sines_only_for_amplitudes_exact_in_binary32() {
    // The binary32 formula counts the rounding of a gain after the sine once: its domain is
    // amplitudes exact in binary32, which every binary32 vector must lie in.
    let vectors: serde_json::Value = serde_json::from_str(VECTORS).expect("valid JSON");
    let binary32: Vec<f64> = vectors["sine_error_bounds"]
        .as_array()
        .expect("bounds")
        .iter()
        .filter(|vector| vector["format"].as_str() == Some(RenderFormat::Binary32.name()))
        .map(|vector| vector["amplitude"].as_f64().expect("an amplitude"))
        .collect();
    assert!(!binary32.is_empty(), "binary32 vectors");
    for amplitude in binary32 {
        let narrowed = Binary32::from_binary64(Binary64::new(amplitude)).to_binary64();
        assert_eq!(
            narrowed.bits(),
            amplitude.to_bits(),
            "{amplitude:e} is exact in binary32"
        );
    }
}

#[test]
fn bounds_its_error_like_the_shared_vectors() {
    let vectors: serde_json::Value = serde_json::from_str(VECTORS).expect("valid JSON");
    let list = vectors["sine_error_bounds"].as_array().expect("bounds");
    for (format, count) in RenderFormat::ALL.into_iter().zip([3_usize, 4, 3]) {
        let full_scale: Vec<_> = list
            .iter()
            .filter(|vector| {
                vector["amplitude"].as_f64() == Some(1.0_f64)
                    && vector["format"].as_str() == Some(format.name())
            })
            .collect();
        assert_eq!(
            full_scale.len(),
            count,
            "every full-scale vector of {format}"
        );
        for vector in full_scale {
            let frames = u32::try_from(vector["frames"].as_u64().expect("frames")).expect("u32");
            let wanted = vector["bound"].as_f64().expect("a bound");
            let derived = bound(format, frames);
            assert!(
                (derived - wanted).abs() <= wanted * 1e-15_f64,
                "{format}, {frames} frames: {derived} instead of {wanted}"
            );
        }
    }
}

/// The frames each measurement of `osc~` renders: 8192, about a second at 8 kHz, so
/// every frequency below is at least five bins from DC and Nyquist at every rate.
const MEASURED_FRAMES: u32 = 8_192;

/// The mean square of the rounding of a full-scale sine to binary32, its errors spread
/// uniformly over an ulp, `ulp² / 12`: `p_k = (2/π)(asin 2^(1-k) - asin 2^-k)` of the
/// samples of a sine lie in `[2^-k, 2^(1-k))`, where the ulp is `2^(-k-23)`; the terms
/// beyond `k = 60` are below 2^-160 of the sum.
fn output_rounding_power() -> f64 {
    (1_u32..=60_u32)
        .map(|octave| {
            let top = libm::exp2(-f64::from(octave) + 1.0_f64);
            let share =
                2.0_f64 / core::f64::consts::PI * (libm::asin(top) - libm::asin(top / 2.0_f64));
            let ulp = libm::exp2(-f64::from(octave) - 23.0_f64);
            share * ulp * ulp / 12.0_f64
        })
        .sum()
}

/// The mean square of the error of a binary32 sine from the rounding of its angle `x`,
/// uniform over `[0, 2π)`, to binary32, spread uniformly over the ulp of `x`: the sine
/// moves by `cos x` times it, so `(1/2π) ∫ cos²(x) ulp(x)² / 12 dx`, octave by octave,
/// with `∫ cos² = x/2 + sin(2x)/4` and an ulp of `2^(k-23)` in `[2^k, 2^(k+1))`; the
/// octaves below `2^-60` add less than 2^-140 of the sum.
fn angle_rounding_power() -> f64 {
    let integral = |x: f64| x / 2.0_f64 + libm::sin(2.0_f64 * x) / 4.0_f64;
    let sum: f64 = (-60_i32..=2_i32)
        .map(|octave| {
            let low = libm::exp2(f64::from(octave));
            let high = if octave == 2_i32 { TAU } else { 2.0_f64 * low };
            let ulp = libm::exp2(f64::from(octave) - 23.0_f64);
            (integral(high) - integral(low)) * ulp * ulp / 12.0_f64
        })
        .sum();
    sum / TAU
}

/// The THD+N a non-periodic full-scale sine of `format` is predicted to measure, from
/// its rounding spread uniformly (`spec/measurements.md`, "Thresholds of `osc~`"): the
/// binary32 rounding of its value, and in binary32 that of its angle too; `None` in
/// binary64, whose error is below the floor of the measurements (by a factor of about 2.5,
/// `bounds_its_binary64_error_below_the_floor`).
fn predicted_thd_plus_n(format: RenderFormat) -> Option<f64> {
    let power = match format {
        RenderFormat::Binary64 => return None,
        RenderFormat::Binary64AsBinary32 => output_rounding_power(),
        RenderFormat::Binary32 => output_rounding_power() + angle_rounding_power(),
    };
    Some(10.0_f64 * libm::log10(power / 0.5_f64))
}

/// How far, in dB, a measured THD+N may lie from [`predicted_thd_plus_n`], by format: the
/// measured distribution over the deterministic domain of
/// `measures_within_its_derived_thresholds_at_every_rate` (the 8 rates of the matrix, the 4
/// spread frequencies, 8192 frames), rounded outward to 0.05 dB. Measured: binary64 given as
/// binary32 from -0.055 to +0.057 dB; binary32 from -0.251 to +0.124 dB, biased low at 0.0007
/// cycle per sample, whose 5.7 periods leave the angles not uniform (the prediction assumes
/// they are; roadmap #12 step 19 computes it from the actual angles). A change of the rounding
/// or of the sine kernel moving a level by more than that shows.
fn prediction_band(format: RenderFormat) -> (f64, f64) {
    match format {
        RenderFormat::Binary32 => (-0.30_f64, 0.15_f64),
        RenderFormat::Binary64 | RenderFormat::Binary64AsBinary32 => (-0.10_f64, 0.10_f64),
    }
}

#[test]
fn measures_within_its_derived_thresholds_at_every_rate() {
    // The thresholds of `osc~` (`spec/measurements.md`, "Thresholds of `osc~`"): the
    // bounds of a signal within `bound(format, frames)` per sample of the ideal full-scale
    // sine, for the fits and the window of each measurement, in every render format. At
    // every rate of the matrix: low, mid and near-Nyquist frequencies, all fractional
    // (they rarely repeat), a quarter of the rate, and a periodic case where the rounding
    // falls into a few harmonics.
    let predicted = predicted_thd_plus_n(RenderFormat::Binary64AsBinary32);
    assert!(
        predicted.is_some_and(|level| (level + 153.73_f64).abs() < 0.01_f64),
        "{predicted:?}"
    );
    for format in RenderFormat::ALL {
        for rate in audio_testkit::matrix::sample_rates() {
            let hz = f64::from(rate.hz());
            let mut cases: Vec<(f64, bool)> = [0.000_7_f64, 0.022_9_f64, 0.287_1_f64, 0.499_3_f64]
                .iter()
                .map(|cycles| (cycles * hz, true))
                .collect();
            cases.push((hz / 4.0_f64, false));
            cases.push((hz / 8.0_f64, false));
            for (wanted, spread) in cases {
                measures_within_its_thresholds(format, rate, to_binary32(wanted), spread);
            }
        }
    }
}

/// Measures `osc~` at `frequency` Hz in `format` at `rate` and holds every level to its
/// bound; `spread` when its rounding is spread (not periodic), where the THD+N is
/// predicted.
fn measures_within_its_thresholds(
    format: RenderFormat,
    rate: SampleRate,
    frequency: f32,
    spread: bool,
) {
    let samples = render_at(format, frequency, rate.hz(), 8_192);
    let what = format!("{format}: {frequency} Hz at {} Hz", rate.hz());
    let measured =
        audio_testkit::measure::measure(&samples, rate, f64::from(frequency)).expect(&what);
    let bounds =
        audio_testkit::measure::sine_bounds(1.0_f64, bound(format, MEASURED_FRAMES), &measured)
            .expect(&what);
    assert!(
        measured.thd_plus_n <= bounds.thd_plus_n,
        "{what}: THD+N {} above {}",
        measured.thd_plus_n,
        bounds.thd_plus_n
    );
    assert!(
        measured.sfdr >= bounds.sfdr,
        "{what}: SFDR {} below {}",
        measured.sfdr,
        bounds.sfdr
    );
    let harmonics = measured.harmonics.as_ref().expect(&what);
    let limits = bounds.harmonics.expect(&what);
    assert!(
        harmonics.snr >= limits.snr,
        "{what}: SNR {} below {}",
        harmonics.snr,
        limits.snr
    );
    for level in [harmonics.thd, harmonics.aliasing].into_iter().flatten() {
        // The aliasing law: below Nyquist, no aliased component above the floor of the
        // output.
        assert!(
            level <= limits.thd,
            "{what}: {level} dB above {}",
            limits.thd
        );
    }
    match predicted_thd_plus_n(format) {
        // The gap to the bound explained: uniform rounding, within the band of the format.
        Some(predicted) if spread => {
            let (low, high) = prediction_band(format);
            assert!(
                (low..=high).contains(&(measured.thd_plus_n - predicted)),
                "{what}: THD+N {} instead of {predicted} within [{low}, {high}] dB",
                measured.thd_plus_n
            );
        }
        Some(_) => {}
        // In binary64, the error of the sine is below what the measurements can
        // resolve: its levels are those of the floor of the measurements alone.
        None => {
            let floor =
                audio_testkit::measure::sine_bounds(1.0_f64, 0.0_f64, &measured).expect(&what);
            assert!(
                measured.thd_plus_n <= floor.thd_plus_n,
                "{what}: THD+N {} above the floor {}",
                measured.thd_plus_n,
                floor.thd_plus_n
            );
        }
    }
}

#[test]
fn bounds_its_binary64_error_below_the_floor() {
    // The numbers of `spec/measurements.md` for binary64 over the measured frames: the
    // error bound of a full-scale sine, about 1.14e-11 (above 10^-11), lies below the floor
    // of the measurements of a full-scale sine, `FLOOR · (peak + Σ|c|)` with a peak of 1 and
    // one coefficient of 1, about 2.9e-11, by a factor of about 2.5 only.
    let error = bound(RenderFormat::Binary64, MEASURED_FRAMES);
    let floor = audio_testkit::measure::FLOOR * 2.0_f64;
    assert!((1.135e-11_f64..1.145e-11_f64).contains(&error), "{error}");
    assert!((2.905e-11_f64..2.915e-11_f64).contains(&floor), "{floor}");
    assert!(
        (2.5_f64..2.6_f64).contains(&(floor / error)),
        "{}",
        floor / error
    );
}

#[test]
fn has_the_thresholds_of_each_render_format() {
    // The numbers `spec/measurements.md` gives for 8192 frames, from the bounds of an
    // orthogonal fit at the floor of a full-scale signal and the window's ENBW: the
    // THD+N bound, the SFDR bound and the gap from the bound to the predicted THD+N.
    let rate = SampleRate::new(8_000).expect("valid rate");
    for (format, thd_plus_n, sfdr, gap) in [
        (RenderFormat::Binary64, -204.8_f64, 198.8_f64, None),
        (
            RenderFormat::Binary64AsBinary32,
            -147.5_f64,
            141.5_f64,
            Some(6.2_f64),
        ),
        (
            RenderFormat::Binary32,
            -127.5_f64,
            121.5_f64,
            Some(13.4_f64),
        ),
    ] {
        let samples = render_at(format, 1_000.0, 8_000, 8_192);
        let measured =
            audio_testkit::measure::measure(&samples, rate, 1_000.0_f64).expect("measurable");
        let bounds =
            audio_testkit::measure::sine_bounds(1.0_f64, bound(format, MEASURED_FRAMES), &measured)
                .expect("bounded");
        assert!(
            (bounds.thd_plus_n - thd_plus_n).abs() < 0.05_f64,
            "{format}: {}",
            bounds.thd_plus_n
        );
        assert!(
            (bounds.sfdr - sfdr).abs() < 0.05_f64,
            "{format}: {}",
            bounds.sfdr
        );
        assert_eq!(
            predicted_thd_plus_n(format).map(|predicted| (bounds.thd_plus_n
                - predicted
                - gap.unwrap_or(0.0_f64))
            .abs()
                < 0.1_f64),
            gap.map(|_| true),
            "{format}: the gap"
        );
    }
}
