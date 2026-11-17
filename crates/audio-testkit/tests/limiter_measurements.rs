//! Measurements of the output stage's true-peak limiter (`spec/engine-design.md`, decision
//! 12; `spec/measurements.md`, "The true-peak limiter"): how far the true peak of a limited
//! sine can rise above the ceiling, and how little the limiter distorts it, each bound
//! derived from the coefficients of the detector's interpolator, for each detector.
//!
//! # The derivation
//!
//! For a sine `x[n] = A sin(ωn + φ)`, every estimate the detector takes is the largest of its
//! candidates: the two samples it estimates the interval between (gain 1, phases `-(d - 1)ω`
//! and `-dω`, `d` the detector's delay, `Detector::delay`), and each phase `p` of the
//! interpolator, a sine of gain `g_p = |H_p(ω)|` and phase `ψ_p = arg H_p(ω)`. So, whatever `φ`
//! and the frame, the estimate lies between `R(ω)·A` and `G(ω)·A`, where `G` is the largest
//! gain and `R(ω) = min over u of max_j g_j |sin(u + ψ_j)|`: the estimator's worst-case
//! under-read, which the oversampling factor and the interpolator's band and ripple set.
//!
//! When a period of the sine fits in the window the limiter holds its needs over
//! (`lookahead + 2` frames), the held need is the largest estimate of the window, between
//! `R·A` and `G·A`. In steady state the fast stage gives the held need and the slow one
//! follows it from below, so the reduction applied is the held need, smoothed by a Hann
//! window of weights summing to one. (Coming down from a louder passage instead, the slow
//! one can stall above the held need by up to `σ = (τ + 1)·2^-53`, `τ` the release in
//! frames, which `spec/measurements.md` adds to the lower end; from silence, as measured
//! here, it is zero.) The limiter adds a margin `E` to every need, and its
//! control path rounds within `ρ` of the exact chain from a need to the gain
//! (`spec/measurements.md`, "The true-peak limiter": `E = 64·M·2^-53` for a maximum
//! lookahead of `M` frames, `ρ = (43·L + 3)·2^-53` for a lookahead of `L`, the gain's own
//! rounding included, `E ≥ ρ`): the gain stays between `c/(G·A) - E - ρ` and `c/(R·A)`,
//! `c` the ceiling's amplitude. Hence:
//! - the limited sine's amplitude, its true peak, is between `c/G - A·(E + ρ)` and `c/R`;
//! - each sample is within half the gain range times `A`, `(c/R - c/G + A·(E + ρ))/2`, of
//!   the sine of the middle gain, plus the rounding of its product, which bounds its THD+N
//!   through the lemmas of `sine_bounds`.
//!
//! These bounds are measured in binary64 only, at the default ceiling and lookahead
//! (`LimiterSettings::DEFAULT`): in binary32 the peak estimate and the gain round beyond `ρ`,
//! so the lower end is derived for binary64 alone.

#![forbid(unsafe_code)]

#[cfg(test)]
/// The measurements, in a test module, where tests may `expect`.
mod tests {

    use audio_core::host::Raw;
    use audio_core::{
        BLOCK_SIZE, Binary64, Block, Detector, LimiterSettings, OutputStage, SampleRate,
        TruePeakLimiter,
    };
    use audio_testkit::matrix;
    use audio_testkit::measure::{measure, sine_bounds};

    /// The default ceiling, -1 dBTP, as an amplitude: `10^(-1/20)`, computed apart.
    const CEILING: f64 = 0.891_250_938_133_745_6;

    /// Points of the scan of `R(ω)` over a half period of the phase.
    const SCAN: u32 = 4_000;

    /// The larger of two numbers, the first when they compare equal: `f64::max` is banned
    /// (`clippy.toml`), since it may return either zero when they are `+0` and `-0`. A copy
    /// of audio-core's test helper (its tests cannot reach this crate), tested here too.
    fn larger(first: f64, second: f64) -> f64 {
        if second > first { second } else { first }
    }

