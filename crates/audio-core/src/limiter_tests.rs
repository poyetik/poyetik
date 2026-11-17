//! Tests of the true-peak limiter.

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use super::{LimiterError, LimiterSettings, Setting, TruePeakLimiter};
use crate::LimiterMode;
use crate::host::Raw;
use crate::tests::{exact_steady, larger, margin_of, rounding_of};
use crate::{BLOCK_SIZE, Binary32, Binary64, Block, Sample, SampleRate};

/// 10^(-1/20), the default ceiling of -1 dBTP as an amplitude, computed apart: a value of
/// binary64, at or above the binary32 one.
const CEILING: f64 = 0.891_250_938_133_745_6_f64;

/// A copy of a limiter, its state included, for the tests that compare two continuations
/// of one state: the library's limiter is not `Clone`, so that a used one never leaves its
/// stage (`OutputStage::limiter`). Every field is named, so that a new one does not compile
/// until it is copied too.
impl<S: Sample> Clone for TruePeakLimiter<S> {
    fn clone(&self) -> Self {
        Self {
            rate: self.rate,
            channels: self.channels,
            maximum: self.maximum,
            maximum_ms: self.maximum_ms,
            wanted: self.wanted,
            settings: self.settings,
            taps: self.taps,
            delay: self.delay,
            filter: self.filter.clone(),
            history: self.history.clone(),
            history_at: self.history_at,
            delayed: self.delayed.clone(),
            position: self.position,
            needed: self.needed.clone(),
            needed_at: self.needed_at,
            peaks: self.peaks.clone(),
            step: self.step,
            ceiling: self.ceiling,
            margin: self.margin,
            keep_fast: self.keep_fast,
            slow_attack: self.slow_attack,
            path: self.path.clone(),
            previous: self.previous.clone(),
            change: self.change,
            rebuild_frames: self.rebuild_frames,
            rebuild: self.rebuild,
            applied: self.applied,
            mode: self.mode,
            protected: self.protected,
            given_binary32: self.given_binary32,
            remaining: self.remaining,
            keep_mode: self.keep_mode,
        }
    }
}

/// Every detector, the default first.
const DETECTORS: [super::Detector; 3] = [
    super::Detector::Precise,
    super::Detector::Standard,
    super::Detector::Mastering,
];

/// Every rate of the shared test matrix.
fn rates() -> Vec<SampleRate> {
    let json: serde_json::Value =
        serde_json::from_str(include_str!("../../../conformance/test-matrix.json"))
            .expect("valid JSON");
    json["sample_rates"]
        .as_array()
        .expect("a list of rates")
        .iter()
        .map(|hz| {
            SampleRate::new(u32::try_from(hz.as_u64().expect("a whole number")).expect("a u32"))
                .expect("a supported rate")
        })
        .collect()
}

/// A binary64 setting value.
const fn value(raw: f64) -> Binary64 {
    Binary64::new(raw)
}

/// Settings from their raw values.
fn settings(ceiling: f64, lookahead: f64, release: f64, adaptivity: f64) -> LimiterSettings {
    LimiterSettings::new(
        value(ceiling),
        value(lookahead),
        value(release),
        value(adaptivity),
    )
    .expect("settings within their ranges")
}

/// The sample type's value of the binary64 `raw`.
fn of<S: Sample>(raw: f64) -> S {
    S::from_binary64(value(raw))
}

/// A deterministic noise, without a generator of its own: the mean of five sines of
/// incommensurate frequencies, one value a draw, always within ±1.
struct Noise(u32);

impl Noise {
    /// The noise from its first draw.
    const fn new() -> Self {
        Self(0)
    }

    /// The next value.
    fn next(&mut self) -> f64 {
        let at = f64::from(self.0);
        self.0 = self.0.wrapping_add(1);
        [
            0.012_3_f64,
            0.057_1_f64,
            0.112_9_f64,
            0.231_1_f64,
            0.417_7_f64,
        ]
        .iter()
        .zip(0_u32..)
        .fold(0.0_f64, |sum, (share, index)| {
            sum + libm::sin(core::f64::consts::TAU * share * at + f64::from(index))
        }) / 5.0_f64
    }
}

/// A sample of the noise scaled to `amplitude`, in binary64 then in `S`.
fn drawn<S: Sample>(draws: &mut Noise, amplitude: f64) -> S {
    of::<S>(draws.next() * amplitude)
}

/// A test's few channels, as the limiter counts them.
fn few(channels: usize) -> u16 {
    u16::try_from(channels).expect("few channels")
}

/// `blocks` blocks of `channels` channels of `signal(frame, channel)`.
fn signal<S: Sample>(
    blocks: usize,
    channels: usize,
    mut sample: impl FnMut(usize, usize) -> S,
) -> Vec<Vec<Block<S>>> {
    (0..blocks)
        .map(|block| {
            (0..channels)
                .map(|channel| {
                    core::array::from_fn(|offset| {
                        sample(
                            block.saturating_mul(BLOCK_SIZE).saturating_add(offset),
                            channel,
                        )
                    })
                })
                .collect()
        })
        .collect()
}

/// `input` through `limiter`, block by block.
fn limited<S: Sample>(
    limiter: &mut TruePeakLimiter<S>,
    input: &[Vec<Block<S>>],
) -> Vec<Vec<Block<S>>> {
    input
        .iter()
        .map(|blocks| {
            let mut out = blocks.clone();
            limiter.apply(&mut out);
            out
        })
        .collect()
}

/// The sample of `channel` at `frame` of `blocks`.
fn at<S: Sample>(blocks: &[Vec<Block<S>>], frame: usize, channel: usize) -> S {
    blocks[frame / BLOCK_SIZE][channel][frame % BLOCK_SIZE]
}

/// Frames in `milliseconds` at `rate`, rounded to the nearest, at least one: the test's
/// own reference, counting whole frames until the next would pass the exact duration less
/// half a frame. Its twin, `frames_of` in audio-testkit's limiter measurements, is a copy
/// (this crate cannot depend on that one): a fix of either reaches both.
fn frames_at(milliseconds: f64, rate: SampleRate) -> usize {
    let exact = milliseconds * f64::from(rate.hz()) / 1_000.0_f64;
    let count = (0_u32..=u32::MAX)
        .find(|count| f64::from(*count) + 0.5_f64 > exact)
        .expect("a count");
    usize::try_from(count).expect("small").max(1)
}

/// The latency of a limiter with `maximum` milliseconds of maximum lookahead and `detector`
/// at `rate`, counted apart: the maximum in frames ([`frames_at`]) and the detector's delay,
/// half its taps, as literals (`delays_by_its_detector_and_maximum_lookahead` pins them). A
/// test sizes its work from a limiter's latency only once it equals this.
fn latency_of(maximum: f64, rate: SampleRate, detector: super::Detector) -> usize {
    let delay = match detector {
        super::Detector::Standard => 6,
        super::Detector::Precise => 16,
        super::Detector::Mastering => 64,
    };
    frames_at(maximum, rate).saturating_add(delay)
}

/// The frames of the rise to a constant ([`rising_to`]).
const RISE: usize = 4_096;

/// The value at `frame` of a constant `level` reached from silence by a raised cosine of
/// [`RISE`] frames: a rise slow enough that no detector reads it above the constant by more
/// than [`exact_steady`] allows, where a step would overshoot by several percent (the
/// interpolator's ringing), a need the slowest release holds long after the constant has
/// settled; the step to silence before it reads no more than the signal before it.
fn rising_to(level: f64, frame: usize) -> f64 {
    let share = f64::from(u32::try_from(frame.min(RISE)).expect("small"))
        / f64::from(u32::try_from(RISE).expect("small"));
    level * (1.0_f64 - libm::cos(core::f64::consts::PI * share)) / 2.0_f64
}

/// The frames a constant reached by [`rising_to`] takes to settle through a limiter of
/// `detector` with `maximum` milliseconds of maximum lookahead at `rate`: the rise, its
/// latency ([`latency_of`]), then three windows of the maximum lookahead (the window held,
/// the Hann smoothing, and one more), and a block.
fn settling(maximum: f64, rate: SampleRate, detector: super::Detector) -> usize {
    RISE.saturating_add(latency_of(maximum, rate, detector))
        .saturating_add(frames_at(maximum, rate).saturating_mul(3))
        .saturating_add(BLOCK_SIZE)
}

/// Checks that each of `outputs`, the last outputs of a constant `level` that has settled
/// ([`settling`]) through a limiter of `detector` with `maximum` milliseconds of maximum
/// lookahead at `rate`, under the ceiling of amplitude `ceiling`, is its exact steady
/// output ([`exact_steady`]).
fn at_its_exact_gain(
    outputs: &[f64],
    (rate, detector, maximum): (SampleRate, super::Detector, f64),
    ceiling: f64,
    level: f64,
    context: &str,
) {
    let (expected, within) = exact_steady(ceiling, detector, frames_at(maximum, rate), level);
    for output in outputs {
        assert!(
            (output - expected).abs() <= within,
            "{context}: a constant {level} settles at {output}, {expected} expected within \
             {within}"
        );
    }
}

/// Feeds `limiter`, in `S`, of `channels` channels, a constant `level` above every need it
/// held before, reached by [`rising_to`], until it settles ([`settling`], its latency
/// checked first), and checks the last output of each channel ([`at_its_exact_gain`]).
fn settles_at_its_exact_gain<S: Sample + Raw>(
    limiter: &mut TruePeakLimiter<S>,
    channels: usize,
    run: (SampleRate, super::Detector, f64),
    ceiling: f64,
    level: f64,
    context: &str,
) {
    let (rate, detector, maximum) = run;
    assert_eq!(
        limiter.latency(),
        latency_of(maximum, rate, detector),
        "{context}: counted apart"
    );
    let blocks = settling(maximum, rate, detector).div_ceil(BLOCK_SIZE);
    let output = limited(
        limiter,
        &signal::<S>(blocks, channels, |frame, _| {
            of::<S>(rising_to(level, frame))
        }),
    );
    let last = blocks.saturating_mul(BLOCK_SIZE).saturating_sub(1);
    let outputs: Vec<f64> = (0..channels)
        .map(|channel| at(&output, last, channel).to_binary64().raw())
        .collect();
    at_its_exact_gain(&outputs, run, ceiling, level, context);
}

#[test]
fn counts_the_frames_of_a_duration_to_the_nearest_and_at_least_one() {
    let rate = SampleRate::new(48_000).expect("a rate");
    for (milliseconds, frames) in [
        (1.5_f64, 72),
        (5.0_f64, 240),
        (0.01_f64, 1),
        // Half a frame: the next one.
        (1.0_f64 / 96.0_f64, 1),
        (1.5_f64 / 48.0_f64, 2),
        (0.0_f64, 1),
    ] {
        assert_eq!(frames_at(milliseconds, rate), frames, "{milliseconds} ms");
    }
}

#[test]
fn accepts_each_setting_at_its_limits_and_refuses_one_past() {
    let ranges = [
        (Setting::Ceiling, -20.0_f64, 0.0_f64),
        (Setting::Lookahead, 0.5_f64, 5.0_f64),
        (Setting::MaximumLookahead, 0.5_f64, 5.0_f64),
        (Setting::Release, 10.0_f64, 1000.0_f64),
        (Setting::Adaptivity, 0.0_f64, 1.0_f64),
    ];
    for (setting, low, high) in ranges {
        assert_eq!(setting.range(), (value(low), value(high)), "{setting:?}");
    }
    let with = |index: usize, raw: f64| {
        let mut values = [-1.0_f64, 1.5_f64, 100.0_f64, 0.5_f64];
        values[index] = raw;
        LimiterSettings::new(
            value(values[0]),
            value(values[1]),
            value(values[2]),
            value(values[3]),
        )
    };
    for (index, (setting, low, high)) in ranges
        .into_iter()
        .filter(|(setting, ..)| *setting != Setting::MaximumLookahead)
        .enumerate()
    {
        for (end, raw) in [("low", low), ("high", high)] {
            assert_eq!(
                with(index, raw).map(|taken| {
                    [
                        taken.ceiling(),
                        taken.lookahead(),
                        taken.release(),
                        taken.adaptivity(),
                    ][index]
                }),
                Ok(value(raw)),
                "{setting:?} at its {end} end, taken as it is"
            );
        }
        for refused in [low.next_down(), high.next_up(), f64::NAN, f64::INFINITY] {
            assert_eq!(
                with(index, refused)
                    .map(|_| ())
                    .map_err(|error| match error {
                        LimiterError::OutOfRange(named, raw) => (named, raw.bits()),
                        other @ (LimiterError::AboveMaximum { .. }
                        | LimiterError::TooManyChannels(_)) => {
                            panic!("out of range: {other:?}")
                        }
                    }),
                Err((setting, refused.to_bits())),
                "{setting:?} refuses {refused}"
            );
        }
    }
    let rate = SampleRate::new(48_000).expect("a rate");
    let refused = |maximum: f64| {
        Err(LimiterError::OutOfRange(
            Setting::MaximumLookahead,
            value(maximum),
        ))
    };
    // Built, its latency the maximum in frames at 48 kHz and the standard delay; or refused
    // for its maximum lookahead, and for nothing else.
    for (maximum, built) in [
        (0.5_f64, Ok(24 + 6)),
        (5.0_f64, Ok(240 + 6)),
        (0.5_f64.next_down(), refused(0.5_f64.next_down())),
        (5.0_f64.next_up(), refused(5.0_f64.next_up())),
    ] {
        assert_eq!(
            TruePeakLimiter::<Binary64>::new(
                rate,
                2,
                value(maximum),
                super::Detector::Standard,
                settings(-1.0_f64, 0.5_f64, 100.0_f64, 0.5_f64),
                LimiterMode::Protected,
            )
            .map(|limiter| limiter.latency()),
            built,
            "a maximum lookahead of {maximum} ms"
        );
    }
}

#[test]
fn says_which_setting_it_refused_and_its_range() {
    assert_eq!(
        format!(
            "{}",
            LimiterError::OutOfRange(Setting::Ceiling, value(0.5_f64))
        ),
        "limiter ceiling 0.5 dBTP is out of range (-20 to 0 dBTP)"
    );
    assert_eq!(
        format!(
            "{}",
            LimiterError::OutOfRange(Setting::Adaptivity, value(2.0_f64))
        ),
        "limiter adaptivity 2 is out of range (0 to 1)"
    );
    assert_eq!(
        format!(
            "{}",
            LimiterError::OutOfRange(Setting::Release, value(5.0_f64))
        ),
        "limiter release 5 ms is out of range (10 to 1000 ms)"
    );
    let rate = SampleRate::new(44_100).expect("a rate");
    let mut limiter = TruePeakLimiter::<Binary32>::new(
        rate,
        1,
        value(2.0_f64),
        super::Detector::Standard,
        settings(-1.0_f64, 2.0_f64, 100.0_f64, 0.5_f64),
        LimiterMode::Protected,
    )
    .expect("a limiter");
    let error = limiter
        .set(settings(-1.0_f64, 2.5_f64, 100.0_f64, 0.5_f64))
        .expect_err("above the maximum");
    assert_eq!(
        format!("{error}"),
        "limiter lookahead 2.5 ms is above its maximum of 2 ms"
    );
    assert_eq!(
        limiter.settings(),
        settings(-1.0_f64, 2.0_f64, 100.0_f64, 0.5_f64),
        "unchanged"
    );
    assert_eq!(
        TruePeakLimiter::<Binary32>::new(
            rate,
            1,
            value(1.0_f64),
            super::Detector::Standard,
            settings(-1.0_f64, 2.0_f64, 100.0_f64, 0.5_f64),
            LimiterMode::Protected
        )
        .map(|_| ()),
        Err(LimiterError::AboveMaximum {
            lookahead: value(2.0_f64),
            maximum: value(1.0_f64)
        }),
        "a lookahead above the maximum it is built with"
    );
}

#[test]
fn takes_up_to_the_largest_channel_count_and_refuses_one_more() {
    // No channel, one and the largest count are taken (`MAX_CHANNELS`, 512, decision 1's
    // maximum, written apart); one more is refused with its count, never allocated.
    let rate = SampleRate::new(8_000).expect("a rate");
    let build = |channels: u16| {
        TruePeakLimiter::<Binary32>::new(
            rate,
            channels,
            value(0.5_f64),
            super::Detector::Standard,
            settings(-1.0_f64, 0.5_f64, 100.0_f64, 0.5_f64),
            LimiterMode::Protected,
        )
        .map(|limiter| (limiter.latency(), limiter.channels()))
    };
    for channels in [0_u16, 1, 512] {
        assert_eq!(
            build(channels),
            Ok((10, channels)),
            "{channels} channels, kept"
        );
    }
    for channels in [513_u16, u16::MAX] {
        assert_eq!(
            build(channels),
            Err(LimiterError::TooManyChannels(channels)),
            "{channels} channels"
        );
    }
    assert_eq!(
        format!("{}", LimiterError::TooManyChannels(513)),
        "too many limiter channels: 513 (at most 512)"
    );
}

/// The detectors and their names as the interfaces write them, as literals: a reorder of
/// [`super::Detector::ALL`] would change the codes of the C interface silently otherwise.
#[test]
fn lists_every_detector_in_the_order_of_its_code_with_its_name() {
    assert_eq!(
        super::Detector::ALL.map(super::Detector::name),
        ["standard", "precise", "mastering"],
        "names, in the order of the codes"
    );
    assert_eq!(
        super::Detector::ALL,
        [
            super::Detector::Standard,
            super::Detector::Precise,
            super::Detector::Mastering
        ],
        "every detector"
    );
}

#[test]
fn checks_one_setting_at_its_limits() {
    for (setting, accepted) in [
        (Setting::Ceiling, -20.0_f64),
        (Setting::Ceiling, 0.0_f64),
        (Setting::MaximumLookahead, 0.5_f64),
        (Setting::MaximumLookahead, 5.0_f64),
    ] {
        assert_eq!(
            setting.checked(value(accepted)),
            Ok(value(accepted)),
            "{setting:?} {accepted}"
        );
    }
    for (setting, refused, message) in [
        (
            Setting::Ceiling,
            -20.25_f64,
            "limiter ceiling -20.25 dBTP is out of range (-20 to 0 dBTP)",
        ),
        (
            Setting::Ceiling,
            0.25_f64,
            "limiter ceiling 0.25 dBTP is out of range (-20 to 0 dBTP)",
        ),
        (
            Setting::MaximumLookahead,
            0.25_f64,
            "limiter maximum lookahead 0.25 ms is out of range (0.5 to 5 ms)",
        ),
        (
            Setting::MaximumLookahead,
            f64::NAN,
            "limiter maximum lookahead NaN ms is out of range (0.5 to 5 ms)",
        ),
    ] {
        assert_eq!(
            setting
                .checked(value(refused))
                .map_err(|error| format!("{error}")),
            Err(message.into()),
            "{setting:?} {refused}"
        );
    }
}

#[test]
fn reads_back_each_setting_it_holds() {
    // Four values, each distinct from its default and from the others.
    let held = settings(-7.5_f64, 2.5_f64, 250.0_f64, 0.25_f64);
    assert_eq!(
        (
            held.ceiling(),
            held.lookahead(),
            held.release(),
            held.adaptivity()
        ),
        (
            value(-7.5_f64),
            value(2.5_f64),
            value(250.0_f64),
            value(0.25_f64)
        )
    );
}

#[test]
fn checks_a_maximum_lookahead_and_the_lookahead_under_it_in_one_place() {
    let held = settings(-1.0_f64, 2.0_f64, 100.0_f64, 0.5_f64);
    for maximum in [2.0_f64, 5.0_f64] {
        assert_eq!(held.within_maximum(value(maximum)), Ok(()), "{maximum} ms");
    }
    for maximum in [0.5_f64.next_down(), 5.0_f64.next_up(), f64::NAN] {
        assert_eq!(
            held.within_maximum(value(maximum))
                .map_err(|error| match error {
                    LimiterError::OutOfRange(setting, raw) => (setting, raw.bits()),
                    other @ (LimiterError::AboveMaximum { .. }
                    | LimiterError::TooManyChannels(_)) => {
                        panic!("out of range: {other:?}")
                    }
                }),
            Err((Setting::MaximumLookahead, maximum.to_bits())),
            "{maximum} ms"
        );
    }
    assert_eq!(
        held.within_maximum(value(2.0_f64.next_down())),
        Err(LimiterError::AboveMaximum {
            lookahead: value(2.0_f64),
            maximum: value(2.0_f64.next_down())
        }),
        "a lookahead above the maximum"
    );
    // A host can keep the error as one of its own (`core::error::Error`).
    let error: &dyn core::error::Error = &LimiterError::TooManyChannels(513);
    assert_eq!(
        format!("{error}"),
        "too many limiter channels: 513 (at most 512)"
    );
    assert!(error.source().is_none(), "no cause beneath it");
}

#[test]
fn declares_its_latency_from_the_maximum_lookahead_at_every_rate() {
    for rate in rates() {
        for maximum in [0.5_f64, 1.5_f64, 5.0_f64] {
            let limiter = TruePeakLimiter::<Binary64>::new(
                rate,
                2,
                value(maximum),
                super::Detector::Standard,
                settings(-1.0_f64, 0.5_f64, 100.0_f64, 0.5_f64),
                LimiterMode::Protected,
            )
            .expect("a limiter");
            assert_eq!(
                limiter.latency(),
                frames_at(maximum, rate) + 6,
                "{maximum} ms at {} Hz",
                rate.hz()
            );
        }
    }
}

/// Checks that `output` is `input` delayed by `latency` frames, bit for bit, zeros before.
fn delayed_exactly<S: Sample + Raw>(
    input: &[Vec<Block<S>>],
    output: &[Vec<Block<S>>],
    latency: usize,
    channels: usize,
) -> bool {
    let frames = input.len().saturating_mul(BLOCK_SIZE);
    (0..frames).all(|frame| {
        (0..channels).all(|channel| {
            let expected = frame
                .checked_sub(latency)
                .map_or(0, |earlier| at(input, earlier, channel).bits());
            at(output, frame, channel).bits() == expected
        })
    })
}

/// The largest sum of the magnitudes of a phase of `detector`'s interpolator, and that
/// phase's taps: the most it multiplies the largest sample by, reached when every sample
/// has the sign of its tap.
fn largest_phase(detector: super::Detector) -> (f64, Vec<f64>) {
    detector
        .coefficients()
        .chunks_exact(detector.taps())
        .map(|phase| {
            let taps: Vec<f64> = phase.iter().map(|tap| tap.raw()).collect();
            (taps.iter().fold(0.0_f64, |sum, tap| sum + tap.abs()), taps)
        })
        .fold((0.0_f64, Vec::new()), |largest, phase| {
            if phase.0 > largest.0 { phase } else { largest }
        })
}

/// The law below the ceiling, in `S`: at every rate, a signal whose true peak stays below
/// the ceiling leaves delayed, bit for bit.
///
/// Its precondition, derived per detector: a phase of the interpolator multiplies samples
/// within `±A` by at most its sum of magnitudes `L1` (2.02 for the standard's, 2.64 for the
/// precise, 3.38 for the mastering one), and the interval's two samples by one, so a noise
/// within `±A` (the mean of five sines, [`Noise`]) reads at most `L1·A`; at `A = 0.99·c/L1`
/// it stays under the ceiling `c` by 1%, far more than the rounding of the coefficients and
/// sums in either precision. The edge itself: the input whose samples take the sign of the
/// largest phase's taps reads exactly `L1·A`, unchanged at 0.99 of the ceiling and limited
/// at 1.01.
fn changes_no_bit_below_the_ceiling<S: Sample + Raw>() {
    let ceiling = CEILING;
    let mut draws = Noise::new();
    for (rate, detector) in rates().into_iter().zip(DETECTORS.into_iter().cycle()) {
        let (sum, taps) = largest_phase(detector);
        let stated = match detector {
            super::Detector::Standard => 2.02_f64,
            super::Detector::Precise => 2.64_f64,
            super::Detector::Mastering => 3.38_f64,
        };
        assert!(
            (sum - stated).abs() <= 0.005_f64,
            "{detector:?}: {sum}, as the comment states"
        );
        let below = 0.99_f64 * ceiling / sum;
        for channels in [1, 2, 3] {
            let mut limiter = TruePeakLimiter::<S>::new(
                rate,
                few(channels),
                value(5.0_f64),
                detector,
                LimiterSettings::DEFAULT,
                LimiterMode::Protected,
            )
            .expect("a limiter");
            let latency = limiter.latency();
            assert_eq!(
                latency,
                latency_of(5.0_f64, rate, detector),
                "counted apart"
            );
            // Past the latency, so that the input leaves the delay.
            let blocks = latency.div_ceil(BLOCK_SIZE).saturating_add(12);
            let input = signal::<S>(blocks, channels, |_, _| drawn(&mut draws, below));
            let output = limited(&mut limiter, &input);
            assert!(
                delayed_exactly(&input, &output, latency, channels),
                "{channels} channels at {} Hz",
                rate.hz()
            );
        }
        for (share, unchanged) in [(0.99_f64, true), (1.01_f64, false)] {
            let level = share * ceiling / sum;
            let mut limiter = TruePeakLimiter::<S>::new(
                rate,
                1,
                value(5.0_f64),
                detector,
                LimiterSettings::DEFAULT,
                LimiterMode::Protected,
            )
            .expect("a limiter");
            let latency = limiter.latency();
            assert_eq!(
                latency,
                latency_of(5.0_f64, rate, detector),
                "counted apart"
            );
            let blocks = latency.div_ceil(BLOCK_SIZE).saturating_add(4);
            // Tap `k` multiplies the sample `k` frames before the newest: the window ending
            // at frame `taps`, its samples of the signs of the taps, zeros around it.
            let newest = taps.len();
            let input = signal::<S>(blocks, 1, |frame, _| {
                newest
                    .checked_sub(frame)
                    .and_then(|back| taps.get(back))
                    .map_or(S::ZERO, |tap| {
                        of::<S>(if *tap < 0.0_f64 { -level } else { level })
                    })
            });
            let output = limited(&mut limiter, &input);
            assert_eq!(
                delayed_exactly(&input, &output, latency, 1),
                unchanged,
                "{detector:?} at {share} of the ceiling at {} Hz",
                rate.hz()
            );
        }
    }
}

#[test]
fn changes_no_bit_below_the_ceiling_in_each_precision() {
    changes_no_bit_below_the_ceiling::<Binary64>();
    changes_no_bit_below_the_ceiling::<Binary32>();
}

/// Blocks of every loud signal: 4,096 frames, longer than the largest latency (5 ms at
/// 384 kHz and the longest detector's delay, the mastering one's; `holds_the_ceiling`
/// checks it against each limiter), so that every rate gives limited output.
const LOUD_BLOCKS: usize = 4_096 / BLOCK_SIZE;

/// Loud test signals, in `S`: noise, then sines from low to near half the rate, each well
/// above the ceiling.
fn loud<S: Sample>(channels: usize, draws: &mut Noise) -> Vec<Vec<Vec<Block<S>>>> {
    let mut signals = vec![signal::<S>(LOUD_BLOCKS, channels, |_, _| {
        drawn(draws, 4.0_f64)
    })];
    for share in [0.001_f64, 0.05_f64, 0.2_f64, 0.45_f64] {
        signals.push(signal::<S>(LOUD_BLOCKS, channels, |frame, channel| {
            let phase =
                core::f64::consts::TAU * share * f64::from(u32::try_from(frame).expect("small"))
                    + f64::from(u32::try_from(channel).expect("small"));
            of::<S>(3.0_f64 * libm::sin(phase))
        }));
    }
    signals
}

/// The largest magnitude of `blocks` after the first `skip` frames, in binary64.
fn largest<S: Sample + Raw>(blocks: &[Vec<Block<S>>], skip: usize, channels: usize) -> f64 {
    let frames = blocks.len().saturating_mul(BLOCK_SIZE);
    (skip..frames)
        .flat_map(|frame| (0..channels).map(move |channel| (frame, channel)))
        .map(|(frame, channel)| at(blocks, frame, channel).to_binary64().raw().abs())
        .fold(0.0_f64, larger)
}

/// The ceiling held, in `S`: on loud signals, no output sample above the ceiling, with no
/// slack.
fn holds_the_ceiling<S: Sample + Raw>() {
    let mut draws = Noise::new();
    let ceiling = CEILING;
    for (at_rate, rate) in rates().into_iter().enumerate() {
        // Every detector, on one channel or two by turns across the rates.
        for (index, detector) in DETECTORS.into_iter().enumerate() {
            let channels = if index.wrapping_add(at_rate).is_multiple_of(2) {
                1
            } else {
                2
            };
            for input in loud::<S>(channels, &mut draws) {
                let mut limiter = TruePeakLimiter::<S>::new(
                    rate,
                    few(channels),
                    value(5.0_f64),
                    detector,
                    LimiterSettings::DEFAULT,
                    LimiterMode::Protected,
                )
                .expect("a limiter");
                let output = limited(&mut limiter, &input);
                let peak = largest(&output, 0, channels);
                assert!(
                    limiter.latency() < LOUD_BLOCKS * BLOCK_SIZE,
                    "limited output"
                );
                assert!(
                    peak <= ceiling,
                    "{peak} above {ceiling} at {} Hz, {channels} channels",
                    rate.hz()
                );
                assert!(
                    peak > ceiling * 0.5_f64,
                    "it does not silence the signal: {peak}"
                );
                // Then a constant above every need of the signal (at most 4 × 3.38 for the
                // noise, `changes_no_bit_below_the_ceiling`), at its exact gain.
                settles_at_its_exact_gain(
                    &mut limiter,
                    channels,
                    (rate, detector, 5.0_f64),
                    ceiling,
                    16.0_f64,
                    &format!("{} Hz, {detector:?}", rate.hz()),
                );
            }
        }
    }
}

/// Holds the ceiling with its lookahead at its maximum, where the window's needs fill the
/// queue of its sliding maximum: a loud 48 Hz sine, whose needs fall for longer than the
/// window, with the shortest release and the fastest adaptivity, at every rate and with
/// every detector.
fn holds_the_ceiling_with_its_lookahead_at_its_maximum<S: Sample + Raw>() {
    let ceiling = CEILING;
    for rate in rates() {
        // Three periods of the sine.
        let frames = usize::try_from(rate.hz()).expect("small") / 16;
        let blocks = frames.div_ceil(BLOCK_SIZE);
        for detector in DETECTORS {
            let mut limiter = TruePeakLimiter::<S>::new(
                rate,
                1,
                value(1.5_f64),
                detector,
                settings(-1.0, 1.5, 10.0, 1.0),
                LimiterMode::Protected,
            )
            .expect("a limiter");
            let hz = f64::from(rate.hz());
            let input = signal(blocks, 1, |frame, _| {
                let at = f64::from(u32::try_from(frame).expect("small"));
                of::<S>(3.0_f64 * libm::sin(core::f64::consts::TAU * 48.0_f64 * at / hz))
            });
            let peak = largest(&limited(&mut limiter, &input), 0, 1);
            assert!(
                peak <= ceiling,
                "{peak} above {ceiling} at {} Hz, {detector:?}",
                rate.hz()
            );
            assert!(
                peak > ceiling * 0.5_f64,
                "it does not silence the signal: {peak}"
            );
            settles_at_its_exact_gain(
                &mut limiter,
                1,
                (rate, detector, 1.5_f64),
                ceiling,
                16.0_f64,
                &format!("{} Hz, {detector:?}", rate.hz()),
            );
        }
    }
}

#[test]
fn holds_the_ceiling_with_its_lookahead_at_its_maximum_in_each_precision() {
    holds_the_ceiling_with_its_lookahead_at_its_maximum::<Binary64>();
    holds_the_ceiling_with_its_lookahead_at_its_maximum::<Binary32>();
}

/// The amplitude of a ceiling of -20 dBTP in binary64, computed apart: the limiter's formula,
/// `exp(-20 × fl(ln 10 / 20))`, is 0.0999999999999999782924… (at 70 digits), whose nearest
/// binary64 lies 2 ulps below 0.1 (`0x3fb9999999999998` against `0x3fb999999999999a`), so
/// that a sample at 0.1 is above the ceiling, not at it.
const MINUS_20_DBTP: f64 = 0.099_999_999_999_999_98_f64;

#[test]
fn pins_the_amplitude_of_minus_20_dbtp_two_ulps_below_a_tenth() {
    assert_eq!(MINUS_20_DBTP.to_bits(), 0x3fb9_9999_9999_9998, "its bits");
    assert_eq!(
        0.1_f64.to_bits() - MINUS_20_DBTP.to_bits(),
        2,
        "two ulps below 0.1"
    );
}

/// The ceilings of the sweep, in dBTP, each with its amplitude `10^(dB/20)` in binary64,
/// computed apart: the ends of the range and the default.
const CEILINGS: [(f64, f64); 3] = [
    (-20.0_f64, MINUS_20_DBTP),
    (-1.0_f64, CEILING),
    (0.0_f64, 1.0_f64),
];

/// The lookaheads of the sweep, in milliseconds: the ends of the range and the default.
const LOOKAHEADS: [f64; 3] = [0.5_f64, 1.5_f64, 5.0_f64];

/// The releases of the sweep, in milliseconds: the ends of the range and the default.
const RELEASES: [f64; 3] = [10.0_f64, 100.0_f64, 1_000.0_f64];

/// The adaptivities of the sweep: the ends of the range and the default.
const ADAPTIVITIES: [f64; 3] = [0.0_f64, 0.5_f64, 1.0_f64];

/// The shapes of the sweep's signals, each scaled to a level.
#[derive(Debug, Clone, Copy)]
enum Shape {
    /// Silence, then the level held (a step).
    Step,
    /// A sine of 997 Hz.
    Sine,
    /// Three sines of incommensurate shares of the rate, their mean.
    ThreeSines,
    /// The level held, then three quarters of it (12 to 9) from the middle of the run.
    Drop,
    /// The level held, then a twenty-fourth of it (12 to 0.5) from the middle of the run.
    Fall,
}

/// Every shape of the sweep.
const SHAPES: [Shape; 5] = [
    Shape::Step,
    Shape::Sine,
    Shape::ThreeSines,
    Shape::Drop,
    Shape::Fall,
];

/// The value of `shape` at `level` at `frame` of a run of `frames` at `rate`, on `channel`
/// (each channel a phase apart).
fn shaped(shape: Shape, level: f64, frame: usize, frames: usize, channel: usize, hz: f64) -> f64 {
    let at = f64::from(u32::try_from(frame).expect("small"));
    let turn = f64::from(u32::try_from(channel).expect("small"));
    let later = frame >= frames / 2;
    match shape {
        Shape::Step => {
            if frame < 64 {
                0.0_f64
            } else {
                level
            }
        }
        Shape::Sine => level * libm::sin(core::f64::consts::TAU * 997.0_f64 * at / hz + turn),
        Shape::ThreeSines => {
            level
                * [0.013_f64, 0.137_f64, 0.411_f64]
                    .iter()
                    .fold(0.0_f64, |sum, share| {
                        sum + libm::sin(core::f64::consts::TAU * share * at + turn)
                    })
                / 3.0_f64
        }
        Shape::Drop => {
            if later {
                level * 0.75_f64
            } else {
                level
            }
        }
        Shape::Fall => {
            if later {
                level / 24.0_f64
            } else {
                level
            }
        }
    }
}

/// One run of the sweep, in `S`: the settings drawn from the grid, the signal at its level.
#[derive(Debug, Clone, Copy)]
struct Run {
    /// The rate.
    rate: SampleRate,
    /// The detector.
    detector: super::Detector,
    /// The ceiling, in dBTP, and its amplitude computed apart.
    ceiling: (f64, f64),
    /// The lookahead, in milliseconds.
    lookahead: f64,
    /// The maximum lookahead, in milliseconds: the lookahead itself or the largest.
    maximum: f64,
    /// The release, in milliseconds.
    release: f64,
    /// The adaptivity.
    adaptivity: f64,
    /// The signal's shape.
    shape: Shape,
    /// The signal's level.
    level: f64,
    /// The channels.
    channels: usize,
}

/// The limiter of `run`, in `S`, its input, and whether its level leaves the output a gain
/// to hold (the margin times the level well below the ceiling).
fn run_of<S: Sample + Raw>(run: Run) -> (TruePeakLimiter<S>, Vec<Vec<Block<S>>>, bool) {
    let limiter = TruePeakLimiter::<S>::new(
        run.rate,
        few(run.channels),
        value(run.maximum),
        run.detector,
        settings(run.ceiling.0, run.lookahead, run.release, run.adaptivity),
        LimiterMode::Protected,
    )
    .expect("a limiter");
    let latency = limiter.latency();
    assert_eq!(
        latency,
        latency_of(run.maximum, run.rate, run.detector),
        "counted apart: {run:?}"
    );
    let window = frames_at(run.lookahead, run.rate);
    // Past the latency by two windows and a little more, the whole level reached; twice the
    // latency for a drop, which comes in the middle of the run.
    let frames = match run.shape {
        Shape::Drop | Shape::Fall => latency.saturating_mul(2),
        Shape::Step | Shape::Sine | Shape::ThreeSines => latency,
    }
    .saturating_add(window.saturating_mul(2))
    .saturating_add(256)
    .next_multiple_of(BLOCK_SIZE);
    let hz = f64::from(run.rate.hz());
    let input = signal::<S>(frames / BLOCK_SIZE, run.channels, |frame, channel| {
        of::<S>(shaped(run.shape, run.level, frame, frames, channel, hz))
    });
    let audible =
        margin_of(frames_at(run.maximum, run.rate)) * run.level <= run.ceiling.1 / 8.0_f64;
    (limiter, input, audible)
}

/// What a run of the sweep gives: its largest output sample, in binary64; whether its level
/// leaves the output a gain to hold; and then, where it does, the last outputs of each
/// channel given four times that level, constant ([`constant_after`]).
type Swept = (f64, bool, Vec<f64>);

/// The constant a run of the sweep is given once its signal has run, in `S`: four times its
/// level, above every need of its signal (each shape reads at most 3.38 times its level, the
/// largest sum of a detector's taps' magnitudes, `changes_no_bit_below_the_ceiling`), so
/// that every stage holds the constant's need ([`exact_steady`]), for as long as it takes to
/// settle ([`settling`]), reached by [`rising_to`]; none where the level leaves no gain to
/// hold.
fn constant_after<S: Sample>(run: Run, audible: bool) -> Vec<Vec<Block<S>>> {
    let blocks = if audible {
        settling(run.maximum, run.rate, run.detector).div_ceil(BLOCK_SIZE)
    } else {
        0
    };
    signal::<S>(blocks, run.channels, |frame, _| {
        of::<S>(rising_to(4.0_f64 * run.level, frame))
    })
}

/// The last output of each channel of `output`, in binary64: none when it is empty.
fn last_outputs<S: Sample>(output: &[Vec<Block<S>>], channels: usize) -> Vec<f64> {
    let Some(last) = output.len().saturating_mul(BLOCK_SIZE).checked_sub(1) else {
        return Vec::new();
    };
    (0..channels)
        .map(|channel| at(output, last, channel).to_binary64().raw())
        .collect()
}

/// A run of the sweep in `S` ([`Swept`]).
fn swept<S: Sample + Raw>(run: Run) -> Swept {
    let (mut limiter, input, audible) = run_of::<S>(run);
    let output = limited(&mut limiter, &input);
    let settled = limited(&mut limiter, &constant_after::<S>(run, audible));
    (
        largest(&output, 0, run.channels),
        audible,
        last_outputs(&settled, run.channels),
    )
}