    /// The smaller of two numbers.
    fn smaller(first: f64, second: f64) -> f64 {
        if second < first { second } else { first }
    }

    /// The candidates of `detector` for a sine of `omega` radians per frame, each a gain and a
    /// phase, over `frames` consecutive frames: those of one frame (the two samples of the
    /// interval it estimates, and each phase of its interpolator), moved by `-omega` each frame
    /// later.
    fn candidates(detector: Detector, omega: f64, frames: u32) -> Vec<(f64, f64)> {
        let middle = f64::from(u32::try_from(detector.delay()).expect("small"));
        let mut one = vec![
            (1.0_f64, -middle * omega),
            (1.0_f64, -(middle - 1.0_f64) * omega),
        ];
        for phase in detector.coefficients().chunks_exact(detector.taps()) {
            let (real, imaginary) = phase.iter().zip(0_u32..).fold(
                (0.0_f64, 0.0_f64),
                |(real, imaginary), (tap, index)| {
                    let angle = omega * f64::from(index);
                    (
                        real + tap.raw() * libm::cos(angle),
                        imaginary - tap.raw() * libm::sin(angle),
                    )
                },
            );
            one.push((
                libm::sqrt(real * real + imaginary * imaginary),
                libm::atan2(imaginary, real),
            ));
        }
        (0..frames)
            .flat_map(|frame| {
                one.iter()
                    .map(move |(gain, phase)| (*gain, phase - omega * f64::from(frame)))
            })
            .collect()
    }

    /// `(R(ω), G(ω))` over a window holding at least a period: the worst-case ratio of the
    /// largest estimate of the window to the peak, scanned at [`SCAN`] points of the half
    /// period and lowered by the scan's largest error (each candidate `g |sin|` moves by at
    /// most `G` per radian, half a step away), and the largest gain. A period of the window is
    /// enough: later frames repeat its phases.
    fn under_read(detector: Detector, omega: f64) -> (f64, f64) {
        let period = core::f64::consts::TAU / omega;
        let frames = (1_u32..=u32::MAX)
            .find(|frames| f64::from(*frames) > period)
            .expect("a period of frames");
        let all = candidates(detector, omega, frames);
        let largest_gain = all
            .iter()
            .fold(0.0_f64, |largest, (gain, _)| larger(largest, *gain));
        let step = core::f64::consts::PI / f64::from(SCAN);
        let scanned = (0..SCAN).fold(f64::INFINITY, |worst, point| {
            let phase_point = step * f64::from(point);
            let estimate = all.iter().fold(0.0_f64, |best, (gain, phase)| {
                larger(best, gain * libm::sin(phase_point + phase).abs())
            });
            smaller(worst, estimate)
        });
        (scanned - largest_gain * step / 2.0_f64, largest_gain)
    }

    /// `frames` frames of the sine `amplitude · sin(2π share n)` through a protected limiter
    /// with `detector` and the default settings at `rate`, its maximum lookahead 1.5 ms, in
    /// binary64.
    fn limited(
        rate: SampleRate,
        detector: Detector,
        share: f64,
        amplitude: f64,
        frames: usize,
    ) -> (Vec<f64>, usize) {
        let mut stage = OutputStage::<Binary64, f64>::limited(
            TruePeakLimiter::<Binary64>::new(
                rate,
                1,
                Binary64::new(1.5_f64),
                detector,
                LimiterSettings::DEFAULT,
                true,
            )
            .expect("a limiter"),
        );
        let mut out = Vec::with_capacity(frames);
        let mut frame = 0_u32;
        while out.len() < frames {
            let block: Block<Binary64> = core::array::from_fn(|offset| {
                let index = f64::from(frame) + f64::from(u32::try_from(offset).expect("small"));
                Binary64::from_raw(amplitude * libm::sin(core::f64::consts::TAU * share * index))
            });
            let applied = stage.apply(|blocks| blocks.copy_from_slice(&[block]));
            out.extend(applied.iter().flatten().map(|sample| sample.raw()));
            frame = frame.saturating_add(u32::try_from(BLOCK_SIZE).expect("small"));
        }
        (out, stage.latency())
    }