/// The largest output sample of `run` computed in binary64 and given as binary32, through the
/// output stage as an engine of that format gives it (`OutputStage::apply_narrowing`), and
/// whether its level leaves the output a gain to hold.
fn swept_given_as_binary32(run: Run) -> Swept {
    let (limiter, input, audible) = run_of::<Binary64>(run);
    let mut stage = crate::OutputStage::<_, f32>::limited(limiter);
    let mut given = |computing: &[Vec<Block<Binary64>>]| -> Vec<Vec<Block<Binary32>>> {
        computing
            .iter()
            .map(|blocks| {
                stage
                    .apply_narrowing(|computed| computed.copy_from_slice(blocks))
                    .to_vec()
            })
            .collect()
    };
    let output = given(&input);
    let settled = given(&constant_after::<Binary64>(run, audible));
    (
        largest(&output, 0, run.channels),
        audible,
        last_outputs(&settled, run.channels),
    )
}

/// The sweep of `sweep` over `levels`: every sample at or below the ceiling, with no slack,
/// over a grid of the settings in which every value of each appears (the index of the run
/// picks each, a dimension's values cycling within every three runs), and over the corners
/// where their extremes meet (the longest window at the highest rate with the lowest
/// ceiling, the slowest release and the most precise detector; and the reverse), for every
/// shape at every level. Where the margin leaves a gain to hold, the output is not silenced, and a constant
/// above the signal then settles at its exact gain.
fn holds_the_ceiling_with_no_slack(levels: &[f64], sweep: fn(Run) -> Swept) {
    let rates = rates();
    let highest = *rates.last().expect("a rate");
    let lowest = rates[0];
    let pick = |index: usize, place: u32| {
        index
            .saturating_add(index.div_euclid(3_usize.pow(place)))
            .rem_euclid(3)
    };
    let mut index = 0_usize;
    for shape in SHAPES {
        for level in levels {
            let ceiling = CEILINGS[pick(index, 1)];
            let lookahead = LOOKAHEADS[pick(index, 2)];
            let grid = Run {
                rate: rates[index.rem_euclid(rates.len())],
                detector: DETECTORS[pick(index, 0)],
                ceiling,
                lookahead,
                maximum: if index.is_multiple_of(2) {
                    5.0_f64
                } else {
                    lookahead
                },
                release: RELEASES[pick(index, 3)],
                adaptivity: ADAPTIVITIES[pick(index, 4)],
                shape,
                level: *level,
                channels: index.rem_euclid(2).saturating_add(1),
            };
            // The most precise detector, the dearest, on one shape only: the margin does not
            // depend on the detector.
            let stressed = Run {
                rate: highest,
                detector: if matches!(shape, Shape::Step) {
                    super::Detector::Mastering
                } else {
                    super::Detector::Standard
                },
                ceiling: CEILINGS[0],
                lookahead: 5.0_f64,
                maximum: 5.0_f64,
                release: 1_000.0_f64,
                adaptivity: 0.0_f64,
                channels: 1,
                ..grid
            };
            let relaxed = Run {
                rate: lowest,
                detector: super::Detector::Standard,
                ceiling: CEILINGS[2],
                lookahead: 0.5_f64,
                maximum: 0.5_f64,
                release: 10.0_f64,
                adaptivity: 1.0_f64,
                ..grid
            };
            for run in [grid, stressed, relaxed] {
                // A level just over the ceiling is given as a share of it.
                let run = Run {
                    level: if run.level < 0.0_f64 {
                        run.ceiling.1 * -run.level
                    } else {
                        run.level
                    },
                    ..run
                };
                let (peak, audible, settled) = sweep(run);
                assert!(peak <= run.ceiling.1, "{peak} above the ceiling: {run:?}");
                assert!(
                    !audible || peak >= run.ceiling.1 / 2.0_f64,
                    "{peak}, silenced: {run:?}"
                );
                assert_eq!(settled.len(), if audible { run.channels } else { 0 });
                at_its_exact_gain(
                    &settled,
                    (run.rate, run.detector, run.maximum),
                    run.ceiling.1,
                    4.0_f64 * run.level,
                    &format!("{run:?}"),
                );
            }
            index = index.saturating_add(1);
        }
    }
}

/// The levels of the sweep in both precisions: just over the ceiling (a share of it,
/// written negative), then from 4 up to 10^9, where binary32's control path once exceeded the
/// ceiling by far.
const LADDER: [f64; 6] = [
    -1.000_1_f64,
    4.0_f64,
    1e3_f64,
    6.5e4_f64,
    1_048_576.0_f64,
    1e9_f64,
];

#[test]
fn holds_the_ceiling_with_no_slack_across_its_settings_levels_and_signals_in_binary64() {
    // Then levels only binary64 holds, up to the largest finite samples, whose estimate
    // overflows.
    let mut levels = LADDER.to_vec();
    levels.extend([1e12_f64, 1e15_f64, 1.7e308_f64]);
    holds_the_ceiling_with_no_slack(&levels, swept::<Binary64>);
}

#[test]
fn holds_the_ceiling_with_no_slack_across_its_settings_levels_and_signals_in_binary64_given_as_binary32()
 {
    // The levels of binary64: the output rounded to binary32 as given, its ceiling rounded
    // toward zero in binary32, so that no sample rounds above it.
    let mut levels = LADDER.to_vec();
    levels.extend([1e12_f64, 1e15_f64, 1.7e308_f64]);
    holds_the_ceiling_with_no_slack(&levels, swept_given_as_binary32);
}

#[test]
fn holds_the_ceiling_with_no_slack_across_its_settings_levels_and_signals_in_binary32() {
    // Then the largest finite samples, whose estimate overflows.
    let mut levels = LADDER.to_vec();
    levels.push(3.4e38_f64);
    holds_the_ceiling_with_no_slack(&levels, swept::<Binary32>);
}

/// The last output of a limiter in `S` at `rate`, with `maximum` milliseconds of maximum
/// lookahead (and lookahead) and the default ceiling, given the constant `level` past its
/// latency and two windows: the steady output of that level, in binary64.
fn steady_output<S: Sample>(rate: SampleRate, maximum: f64, level: f64) -> f64 {
    let mut limiter = TruePeakLimiter::<S>::new(
        rate,
        1,
        value(maximum),
        super::Detector::Standard,
        settings(-1.0_f64, maximum, 100.0_f64, 0.5_f64),
        LimiterMode::Protected,
    )
    .expect("a limiter");
    let frames = frames_at(maximum, rate)
        .saturating_mul(3)
        .saturating_add(super::Detector::Standard.delay())
        .saturating_add(BLOCK_SIZE);
    let output = limited(
        &mut limiter,
        &signal::<S>(frames.div_ceil(BLOCK_SIZE), 1, |_, _| of::<S>(level)),
    );
    let last = output.len().saturating_mul(BLOCK_SIZE).saturating_sub(1);
    at(&output, last, 0).to_binary64().raw()
}

/// The margin's cost, in `S`, at the ends of the rates and windows (8 kHz and 0.5 ms, 384 kHz
/// and 5 ms): a steady level `p` comes out at `c/g - p·E` (`g` the detector's reading of a
/// constant, the same at every level), so a tenth of `c/E` comes out `p·E`, a tenth of the
/// ceiling, below a level of `10c`, within 1% (the roundings, binary32's included, below
/// 2^-20 of the ceiling); and four times `c/E` is silenced, exactly: its need is at least
/// `1 + 3E/4`, past one by more than the control path's rounding `ρ ≤ (43M + 3)·2^-53`,
/// under `3E/4` (`64M` against `43M + 3`, `M ≥ 4`), so its gain is zero.
fn lowers_its_output_by_the_margin_times_the_level_then_silences_it<S: Sample>() {
    let ceiling = CEILING;
    for (hz, maximum) in [(8_000_u32, 0.5_f64), (384_000_u32, 5.0_f64)] {
        let rate = SampleRate::new(hz).expect("a rate");
        let margin = margin_of(frames_at(maximum, rate));
        let quiet = of::<S>(10.0_f64 * ceiling).to_binary64().raw();
        let loud = of::<S>(0.1_f64 * ceiling / margin).to_binary64().raw();
        let lost =
            steady_output::<S>(rate, maximum, quiet) - steady_output::<S>(rate, maximum, loud);
        let expected = (loud - quiet) * margin;
        assert!(
            (lost - expected).abs() <= 0.01_f64 * expected,
            "{hz} Hz: {lost} lost, {expected} expected"
        );
        let silenced = steady_output::<S>(rate, maximum, 4.0_f64 * ceiling / margin);
        assert_eq!(silenced.to_bits(), 0, "{hz} Hz: silenced, {silenced}");
    }
}

#[test]
fn lowers_its_output_by_the_margin_times_the_level_then_silences_it_in_each_precision() {
    lowers_its_output_by_the_margin_times_the_level_then_silences_it::<Binary64>();
    lowers_its_output_by_the_margin_times_the_level_then_silences_it::<Binary32>();
}

#[test]
fn holds_the_ceiling_in_each_precision() {
    holds_the_ceiling::<Binary64>();
    holds_the_ceiling::<Binary32>();
}

/// Raw, in `S`: even a loud signal leaves delayed, bit for bit.
fn passes_everything_raw<S: Sample + Raw>() {
    let mut draws = Noise::new();
    for rate in rates() {
        for input in loud::<S>(2, &mut draws) {
            let mut limiter = TruePeakLimiter::<S>::new(
                rate,
                2,
                value(5.0_f64),
                super::Detector::Standard,
                LimiterSettings::DEFAULT,
                LimiterMode::Switchable,
            )
            .expect("a limiter");
            let latency = limiter.latency();
            assert_eq!(
                latency,
                latency_of(5.0_f64, rate, super::Detector::Standard),
                "counted apart"
            );
            let output = limited(&mut limiter, &input);
            assert!(
                delayed_exactly(&input, &output, latency, 2),
                "raw at {} Hz",
                rate.hz()
            );
        }
    }
}

#[test]
fn passes_everything_raw_in_each_precision() {
    passes_everything_raw::<Binary64>();
    passes_everything_raw::<Binary32>();
}

/// Switched to raw while limiting, in `S`: once the crossfade has arrived, the output is
/// the input delayed, bit for bit.
fn returns_exactly_to_raw<S: Sample + Raw>() {
    let mut draws = Noise::new();
    let rate = SampleRate::new(48_000).expect("a rate");
    let loud_input = signal::<S>(100, 2, |_, _| drawn(&mut draws, 4.0_f64));
    let mut limiter = TruePeakLimiter::<S>::new(
        rate,
        2,
        value(1.5_f64),
        super::Detector::Standard,
        LimiterSettings::DEFAULT,
        LimiterMode::Protected,
    )
    .expect("a limiter");
    let latency = limiter.latency();
    assert_eq!(
        latency,
        latency_of(1.5_f64, rate, super::Detector::Standard),
        "counted apart"
    );
    let _limited = limited(&mut limiter, &loud_input);
    limiter.set_protected(false);
    assert!(!limiter.protected(), "raw from now on");
    // 20 ms constant, snapping once `1 - share` is 1 in binary64, the control path's
    // precision (the share below 2^-54): about 38 time constants, 750 ms; a second is 750
    // blocks at 48 kHz.
    let fading = signal::<S>(750, 2, |_, _| drawn(&mut draws, 4.0_f64));
    let _faded = limited(&mut limiter, &fading);
    let after = signal::<S>(50, 2, |_, _| drawn(&mut draws, 4.0_f64));
    let output = limited(&mut limiter, &after);
    // The first `latency` frames are still the fading input's tail.
    let frames = after.len().saturating_mul(BLOCK_SIZE);
    assert!(
        (latency..frames).all(
            |frame| (0..2).all(|channel| at(&output, frame, channel).bits()
                == at(&after, frame.saturating_sub(latency), channel).bits())
        ),
        "raw again, bit for bit"
    );
}

#[test]
fn returns_exactly_to_raw_in_each_precision() {
    returns_exactly_to_raw::<Binary64>();
    returns_exactly_to_raw::<Binary32>();
}

#[test]
fn holds_the_largest_need_of_its_window_whatever_the_lookahead() {
    // The sliding maximum against the largest need of the window counted apart, at every
    // rate, with the lookahead at its maximum (the queue full) and below it, for needs that
    // fall for longer than the window (each need the largest until it leaves), rise and
    // wander.
    for rate in rates() {
        for (maximum, lookahead) in [(0.5_f64, 0.5_f64), (5.0_f64, 5.0_f64), (5.0_f64, 1.5_f64)] {
            let shapes: [&dyn Fn(usize) -> f64; 3] = [
                &|step| 1.0_f64 / (1.0_f64 + f64::from(u32::try_from(step).expect("small"))),
                &|step| f64::from(u32::try_from(step).expect("small")),
                &|step| {
                    let mut draws = Noise(u32::try_from(step).expect("small"));
                    draws.next().abs()
                },
            ];
            for (shape, need) in shapes.iter().enumerate() {
                let mut limiter = TruePeakLimiter::<Binary64>::new(
                    rate,
                    1,
                    value(maximum),
                    super::Detector::Standard,
                    settings(-1.0, lookahead, 100.0, 0.5),
                    LimiterMode::Protected,
                )
                .expect("a limiter");
                let span = frames_at(lookahead, rate) + 2;
                let needs: Vec<f64> = (0..4 * span).map(need).collect();
                for (step, &value_now) in needs.iter().enumerate() {
                    limiter.path.enqueue(
                        u64::try_from(step).expect("small"),
                        Binary64::new(value_now),
                    );
                    let window = &needs[step.saturating_sub(span - 1)..=step];
                    let expected = window.iter().copied().fold(0.0_f64, larger);
                    assert_eq!(
                        limiter.path.held().raw(),
                        expected,
                        "{} Hz, {lookahead} of {maximum} ms, shape {shape}, step {step}",
                        rate.hz()
                    );
                }
            }
        }
    }
}

/// The ceiling held, in `S`, at every rate, while the lookahead changes live: each new one
/// set once the change before it is over, and while one runs, rising toward a longer
/// window or with the old path frozen beside a shorter one (a set then waits for it). On
/// two channels of noise limited near the ceiling, every segment between two settings is
/// at or below it, with no slack, and above half of it, so that a stage silenced by a
/// change fails.
fn holds_the_ceiling_when_its_lookahead_changes_live<S: Sample + Raw>() {
    let mut draws = Noise::new();
    let ceiling = CEILING;
    for rate in rates() {
        let mut limiter = TruePeakLimiter::<S>::new(
            rate,
            2,
            value(5.0_f64),
            super::Detector::Standard,
            settings(-1.0_f64, 5.0_f64, 100.0_f64, 0.5_f64),
            LimiterMode::Protected,
        )
        .expect("a limiter");
        let latency = limiter.latency();
        assert_eq!(
            latency,
            latency_of(5.0_f64, rate, super::Detector::Standard),
            "counted apart"
        );
        let longest = frames_at(5.0_f64, rate);
        // Each lookahead and the frames run after it is set: past a whole change and four
        // windows; one block (within the change it starts, or the one running); and a window
        // and a half of the longest (within a growth or the shrink after it).
        let settled = latency.saturating_add(longest.saturating_mul(4));
        let schedule = [
            (5.0_f64, settled),
            (0.5_f64, BLOCK_SIZE),
            (3.0_f64, BLOCK_SIZE),
            (1.5_f64, settled),
            (5.0_f64, longest.saturating_add(longest.div_euclid(2))),
            (0.5_f64, settled),
            (1.5_f64, BLOCK_SIZE),
            (5.0_f64, settled),
        ];
        let mut skip = latency;
        for (lookahead, frames) in schedule {
            limiter
                .set(settings(-1.0_f64, lookahead, 100.0_f64, 0.5_f64))
                .expect("within the maximum");
            let input = signal::<S>(frames.div_ceil(BLOCK_SIZE), 2, |_, _| {
                drawn(&mut draws, 4.0_f64)
            });
            let output = limited(&mut limiter, &input);
            let peak = largest(&output, skip, 2);
            assert!(
                peak <= ceiling,
                "{peak} at {} Hz, {lookahead} ms",
                rate.hz()
            );
            assert!(
                peak > ceiling * 0.5_f64,
                "{peak} at {} Hz, {lookahead} ms: limited, not silenced",
                rate.hz()
            );
            // A copy, then a constant above every need of the noise: its exact gain, whether
            // a change still runs or not.
            settles_at_its_exact_gain(
                &mut limiter.clone(),
                2,
                (rate, super::Detector::Standard, 5.0_f64),
                ceiling,
                16.0_f64,
                &format!("{} Hz, {lookahead} ms", rate.hz()),
            );
            skip = 0;
        }
    }
}

#[test]
fn holds_the_ceiling_when_its_lookahead_changes_live_in_each_precision() {
    holds_the_ceiling_when_its_lookahead_changes_live::<Binary64>();
    holds_the_ceiling_when_its_lookahead_changes_live::<Binary32>();
}

/// The most frames a descent from a reduction by its window's attack, `hp × reduction` per
/// frame, lasts: `⌈1 / hp⌉` steps take any reduction to zero, and one more frame absorbs the
/// rounding of their differences, a few units of 2^-53 of it, far below a step.
fn descent_frames(window: usize) -> usize {
    let peak = hann_peak(window);
    let steps = (1_u32..=u32::MAX)
        .find(|steps| f64::from(*steps) * peak >= 1.0_f64)
        .expect("a count");
    usize::try_from(steps).expect("small").saturating_add(1)
}

/// A limiter of one channel at `rate`, a maximum lookahead of 5 ms and the lookahead
/// `from`, with the default ceiling, release and adaptivity: fresh, or `used`, after a change
/// to another lookahead (0.5 ms, or 5 ms from 2.5 ms and below) and back, each run to its end
/// on loud noise from `draws`, so that both its paths hold releases of an earlier life (a
/// fresh limiter's second path has a ring of zeros). Each change ends within its rebuild,
/// 16 maximum windows of rise (the warm-start tests' own limit), the old window and one
/// frame frozen, and a descent ([`descent_frames`]), counted apart.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "frame counts of a few thousand, far from overflow"
)]
fn limiter_at(
    rate: SampleRate,
    from: f64,
    used: bool,
    draws: &mut Noise,
) -> TruePeakLimiter<Binary64> {
    let mut limiter = TruePeakLimiter::<Binary64>::new(
        rate,
        1,
        value(5.0_f64),
        super::Detector::Standard,
        settings(-1.0_f64, from, 100.0_f64, 0.5_f64),
        LimiterMode::Protected,
    )
    .expect("a limiter");
    if used {
        let maximum = frames_at(5.0_f64, rate);
        let cap = rebuild_frames_of(maximum) + 17 * maximum + 1 + descent_frames(maximum);
        let other = if from > 2.5_f64 { 0.5_f64 } else { 5.0_f64 };
        for lookahead in [other, from] {
            limiter
                .set(settings(-1.0_f64, lookahead, 100.0_f64, 0.5_f64))
                .expect("within the maximum");
            let mut frames = 0;
            while limiter.change.is_some() {
                let _: f64 = gain_of(&mut limiter, 4.0_f64 * draws.next().abs());
                frames += 1;
                assert!(frames <= cap, "still changing to {lookahead} ms");
            }
        }
    }
    limiter
}

/// Runs `limiter`, its lookahead just set, and `unchanged`, a copy of it from before, through
/// the rebuild, on the peaks `peak` gives: the change set, the rebuild's frames, as many as
/// [`rebuild_frames_of`] counts at the `maximum` given, each giving the gain of the copy, bit
/// for bit, with the old path in force alone; then the change started.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "frame counts of a few thousand, far from overflow"
)]
fn rebuilt_beside(
    limiter: &mut TruePeakLimiter<Binary64>,
    unchanged: &mut TruePeakLimiter<Binary64>,
    maximum: usize,
    mut peak: impl FnMut() -> f64,
    context: &str,
) {
    let rebuild = rebuild_frames_of(maximum);
    assert_eq!(
        limiter.rebuild_frames, rebuild,
        "{context}: the rebuild's frames"
    );
    let old = unchanged.path.lookahead;
    for frame in 0..rebuild {
        assert_eq!(
            limiter.change,
            Some(super::Stage::Rebuilding(rebuild - frame)),
            "{context}, frame {frame} of the rebuild"
        );
        assert_eq!(
            limiter.path.lookahead, old,
            "{context}: the old path in force"
        );
        let at = peak();
        assert_eq!(
            gain_of(limiter, at).to_bits(),
            gain_of(unchanged, at).to_bits(),
            "{context}, frame {frame} of the rebuild: the old path's gain"
        );
    }
    assert!(
        !matches!(limiter.change, Some(super::Stage::Rebuilding(_))),
        "{context}: started"
    );
}

#[test]
fn rebuilds_a_new_path_over_frames_exactly_as_at_once_on_its_last_frame() {
    // At every rate, growing and shrinking, on loud noise whose needs differ from frame to
    // frame (bursts arriving while the new path is rebuilt): a limiter whose new path is
    // rebuilt over its frames, and a copy
    // left unchanged through them, then rebuilt at once on the last of them (its rebuild of
    // no frame), give the same gain on every frame, bit for bit, the old path's through the
    // rebuild, and stand at the same stage of the change after it, through the whole change
    // and after it (four maximum windows and the longest change of these, the test's own
    // run).
    let mut draws = Noise::new();
    for rate in rates() {
        let maximum = frames_at(5.0_f64, rate);
        let rebuild = rebuild_frames_of(maximum);
        for used in [false, true] {
            for (from, to) in [
                (0.5_f64, 5.0_f64),
                (5.0_f64, 0.5_f64),
                (1.5_f64, 3.0_f64),
                (3.0_f64, 1.5_f64),
                (4.9_f64, 5.0_f64),
                (5.0_f64, 4.9_f64),
            ] {
                let context = format!("{} Hz, {from} to {to} ms, used {used}", rate.hz());
                let mut spread = limiter_at(rate, from, used, &mut draws);
                for _ in 0..2 * maximum {
                    let _: f64 = gain_of(&mut spread, 4.0_f64 * draws.next().abs());
                }
                let mut once = spread.clone();
                spread
                    .set(settings(-1.0_f64, to, 100.0_f64, 0.5_f64))
                    .expect("within the maximum");
                assert_eq!(
                    spread.rebuild_frames, rebuild,
                    "{context}: the rebuild's frames"
                );
                for frame in 0..rebuild {
                    let peak = 4.0_f64 * draws.next().abs();
                    assert_eq!(
                        gain_of(&mut spread, peak).to_bits(),
                        gain_of(&mut once, peak).to_bits(),
                        "{context}, frame {frame} of the rebuild"
                    );
                }
                once.rebuild_frames = 0;
                once.set(settings(-1.0_f64, to, 100.0_f64, 0.5_f64))
                    .expect("within the maximum");
                assert_eq!(spread.change, once.change, "{context}: the change started");
                assert!(
                    !matches!(spread.change, Some(super::Stage::Rebuilding(_))),
                    "{context}: started"
                );
                for frame in 0..8 * maximum {
                    let peak = 4.0_f64 * draws.next().abs();
                    assert_eq!(
                        gain_of(&mut spread, peak).to_bits(),
                        gain_of(&mut once, peak).to_bits(),
                        "{context}, frame {frame} of the change"
                    );
                    assert_eq!(spread.change, once.change, "{context}, frame {frame}");
                }
                assert_eq!(spread.change, None, "{context}: over");
            }
        }
    }
}

#[test]
fn rises_from_a_cap_of_the_largest_reduction_its_window_holds_or_the_old_stages() {
    // At every rate, growing while the release of an old burst decays (the burst's needs out
    // of both windows, the old path's released reductions falling frame by frame): the cap
    // the change rises with is the largest of the old path's release stages and of the
    // reductions the new window holds when it starts, its `L - 1` rebuilt ones, no more
    // (`Stage::Rising`); a reduction of an earlier frame, larger while the release decays,
    // would raise it.
    let mut draws = Noise::new();
    for rate in rates() {
        let maximum = frames_at(5.0_f64, rate);
        for used in [false, true] {
            for (from, to) in [(0.5_f64, 5.0_f64), (1.5_f64, 3.0_f64), (4.9_f64, 5.0_f64)] {
                let new = frames_at(to, rate);
                let context = format!("{} Hz, {from} to {to} ms, used {used}", rate.hz());
                let mut limiter = limiter_at(rate, from, used, &mut draws);
                for frame in 0..4 * maximum + 64 {
                    let peak = if frame < 10 { 2.0_f64 } else { 0.5_f64 };
                    let _: f64 = gain_of(&mut limiter, peak);
                }
                limiter
                    .set(settings(-1.0_f64, to, 100.0_f64, 0.5_f64))
                    .expect("within the maximum");
                for _ in 0..rebuild_frames_of(maximum) {
                    let _: f64 = gain_of(&mut limiter, 0.5_f64);
                }
                let Some(super::Stage::Rising { cap, .. }) = limiter.change else {
                    panic!("{context}: rising: {:?}", limiter.change);
                };
                let held = (0..new - 1).fold(
                    larger(limiter.previous.fast.raw(), limiter.previous.slow.raw()),
                    |largest, back| larger(largest, limiter.path.released_before(back).raw()),
                );
                assert!(held > 0.0_f64, "{context}: a release still decaying");
                assert_eq!(cap.raw().to_bits(), held.to_bits(), "{context}: the cap");
            }
        }
    }
}

/// Checks that the Hann sums of `path` are those of its window, computed apart: the plain
/// sum of its last `L` released reductions and their sums times the cosine and the sine of
/// `2π (k + 1) / (L + 1)` at each position `k` (libm), within the bounds of
/// `spec/measurements.md`, "The true-peak limiter", step 4, for sums exact at most `L - 1`
/// frames before: `(L(L - 1) + 2(L - 1)(L + 1))uV` for the plain sum and
/// `(L(L + 20) + (L - 1)(38L + 37))uV(1 + 29u)^L` for its turning parts, below
/// `3L²uV` and `40L²uV` (`L ≥ 1`, `(1 + 29u)^L` below `1 + 2^-40` up to 1,920 frames),
/// `V` the window's largest reduction. While a shrink scales its rebuilt reductions, one per
/// frame as each leaves the window, `scaling` of them still to scale by `by` (whose sums it
/// scaled already), the window's values at those positions (the `scaling` below the last)
/// are taken scaled.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "window lengths of at most 1,920 frames and reductions below two, far from overflow"
)]
fn sums_of_its_window(path: &super::Path, (scaling, by): (usize, f64), context: &str) {
    let length = path.lookahead;
    let span = f64::from(u32::try_from(length + 1).expect("small"));
    let window: Vec<f64> = (0..length)
        .map(|back| {
            let value = path.released_before(back).raw();
            if back + 1 < length && back + 1 + scaling >= length {
                value * by
            } else {
                value
            }
        })
        .collect();
    let largest = window.iter().copied().fold(0.0_f64, larger);
    // Its count of non-zero reductions, which decides whether its reduction is zero.
    assert_eq!(
        path.nonzero,
        window.iter().filter(|value| **value != 0.0_f64).count(),
        "{context}: the count of non-zero reductions"
    );
    let angle = |position: usize| {
        core::f64::consts::TAU * f64::from(u32::try_from(position + 1).expect("small")) / span
    };
    let exact = (
        window.iter().sum::<f64>(),
        window
            .iter()
            .enumerate()
            .map(|(at, value)| value * libm::cos(angle(at)))
            .sum::<f64>(),
        window
            .iter()
            .enumerate()
            .map(|(at, value)| value * libm::sin(angle(at)))
            .sum::<f64>(),
    );
    let squared = f64::from(u32::try_from(length * length).expect("small"));
    let unit = libm::exp2(-53.0_f64) * largest * squared;
    for (got, wanted, bound) in [
        (path.sums.sum.raw(), exact.0, 3.0_f64 * unit),
        (path.sums.real.raw(), exact.1, 40.0_f64 * unit),
        (path.sums.imaginary.raw(), exact.2, 40.0_f64 * unit),
    ] {
        assert!(
            (got - wanted).abs() <= bound,
            "{context}: {got} for {wanted}, {bound} allowed"
        );
    }
}

#[test]
fn keeps_the_hann_sums_of_each_window_through_a_change() {
    // At every rate, growing and shrinking, on loud noise: on every frame of the rebuild, of
    // the change and of eight maximum windows after it, the sums of the path in force are
    // those of its window (`sums_of_its_window`), and so are the old path's while it runs
    // beside it (rising or frozen), but on the rebuild's last frame, whose new path's sums
    // replace its others on the next; a rebuilt reduction scaled when it should not be, or
    // not when it should, a reduction added at another position, puts them apart.
    let mut draws = Noise::new();
    for rate in rates() {
        let maximum = frames_at(5.0_f64, rate);
        for used in [false, true] {
            for (from, to) in [(0.5_f64, 5.0_f64), (5.0_f64, 0.5_f64), (5.0_f64, 4.9_f64)] {
                let context = format!("{} Hz, {from} to {to} ms, used {used}", rate.hz());
                let mut limiter = limiter_at(rate, from, used, &mut draws);
                for _ in 0..2 * maximum {
                    let _: f64 = gain_of(&mut limiter, 4.0_f64 * draws.next().abs());
                }
                limiter
                    .set(settings(-1.0_f64, to, 100.0_f64, 0.5_f64))
                    .expect("within the maximum");
                for frame in 0..10 * maximum {
                    // A release after a third of the run, so that a shrink scales its history.
                    let peak = if frame < 3 * maximum {
                        4.0_f64 * draws.next().abs()
                    } else {
                        0.5_f64
                    };
                    let rebuilding = matches!(limiter.change, Some(super::Stage::Rebuilding(_)));
                    let _: f64 = gain_of(&mut limiter, peak);
                    let here = format!("{context}, frame {frame}");
                    // On the rebuild's last frame the new path's sums are those it accumulated,
                    // replacing its others on the next frame, its first in force.
                    let started =
                        rebuilding && !matches!(limiter.change, Some(super::Stage::Rebuilding(_)));
                    let pending = match limiter.change {
                        Some(super::Stage::Frozen { scaling, by, .. }) => (scaling, by.raw()),
                        _ => (0, 1.0_f64),
                    };
                    if !started {
                        sums_of_its_window(&limiter.path, pending, &here);
                    }
                    if matches!(
                        limiter.change,
                        Some(super::Stage::Rising { .. } | super::Stage::Frozen { .. })
                    ) {
                        sums_of_its_window(
                            &limiter.previous,
                            (0, 1.0_f64),
                            &format!("{here}, the old path"),
                        );
                    }
                }
                assert_eq!(limiter.change, None, "{context}: over");
            }
        }
    }
}

#[test]
fn keeps_the_old_attack_for_a_burst_only_the_old_window_saw_when_shrinking() {
    // At every rate, shrinking from 5 ms to 0.5 and 1.5 ms while a burst (10 frames of
    // peaks of 2.0 over peaks of 0.5) lies within the old window but beyond the new one when
    // the change starts: the old path, frozen, has seen it, the new one carries nothing from
    // beyond its own window, and its attack of the burst, as long as the old one's but
    // steeper, ends on the same frame and stays under it; so every step of the gain, through
    // the rebuild, the change and the burst, is at most the old window's attack of the
    // burst's need, `need × hp(old)`, plus the rounding (`2ρ + 8 × 2^-53`), all computed
    // apart. A new path whose fast stage took the old path's released reduction, which holds
    // needs beyond the new window, attacks the burst early, at the new window's rate.
    let ceiling = CEILING;
    for rate in rates() {
        let maximum = frames_at(5.0_f64, rate);
        let rebuild = rebuild_frames_of(maximum);
        let need = need_above(2.0_f64, ceiling, margin_of(maximum));
        let allowed = need * hann_peak(maximum)
            + 2.0_f64 * rounding_of(maximum)
            + 8.0_f64 * libm::exp2(-53.0_f64);
        for to in [0.5_f64, 1.5_f64] {
            let new = frames_at(to, rate);
            let context = format!("{} Hz, 5 to {to} ms", rate.hz());
            // The change's first frame `t` is the set frame plus the rebuild and one; at
            // `t - 1` the old window holds the needs up to `t - 1 - maximum + maximum`, the
            // new one up to `t - 1 - maximum + new`: the burst's 10 needs from `onset` lie
            // between, for every `t` in the range below.
            let onset = 4 * maximum + 128;
            let (low, high) = (onset + 9 + 1, onset + maximum - new);
            assert!(low <= high, "{context}: a range of changes");
            let peak = |frame: usize| {
                if (onset..onset + 10).contains(&frame) {
                    2.0_f64
                } else {
                    0.5_f64
                }
            };
            let mut limiter = TruePeakLimiter::<Binary64>::new(
                rate,
                1,
                value(5.0_f64),
                super::Detector::Standard,
                settings(-1.0_f64, 5.0_f64, 100.0_f64, 0.5_f64),
                LimiterMode::Protected,
            )
            .expect("a limiter");
            let stride = (high - low).div_ceil(8).max(1);
            let mut gains = Vec::new();
            for frame in 0..high - rebuild {
                gains.push(gain_of(&mut limiter, peak(frame)));
                // `t = frame + 1 + rebuild`, from `low` to `high`.
                let Some(first) = (frame + 1 + rebuild).checked_sub(low) else {
                    continue;
                };
                if first % stride != 0 {
                    continue;
                }
                let mut changed = limiter.clone();
                changed
                    .set(settings(-1.0_f64, to, 100.0_f64, 0.5_f64))
                    .expect("within the maximum");
                let mut run = vec![gains[frame]];
                for at in frame + 1..onset + 2 * maximum + 64 {
                    run.push(gain_of(&mut changed, peak(at)));
                }
                steps_within(
                    &run,
                    allowed,
                    &format!("{context}, set after frame {frame}"),
                );
                assert!(
                    run.iter().any(|gain| *gain < 1.0_f64 - need / 2.0_f64),
                    "{context}, set after frame {frame}: the burst limited"
                );
            }
        }
    }
}

#[test]
fn rebuilds_a_new_path_by_thirty_two_steps_of_each_part_per_frame() {
    // At every rate, growing and shrinking at the extremes and near-equal, the rebuild's
    // work per frame, counted apart: `rebuild_frames_of` frames (`⌈(2M + 1) / 32⌉`); on
    // each frame `i` but the last, at most 32 more of each part, never one later than the
    // frame computed (`left = R - i` frames before the rebuild's last):
    // - turns of the new path's phasor, up to its window;
    // - frames of its hold, from `run + L + 1` frames before the rebuild's last frame to it
    //   (`run`: `L - 1` growing, `old - L` shrinking), those at least `left` frames before;
    // - shrinking, rebuilt reductions, one per frame of the window's last `L - 1`, the
    //   oldest first, those at least `left` frames before the last (growing, none: its
    //   rebuilt reductions come with its hold, its budget on the last frame);
    // the last frame does what is left, at most 32 of each, then starts the change.
    // The rebuild's frames at each rate, at a maximum of 5 ms, as `spec/limitations.md`
    // states them.
    let stated = [
        (8_000, 3),
        (11_025, 4),
        (16_000, 6),
        (22_050, 7),
        (44_100, 14),
        (48_000, 16),
        (96_000, 31),
        (384_000, 121),
    ];
    assert_eq!(
        rates().iter().map(|rate| rate.hz()).collect::<Vec<_>>(),
        stated.map(|(hz, _)| hz),
        "the matrix's rates"
    );
    for (rate, (_, frames)) in rates().into_iter().zip(stated) {
        let maximum = frames_at(5.0_f64, rate);
        let rebuild = rebuild_frames_of(maximum);
        assert_eq!(rebuild, frames, "{} Hz: as stated", rate.hz());
        for (from, to) in [
            (0.5_f64, 5.0_f64),
            (5.0_f64, 0.5_f64),
            (4.9_f64, 5.0_f64),
            (5.0_f64, 4.9_f64),
        ] {
            let (old, new) = (frames_at(from, rate), frames_at(to, rate));
            let context = format!("{} Hz, {from} to {to} ms", rate.hz());
            let growing = new > old;
            let run = if growing { new - 1 } else { old - new };
            let holds = run + new + 2;
            // The terms available `left` frames before the rebuild's last frame.
            let terms = |left: usize| {
                if growing {
                    0
                } else {
                    (new - 1).saturating_sub(left)
                }
            };
            assert!(
                holds <= 32 * rebuild,
                "{context}: within the rebuild's frames"
            );
            let mut limiter = TruePeakLimiter::<Binary64>::new(
                rate,
                1,
                value(5.0_f64),
                super::Detector::Standard,
                settings(-1.0_f64, from, 100.0_f64, 0.5_f64),
                LimiterMode::Protected,
            )
            .expect("a limiter");
            for _ in 0..2 * maximum {
                let _: f64 = gain_of(&mut limiter, 2.0_f64);
            }
            limiter
                .set(settings(-1.0_f64, to, 100.0_f64, 0.5_f64))
                .expect("within the maximum");
            assert_eq!(
                limiter.rebuild_frames, rebuild,
                "{context}: the rebuild's frames"
            );
            let done = |rebuilt: &TruePeakLimiter<Binary64>| {
                (
                    rebuilt.rebuild.turns,
                    holds - rebuilt.rebuild.unheld,
                    rebuilt.rebuild.terms,
                )
            };
            assert_eq!(done(&limiter), (0, 0, 0), "{context}: none done");
            let (mut turned, mut held, mut taken) = (0, 0, 0);
            for frame in 1..rebuild {
                let _: f64 = gain_of(&mut limiter, 2.0_f64);
                let left = rebuild - frame;
                turned = (turned + 32).min(new);
                held = (held + 32).min(holds.saturating_sub(left));
                taken = (taken + 32).min(terms(left));
                assert_eq!(
                    done(&limiter),
                    (turned, held, taken),
                    "{context}, frame {frame} of the rebuild"
                );
            }
            assert!(
                new - turned <= 32 && holds - held <= 32 && terms(0) - taken <= 32,
                "{context}: at most 32 of each on the last frame"
            );
            let _: f64 = gain_of(&mut limiter, 2.0_f64);
            assert!(
                !matches!(limiter.change, Some(super::Stage::Rebuilding(_))),
                "{context}: started"
            );
            assert_eq!(limiter.path.lookahead, new, "{context}: the new window");
        }
    }
}

#[test]
fn runs_a_shorter_lookahead_beside_the_old_path_frozen_for_the_old_window_then_descends() {
    // At every rate, shrinking on loud noise whose needs differ from frame to frame: after
    // the rebuild (`rebuilt_beside`), the old path is frozen for the old window and one
    // frame (it ramps the needs it saw, which leave it by then), counted down frame by
    // frame, then the descent lasts at most `descent_frames` of the old window before the
    // new path is in force alone.
    let mut draws = Noise::new();
    for rate in rates() {
        for used in [false, true] {
            for (from, to) in [(5.0_f64, 0.5_f64), (1.5_f64, 0.5_f64), (5.0_f64, 4.9_f64)] {
                let (old, new) = (frames_at(from, rate), frames_at(to, rate));
                let context = format!("{} Hz, {from} to {to} ms, used {used}", rate.hz());
                let mut limiter = limiter_at(rate, from, used, &mut draws);
                let maximum = frames_at(5.0_f64, rate);
                for _ in 0..2 * maximum {
                    let _: f64 = gain_of(&mut limiter, 4.0_f64 * draws.next().abs());
                }
                let mut unchanged = limiter.clone();
                limiter
                    .set(settings(-1.0_f64, to, 100.0_f64, 0.5_f64))
                    .expect("within the maximum");
                rebuilt_beside(
                    &mut limiter,
                    &mut unchanged,
                    maximum,
                    || 4.0_f64 * draws.next().abs(),
                    &context,
                );
                assert_eq!(limiter.path.lookahead, new, "{context}: the new window");
                assert_eq!(limiter.previous.lookahead, old, "{context}: the old one");
                // Frozen for the old window, the new window's `new - 1` rebuilt reductions scaled
                // one per frame by one share, at most one.
                let Some(super::Stage::Frozen { left, scaling, by }) = limiter.change else {
                    panic!("{context}: frozen: {:?}", limiter.change);
                };
                assert_eq!((left, scaling), (old, new - 1), "{context}: frozen");
                assert!(
                    by.raw() > 0.0_f64 && by.raw() <= 1.0_f64,
                    "{context}: a share {by:?}"
                );
                for frame in 1..=old {
                    let _: f64 = gain_of(&mut limiter, 4.0_f64 * draws.next().abs());
                    assert_eq!(
                        limiter.change,
                        Some(super::Stage::Frozen {
                            left: old - frame,
                            scaling: (new - 1).saturating_sub(frame),
                            by
                        }),
                        "{context}, frame {frame}"
                    );
                }
                let mut descent = 0;
                loop {
                    let _: f64 = gain_of(&mut limiter, 4.0_f64 * draws.next().abs());
                    match limiter.change {
                        Some(super::Stage::Descending { .. }) => descent += 1,
                        None => break,
                        stage => panic!("{context}: {stage:?} after the old window"),
                    }
                    assert!(
                        descent <= descent_frames(old),
                        "{context}: still descending after {descent} frames"
                    );
                }
                assert_eq!(limiter.path.lookahead, new, "{context}: in force");
            }
        }
    }
}