    /// Frames of `milliseconds` at `rate`, rounded to the nearest, at least one: the test's own
    /// count, as the limiter must take it. A copy of `frames_at` in audio-core's limiter
    /// tests (that crate cannot depend on this one): a fix of either reaches both.
    fn frames_of(milliseconds: f64, rate: SampleRate) -> u32 {
        let exact = milliseconds * f64::from(rate.hz()) / 1_000.0_f64;
        let count = (0_u32..=u32::MAX)
            .find(|count| f64::from(*count) + 0.5_f64 > exact)
            .expect("a count");
        count.max(1)
    }

    #[test]
    fn counts_frames_and_takes_the_larger_number_as_the_core_tests_do() {
        let rate = SampleRate::new(48_000).expect("a rate");
        for (milliseconds, frames) in [
            (1.5_f64, 72),
            (5.0_f64, 240),
            (0.01_f64, 1),
            (1.0_f64 / 96.0_f64, 1),
            (1.5_f64 / 48.0_f64, 2),
            (0.0_f64, 1),
        ] {
            assert_eq!(frames_of(milliseconds, rate), frames, "{milliseconds} ms");
        }
        assert_eq!(larger(1.0_f64, 2.0_f64).to_bits(), 2.0_f64.to_bits());
        assert_eq!(larger(2.0_f64, 1.0_f64).to_bits(), 2.0_f64.to_bits());
        assert_eq!(larger(0.0_f64, -0.0_f64).to_bits(), 0.0_f64.to_bits());
        assert_eq!(larger(-0.0_f64, 0.0_f64).to_bits(), (-0.0_f64).to_bits());
    }

    #[test]
    fn limits_a_sine_within_the_estimators_under_read_and_distorts_it_within_its_gain_range() {
        // Shares of the rate from a tenth to just below half: at 8 kHz, the shortest window
        // (12 frames of lookahead and 2) holds a period from a share of 1/14 up.
        let shares = [0.1_f64, 0.25_f64, 0.4_f64, 0.45_f64, 0.49_f64];
        for rate in matrix::sample_rates() {
            let window = frames_of(1.5_f64, rate) + 2;
            for (share, detector) in shares.into_iter().zip(
                [Detector::Precise, Detector::Standard, Detector::Mastering]
                    .into_iter()
                    .cycle(),
            ) {
                assert!(
                    1.0_f64 / share <= f64::from(window),
                    "a period fits in the window at {} Hz",
                    rate.hz()
                );
                let (low, high) = under_read(detector, core::f64::consts::TAU * share);
                for amplitude in [2.0_f64, 10.0_f64] {
                    // A second of warm-up beyond the latency, then 8,192 frames measured.
                    let warm = usize::try_from(rate.hz()).expect("small");
                    let (out, latency) = limited(
                        rate,
                        detector,
                        share,
                        amplitude,
                        warm + latency_bound(rate) + 8_192,
                    );
                    assert!(
                        latency <= latency_bound(rate),
                        "the latency within its bound"
                    );
                    let tail = &out[out.len() - 8_192..];
                    let measured =
                        measure(tail, rate, share * f64::from(rate.hz())).expect("a sine");
                    // The margin and the control path's roundings, at the one lookahead and
                    // maximum of these limiters.
                    let units = |count: f64| count * libm::exp2(-53.0_f64);
                    let frames = f64::from(frames_of(1.5_f64, rate));
                    let control = units(64.0_f64 * frames) + units(43.0_f64 * frames + 3.0_f64);
                    let (lowest, highest) = (CEILING / high - amplitude * control, CEILING / low);
                    assert!(
                        measured.fundamental <= highest + measured.sine.floor,
                        "true peak {} above {highest} (R {low}) at {share} of {} Hz",
                        measured.fundamental,
                        rate.hz()
                    );
                    assert!(
                        measured.fundamental >= lowest - measured.sine.floor,
                        "true peak {} below {lowest} (G {high}): limited too much",
                        measured.fundamental
                    );
                    // Each sample within half the gain range of the middle sine, and the
                    // rounding of its product (an ulp of the output, doubled).
                    let error = (highest - lowest) / 2.0_f64 + 2.0_f64 * f64::EPSILON * highest;
                    let bounds =
                        sine_bounds(highest.midpoint(lowest), error, &measured).expect("bounds");
                    assert!(
                        measured.thd_plus_n <= bounds.thd_plus_n,
                        "THD+N {} above {} at {share} of {} Hz",
                        measured.thd_plus_n,
                        bounds.thd_plus_n,
                        rate.hz()
                    );
                }
            }
        }
    }