#[test]
fn rises_to_a_longer_lookahead_never_below_the_old_path_then_descends() {
    // At every rate, growing on loud noise: after the rebuild (`rebuilt_beside`), while the
    // change rises, every reduction is at
    // least the old path's, as a copy left unchanged gives it, so no gain is above that
    // copy's (the larger of the two is exact, the gain's rounding monotonic); then the
    // descent lasts at most `descent_frames` of the new window before the new path is in
    // force alone. The rise ends on noise within 16 maximum windows (the test's own limit).
    let mut draws = Noise::new();
    for rate in rates() {
        for used in [false, true] {
            for (from, to) in [(0.5_f64, 5.0_f64), (1.5_f64, 3.0_f64), (4.9_f64, 5.0_f64)] {
                let (old, new) = (frames_at(from, rate), frames_at(to, rate));
                let maximum = frames_at(5.0_f64, rate);
                let context = format!("{} Hz, {from} to {to} ms, used {used}", rate.hz());
                let mut limiter = limiter_at(rate, from, used, &mut draws);
                for _ in 0..2 * maximum {
                    let _: f64 = gain_of(&mut limiter, 4.0_f64 * draws.next().abs());
                }
                // The needs of the last `2 × maximum + 2` frames kept, and the largest Hann weight
                // of every window up to the maximum.
                assert_eq!(
                    limiter.needed.len(),
                    2 * maximum + 2,
                    "{context}: the needs kept"
                );
                assert_eq!(limiter.peaks.len(), maximum, "{context}: the weights");
                // Each within the rounding of both computations, `48 × 2^-53 / (L + 1)`
                // (`computes_the_largest_hann_weight_of_every_window_up_to_the_longest`).
                for length in [1, old, new, maximum] {
                    let span = f64::from(u32::try_from(length + 1).expect("small"));
                    assert!(
                        (limiter.peaks[length - 1].raw() - hann_peak(length)).abs()
                            <= 48.0_f64 * libm::exp2(-53.0_f64) / span,
                        "{context}: the weight of {length} frames"
                    );
                }
                let mut unchanged = limiter.clone();
                limiter
                    .set(settings(-1.0_f64, to, 100.0_f64, 0.5_f64))
                    .expect("within the maximum");
                rebuilt_beside(
                    &mut limiter,
                    &mut unchanged,
                    maximum,
                    || 4.0_f64 * draws.next().abs(),
                    &context,
                );
                assert_eq!(limiter.path.lookahead, new, "{context}: the new window");
                assert_eq!(limiter.previous.lookahead, old, "{context}: the old one");
                let mut rising = 0;
                while matches!(limiter.change, Some(super::Stage::Rising { .. })) {
                    let peak = 4.0_f64 * draws.next().abs();
                    let (gain, kept) = (gain_of(&mut limiter, peak), gain_of(&mut unchanged, peak));
                    assert!(
                        gain <= kept,
                        "{context}, frame {rising}: a gain of {gain} above the old path's {kept}"
                    );
                    rising += 1;
                    assert!(rising <= 16 * maximum, "{context}: still rising");
                }
                assert!(rising >= 1, "{context}: rose");
                let mut descent = 0;
                while limiter.change.is_some() {
                    assert!(
                        matches!(limiter.change, Some(super::Stage::Descending { .. })),
                        "{context}: {:?} after the rise",
                        limiter.change
                    );
                    let _: f64 = gain_of(&mut limiter, 4.0_f64 * draws.next().abs());
                    descent += 1;
                    assert!(
                        descent <= descent_frames(new),
                        "{context}: still descending after {descent} frames"
                    );
                }
                assert_eq!(limiter.path.lookahead, new, "{context}: in force");
            }
        }
    }
}

#[test]
fn rises_by_each_unseen_need_times_the_hann_weight_of_its_depth() {
    // At every rate, at maxima of 5 and 1 ms, growing to the maximum and below it: a single
    // need `N` (a peak of 2.0 over silence) `D` frames before the first frame that needs it,
    // counted from the change's first frame, gives the rise a budget of `N × hp(D)`
    // (`spec/measurements.md`, "The true-peak limiter", step 6), `hp(D)` the largest Hann
    // weight of `D` frames computed apart, for `D` from the old window to the new one (below
    // the maximum: a need of the frame after enters the new path's hold, whose attack ramps
    // it); none for a need the old path holds already (`D` one below the old window) nor one
    // farther than the new window. Within the roundings of both: the need within `8u` of
    // `1 - c/p + E` (three roundings each way, step 3, and the ceiling's), each weight
    // within `48u / (D + 1)` of the exact one
    // (`computes_the_largest_hann_weight_of_every_window_up_to_the_longest`), and each
    // product one rounding, far below a weight's change from one `D` to the next.
    for rate in rates() {
        for (most, from, to) in [
            (5.0_f64, 1.5_f64, 5.0_f64),
            (5.0_f64, 1.5_f64, 3.0_f64),
            (1.0_f64, 0.5_f64, 1.0_f64),
            (1.0_f64, 0.5_f64, 0.75_f64),
        ] {
            let maximum = frames_at(most, rate);
            let (old, new) = (frames_at(from, rate), frames_at(to, rate));
            let farthest = new.min(maximum - 1);
            let rebuild = rebuild_frames_of(maximum);
            let middle = usize::midpoint(old, farthest) | 1;
            let mut depths = vec![old - 1, old, old + 1, middle, middle + 1, farthest];
            if farthest + 1 < maximum {
                depths.push(farthest + 1);
            }
            for depth in depths {
                let context = format!("{} Hz, {from} to {to} of {most} ms, D {depth}", rate.hz());
                let mut limiter = TruePeakLimiter::<Binary64>::new(
                    rate,
                    1,
                    value(most),
                    super::Detector::Standard,
                    settings(-1.0_f64, from, 100.0_f64, 0.5_f64),
                    LimiterMode::Protected,
                )
                .expect("a limiter");
                // Set after `2 × maximum` frames, the rebuild's last frame `c` the `rebuild`-th
                // after them, the need of the frame `c - (maximum - 1 - D)`.
                let last = 2 * maximum + rebuild - 1;
                let burst = last - (maximum - 1 - depth);
                for frame in 0..=last {
                    if frame == 2 * maximum {
                        limiter
                            .set(settings(-1.0_f64, to, 100.0_f64, 0.5_f64))
                            .expect("within the maximum");
                    }
                    let _: f64 =
                        gain_of(&mut limiter, if frame == burst { 2.0_f64 } else { 0.5_f64 });
                }
                let Some(super::Stage::Rising { budget, .. }) = limiter.change else {
                    panic!("{context}: rising: {:?}", limiter.change);
                };
                if (old..=farthest).contains(&depth) {
                    let need = need_above(2.0_f64, CEILING, margin_of(maximum));
                    let weight = hann_peak(depth);
                    let span = f64::from(u32::try_from(depth + 1).expect("small"));
                    let within = (8.0_f64 * weight + 48.0_f64 * need / span + 2.0_f64)
                        * libm::exp2(-53.0_f64);
                    assert!(
                        (budget.raw() - need * weight).abs() <= within,
                        "{context}: a budget of {} for {}, within {within}",
                        budget.raw(),
                        need * weight
                    );
                } else {
                    assert_eq!(budget.raw().to_bits(), 0, "{context}: no budget");
                }
            }
        }
    }
}

#[test]
fn ends_a_change_in_silence_as_soon_as_each_stage_allows() {
    // At every rate, in silence (no need, every reduction zero): after the rebuild's frames
    // (`rebuild_frames_of`), growing, the rise reaches the new path on its first frame (both
    // zero) and the descent from zero ends on the next, so the change is over two frames
    // later; shrinking, the old path stays frozen for its window and one frame, then the
    // same descent: over the old window and two frames later. Each time the gain is none,
    // the input passed through.
    for rate in rates() {
        for (from, to) in [(0.5_f64, 5.0_f64), (5.0_f64, 0.5_f64), (1.5_f64, 3.0_f64)] {
            let old = frames_at(from, rate);
            let rebuild = rebuild_frames_of(frames_at(5.0_f64, rate));
            let frames = rebuild + if to > from { 2 } else { old + 2 };
            let context = format!("{} Hz, {from} to {to} ms", rate.hz());
            let mut limiter = TruePeakLimiter::<Binary64>::new(
                rate,
                1,
                value(5.0_f64),
                super::Detector::Standard,
                settings(-1.0_f64, from, 100.0_f64, 0.5_f64),
                LimiterMode::Protected,
            )
            .expect("a limiter");
            for _ in 0_u32..64 {
                assert_eq!(gain_of(&mut limiter, 0.5_f64), 1.0_f64, "{context}: none");
            }
            limiter
                .set(settings(-1.0_f64, to, 100.0_f64, 0.5_f64))
                .expect("within the maximum");
            assert_eq!(
                limiter.rebuild_frames, rebuild,
                "{context}: the rebuild's frames"
            );
            for frame in 1..=frames {
                assert_ne!(limiter.change, None, "{context}, frame {frame}: running");
                assert_eq!(
                    limiter.gain(value(0.5_f64)),
                    None,
                    "{context}, frame {frame}: no gain"
                );
            }
            assert_eq!(limiter.change, None, "{context}: over");
            assert_eq!(
                limiter.path.lookahead,
                frames_at(to, rate),
                "{context}: in force"
            );
        }
    }
}

/// Whether `value` is zero or normal: what a host that flushes subnormal numbers to zero
/// reads as the same number.
fn zero_or_normal(value: Binary64) -> bool {
    value.raw() == 0.0_f64 || value.raw().abs() >= f64::MIN_POSITIVE
}

/// Checks that every value the change of `limiter` stores (its stage's), and the release
/// stages and released reductions of both its paths, are zero or normal
/// ([`zero_or_normal`]): none that a host flushing subnormal numbers would read as zero,
/// stalling a stage or miscounting a window's non-zero reductions.
fn normal_state(limiter: &TruePeakLimiter<Binary64>, context: &str) {
    let staged: Vec<Binary64> = match limiter.change {
        Some(super::Stage::Rising {
            rising,
            cap,
            budget,
        }) => vec![rising, cap, budget],
        Some(super::Stage::Frozen { by, .. }) => vec![by],
        Some(super::Stage::Descending { level, fall }) => vec![level, fall],
        Some(super::Stage::Rebuilding(_)) | None => vec![],
    };
    for path in [&limiter.path, &limiter.previous] {
        for value in staged
            .iter()
            .chain([&path.fast, &path.slow])
            .chain(&path.released)
        {
            assert!(
                zero_or_normal(*value),
                "{context}: {value:?} stored, subnormal, at {:?}",
                limiter.change
            );
        }
    }
}

#[test]
fn keeps_every_value_of_a_change_zero_or_normal_in_a_release_tail() {
    // At every rate, growing and shrinking while the release stages hold `2^-1020` (the tail
    // of a long release, the slowest, so that it stays there through the change): each step
    // of a stage, the reduction times the largest Hann weight of a window of at least 4
    // frames (below a quarter), is below `2^-1022`, subnormal. Every value the change stores
    // must be zero or normal on every frame, as a host that flushes subnormal numbers to
    // zero reads them, the same on every host (`spec/engine-design.md`, decision 11), and the
    // change over within its bounds (`constant_change_frames`, a growth's rise reaching the
    // new path at once on such a reduction).
    for rate in rates() {
        let maximum = frames_at(5.0_f64, rate);
        for (from, to) in [(0.5_f64, 5.0_f64), (5.0_f64, 0.5_f64), (1.5_f64, 3.0_f64)] {
            let context = format!("{} Hz, {from} to {to} ms", rate.hz());
            let made = |lookahead: f64| settings(-1.0_f64, lookahead, 1_000.0_f64, 0.0_f64);
            let mut limiter = TruePeakLimiter::<Binary64>::new(
                rate,
                1,
                value(5.0_f64),
                super::Detector::Standard,
                made(from),
                LimiterMode::Protected,
            )
            .expect("a limiter");
            let tail = Binary64::new(libm::exp2(-1_020.0_f64));
            (limiter.path.fast, limiter.path.slow) = (tail, tail);
            for _ in 0..2 * maximum {
                let _: f64 = gain_of(&mut limiter, 0.5_f64);
            }
            assert!(
                limiter.path.smoothed().raw() >= f64::MIN_POSITIVE,
                "{context}: a reduction in force"
            );
            limiter.set(made(to)).expect("within the maximum");
            let (old, new) = (frames_at(from, rate), frames_at(to, rate));
            let cap = constant_change_frames(maximum, old, new);
            let mut frames = 0;
            while limiter.change.is_some() {
                let _: f64 = gain_of(&mut limiter, 0.5_f64);
                normal_state(&limiter, &format!("{context}, frame {frames}"));
                frames += 1;
                assert!(frames <= cap, "{context}: still changing");
            }
        }
    }
}

#[test]
fn counts_a_leaving_reduction_scaled_to_zero_as_zero() {
    // A path of 8 frames whose rebuilt reductions are `2^-1020` each, scaled by an eighth
    // as each is about to leave: the product, `2^-1023`, is subnormal, flushed to zero, and
    // the count of its window's non-zero reductions follows, one less each time.
    let mut path = super::Path::new(16);
    path.prepare(8, Binary64::new(hann_peak(8)));
    let _: usize = path.turn_phasor(0, 8);
    let tail = Binary64::new(libm::exp2(-1_020.0_f64));
    for position in (1..8).rev() {
        path.record(tail, position);
    }
    assert_eq!(path.nonzero, 7, "the rebuilt reductions counted");
    // As a frozen frame does: a frame of the path, then the reduction leaving on the next
    // scaled.
    for scaled in 1..=7 {
        let _: Binary64 = path.smooth(Binary64::ZERO);
        path.scale_leaving(Binary64::new(0.125_f64));
        assert_eq!(path.nonzero, 7 - scaled, "after {scaled} scaled");
    }
}

#[test]
fn takes_a_lookahead_set_during_a_change_once_it_is_over() {
    // At every rate, on a constant need (peaks of 2.0, settled): from 1.5 to 0.5 ms, then 3
    // and 5 ms set while that change runs, the first while its new path is rebuilt, the
    // second while it descends. The change to 0.5 ms runs to its end (its rebuild, the old
    // window and one frame frozen, then at most `descent_frames` of it), the last lookahead
    // asked for (5 ms) starts its rebuild on that frame and its change after it, the one in
    // between never in force. The old path settled on the constant need, the new one rebuilds its window at
    // that need too, so the rise, which starts from the reduction in force, reaches the new
    // path on its first frame (their roundings far below the rise's step, the new window's
    // attack of that need), and the descent from there falls below it on the next (by that
    // step again): the growth is over after two frames.
    for rate in rates() {
        let mut limiter = TruePeakLimiter::<Binary64>::new(
            rate,
            1,
            value(5.0_f64),
            super::Detector::Standard,
            settings(-1.0_f64, 1.5_f64, 100.0_f64, 0.5_f64),
            LimiterMode::Protected,
        )
        .expect("a limiter");
        let (short, middle, long) = (
            frames_at(0.5_f64, rate),
            frames_at(1.5_f64, rate),
            frames_at(5.0_f64, rate),
        );
        let context = format!("{} Hz", rate.hz());
        let set = |changed: &mut TruePeakLimiter<Binary64>, lookahead: f64| {
            changed
                .set(settings(-1.0_f64, lookahead, 100.0_f64, 0.5_f64))
                .expect("within the maximum");
        };
        for _ in 0..4 * long {
            let _: f64 = gain_of(&mut limiter, 2.0_f64);
        }
        let rebuild = rebuild_frames_of(long);
        assert_eq!(
            limiter.rebuild_frames, rebuild,
            "{context}: the rebuild's frames"
        );
        set(&mut limiter, 0.5_f64);
        for _ in 0..rebuild / 2 {
            let _: f64 = gain_of(&mut limiter, 2.0_f64);
        }
        assert!(
            matches!(limiter.change, Some(super::Stage::Rebuilding(_))),
            "{context}: rebuilding"
        );
        set(&mut limiter, 3.0_f64);
        for _ in rebuild / 2..rebuild {
            let _: f64 = gain_of(&mut limiter, 2.0_f64);
        }
        assert_eq!(
            limiter.path.lookahead, short,
            "{context}: the change running kept"
        );
        assert_eq!(
            limiter.settings().lookahead(),
            value(3.0_f64),
            "{context}: the setting asked for"
        );
        for _ in 0..=middle {
            assert!(
                matches!(limiter.change, Some(super::Stage::Frozen { .. })),
                "{context}: frozen"
            );
            let _: f64 = gain_of(&mut limiter, 2.0_f64);
        }
        assert!(
            matches!(limiter.change, Some(super::Stage::Descending { .. })),
            "{context}: descending: {:?}",
            limiter.change
        );
        set(&mut limiter, 5.0_f64);
        let mut descent = 0;
        while limiter.path.lookahead == short {
            let _: f64 = gain_of(&mut limiter, 2.0_f64);
            descent += 1;
            assert!(
                descent <= descent_frames(middle) + rebuild,
                "{context}: still toward 0.5 ms"
            );
        }
        assert_eq!(
            limiter.path.lookahead, long,
            "{context}: the last asked for, at once"
        );
        assert!(
            matches!(limiter.change, Some(super::Stage::Rising { .. })),
            "{context}: started: {:?}",
            limiter.change
        );
        let _: f64 = gain_of(&mut limiter, 2.0_f64);
        assert!(
            matches!(limiter.change, Some(super::Stage::Descending { .. })),
            "{context}: descending: {:?}",
            limiter.change
        );
        let _: f64 = gain_of(&mut limiter, 2.0_f64);
        assert_eq!(limiter.change, None, "{context}: over");
        assert_eq!(limiter.path.lookahead, long, "{context}: in force");
    }
}

/// Runs `limiter` on the peak `peak` until the change running is over, at most `cap` frames
/// (a bound computed apart by the caller), and returns the frames run.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "frame counts of a few thousand, far from overflow"
)]
fn until_over(
    limiter: &mut TruePeakLimiter<Binary64>,
    peak: f64,
    cap: usize,
    context: &str,
) -> usize {
    let mut frames = 0;
    while limiter.change.is_some() {
        assert!(frames < cap, "{context}: still changing after {cap} frames");
        let _: f64 = gain_of(limiter, peak);
        frames += 1;
    }
    frames
}

/// The most frames a change of the lookahead from `old` to `new` frames takes at a maximum
/// of `maximum` frames on a constant need or none, computed apart: its rebuild, then,
/// shrinking, the old window and one frame frozen and the descent ([`descent_frames`]);
/// growing, the rise, which reaches the new path at once on such a need, and the descent
/// from there, two frames, bounded here by the descent's own count.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "frame counts of a few thousand, far from overflow"
)]
fn constant_change_frames(maximum: usize, old: usize, new: usize) -> usize {
    let after = if new > old {
        2 + descent_frames(new)
    } else {
        old + 1 + descent_frames(old)
    };
    rebuild_frames_of(maximum) + after
}

/// Settings away from the defaults (a ceiling of -3 dBTP, the shortest release, an
/// adaptivity of 0.25) with the lookahead `lookahead`.
fn unlike_defaults(lookahead: f64) -> LimiterSettings {
    settings(-3.0_f64, lookahead, 10.0_f64, 0.25_f64)
}

#[test]
fn reads_no_release_of_an_earlier_life_into_a_growth_soon_after_another_change() {
    // At every rate, the two control paths used by turns: 0.5 ms on loud peaks, grown to
    // 1 ms (the 0.5 ms path stops, its last releases loud); 30 releases of peaks under the
    // ceiling, needing nothing; shrunk to 0.5 ms (that path taken up again), then grown to
    // 5 ms as soon as that change is over, within the new window of it. Nothing has been
    // needed for 30 releases: every stage of either path has decayed by `e^-30` at least,
    // from at most `1 + 2^-30` (`spec/measurements.md`, "The true-peak limiter", step 2),
    // each frame's roundings at most `(1 + 2u)` of it over at most 115,200 frames, so every
    // reduction is below `2^-40` (`(1 + 2^-30) e^-30 (1 + 2^-52)^115200` is `9.4·10^-14`);
    // the growth must keep it there, not rebuild its window from the releases the path in
    // force made in an earlier life, before its own change, loud ones.
    for rate in rates() {
        let maximum = frames_at(5.0_f64, rate);
        let context = format!("{} Hz", rate.hz());
        let (short, middle) = (frames_at(0.5_f64, rate), frames_at(1.0_f64, rate));
        let mut limiter = TruePeakLimiter::<Binary64>::new(
            rate,
            1,
            value(5.0_f64),
            super::Detector::Standard,
            unlike_defaults(0.5_f64),
            LimiterMode::Protected,
        )
        .expect("a limiter");
        for _ in 0..frames_at(100.0_f64, rate) {
            let _: f64 = gain_of(&mut limiter, 4.0_f64);
        }
        limiter
            .set(unlike_defaults(1.0_f64))
            .expect("within the maximum");
        let _: usize = until_over(
            &mut limiter,
            4.0_f64,
            constant_change_frames(maximum, short, middle),
            &format!("{context}: to 1 ms"),
        );
        // 30 releases of 10 ms, and the needs of the loud peaks out of every window.
        for _ in 0..frames_at(300.0_f64, rate) + 2 * maximum + 2 {
            let _: f64 = gain_of(&mut limiter, 0.5_f64);
        }
        limiter
            .set(unlike_defaults(0.5_f64))
            .expect("within the maximum");
        let _: usize = until_over(
            &mut limiter,
            0.5_f64,
            constant_change_frames(maximum, middle, short),
            &format!("{context}: to 0.5 ms"),
        );
        limiter
            .set(unlike_defaults(5.0_f64))
            .expect("within the maximum");
        let mut frame = 0;
        while frame < constant_change_frames(maximum, short, maximum) + maximum {
            let reduction = 1.0_f64 - gain_of(&mut limiter, 0.5_f64);
            assert!(
                reduction <= libm::exp2(-40.0_f64),
                "{context}, frame {frame} of the growth: a reduction of {reduction} on no need"
            );
            frame += 1;
        }
        assert_eq!(limiter.change, None, "{context}: over");
    }
}

#[test]
fn rebuilds_a_growths_window_from_the_releases_of_the_old_paths_own_life() {
    // At every rate, the two paths used by turns: 1 ms on loud noise, grown to 2 ms on it;
    // then peaks under the ceiling (no need) with the slowest release, so that every stage
    // still holds the loud releases; shrunk to 0.5 ms (the path of 1 ms taken up again), then
    // grown to 5 ms as soon as that change is over. On the growth's rebuild every hold of the
    // new path is zero (no need in its windows), so each reduction it rebuilds is exactly
    // the old path's released reduction of that frame, the one in force since its own
    // change (its rebuilt reductions, then its releases, recorded here frame by frame), and
    // zero for a frame before: never a release of the path's earlier life. The old path's
    // life is counted apart, from the frames the test ran; some rebuilt frames lie before
    // it, so that a growth reading the ring beyond it, or one frame short of its start, fails.
    let mut draws = Noise::new();
    for rate in rates() {
        let maximum = frames_at(5.0_f64, rate);
        let rebuild = rebuild_frames_of(maximum);
        let context = format!("{} Hz", rate.hz());
        let (short, two) = (frames_at(0.5_f64, rate), frames_at(2.0_f64, rate));
        let made = |lookahead: f64| settings(-2.0_f64, lookahead, 1_000.0_f64, 0.75_f64);
        let mut limiter = TruePeakLimiter::<Binary64>::new(
            rate,
            1,
            value(5.0_f64),
            super::Detector::Standard,
            made(1.0_f64),
            LimiterMode::Protected,
        )
        .expect("a limiter");
        for _ in 0..4 * maximum {
            let _: f64 = gain_of(&mut limiter, 4.0_f64 * draws.next().abs() + 1.0_f64);
        }
        limiter.set(made(2.0_f64)).expect("within the maximum");
        let mut growing = 0;
        while limiter.change.is_some() {
            let _: f64 = gain_of(&mut limiter, 4.0_f64 * draws.next().abs() + 1.0_f64);
            growing += 1;
            assert!(growing <= 16 * maximum, "{context}: still growing to 2 ms");
        }
        // No need from here on: every window of either path empty of needs after twice the
        // maximum and two frames.
        for _ in 0..2 * maximum + 2 {
            let _: f64 = gain_of(&mut limiter, 0.5_f64);
        }
        limiter.set(made(0.5_f64)).expect("within the maximum");
        for _ in 0..rebuild {
            let _: f64 = gain_of(&mut limiter, 0.5_f64);
        }
        assert_eq!(limiter.path.lookahead, short, "{context}: shrunk");
        // The rebuild's last frame was the one before the change's first: from it, the old
        // path's life holds its `short - 1` rebuilt reductions, then one release a frame.
        let after = until_over(
            &mut limiter,
            0.5_f64,
            constant_change_frames(maximum, two, short),
            &format!("{context}: to 0.5 ms"),
        );
        let lived = after + short - 1;
        assert!(lived < maximum, "{context}: a life within the ring");
        // Read once its shrink is over, its rebuilt reductions scaled: the oldest first.
        let mut life: Vec<f64> = (0..lived)
            .rev()
            .map(|back| limiter.path.released_before(back).raw())
            .collect();
        limiter.set(made(5.0_f64)).expect("within the maximum");
        for _ in 0..rebuild {
            let _: f64 = gain_of(&mut limiter, 0.5_f64);
            // The old path in force through the rebuild, its last frame included.
            let old = if matches!(limiter.change, Some(super::Stage::Rebuilding(_))) {
                &limiter.path
            } else {
                &limiter.previous
            };
            life.push(old.released_before(0).raw());
        }
        assert_eq!(limiter.path.lookahead, maximum, "{context}: grown");
        assert_eq!(limiter.previous.lookahead, short, "{context}: the old path");
        assert!(
            life.len() < maximum - 1,
            "{context}: rebuilt frames before the old path's life"
        );
        assert!(
            life.iter().all(|release| *release > 0.0_f64),
            "{context}: the old path still releasing"
        );
        // The reduction rebuilt for the frame `back` before the rebuild's last: the old
        // path's release of that frame, the newest of `life` that last frame's.
        for back in 0..maximum - 1 {
            let expected = (life.len() - 1)
                .checked_sub(back)
                .map_or(0.0_f64, |at| life[at]);
            assert_eq!(
                limiter.path.released_before(back).raw().to_bits(),
                expected.to_bits(),
                "{context}: the reduction rebuilt {back} frames before the change"
            );
        }
    }
}

#[test]
fn rises_no_higher_while_growing_than_limiters_always_at_its_lookaheads() {
    // At every rate, over a seeded random sequence of lookaheads set live (five, from 0.5 to
    // 5 ms, each set on a random frame, during a change too) on noise with random bursts,
    // the settings away from the defaults, beside a limiter always at each of those
    // lookaheads on the same peaks from the first frame: while a growth rebuilds its path
    // or rises, every reduction is at most the largest of theirs, plus both control paths'
    // rounding `2ρ` and both gains' (`spec/measurements.md`, "The true-peak limiter", steps
    // 4 and 5). Each value the growth weighs is a release of needs some path held, the old
    // path's stages carried from the windows before it, so the limiter at the longest
    // window, which holds every need any shorter one holds, is not below it; a law found to
    // hold, not derived. A growth that rebuilt its window from releases of an earlier life of
    // the path in force, needs that have left every window, rises above them all.
    let lookaheads = [0.5_f64, 1.0_f64, 2.0_f64, 3.5_f64, 5.0_f64];
    let mut draws = crate::math::tests::splitmix::Draws::new();
    let mut noise = Noise::new();
    for rate in rates() {
        let maximum = frames_at(5.0_f64, rate);
        let context = format!("{} Hz", rate.hz());
        let made = |lookahead: f64| settings(-2.0_f64, lookahead, 30.0_f64, 0.75_f64);
        let build = |lookahead: f64| {
            TruePeakLimiter::<Binary64>::new(
                rate,
                1,
                value(5.0_f64),
                super::Detector::Standard,
                made(lookahead),
                LimiterMode::Protected,
            )
            .expect("a limiter")
        };
        let mut limiter = build(0.5_f64);
        let mut always: Vec<TruePeakLimiter<Binary64>> = lookaheads
            .iter()
            .map(|lookahead| build(*lookahead))
            .collect();
        let slack = 2.0_f64 * rounding_of(maximum) + 2.0_f64 * libm::exp2(-53.0_f64);
        let (mut burst, mut checked) = (0, 0_u32);
        for frame in 0..64 * maximum {
            // A lookahead set on one frame in `maximum / 4`, on average.
            if draws
                .next()
                .is_multiple_of(u64::try_from(maximum / 4).expect("small"))
            {
                let at = usize::try_from(draws.next() % 5).expect("small");
                limiter
                    .set(made(lookaheads[at]))
                    .expect("within the maximum");
            }
            // A burst of 1 to 64 frames on one frame in 200, on average.
            if burst == 0 && draws.next().is_multiple_of(200) {
                burst = 1 + usize::try_from(draws.next() % 64).expect("small");
            }
            let level = if burst > 0 {
                burst -= 1;
                4.0_f64
            } else {
                0.5_f64
            };
            let peak = level * noise.next().abs();
            let growing = match limiter.change {
                Some(super::Stage::Rebuilding(_)) => limiter.rebuild.longer,
                Some(super::Stage::Rising { .. }) => true,
                _ => false,
            };
            let reduction = 1.0_f64 - gain_of(&mut limiter, peak);
            let most = always
                .iter_mut()
                .map(|reference| 1.0_f64 - gain_of(reference, peak))
                .fold(0.0_f64, larger);
            if growing {
                assert!(
                    reduction <= most + slack,
                    "{context}, frame {frame}: a reduction of {reduction} growing, at most \
                     {most} always at a lookahead"
                );
                checked += 1;
            }
        }
        assert!(checked > 0, "{context}: growths checked");
    }
}

/// The largest weight of a Hann window of `length` frames,
/// `max_k (1 - cos(2π k / (length + 1))) / (length + 1)` for `k` from 1 to `length`,
/// computed apart: per unit of a need, the most one frame of the window's attack moves the
/// reduction.
fn hann_peak(length: usize) -> f64 {
    let span = f64::from(u32::try_from(length.saturating_add(1)).expect("small"));
    (1..=length).fold(0.0_f64, |peak, index| {
        let turns = f64::from(u32::try_from(index).expect("small"));
        larger(
            peak,
            (1.0_f64 - libm::cos(core::f64::consts::TAU * turns / span)) / span,
        )
    })
}

/// The gain of the control path for a frame whose peak is `peak`, in binary64: 1 when it
/// gives none (no reduction).
fn gain_of(limiter: &mut TruePeakLimiter<Binary64>, peak: f64) -> f64 {
    limiter.gain(value(peak)).map_or(1.0_f64, Binary64::raw)
}

/// The largest one-frame step of `gains`, and its frame.
fn largest_step(gains: &[f64]) -> (usize, f64) {
    gains
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).abs())
        .enumerate()
        .fold((0, 0.0_f64), |worst, (frame, step)| {
            if step > worst.1 { (frame, step) } else { worst }
        })
}

#[test]
fn proves_a_growths_rise_stays_above_the_old_windows_attack_up_to_the_longest_window() {
    // The inequality the step bound of a growing lookahead rests on, `spec/measurements.md`,
    // "The true-peak limiter": for every old window of `L` frames, every `s ≥ 1` with
    // `L + s` up to the longest maximum lookahead (5 ms at the highest rate, 1,920 frames),
    // and every `y` from 1 to `L`,
    //
    //     (y + s) × hp(L + s) ≥ C_L(y),
    //
    // `hp(D)` the largest Hann weight of a window of `D` frames and `C_L(y)` the share of the
    // first `y` weights of a window of `L`: the rise of `y + s` frames at the attack of
    // `L + s` frames stays above the old window's attack, `y` frames into it, begun `s`
    // frames after the rise. Every case, exhaustively, in binary64 with a derived allowance:
    // each weight is within `23 × 2^-53 / (L + 1)` of its exact value (the angle's three
    // roundings, `3.0001 × 2^-53 × 2π`; libm's cosine within an ulp; the difference and the
    // quotient one rounding each), so `C_L(y)`, a sum of `y ≤ L` of them, is within
    // `(23 + L) × 2^-53` (each partial sum at most 1), the left side within `25 × 2^-53` (a
    // weight times at most `L + s` frames, and its product) and their difference within two
    // more: a computed margin of at least `(L + 50) × 2^-53` proves the exact one at least
    // zero. One case has none: `L = s = y = 1`, where both sides are exactly 1, `C_1(1) =
    // (1 - cos π) / 2` and `2 × hp(2) = 2 (1 - cos(2π / 3)) / 3`, as their computed values
    // within that allowance confirm.
    let longest = rates()
        .into_iter()
        .map(|rate| frames_at(5.0_f64, rate))
        .fold(0, usize::max);
    assert_eq!(longest, 1_920, "5 ms at 384 kHz");
    let unit = libm::exp2(-53.0_f64);
    let peaks: Vec<f64> = (0..=longest).map(hann_peak).collect();
    let mut cases = 0_u64;
    for window in 1..longest {
        let span = f64::from(u32::try_from(window + 1).expect("small"));
        let shares: Vec<f64> = (1..=window)
            .scan(0.0_f64, |share, index| {
                let turns = f64::from(u32::try_from(index).expect("small"));
                *share += (1.0_f64 - libm::cos(core::f64::consts::TAU * turns / span)) / span;
                Some(*share)
            })
            .collect();
        let allowance = f64::from(u32::try_from(window + 50).expect("small")) * unit;
        for later in 1..=longest - window {
            let peak = peaks[window + later];
            let mut frames = f64::from(u32::try_from(later).expect("small"));
            for (rise, share) in shares.iter().enumerate() {
                frames += 1.0_f64;
                let margin = frames * peak - share;
                if margin < allowance {
                    assert_eq!(
                        (window, later, rise + 1),
                        (1, 1, 1),
                        "L {window}, s {later}, y {}: a margin of {margin}, {allowance} \
                         needed",
                        rise + 1
                    );
                    assert!(
                        (share - 1.0_f64).abs() <= 24.0_f64 * unit
                            && (frames * peak - 1.0_f64).abs() <= 26.0_f64 * unit,
                        "both sides 1: {share} and {}",
                        frames * peak
                    );
                }
            }
            cases += u64::try_from(window).expect("small");
        }
    }
    // Every case: `Σ_{L=1}^{1919} L × (1920 - L)`.
    assert_eq!(cases, 1_179_647_680, "every case");
}

/// The bursts the control path is driven with while its lookahead changes: each its width
/// in frames (0 for twice the maximum lookahead), its peak, the floor of peaks around it,
/// and whether it runs at a maximum lookahead above [`DENSE`] frames too (one of each
/// floor and width, so that the sweep there stays within seconds). Below the ceiling, the
/// floor needs nothing; 1.2 and 1.0 a sustained reduction, which the burst rises from; 16
/// a need near the whole signal.
const BURSTS: [(usize, f64, f64, bool); 6] = [
    (1, 2.0_f64, 0.5_f64, true),
    (10, 2.0_f64, 0.5_f64, false),
    (0, 2.0_f64, 0.5_f64, false),
    (10, 16.0_f64, 0.5_f64, false),
    (10, 4.0_f64, 1.2_f64, true),
    (0, 16.0_f64, 1.0_f64, true),
];

/// The largest maximum lookahead, in frames, at which every burst of [`BURSTS`] runs:
/// 96 kHz's 480 frames and below.
const DENSE: usize = 512;

/// The frames a new path is rebuilt over before its change starts, at a maximum lookahead
/// of `maximum` frames, as decision 12 states them, computed apart: a rebuild runs at most
/// `2 × maximum + 1` frames of the new path's hold, 32 per frame.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "frame counts of a few thousand, far from overflow"
)]
fn rebuild_frames_of(maximum: usize) -> usize {
    (2 * maximum + 1).div_ceil(32)
}

/// The frames a change may be made after, before the burst's first frame `onset`, of
/// `width` frames, at a maximum lookahead of `maximum` frames between the lookaheads `old`
/// and `new`: from `2 × maximum + 64` frames before the burst arrives to past its last need
/// leaving the delay. Every one up to 64 frames of maximum; past that, every `maximum / 16`
/// frames (rounded up) and every one within 8 frames of where the regime changes: the
/// burst at the change's first frame, and each `D`, the frames from the change's first frame
/// to the first that needs the burst, of 1, either window, the maximum, the maximum plus
/// either window and plus twice the longer (the burst arriving as the change ends or after
/// it).
#[expect(
    clippy::arithmetic_side_effects,
    reason = "frame counts of a few thousand, the earliest past twice the maximum"
)]
fn change_frames(
    onset: usize,
    width: usize,
    maximum: usize,
    (old, new): (usize, usize),
) -> Vec<usize> {
    let rebuild = rebuild_frames_of(maximum);
    let first = onset - (2 * maximum + 64);
    let last = onset + width + maximum + 4;
    let stride = if maximum <= 64 {
        1
    } else {
        maximum.div_ceil(16)
    };
    let longer = old.max(new);
    // `D` for a change set after `frame`: `onset + maximum - 1 - frame - rebuild`; and the
    // burst arriving while the new path is rebuilt (`D` from the maximum to the maximum plus
    // the rebuild).
    let edges = [
        1,
        old,
        new,
        maximum,
        maximum + 1,
        maximum + old,
        maximum + new,
        maximum + 2 * longer,
        maximum + rebuild,
        maximum + rebuild / 2,
    ]
    .map(|depth| (onset + maximum - 1).saturating_sub(depth + rebuild));
    (first..=last)
        .filter(|frame| {
            (frame - first).is_multiple_of(stride)
                || *frame == onset
                || *frame == last
                || edges.iter().any(|edge| frame.abs_diff(*edge) <= 8)
        })
        .collect()
}