    /// The largest latency of a limiter of 1.5 ms of maximum lookahead at `rate`: its frames
    /// and the longest detector's delay, 64.
    fn latency_bound(rate: SampleRate) -> usize {
        usize::try_from(frames_of(1.5_f64, rate))
            .expect("small")
            .saturating_add(64)
    }

    #[test]
    fn reads_the_peak_low_by_at_most_what_the_spec_states_for_each_detector() {
        // Decision 12's figures, band by band: the largest under-read over shares of the rate
        // from the grid's first point, 0.0025, to each detector's band edge, on a grid of
        // 0.0025, the edge itself included; the figures round the worst point of the grid up:
        // 0.4025, 0.2709 and 0.1253 dB (the scan's error taken off, as `under_read` does), at
        // 0.3425, 0.395 and 0.445 of the rate, which this test finds.
        for (detector, edge, decibels, worst) in [
            (
                Detector::Standard,
                0.417_f64,
                0.45_f64,
                (0.3425_f64, 0.4025_f64),
            ),
            (
                Detector::Precise,
                0.47_f64,
                0.3_f64,
                (0.395_f64, 0.2709_f64),
            ),
            (
                Detector::Mastering,
                0.485_f64,
                0.15_f64,
                (0.445_f64, 0.1253_f64),
            ),
        ] {
            let mut found = (0.0_f64, 0.0_f64);
            // The grid's points from 0.0025, in steps of 0.0025, and the edge.
            for step in 1_u32..=u32::MAX {
                let share = f64::from(step) * 0.0025_f64;
                if share >= edge + 0.0025_f64 {
                    break;
                }
                let point = smaller(share, edge);
                let (low, _) = under_read(detector, core::f64::consts::TAU * point);
                let read = -20.0_f64 * libm::log10(low);
                assert!(
                    read <= decibels,
                    "{detector:?}: {read} dB low at {point} of the rate"
                );
                if read > found.1 {
                    found = (point, read);
                }
            }
            assert!(
                (found.0 - worst.0).abs() < 1e-9_f64 && (found.1 - worst.1).abs() <= 0.000_05_f64,
                "{detector:?}: the worst point {found:?}, {worst:?} stated"
            );
        }
        // Above its band, BS.1770's interpolator rolls off: 2.2 dB at 0.45 of the rate and
        // 14.7 dB at 0.49, decision 12's figures, each within 0.05 dB.
        for (share, decibels) in [(0.45_f64, 2.2_f64), (0.49_f64, 14.7_f64)] {
            let (rolled, _) = under_read(Detector::Standard, core::f64::consts::TAU * share);
            let read = -20.0_f64 * libm::log10(rolled);
            assert!(
                (read - decibels).abs() <= 0.05_f64,
                "the standard's roll-off at {share}: {read} dB"
            );
        }
    }
}