/// A lookahead changed live from `from` to `to` milliseconds (maximum 5 ms), at every rate,
/// while the control path holds each burst of [`BURSTS`], the change made at every depth of
/// [`change_frames`], from twice the maximum before the burst arrives to past its last need:
/// every frame's reduction covers the needs of the samples it applies to, and each one-frame
/// step of the gain is at most the attack the change allows, `need × max(hp(new), hp(D))`
/// growing and `need × hp(new)` shrinking, plus the rounding `2ρ + 8 × 2^-53`.
///
/// - `need` is the burst's, `1 - c / peak + E`, `hp(L)` the largest Hann weight of a window of
///   `L` frames ([`hann_peak`]) and `D` the frames from the change's first frame to the
///   first that needs the burst's first need (`onset + maximum - 1`, the needs covered at a
///   frame `n` being those of `n - maximum - 1` to `n - maximum + 1`), the term dropped when
///   `D < 1`; all computed apart, as are the margin `E` ([`margin_of`]) and `ρ`
///   ([`rounding_of`]).
/// - The change starts [`rebuild_frames_of`] frames after it is set, the old path alone in
///   force until then: through those frames, every step at most the old window's attack of
///   that need, as before a change.
/// - Covered: each reduction, `1 - gain`, at least the largest of those needs less `ρ` and
///   the gain's rounding.
/// - Before the change, every step at most the old window's attack of that need.
/// - The change over by the end of each run: `maximum + 2 × longer + 64` frames past the
///   burst's first frame or the change, the later (the burst's attack complete by then).
/// - The burst limited: some gain below `1 - need / 2`.
///
/// The control path alone (`TruePeakLimiter::gain`, a frame's peak in, its gain out), the
/// detector and the delay apart: the frames before a change are the same at every depth,
/// run once, and each depth branches from them by a copy of the limiter, changed and run.
fn steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes(from: f64, to: f64) {
    for rate in rates() {
        let maximum = frames_at(5.0_f64, rate);
        for (width, level, floor, everywhere) in BURSTS {
            if maximum > DENSE && !everywhere {
                continue;
            }
            let width = if width == 0 {
                maximum.saturating_mul(2)
            } else {
                width
            };
            swept_through_a_change(rate, (from, to), (width, level, floor));
        }
    }
}

/// One rate and burst of
/// [`steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes`]: the
/// lookahead changed from `from` to `to` milliseconds, around `width` frames of peaks of
/// `level` over a floor of `floor`.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "frame counts of a few thousand and reductions below two, far from overflow"
)]
fn swept_through_a_change(
    rate: SampleRate,
    (from, to): (f64, f64),
    (width, level, floor): (usize, f64, f64),
) {
    let ceiling = CEILING;
    let unit = libm::exp2(-53.0_f64);
    let maximum = frames_at(5.0_f64, rate);
    let (old, new) = (frames_at(from, rate), frames_at(to, rate));
    let context = format!(
        "{} Hz, {from} to {to} ms, {width} frames of {level} over {floor}",
        rate.hz()
    );
    assert_ne!(old, new, "{context}: two lookaheads");
    let longer = old.max(new);
    let (margin, rounding) = (margin_of(maximum), rounding_of(maximum));
    let slack = 2.0_f64 * rounding + 8.0_f64 * unit;
    let need = |peak: f64| need_above(peak, ceiling, margin);
    // The floor settled long before the earliest change.
    let onset = 4 * maximum + 128;
    let peak = |frame: usize| {
        if (onset..onset + width).contains(&frame) {
            level
        } else {
            floor
        }
    };
    let largest = need(level);
    let rebuild = rebuild_frames_of(maximum);
    let unchanged = largest * hann_peak(old) + slack;
    // The needs of frames `frame - maximum - 1` to `frame - maximum + 1`, those before the
    // first frame none.
    let covered = |frame: usize, gain: f64| {
        let held = (frame + 1).checked_sub(maximum).map_or(0.0_f64, |newest| {
            (newest.saturating_sub(2)..=newest)
                .map(|at| need(peak(at)))
                .fold(0.0_f64, larger)
        });
        assert!(
            1.0_f64 - gain >= held - rounding - 8.0_f64 * unit,
            "{context}: frame {frame} reduced by {}, {held} needed",
            1.0_f64 - gain
        );
    };
    let frames = change_frames(onset, width, maximum, (old, new));
    let mut limiter = TruePeakLimiter::<Binary64>::new(
        rate,
        1,
        value(5.0_f64),
        super::Detector::Standard,
        settings(-1.0_f64, from, 100.0_f64, 0.5_f64),
        LimiterMode::Protected,
    )
    .expect("a limiter");
    let mut before = Vec::new();
    let mut next = frames.iter().copied().peekable();
    for frame in 0..=*frames.last().expect("frames") {
        let gain = gain_of(&mut limiter, peak(frame));
        covered(frame, gain);
        before.push(gain);
        if next.next_if_eq(&frame).is_none() {
            continue;
        }
        let mut changed = limiter.clone();
        changed
            .set(settings(-1.0_f64, to, 100.0_f64, 0.5_f64))
            .expect("within the maximum");
        // The rebuild, the old path alone in force, then `D`, from the change's first frame.
        assert_eq!(
            changed.rebuild_frames, rebuild,
            "{context}: the rebuild's frames"
        );
        let start = frame + 1 + rebuild;
        let depth = (onset + maximum).checked_sub(start);
        let attack = match depth {
            Some(depth) if new > old && depth >= 1 => larger(hann_peak(new), hann_peak(depth)),
            _ => hann_peak(new),
        };
        let allowed = largest * attack + slack;
        let end = frame.max(onset) + maximum + 2 * longer + 64 + rebuild;
        let mut gains = vec![gain];
        for at in frame + 1..=end {
            let after = gain_of(&mut changed, peak(at));
            covered(at, after);
            gains.push(after);
        }
        // Through the rebuild, the old path's own steps; from the frame before the change.
        let (rebuilding, changing) = (&gains[..=rebuild], &gains[rebuild..]);
        steps_within(
            rebuilding,
            unchanged,
            &format!("{context}, set after frame {frame}, into the rebuild"),
        );
        steps_within(
            changing,
            allowed,
            &format!("{context}, set after frame {frame} (D {depth:?}), from the change"),
        );
        assert_eq!(changed.change, None, "{context}, after frame {frame}: over");
        assert!(
            before
                .iter()
                .chain(&gains)
                .any(|reduced| *reduced < 1.0_f64 - largest / 2.0_f64),
            "{context}, after frame {frame}: the burst limited"
        );
    }
    steps_within(&before, unchanged, &format!("{context}, before a change"));
}

/// The need of a peak `peak` over the ceiling `ceiling` with the margin `margin`, computed
/// apart: `1 - ceiling / peak + margin` above the ceiling, none at or below it.
fn need_above(peak: f64, ceiling: f64, margin: f64) -> f64 {
    if peak > ceiling {
        1.0_f64 - ceiling / peak + margin
    } else {
        0.0_f64
    }
}

/// Checks that no one-frame step of `gains` is above `allowed`, saying where.
fn steps_within(gains: &[f64], allowed: f64, context: &str) {
    let (at, step) = largest_step(gains);
    assert!(
        step <= allowed,
        "{context}: a step of {step} {at} frames in, {allowed} allowed"
    );
}

#[test]
fn steps_within_the_attack_of_each_depth_and_covers_growing_live_from_0_5_to_5_ms() {
    steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes(0.5_f64, 5.0_f64);
}

#[test]
fn steps_within_the_attack_of_each_depth_and_covers_growing_live_from_1_5_to_5_ms() {
    steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes(1.5_f64, 5.0_f64);
}

#[test]
fn steps_within_the_attack_of_each_depth_and_covers_growing_live_from_0_5_to_1_5_ms() {
    steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes(0.5_f64, 1.5_f64);
}

#[test]
fn steps_within_the_attack_of_each_depth_and_covers_growing_live_from_3_to_4_8_ms() {
    steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes(3.0_f64, 4.8_f64);
}

#[test]
fn steps_within_the_attack_of_each_depth_and_covers_growing_live_from_2_5_to_5_ms() {
    steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes(2.5_f64, 5.0_f64);
}

#[test]
fn steps_within_the_attack_of_each_depth_and_covers_growing_live_from_4_9_to_5_ms() {
    steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes(4.9_f64, 5.0_f64);
}

#[test]
fn steps_within_the_new_attack_and_covers_shrinking_live_from_5_to_0_5_ms() {
    steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes(5.0_f64, 0.5_f64);
}

#[test]
fn steps_within_the_new_attack_and_covers_shrinking_live_from_5_to_1_5_ms() {
    steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes(5.0_f64, 1.5_f64);
}

#[test]
fn steps_within_the_new_attack_and_covers_shrinking_live_from_1_5_to_0_5_ms() {
    steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes(1.5_f64, 0.5_f64);
}

#[test]
fn steps_within_the_new_attack_and_covers_shrinking_live_from_4_8_to_3_ms() {
    steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes(4.8_f64, 3.0_f64);
}

#[test]
fn steps_within_the_new_attack_and_covers_shrinking_live_from_5_to_2_5_ms() {
    steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes(5.0_f64, 2.5_f64);
}

#[test]
fn steps_within_the_new_attack_and_covers_shrinking_live_from_5_to_4_9_ms() {
    steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes(5.0_f64, 4.9_f64);
}

#[test]
fn names_the_lookahead_and_its_maximum_in_milliseconds() {
    assert_eq!(
        format!(
            "{}",
            LimiterError::OutOfRange(Setting::Lookahead, value(6.0_f64))
        ),
        "limiter lookahead 6 ms is out of range (0.5 to 5 ms)"
    );
    assert_eq!(
        format!(
            "{}",
            LimiterError::OutOfRange(Setting::MaximumLookahead, value(0.25_f64))
        ),
        "limiter maximum lookahead 0.25 ms is out of range (0.5 to 5 ms)"
    );
}

/// Switched to protected while raw, in `S`: once the crossfade has arrived, the ceiling
/// holds again.
fn holds_the_ceiling_once_protected_again<S: Sample + Raw>() {
    let mut draws = Noise::new();
    let ceiling = CEILING;
    let rate = SampleRate::new(48_000).expect("a rate");
    let mut limiter = TruePeakLimiter::<S>::new(
        rate,
        2,
        value(1.5_f64),
        super::Detector::Standard,
        LimiterSettings::DEFAULT,
        LimiterMode::Switchable,
    )
    .expect("a limiter");
    let latency = limiter.latency();
    assert_eq!(
        latency,
        latency_of(1.5_f64, rate, super::Detector::Standard),
        "counted apart"
    );
    let raw = limited(
        &mut limiter,
        &signal::<S>(20, 2, |_, _| drawn(&mut draws, 4.0_f64)),
    );
    assert!(largest(&raw, 0, 2) > 3.0_f64, "raw passes the loud signal");
    limiter.set_protected(true);
    assert!(limiter.protected(), "protected from now on");
    let _fading = limited(
        &mut limiter,
        &signal::<S>(750, 2, |_, _| drawn(&mut draws, 4.0_f64)),
    );
    let protected = limited(
        &mut limiter,
        &signal::<S>(50, 2, |_, _| drawn(&mut draws, 4.0_f64)),
    );
    let peak = largest(&protected, latency, 2);
    assert!(peak <= ceiling, "{peak} once protected again");
    // Not silenced: the loud noise limited near the ceiling, its largest sample above half
    // of it (a stage giving zeros, or a gain stuck low, fails here).
    assert!(peak > ceiling * 0.5_f64, "{peak}: limited, not silenced");
    settles_at_its_exact_gain(
        &mut limiter,
        2,
        (rate, super::Detector::Standard, 1.5_f64),
        ceiling,
        16.0_f64,
        "protected again",
    );
}

#[test]
fn holds_the_ceiling_once_protected_again_in_each_precision() {
    holds_the_ceiling_once_protected_again::<Binary64>();
    holds_the_ceiling_once_protected_again::<Binary32>();
}

/// A ceiling lowered live, in `S`, at every rate, with each detector: every sample is at or
/// below the old ceiling over the maximum lookahead from the change (the samples leaving the
/// delay then keep the needs computed for the old one), and at or below the new one from
/// then on, within the latency; some sample before is above the new one, so that the
/// boundary is where it is said.
fn lowers_its_ceiling_live_after_its_maximum_lookahead<S: Sample + Raw>() {
    // 0 dBTP, then -20 dBTP: amplitudes 1 and `MINUS_20_DBTP`, computed apart.
    let (old, new) = (1.0_f64, MINUS_20_DBTP);
    for rate in rates() {
        // The maximum lookahead, 5 ms, in frames, counted apart.
        let maximum = frames_at(5.0_f64, rate);
        for detector in DETECTORS {
            let mut limiter = TruePeakLimiter::<S>::new(
                rate,
                1,
                value(5.0_f64),
                detector,
                settings(0.0_f64, 1.5_f64, 100.0_f64, 0.5_f64),
                LimiterMode::Protected,
            )
            .expect("a limiter");
            let latency = limiter.latency();
            assert_eq!(
                latency,
                latency_of(5.0_f64, rate, detector),
                "counted apart"
            );
            // Settled at the old ceiling: past the latency by two windows and more.
            let blocks = latency
                .saturating_add(frames_at(1.5_f64, rate).saturating_mul(2))
                .saturating_add(256)
                .div_ceil(BLOCK_SIZE);
            let loud = |frame: usize, _channel: usize| {
                let at = f64::from(u32::try_from(frame).expect("small"));
                of::<S>(3.0_f64 * libm::sin(core::f64::consts::TAU * 0.05_f64 * at))
            };
            let input = signal::<S>(blocks.saturating_mul(2), 1, loud);
            let (before, after) = input.split_at(blocks);
            let _settled = limited(&mut limiter, before);
            assert_eq!(
                limiter.set(settings(-20.0_f64, 1.5_f64, 100.0_f64, 0.5_f64)),
                Ok(()),
                "lowered"
            );
            let output = limited(&mut limiter, after);
            let during = (0..maximum).map(|frame| at(&output, frame, 0).to_binary64().raw().abs());
            let largest_during = during.fold(0.0_f64, larger);
            let largest_after = largest(&output, maximum, 1);
            let context = format!("{} Hz, {detector:?}", rate.hz());
            assert!(
                largest_during <= old,
                "{context}: {largest_during} above the old"
            );
            assert!(
                largest_during > new,
                "{context}: {largest_during}, the old ceiling held over the maximum lookahead"
            );
            assert!(
                largest_after <= new,
                "{context}: {largest_after} above the new"
            );
            // Then a constant above every need of the sine: the exact gain of the new ceiling.
            settles_at_its_exact_gain(
                &mut limiter,
                1,
                (rate, detector, 5.0_f64),
                new,
                16.0_f64,
                &context,
            );
        }
    }
}

#[test]
fn lowers_its_ceiling_live_after_its_maximum_lookahead_in_each_precision() {
    lowers_its_ceiling_live_after_its_maximum_lookahead::<Binary64>();
    lowers_its_ceiling_live_after_its_maximum_lookahead::<Binary32>();
}

/// Live settings, in `S`: a lookahead above the maximum refused, the settings in force
/// kept; a new release with the same lookahead taken as it is.
fn changes_its_settings_live<S: Sample>() {
    let rate = SampleRate::new(96_000).expect("a rate");
    let mut limiter = TruePeakLimiter::<S>::new(
        rate,
        2,
        value(3.0_f64),
        super::Detector::Standard,
        settings(-1.0_f64, 3.0_f64, 100.0_f64, 0.5_f64),
        LimiterMode::Protected,
    )
    .expect("a limiter");
    assert_eq!(
        limiter.set(settings(-1.0_f64, 4.0_f64, 100.0_f64, 0.5_f64)),
        Err(LimiterError::AboveMaximum {
            lookahead: value(4.0_f64),
            maximum: value(3.0_f64)
        }),
        "above the maximum"
    );
    let released = settings(-3.0_f64, 3.0_f64, 500.0_f64, 1.0_f64);
    assert_eq!(limiter.set(released), Ok(()), "the same lookahead");
    assert_eq!(limiter.settings(), released, "taken");
    assert_eq!(released.lookahead(), value(3.0_f64), "its lookahead");
    assert_eq!(
        released.with_lookahead(value(0.5_f64)),
        Ok(settings(-3.0_f64, 0.5_f64, 500.0_f64, 1.0_f64)),
        "another lookahead, the rest kept"
    );
    assert_eq!(
        released.with_lookahead(value(6.0_f64)),
        Err(LimiterError::OutOfRange(Setting::Lookahead, value(6.0_f64))),
        "a lookahead out of range"
    );
}

#[test]
fn changes_its_settings_live_in_each_precision() {
    changes_its_settings_live::<Binary64>();
    changes_its_settings_live::<Binary32>();
}

/// The crossfade, in `S`: gradual (a block after a switch, the share applied lies strictly
/// between its ends), then arrived exactly (a second later, 50 time constants of 20 ms).
fn crossfades_gradually_then_arrives_exactly<S: Sample>() {
    let rate = SampleRate::new(48_000).expect("a rate");
    // The share applied is the control path's, binary64 in each precision.
    let one = value(1.0_f64);
    let zero = value(0.0_f64);
    let mut draws = Noise::new();
    let mut limiter = TruePeakLimiter::<S>::new(
        rate,
        1,
        value(1.5_f64),
        super::Detector::Standard,
        LimiterSettings::DEFAULT,
        LimiterMode::Protected,
    )
    .expect("a limiter");
    assert_eq!(limiter.applied, one, "protected from the first frame");
    for (protect, end) in [(false, zero), (true, one)] {
        limiter.set_protected(protect);
        let _block = limited(
            &mut limiter,
            &signal::<S>(1, 1, |_, _| drawn(&mut draws, 4.0_f64)),
        );
        assert!(
            limiter.applied > zero && limiter.applied < one,
            "{protect}: gradual, {:?} after a block",
            limiter.applied
        );
        let _second = limited(
            &mut limiter,
            &signal::<S>(750, 1, |_, _| drawn(&mut draws, 4.0_f64)),
        );
        assert_eq!(limiter.applied, end, "{protect}: arrived exactly");
    }
}

#[test]
fn crossfades_gradually_then_arrives_exactly_in_each_precision() {
    crossfades_gradually_then_arrives_exactly::<Binary64>();
    crossfades_gradually_then_arrives_exactly::<Binary32>();
}

/// The bookkeeping of a sustained limit, in `S`: on a constant loud input, the sliding
/// maximum keeps one entry (an equal need replaces the one before it), and the Hann sums
/// accumulated over a window replace the slid ones once per window: the frames until the
/// next swap count down from the window's length less one to zero every frame, so that
/// after `f` frames from a swap due on the first, `(L - f mod L) mod L` are left.
fn keeps_its_windows_exact_and_small<S: Sample>() {
    let rate = SampleRate::new(48_000).expect("a rate");
    let mut limiter = TruePeakLimiter::<S>::new(
        rate,
        1,
        value(1.5_f64),
        super::Detector::Standard,
        LimiterSettings::DEFAULT,
        LimiterMode::Protected,
    )
    .expect("a limiter");
    let window = limiter.path.lookahead;
    assert_eq!(window, 72, "1.5 ms at 48 kHz");
    for block in 0_usize..20 {
        let _block = limited(&mut limiter, &signal::<S>(1, 1, |_, _| of::<S>(2.0_f64)));
        let frames = block.saturating_add(1).saturating_mul(BLOCK_SIZE);
        assert_eq!(
            limiter.path.swap_in,
            window
                .saturating_sub(frames.checked_rem(window).expect("a window"))
                .checked_rem(window)
                .expect("a window"),
            "after {frames} frames"
        );
        // Once the window has seen only the constant (its 74 frames, the 12 of the
        // interpolator before them): two blocks.
        if block >= 2 {
            assert_eq!(limiter.path.queued, 1, "one need held after block {block}");
        }
    }
}

#[test]
fn keeps_its_windows_exact_and_small_in_each_precision() {
    keeps_its_windows_exact_and_small::<Binary64>();
    keeps_its_windows_exact_and_small::<Binary32>();
}

#[test]
fn builds_each_detector_with_its_delay_and_a_linear_phase_interpolator() {
    use super::Detector;
    for (detector, phases, taps) in [
        (Detector::Standard, 4_usize, 12_usize),
        (Detector::Precise, 4, 32),
        (Detector::Mastering, 8, 128),
    ] {
        assert_eq!(
            (detector.phases(), detector.taps(), detector.delay()),
            (phases, taps, taps / 2),
            "{detector:?}"
        );
        let coefficients = detector.coefficients();
        assert_eq!(coefficients.len(), phases * taps, "{detector:?}: every tap");
        // Linear phase: the interpolator's points, phase-major, read backwards are the
        // same, tap `k` of phase `p` equal to tap `taps - 1 - k` of phase `phases - 1 - p`.
        for phase in 0..phases {
            for tap in 0..taps {
                let (one, other) = (
                    coefficients[phase * taps + tap].raw(),
                    coefficients[(phases - 1 - phase) * taps + (taps - 1 - tap)].raw(),
                );
                assert!(
                    (one - other).abs() <= 4.0_f64 * f64::EPSILON,
                    "{detector:?}: {phase}, {tap}"
                );
            }
        }
    }
    let standard: Vec<u64> = Detector::Standard
        .coefficients()
        .iter()
        .map(|tap| tap.bits())
        .collect();
    let table: Vec<u64> = crate::TRUE_PEAK_FILTER
        .iter()
        .flatten()
        .map(|tap| tap.to_bits())
        .collect();
    assert_eq!(standard, table, "the standard's own table");
    for detector in [Detector::Precise, Detector::Mastering] {
        for phase in detector.coefficients().chunks_exact(detector.taps()) {
            let gain = phase.iter().fold(0.0_f64, |sum, tap| sum + tap.raw());
            assert!(
                (gain - 1.0_f64).abs() <= 1e-14_f64,
                "{detector:?}: a unit gain at DC, {gain}"
            );
        }
    }
}

/// The FNV-1a 64 digest of `words`, each in its 8 little-endian bytes.
fn digest(words: impl Iterator<Item = u64>) -> u64 {
    words
        .flat_map(u64::to_le_bytes)
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
}

#[test]
fn pins_the_bits_of_each_windowed_interpolator_in_each_precision() {
    // The windowed interpolators are computed when a limiter is built (`libm`'s sine and
    // square root, the Bessel series, a division per tap): any change of that code, or of
    // `libm`, moves the bits of every render through them, which no conformance case holds
    // yet for these detectors. Their bits are pinned here, in binary64 and as the binary32
    // limiter takes them (each rounded to the nearest), so that such a change is a decision.
    use super::Detector;
    for (detector, binary64, binary32) in [
        (
            Detector::Precise,
            0x76a8_7394_dff0_b625_u64,
            0xa934_bf56_710f_5f2d_u64,
        ),
        (
            Detector::Mastering,
            0xb785_a2c1_db02_9f81_u64,
            0xfdd5_390b_c820_0581_u64,
        ),
    ] {
        let coefficients = detector.coefficients();
        assert_eq!(
            (
                digest(coefficients.iter().map(|tap| tap.bits())),
                digest(
                    coefficients
                        .iter()
                        .map(|tap| Binary32::from_binary64(*tap).bits())
                )
            ),
            (binary64, binary32),
            "{detector:?}"
        );
    }
}

#[test]
fn delays_by_its_detector_and_maximum_lookahead() {
    let rate = SampleRate::new(48_000).expect("a rate");
    for (detector, delay) in [
        (super::Detector::Standard, 6),
        (super::Detector::Precise, 16),
        (super::Detector::Mastering, 64),
    ] {
        let limiter = TruePeakLimiter::<Binary32>::new(
            rate,
            1,
            value(1.5_f64),
            detector,
            LimiterSettings::DEFAULT,
            LimiterMode::Protected,
        )
        .expect("a limiter");
        assert_eq!(
            limiter.latency(),
            72 + delay,
            "{detector:?}: 1.5 ms at 48 kHz and its delay"
        );
    }
}

/// The gain a limiter at 48 kHz with `detector` Precise, 1.5 ms of lookahead and
/// `settings` applies to a constant input `level(frame)`, frame by frame: each output over
/// its delayed input, from the latency on.
fn gains(
    settings: LimiterSettings,
    frames: usize,
    mut level: impl FnMut(usize) -> f64,
    mut change: impl FnMut(usize, &mut TruePeakLimiter<Binary64>),
) -> Vec<f64> {
    let rate = SampleRate::new(48_000).expect("a rate");
    let mut limiter = TruePeakLimiter::<Binary64>::new(
        rate,
        1,
        value(1.5_f64),
        super::Detector::Precise,
        settings,
        LimiterMode::Protected,
    )
    .expect("a limiter");
    let latency = limiter.latency();
    assert_eq!(
        latency,
        latency_of(1.5_f64, rate, super::Detector::Precise),
        "counted apart"
    );
    let input: Vec<f64> = (0..frames).map(&mut level).collect();
    let mut output = Vec::with_capacity(frames);
    for (index, block) in input.chunks(BLOCK_SIZE).enumerate() {
        change(index.saturating_mul(BLOCK_SIZE), &mut limiter);
        let mut blocks = [core::array::from_fn::<_, BLOCK_SIZE, _>(|offset| {
            of::<Binary64>(block.get(offset).copied().unwrap_or(0.0_f64))
        })];
        limiter.apply(&mut blocks);
        output.extend(blocks[0].iter().map(|sample| sample.raw()));
    }
    (latency..frames)
        .map(|frame| output[frame] / input[frame.saturating_sub(latency)])
        .collect()
}

/// The frames, after `from`, until the gain stays at or above `1 - left` for good.
fn recovery(gains: &[f64], from: usize, left: f64) -> usize {
    let last_low = gains
        .iter()
        .rposition(|gain| *gain < 1.0_f64 - left)
        .expect("a reduction");
    last_low.saturating_add(1).saturating_sub(from)
}

#[test]
fn recovers_fast_from_a_transient_and_slowly_from_a_sustained_level_as_its_adaptivity_says() {
    // A quiet level of 0.5 (below the ceiling, 0.891), with a burst of 2.0 lasting 10
    // frames (a transient) or 48,000 (a second, sustained). The need of the burst is
    // d = 1 - 0.891/2, about 0.554; the gain recovers within 1% of unity when what is left of
    // the reduction falls below 0.01: about ln(d / 0.01) time constants, 4.01.
    // - The fast stage releases with release × (1 - a) + release / 10 × a: 100 ms at an
    //   adaptivity a of 0, 10 ms at 1.
    // - The slow stage follows the held need with the release's 100 ms, up and down: after a
    //   transient it has barely risen (the burst and the 74 frames of the window held,
    //   84 frames of 4,800, so d × 84 / 4,800 < 0.01), after a sustained level it holds d.
    // So with a of 1: about 40 ms after the transient (1,925 frames), 401 ms after the
    // sustained level (19,250 frames); with a of 0, about 401 ms after both. Each within the
    // window held and the Hann smoothing (74 + 72 frames) of its prediction.
    //
    // The reduction recovers from the largest need held at the burst's end, measured: a
    // step of the input rings in the interpolator, so its estimate rises a little above 2.0
    // and its need above 0.554 (which it must reach at least).
    for (adaptivity, fast_ms) in [(1.0_f64, 10.0_f64), (0.0_f64, 100.0_f64)] {
        let settings = settings(-1.0_f64, 1.5_f64, 100.0_f64, adaptivity);
        for burst in [10_usize, 48_000] {
            let start = 4_800_usize;
            let frames = start + burst + 48_000;
            let measured = gains(
                settings,
                frames,
                |frame| {
                    if (start..start + burst).contains(&frame) {
                        2.0_f64
                    } else {
                        0.5_f64
                    }
                },
                |_, _| (),
            );
            // The release starts from the reduction held around the burst's end (its last
            // step rings too), within a window and its smoothing on each side.
            let end = start + burst;
            let deepest = 1.0_f64
                - measured[end - 150..end + 150]
                    .iter()
                    .fold(
                        1.0_f64,
                        |least, gain| if *gain < least { *gain } else { least },
                    );
            assert!(deepest >= 0.554_f64, "the burst's need at least: {deepest}");
            // The fast stage releases from the deepest; the slow one from the need of the
            // level, 1 - 0.891/2 (the interpolator's phases have a unit gain at DC), times
            // the share of it it reached over the burst and the window held after it. The
            // gain recovers when both have fallen below 0.01.
            let slow = 0.554_375_f64
                * (1.0_f64
                    - libm::exp(
                        -f64::from(u32::try_from(burst + 74).expect("small")) / 4_800.0_f64,
                    ));
            let fast = libm::log(deepest / 0.01_f64) * fast_ms * 48.0_f64;
            let predicted = larger(fast, libm::log(slow / 0.01_f64) * 4_800.0_f64);
            let recovered = recovery(&measured, start + burst, 0.01_f64);
            let slack = 74.0_f64 + 72.0_f64 + 2.0_f64;
            let recovered = f64::from(u32::try_from(recovered).expect("small"));
            assert!(
                (recovered - predicted).abs() <= slack,
                "adaptivity {adaptivity}, a burst of {burst} frames: recovered in {recovered} frames, {predicted} predicted"
            );
        }
    }
}

/// The time constants, in `S`: each one-pole keeps exp(-1000 / (ms × Hz)) per frame,
/// computed here apart, at every rate of the matrix and at the ends and middle of the release
/// and adaptivity ranges: the fast release (release × (1 - a) + release / 10 × a), the slow
/// stage's attack (1 - its keep, at the release), and the crossfade between raw and
/// protected (20 ms). The control path is binary64 in each precision, so binary32 keeps
/// them to the same bound (a keep rounded to binary32 would move a time constant by up to
/// 2^-25 / (1 - keep), 0.93% at 352.8 kHz and 1,000 ms).
///
/// The bound, 4ε (ε = 2^-52), absolute: each keep is `exp` of `-1 / frames`, the limiter's
/// argument and the test's each within two roundings (2^-52 relative) of it, at most 1/80
/// (10 ms at 8 kHz), so the arguments differ by at most 2^-52 / 80 and their `exp`, at most 1
/// ulp each (`libm`, 2^-53 below 1), by at most 2^-52 + 2^-58: within 2ε of each other. The
/// keeps lie in [1/2, 1), so `1 - keep` is exact on both sides (Sterbenz): the slow attack is
/// within the same 2ε absolute, where a relative bound on it would be far looser (it is
/// as small as 2.6e-6). 4ε doubles it.
fn keeps_its_time_constants_in_milliseconds_at_every_rate<S: Sample>() {
    let kept = |milliseconds: f64, hz: f64| libm::exp(-1_000.0_f64 / (milliseconds * hz));
    let close = |measured: f64, expected: f64, what: &str| {
        assert!(
            (measured - expected).abs() <= 4.0_f64 * f64::EPSILON,
            "{what}: {measured}, {expected} expected"
        );
    };
    for rate in rates() {
        let hz = f64::from(rate.hz());
        for release in [10.0_f64, 100.0_f64, 1_000.0_f64] {
            for adaptivity in [0.0_f64, 0.5_f64, 1.0_f64] {
                let limiter = TruePeakLimiter::<S>::new(
                    rate,
                    1,
                    value(5.0_f64),
                    super::Detector::Standard,
                    settings(-1.0, 1.5, release, adaptivity),
                    LimiterMode::Protected,
                )
                .expect("a limiter");
                let fast = release * (1.0_f64 - adaptivity) + release / 10.0_f64 * adaptivity;
                let what = format!("{hz} Hz, release {release} ms, adaptivity {adaptivity}");
                close(
                    limiter.keep_fast.raw(),
                    kept(fast, hz),
                    &format!("fast, {what}"),
                );
                close(
                    limiter.slow_attack.raw(),
                    1.0_f64 - kept(release, hz),
                    &format!("slow, {what}"),
                );
                close(
                    limiter.keep_mode.raw(),
                    kept(20.0_f64, hz),
                    &format!("mode, {what}"),
                );
            }
        }
    }
}

#[test]
fn keeps_its_time_constants_in_milliseconds_at_every_rate_in_each_precision() {
    keeps_its_time_constants_in_milliseconds_at_every_rate::<Binary64>();
    keeps_its_time_constants_in_milliseconds_at_every_rate::<Binary32>();
}

/// The gap the slow stage's one-pole leaves to the need `to`, in `S`, driven from the need
/// `from` with the step `attack` until it no longer moves.
///
/// Each step closes the share `attack` of the gap, and the stage stops once the step rounds
/// away, the gap within half an ulp of the reduction over `attack` (`2^-54 / attack` for
/// reductions in [1/2, 1), the only ones the caller drives); from a gap of at most 1, that
/// takes fewer than `ln(2^53) / attack`, about `36.7 / attack`, steps: a stage still moving
/// after `most` steps, `40 / attack` computed by the caller from an independent reference of
/// `attack` (never from the code under test, which a fault could make tiny), never
/// settles, which fails here at once rather than running on.
fn stalled_gap<S: Sample + Raw>(from: f64, to: f64, attack: Binary64, most: f64) -> f64 {
    let (target, step) = (of::<S>(to), S::from_binary64(attack));
    let mut slow = of::<S>(from);
    let mut steps = 0.0_f64;
    loop {
        let next = crate::math::approached(slow, target, step);
        if next == slow {
            return (slow.to_binary64().raw() - target.to_binary64().raw()).abs();
        }
        slow = next;
        steps += 1.0_f64;
        assert!(
            steps <= most,
            "the slow stage still moves after {steps} steps"
        );
    }
}

#[test]
fn releases_its_slow_stage_to_within_a_derived_gap_of_a_falling_need() {
    // The slow stage at its slowest (1,000 ms at 384 kHz, the share of the gap closed per
    // frame `a`, `reference` below), from the need of a level of 12 to that of 9 over a
    // ceiling of -1 dBTP, driven to its fixed point: it stops when the step it would take
    // rounds away, `|gap| × a` within half an ulp of the reduction, so its gap is then at most
    // `2^-54 / a` (the reductions lie in [1/2, 1), whose ulp is 2^-53), and the step's own
    // roundings (three, relative) widen it by less than a part in 2^50: a gap within
    // `bound`, `2^-52 / a`, twice that and more. The control path runs in binary64 in each
    // precision (decision 12): the same stage in binary32 stalls near `2^-25 / a` (its ulp
    // there 2^-24), above `stalled_at_least`, `2^-26 / a`, the regression this guards
    // against.
    let rate = SampleRate::new(384_000).expect("a rate");
    let attack = super::step(super::keep(value(1_000.0_f64), rate));
    // The step's reference, independent of the code: `1 - exp(-1 / frames)` for the
    // 384,000 frames of 1,000 ms at 384 kHz, as `-expm1`, exact to an ulp. The code's
    // `keep` is within 3 ε of `exp(-1 / frames)` relatively and `1 - keep` is exact
    // (Sterbenz), so the step it gives is within `3 × 2^-53 / a` of the reference,
    // relatively: checked before the walk, whose bound comes from the reference alone.
    let reference = -libm::expm1(-1.0_f64 / 384_000.0_f64);
    let within = 3.0_f64 * libm::exp2(-53.0_f64) / reference;
    assert!(
        ((attack.raw() - reference) / reference).abs() <= within,
        "the slow stage's step: {} for {reference}",
        attack.raw()
    );
    let most = 40.0_f64 / reference;
    let ceiling = CEILING;
    let (from, to) = (1.0_f64 - ceiling / 12.0_f64, 1.0_f64 - ceiling / 9.0_f64);
    let gap = stalled_gap::<Binary64>(from, to, attack, most);
    let bound = libm::exp2(-52.0_f64) / reference;
    assert!(gap <= bound, "{gap} left, {bound} allowed");
    let stalled = stalled_gap::<Binary32>(from, to, attack, most);
    let stalled_at_least = libm::exp2(-26.0_f64) / reference;
    assert!(
        stalled > stalled_at_least,
        "binary32 stalls: {stalled}, at least {stalled_at_least}"
    );
}

#[test]
fn releases_the_slow_stage_of_a_binary32_limiter_in_binary64() {
    // The limiter itself, in binary32, its control path in binary64 (decision 12): its slow
    // stage, at 8 kHz and the shortest release (10 ms, 80 frames, its share of the gap per
    // frame `a` about 0.0124, `reference` below), driven up by a sustained peak of 12, then
    // down by one of 9, stalls within `2^-52 / a` of the need it holds, the bound of the
    // walk above; a slow stage in binary32 stalls about `2^-25 / a` away, eight orders of
    // magnitude further (the walk above shows it). Each sustained peak runs `40 / a` frames
    // (more than the `36.7 / a` that settles it, from the reference alone).
    let rate = SampleRate::new(8_000).expect("a rate");
    let mut limiter = TruePeakLimiter::<Binary32>::new(
        rate,
        1,
        value(0.5_f64),
        super::Detector::Standard,
        settings(-1.0_f64, 0.5_f64, 10.0_f64, 0.5_f64),
        LimiterMode::Protected,
    )
    .expect("a limiter");
    // The stage's type, whatever the limiter's: a slow stage that followed `S` does not
    // compile here.
    let stage: Binary64 = limiter.path.slow;
    assert_eq!(stage, Binary64::ZERO, "no reduction yet");
    let reference = -libm::expm1(-1.0_f64 / 80.0_f64);
    let attack = limiter.slow_attack.raw();
    assert!(
        ((attack - reference) / reference).abs() <= 3.0_f64 * libm::exp2(-53.0_f64) / reference,
        "the slow stage's step: {attack} for {reference}"
    );
    let frames = 3_240_u32;
    assert!(
        f64::from(frames) >= 40.0_f64 / reference,
        "{frames} frames settle it"
    );
    for peak in [12.0_f64, 9.0_f64] {
        for _ in 0..frames {
            let _: Option<Binary32> = limiter.gain(of::<Binary32>(peak));
        }
        let (slow, held) = (limiter.path.slow.raw(), limiter.path.held().raw());
        let bound = libm::exp2(-52.0_f64) / reference;
        assert!(
            (slow - held).abs() <= bound,
            "after a peak of {peak}: the slow stage at {slow}, {held} held, {bound} allowed"
        );
        assert!(held > 0.5_f64, "a reduction of [1/2, 1): {held}");
    }
}

#[test]
fn moves_its_gain_without_a_click_when_a_setting_changes_live() {
    // A constant input of 2.0 above the ceiling; the ceiling changes live from -1 to -6
    // dBTP, then back, then the lookahead from 1.5 to 0.5 ms. A lower ceiling enters the
    // window held and is smoothed by the Hann window: per frame, the gain moves by at most
    // the change of the reduction times the window's largest weight, 2 / (lookahead + 1).
    // A higher one is released by the fast stage: per frame, by at most the reduction
    // times 1 - its keep, 1 - exp(-1 / 2,640) at a release of 100 ms and an adaptivity of
    // 0.5 (55 ms). The reductions: 0.554 at -1 dBTP, 0.749 at -6.
    let lower = settings(-6.0_f64, 1.5_f64, 100.0_f64, 0.5_f64);
    let higher = settings(-1.0_f64, 1.5_f64, 100.0_f64, 0.5_f64);
    let shorter = settings(-1.0_f64, 0.5_f64, 100.0_f64, 0.5_f64);
    let measured = gains(
        higher,
        96_000,
        |_| 2.0_f64,
        |frame, limiter| {
            let set = match frame {
                24_000 => Some(lower),
                48_000 => Some(higher),
                72_000 => Some(shorter),
                _ => None,
            };
            if let Some(settings) = set {
                limiter.set(settings).expect("within its ranges");
            }
        },
    );
    let attack = (0.749_f64 - 0.554_f64) * 2.0_f64 / 73.0_f64;
    let release = 0.749_f64 * (1.0_f64 - libm::exp(-1.0_f64 / 2_640.0_f64));
    let bound = larger(attack, release) * (1.0_f64 + 1e-6_f64);
    // From a frame after the start, when the first need has entered.
    for (frame, pair) in measured.windows(2).enumerate().skip(400) {
        assert!(
            (pair[1] - pair[0]).abs() <= bound,
            "a step of {} at frame {frame}",
            pair[1] - pair[0]
        );
    }
}
