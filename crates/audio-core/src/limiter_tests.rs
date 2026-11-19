//! Tests of the true-peak limiter.

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use super::{LimiterError, LimiterSettings, Setting, TruePeakLimiter};
use crate::LimiterMode;
use crate::host::Raw;
use crate::tests::{
    exact_steady, hann_peak, hann_sums_bounds, larger, margin_of, rounding_of, weight_rounding,
    weights_apart,
};
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
            held: self.held,
            release_from: self.release_from,
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
/// half a frame; a duration that is not a number, negative or past what a `u32` counts
/// refused before any count. Its twin of the same name, `audio_testkit::frames_of_duration`,
/// which every test outside this crate uses, is a copy (this crate cannot depend on that
/// one): a fix of either is made to both.
fn frames_of_duration(milliseconds: f64, rate: SampleRate) -> usize {
    let exact = milliseconds * f64::from(rate.hz()) / 1_000.0_f64;
    assert!(
        exact >= 0.0_f64 && exact <= f64::from(u32::MAX),
        "a duration of {milliseconds} ms at {} Hz: not a number, negative or past the count",
        rate.hz()
    );
    // Every count whose next half frame is still within the duration: the count itself.
    (0_u32..=u32::MAX)
        .take_while(|count| f64::from(*count) + 0.5_f64 <= exact)
        .count()
        .max(1)
}

/// The latency of a limiter with `maximum` milliseconds of maximum lookahead and `detector`
/// at `rate`, counted apart: the maximum in frames ([`frames_of_duration`]) and the detector's delay,
/// half its taps, as literals (`delays_by_its_detector_and_maximum_lookahead` pins them). A
/// test sizes its work from a limiter's latency only once it equals this.
fn latency_of(maximum: f64, rate: SampleRate, detector: super::Detector) -> usize {
    let delay = match detector {
        super::Detector::Standard => 6,
        super::Detector::Precise => 16,
        super::Detector::Mastering => 64,
    };
    frames_of_duration(maximum, rate).saturating_add(delay)
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
        .saturating_add(frames_of_duration(maximum, rate).saturating_mul(3))
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
    let (expected, within) =
        exact_steady(ceiling, detector, frames_of_duration(maximum, rate), level);
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
        assert_eq!(
            frames_of_duration(milliseconds, rate),
            frames,
            "{milliseconds} ms"
        );
    }
}

#[test]
#[should_panic(expected = "a duration of NaN ms at 48000 Hz")]
fn refuses_a_duration_that_is_not_a_number() {
    let _: usize = frames_of_duration(f64::NAN, SampleRate::new(48_000).expect("a rate"));
}

#[test]
#[should_panic(expected = "a duration of -1 ms at 48000 Hz")]
fn refuses_a_negative_duration() {
    let _: usize = frames_of_duration(-1.0_f64, SampleRate::new(48_000).expect("a rate"));
}

#[test]
#[should_panic(expected = "a duration of 100000000000 ms at 48000 Hz")]
fn refuses_a_duration_past_the_count() {
    // 4.8·10^12 frames, past the 2^32 - 1 a `u32` counts.
    let _: usize = frames_of_duration(1e11_f64, SampleRate::new(48_000).expect("a rate"));
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
                frames_of_duration(maximum, rate) + 6,
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
    let window = frames_of_duration(run.lookahead, run.rate);
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
        margin_of(frames_of_duration(run.maximum, run.rate)) * run.level <= run.ceiling.1 / 8.0_f64;
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
/// shape at every level. Where the margin leaves a gain to hold, the output is not silenced, and a
/// constant above the signal then settles at its exact gain.
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
/// lookahead (and lookahead) and the default ceiling, given a step from silence to the
/// constant `level`, past its latency and two windows: the output then, in binary64. Not a
/// steady output: the interpolator's ringing on the step holds a larger need through the
/// release, by a factor of the detector's reading, the same at every level. Its run is
/// sized by the latency counted apart ([`latency_of`]), asserted first.
fn output_two_windows_past_the_latency<S: Sample>(
    rate: SampleRate,
    maximum: f64,
    level: f64,
) -> f64 {
    let mut limiter = TruePeakLimiter::<S>::new(
        rate,
        1,
        value(maximum),
        super::Detector::Standard,
        settings(-1.0_f64, maximum, 100.0_f64, 0.5_f64),
        LimiterMode::Protected,
    )
    .expect("a limiter");
    let latency = latency_of(maximum, rate, super::Detector::Standard);
    assert_eq!(limiter.latency(), latency, "counted apart");
    let frames = latency
        .saturating_add(frames_of_duration(maximum, rate).saturating_mul(2))
        .saturating_add(BLOCK_SIZE);
    let output = limited(
        &mut limiter,
        &signal::<S>(frames.div_ceil(BLOCK_SIZE), 1, |_, _| of::<S>(level)),
    );
    let last = output.len().saturating_mul(BLOCK_SIZE).saturating_sub(1);
    at(&output, last, 0).to_binary64().raw()
}

/// The margin's cost, in `S`, at the ends of the rates and windows (8 kHz and 0.5 ms, 384 kHz
/// and 5 ms): a level `p` comes out at `c/g - p·E` (`g` the detector's reading of it, its
/// step's ringing included, the same at every level, so that it cancels in the difference of
/// two levels; [`output_two_windows_past_the_latency`]), so a tenth of `c/E` comes out `p·E`, a
/// tenth of the ceiling, below a level of `10c`, within 1% (the roundings, binary32's included,
/// below 2^-20 of the ceiling); and four times `c/E` is silenced, exactly: its need is at least
/// `1 + 3E/4`, past one by more than the control path's rounding `ρ ≤ (43M + 3)·2^-53`, under
/// `3E/4` (`64M` against `43M + 3`, `M ≥ 4`), so its gain is zero.
fn lowers_its_output_by_the_margin_times_the_level_then_silences_it<S: Sample>() {
    let ceiling = CEILING;
    for (hz, maximum) in [(8_000_u32, 0.5_f64), (384_000_u32, 5.0_f64)] {
        let rate = SampleRate::new(hz).expect("a rate");
        let margin = margin_of(frames_of_duration(maximum, rate));
        let quiet = of::<S>(10.0_f64 * ceiling).to_binary64().raw();
        let loud = of::<S>(0.1_f64 * ceiling / margin).to_binary64().raw();
        let lost = output_two_windows_past_the_latency::<S>(rate, maximum, quiet)
            - output_two_windows_past_the_latency::<S>(rate, maximum, loud);
        let expected = (loud - quiet) * margin;
        assert!(
            (lost - expected).abs() <= 0.01_f64 * expected,
            "{hz} Hz: {lost} lost, {expected} expected"
        );
        let silenced =
            output_two_windows_past_the_latency::<S>(rate, maximum, 4.0_f64 * ceiling / margin);
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
                let span = frames_of_duration(lookahead, rate) + 2;
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

/// The ceiling held, in `S`, at every rate and at maxima of 5 and 1 ms (rebuilt in one frame
/// at 8 and 11.025 kHz), while the lookahead changes live: each new one
/// set once the change before it is over, and while one runs, rising toward a longer
/// window or shrinking under the old path beside a shorter one (a set then waits for it). On
/// two channels of noise limited near the ceiling, every segment between two settings is
/// at or below it, with no slack, and above half of it, so that a stage silenced by a
/// change fails.
fn holds_the_ceiling_when_its_lookahead_changes_live<S: Sample + Raw>() {
    let mut draws = Noise::new();
    let ceiling = CEILING;
    // Each maximum, with a lookahead within it, a shorter one and one between them.
    for (most, short, between) in [(5.0_f64, 0.5_f64, 1.5_f64), (1.0_f64, 0.5_f64, 0.75_f64)] {
        for rate in rates() {
            let mut limiter = TruePeakLimiter::<S>::new(
                rate,
                2,
                value(most),
                super::Detector::Standard,
                settings(-1.0_f64, most, 100.0_f64, 0.5_f64),
                LimiterMode::Protected,
            )
            .expect("a limiter");
            let latency = limiter.latency();
            assert_eq!(
                latency,
                latency_of(most, rate, super::Detector::Standard),
                "counted apart"
            );
            let longest = frames_of_duration(most, rate);
            // Each lookahead and the frames run after it is set: past a whole change and four
            // windows; one block (within the change it starts, or the one running); and a window
            // and a half of the longest (within a growth or the shrink after it).
            let settled = latency.saturating_add(longest.saturating_mul(4));
            let schedule = [
                (most, settled),
                (short, BLOCK_SIZE),
                (most * 0.6_f64, BLOCK_SIZE),
                (between, settled),
                (most, longest.saturating_add(longest.div_euclid(2))),
                (short, settled),
                (between, BLOCK_SIZE),
                (most, settled),
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
                    "{peak} at {} Hz, {lookahead} of {most} ms",
                    rate.hz()
                );
                assert!(
                    peak > ceiling * 0.5_f64,
                    "{peak} at {} Hz, {lookahead} of {most} ms: limited, not silenced",
                    rate.hz()
                );
                // A copy, then a constant above every need of the noise: its exact gain, whether
                // a change still runs or not.
                settles_at_its_exact_gain(
                    &mut limiter.clone(),
                    2,
                    (rate, super::Detector::Standard, most),
                    ceiling,
                    16.0_f64,
                    &format!("{} Hz, {lookahead} of {most} ms", rate.hz()),
                );
                skip = 0;
            }
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

/// A limiter of one channel at `rate`, a maximum lookahead of `most` and the lookahead
/// `from` milliseconds, with the default ceiling, release and adaptivity: fresh, or `used`,
/// after a change to another lookahead (0.5 ms, or the maximum from half of it and below)
/// and back, each run to its end
/// on loud noise from `draws`, so that both its paths hold releases of an earlier life (a
/// fresh limiter's second path has a ring of zeros). Each change ends within the frame it
/// begins after, its rebuild, 16 maximum windows of rise or shrink (the warm-start tests'
/// own limit) and a descent ([`descent_frames`]), counted apart.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "frame counts of a few thousand, far from overflow"
)]
fn limiter_at(
    rate: SampleRate,
    most: f64,
    from: f64,
    used: bool,
    draws: &mut Noise,
) -> TruePeakLimiter<Binary64> {
    let mut limiter = TruePeakLimiter::<Binary64>::new(
        rate,
        1,
        value(most),
        super::Detector::Standard,
        settings(-1.0_f64, from, 100.0_f64, 0.5_f64),
        LimiterMode::Protected,
    )
    .expect("a limiter");
    if used {
        let maximum = frames_of_duration(most, rate);
        let cap = rebuild_frames_of(maximum) + 17 * maximum + 1 + descent_frames(maximum);
        let other = if from > most / 2.0_f64 { 0.5_f64 } else { most };
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
/// the rebuild, on the peaks `peak` gives: the change asked for, the frame it begins after,
/// then the rebuild's frames, as many as [`rebuild_frames_of`] counts at the `maximum` given,
/// each giving the gain of the copy, bit for bit, with the old path in force alone; then the
/// change started.
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
    assert_eq!(
        limiter.change,
        Some(super::Stage::Pending),
        "{context}: asked for"
    );
    let first = peak();
    assert_eq!(
        gain_of(limiter, first).to_bits(),
        gain_of(unchanged, first).to_bits(),
        "{context}, the frame it begins after: the old path's gain"
    );
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
    // rebuilt over its frames, and a copy taken on the frame its change begins (the one after
    // the set) whose rebuild is left undone through them, then done at once on the last of
    // them (`finish_rebuild` on the frame computed last), give the same gain on every frame,
    // bit for bit, the old path's through the rebuild, and stand at the same stage of the
    // change after it, through the whole change and after it (eight maximum windows); at
    // maxima of 5 and 1 ms, the latter rebuilt in one frame at 8 and 11.025 kHz. Growths set
    // as soon as a shrink to 0.5 ms is over, while the path it put in force has lived less
    // than the new window, wait for that life past their rebuild's frames: `new - short -
    // after - 1` frames, `after` the frames from the takeover to the end of the shrink,
    // counted by the test, and the frame the growth begins after (decision 12), so that the
    // rebuild through a wait gives the same bits too.
    let mut draws = Noise::new();
    let (mut in_one_frame, mut waits) = (0_u32, 0_u32);
    for rate in rates() {
        for used in [false, true] {
            for (most, from, to, soon) in [
                (5.0_f64, 0.5_f64, 5.0_f64, false),
                (5.0_f64, 5.0_f64, 0.5_f64, false),
                (5.0_f64, 1.5_f64, 3.0_f64, false),
                (5.0_f64, 3.0_f64, 1.5_f64, false),
                (5.0_f64, 4.9_f64, 5.0_f64, false),
                (5.0_f64, 5.0_f64, 4.9_f64, false),
                (1.0_f64, 0.5_f64, 1.0_f64, false),
                (1.0_f64, 1.0_f64, 0.5_f64, false),
                (5.0_f64, 1.0_f64, 5.0_f64, true),
                (1.0_f64, 1.0_f64, 1.0_f64, true),
            ] {
                let maximum = frames_of_duration(most, rate);
                let rebuild = rebuild_frames_of(maximum);
                in_one_frame += u32::from(rebuild == 1);
                let context = format!(
                    "{} Hz, {from} to {to} of {most} ms, used {used}, soon {soon}",
                    rate.hz()
                );
                let mut spread = limiter_at(rate, most, from, used, &mut draws);
                for _ in 0..2 * maximum {
                    let _: f64 = gain_of(&mut spread, 4.0_f64 * draws.next().abs());
                }
                // The frames the rebuild lasts: its own, or a growth's wait.
                let lasts = if soon {
                    let (short, new) = (
                        frames_of_duration(0.5_f64, rate),
                        frames_of_duration(to, rate),
                    );
                    spread
                        .set(settings(-1.0_f64, 0.5_f64, 100.0_f64, 0.5_f64))
                        .expect("within the maximum");
                    for _ in 0..=rebuild {
                        let _: f64 = gain_of(&mut spread, 4.0_f64 * draws.next().abs());
                    }
                    assert_eq!(spread.path.lookahead, short, "{context}: taken over");
                    let mut after = 0;
                    while spread.change.is_some() {
                        let _: f64 = gain_of(&mut spread, 4.0_f64 * draws.next().abs());
                        after += 1;
                        assert!(after <= 16 * maximum, "{context}: still shrinking");
                    }
                    let wait = (new - short).saturating_sub(after + 1);
                    waits += u32::from(wait > rebuild);
                    rebuild.max(wait)
                } else {
                    rebuild
                };
                spread
                    .set(settings(-1.0_f64, to, 100.0_f64, 0.5_f64))
                    .expect("within the maximum");
                assert_eq!(
                    spread.rebuild_frames, rebuild,
                    "{context}: the rebuild's frames"
                );
                let mut once = spread.clone();
                let first = 4.0_f64 * draws.next().abs();
                assert_eq!(
                    gain_of(&mut spread, first).to_bits(),
                    gain_of(&mut once, first).to_bits(),
                    "{context}, the frame it begins after"
                );
                assert_eq!(
                    spread.change,
                    Some(super::Stage::Rebuilding(lasts)),
                    "{context}: begun"
                );
                // Its rebuild left undone until its last frame.
                once.change = None;
                for frame in 0..lasts {
                    let noise = 4.0_f64 * draws.next().abs();
                    assert_eq!(
                        gain_of(&mut spread, noise).to_bits(),
                        gain_of(&mut once, noise).to_bits(),
                        "{context}, frame {frame} of the rebuild"
                    );
                }
                // Rebuilt at once: the whole rebuild on the frame computed last.
                once.finish_rebuild();
                assert_eq!(spread.change, once.change, "{context}: the change started");
                assert!(
                    !matches!(spread.change, Some(super::Stage::Rebuilding(_))),
                    "{context}: started"
                );
                for frame in 0..8 * maximum {
                    let noise = 4.0_f64 * draws.next().abs();
                    assert_eq!(
                        gain_of(&mut spread, noise).to_bits(),
                        gain_of(&mut once, noise).to_bits(),
                        "{context}, frame {frame} of the change"
                    );
                    assert_eq!(spread.change, once.change, "{context}, frame {frame}");
                }
                assert_eq!(spread.change, None, "{context}: over");
            }
        }
    }
    assert!(in_one_frame > 0, "rebuilds of one frame");
    assert!(waits > 0, "rebuilds through a wait");
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
        let maximum = frames_of_duration(5.0_f64, rate);
        for used in [false, true] {
            for (from, to) in [(0.5_f64, 5.0_f64), (1.5_f64, 3.0_f64), (4.9_f64, 5.0_f64)] {
                let new = frames_of_duration(to, rate);
                let context = format!("{} Hz, {from} to {to} ms, used {used}", rate.hz());
                let mut limiter = limiter_at(rate, 5.0_f64, from, used, &mut draws);
                for frame in 0..4 * maximum + 64 {
                    let peak = if frame < 10 { 2.0_f64 } else { 0.5_f64 };
                    let _: f64 = gain_of(&mut limiter, peak);
                }
                limiter
                    .set(settings(-1.0_f64, to, 100.0_f64, 0.5_f64))
                    .expect("within the maximum");
                for _ in 0..=rebuild_frames_of(maximum) {
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
/// `2π (k + 1) / (L + 1)` at each position `k` (libm), within the bounds derived for sums
/// exact at most `L - 1` frames before, the reference's own error included
/// ([`hann_sums_bounds`]). `seen` is at least every value the sums carried since they were exact,
/// the window and the `L - 1` values before it ([`hann_sums_bounds`]'s `V`), which the
/// caller keeps as it runs, since the ring holds `maximum` values only.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "window lengths of at most 1,920 frames and reductions below two, far from overflow"
)]
fn sums_of_its_window(path: &super::Path, seen: f64, context: &str) {
    let length = path.lookahead;
    let span = f64::from(u32::try_from(length + 1).expect("small"));
    let window: Vec<f64> = (0..length)
        .map(|back| path.released_before(back).raw())
        .collect();
    let largest = window.iter().copied().fold(seen, larger);
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
    let (plain, turning) = hann_sums_bounds(length, largest);
    for (got, wanted, bound) in [
        (path.sums.sum.raw(), exact.0, plain),
        (path.sums.real.raw(), exact.1, turning),
        (path.sums.imaginary.raw(), exact.2, turning),
    ] {
        assert!(
            (got - wanted).abs() <= bound,
            "{context}: {got} for {wanted}, {bound} allowed"
        );
    }
}

#[test]
fn carries_the_turning_sums_within_their_factor_up_to_the_longest_window() {
    // The precondition of `hann_sums_bounds`' factor `1 + 2^-37`: `29(L - 1) < 2^16` for the
    // longest window, the largest maximum lookahead (5 ms) at the highest rate of the matrix,
    // in integers.
    let longest = rates()
        .into_iter()
        .map(|rate| frames_of_duration(5.0_f64, rate))
        .max()
        .expect("rates");
    assert!(29 * (longest - 1) < 1_usize << 16_u32, "{longest} frames");
}

#[test]
fn keeps_the_hann_sums_of_each_window_through_a_change() {
    // At every rate, growing and shrinking, on loud noise: on every frame of the rebuild, of
    // the change and of eight maximum windows after it, the sums of the path in force are
    // those of its window (`sums_of_its_window`), and so are the old path's while it runs
    // beside it (rising, shrinking or descending), but on the rebuild's last frame, whose new
    // path's sums replace its others on the next; a reduction added at another position puts
    // them apart.
    let mut draws = Noise::new();
    for rate in rates() {
        let maximum = frames_of_duration(5.0_f64, rate);
        for used in [false, true] {
            for (from, to) in [(0.5_f64, 5.0_f64), (5.0_f64, 0.5_f64), (5.0_f64, 4.9_f64)] {
                let context = format!("{} Hz, {from} to {to} ms, used {used}", rate.hz());
                let mut limiter = limiter_at(rate, 5.0_f64, from, used, &mut draws);
                for _ in 0..2 * maximum {
                    let _: f64 = gain_of(&mut limiter, 4.0_f64 * draws.next().abs());
                }
                // The largest value either path's sums carried since they were last exact: the
                // whole of each ring when the change is set, at least the `2L - 1` values back
                // where `2L - 1` is at most the maximum, then every value released since.
                let ring = |path: &super::Path| {
                    path.released
                        .iter()
                        .fold(0.0_f64, |most, value| larger(most, value.raw()))
                };
                let mut seen = larger(ring(&limiter.path), ring(&limiter.previous));
                limiter
                    .set(settings(-1.0_f64, to, 100.0_f64, 0.5_f64))
                    .expect("within the maximum");
                for frame in 0..10 * maximum {
                    // A release after a third of the run.
                    let peak = if frame < 3 * maximum {
                        4.0_f64 * draws.next().abs()
                    } else {
                        0.5_f64
                    };
                    let rebuilding = matches!(limiter.change, Some(super::Stage::Rebuilding(_)));
                    let _: f64 = gain_of(&mut limiter, peak);
                    seen = larger(seen, ring(&limiter.path));
                    seen = larger(seen, ring(&limiter.previous));
                    let here = format!("{context}, frame {frame}");
                    // On the rebuild's last frame the new path's sums are those it accumulated,
                    // replacing its others on the next frame, its first in force.
                    let started =
                        rebuilding && !matches!(limiter.change, Some(super::Stage::Rebuilding(_)));
                    if !started {
                        sums_of_its_window(&limiter.path, seen, &here);
                    }
                    if matches!(
                        limiter.change,
                        Some(
                            super::Stage::Rising { .. }
                                | super::Stage::Shrinking { .. }
                                | super::Stage::Descending { .. }
                        )
                    ) {
                        sums_of_its_window(
                            &limiter.previous,
                            seen,
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
fn attacks_a_burst_only_the_old_window_saw_under_the_old_path_when_shrinking() {
    // At every rate, shrinking from 5 ms to 0.5 and 1.5 ms while a burst (10 frames of
    // peaks of 2.0 over peaks of 0.5) lies within the old window but beyond the new one when
    // the change starts: the old path has seen it, the new one carries nothing from beyond its
    // own window, and attacks it later and steeper, at the new window's attack, as a limiter
    // always at that window would. The reduction is never above the old path's (which runs
    // on, beside the new one: as a copy left unchanged gives it, the smaller of the two exact,
    // the gain's rounding monotonic) while it shrinks, and every step of the gain, through the rebuild, the
    // change and the burst, is at most the new window's attack of the burst's need,
    // `need × hp(new)` (`spec/measurements.md`, "The true-peak limiter", step 6), plus the
    // rounding its derivation gives for that attack ([`step_slack`]), all computed apart.
    let ceiling = CEILING;
    for rate in rates() {
        let maximum = frames_of_duration(5.0_f64, rate);
        let rebuild = rebuild_frames_of(maximum);
        let need = need_above(2.0_f64, ceiling, margin_of(maximum));
        for to in [0.5_f64, 1.5_f64] {
            let new = frames_of_duration(to, rate);
            let attack = hann_peak(new);
            let allowed = need * attack + step_slack(rounding_of(maximum), 0.0_f64, attack, new);
            let context = format!("{} Hz, 5 to {to} ms", rate.hz());
            // The change's first frame `t` is the set frame plus the rebuild and two; at
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
            for frame in 0..high - rebuild - 1 {
                gains.push(gain_of(&mut limiter, peak(frame)));
                // `t = frame + 2 + rebuild`, from `low` to `high`.
                let Some(first) = (frame + 2 + rebuild).checked_sub(low) else {
                    continue;
                };
                if first % stride != 0 {
                    continue;
                }
                let (mut changed, mut unchanged) = (limiter.clone(), limiter.clone());
                changed
                    .set(settings(-1.0_f64, to, 100.0_f64, 0.5_f64))
                    .expect("within the maximum");
                let mut run = vec![gains[frame]];
                for at in frame + 1..onset + 2 * maximum + 64 {
                    let shrinking = matches!(changed.change, Some(super::Stage::Shrinking { .. }));
                    let (gain, old) = (
                        gain_of(&mut changed, peak(at)),
                        gain_of(&mut unchanged, peak(at)),
                    );
                    assert!(
                        !shrinking || gain >= old,
                        "{context}, set after frame {frame}, frame {at}: a gain of {gain} below \
                         the old path's {old}"
                    );
                    run.push(gain);
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
        let maximum = frames_of_duration(5.0_f64, rate);
        let rebuild = rebuild_frames_of(maximum);
        assert_eq!(rebuild, frames, "{} Hz: as stated", rate.hz());
        for (from, to) in [
            (0.5_f64, 5.0_f64),
            (5.0_f64, 0.5_f64),
            (4.9_f64, 5.0_f64),
            (5.0_f64, 4.9_f64),
        ] {
            let (old, new) = (frames_of_duration(from, rate), frames_of_duration(to, rate));
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
            // The frame it begins after.
            let _: f64 = gain_of(&mut limiter, 2.0_f64);
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
fn runs_a_shorter_lookahead_under_the_old_path_until_its_rebuilt_reductions_have_left() {
    // At every rate, shrinking on loud noise whose needs differ from frame to frame: after
    // the rebuild (`rebuilt_beside`), the change keeps the reduction at most the old path's
    // (which runs on, beside it: as a copy left unchanged gives it, so no gain is below that
    // copy's, the smaller of the two exact, the gain's rounding monotonic) and at least the
    // new path's (each a mean of the same needs) for as long as the new window holds a
    // reduction it rebuilt, its `new - 1` counted down frame by frame, then until the level
    // falling from the reduction in force has reached the new path; then it is over, or,
    // the new path above the old one, it rises to it and descends, as a growth does, within
    // 16 maximum windows (the test's own limit).
    let mut draws = Noise::new();
    for rate in rates() {
        for used in [false, true] {
            for (from, to) in [(5.0_f64, 0.5_f64), (1.5_f64, 0.5_f64), (5.0_f64, 4.9_f64)] {
                let (old, new) = (frames_of_duration(from, rate), frames_of_duration(to, rate));
                let context = format!("{} Hz, {from} to {to} ms, used {used}", rate.hz());
                let mut limiter = limiter_at(rate, 5.0_f64, from, used, &mut draws);
                let maximum = frames_of_duration(5.0_f64, rate);
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
                let mut frame = 0;
                while let Some(super::Stage::Shrinking { scaling, .. }) = limiter.change {
                    assert_eq!(
                        scaling,
                        (new - 1).saturating_sub(frame),
                        "{context}, frame {frame}: the rebuilt reductions left"
                    );
                    let peak = 4.0_f64 * draws.next().abs();
                    let gain = gain_of(&mut limiter, peak);
                    let old_gain = gain_of(&mut unchanged, peak);
                    assert!(
                        gain >= old_gain,
                        "{context}, frame {frame}: a gain of {gain} below the old path's {old_gain}"
                    );
                    // At least the smaller of the two paths' reductions, both gains within
                    // `u/2` of `1 - r` and the test's differences within `u/2` each.
                    let least = smaller_of(1.0_f64 - old_gain, limiter.path.smoothed().raw());
                    assert!(
                        1.0_f64 - gain >= least - 2.0_f64 * libm::exp2(-53.0_f64),
                        "{context}, frame {frame}: a reduction of {} below both paths' {least}",
                        1.0_f64 - gain
                    );
                    frame += 1;
                    assert!(frame <= 16 * maximum, "{context}: still shrinking");
                }
                assert!(
                    frame >= (new - 1).max(1),
                    "{context}: shrank {frame} frames"
                );
                let mut after = 0;
                while limiter.change.is_some() {
                    assert!(
                        matches!(
                            limiter.change,
                            Some(super::Stage::Rising { .. } | super::Stage::Descending { .. })
                        ),
                        "{context}: {:?} after the shrink",
                        limiter.change
                    );
                    let _: f64 = gain_of(&mut limiter, 4.0_f64 * draws.next().abs());
                    after += 1;
                    assert!(after <= 16 * maximum, "{context}: still changing");
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
    // descent, which glides over the gap to the new path in the frames between the two
    // windows or falls by the new window's attack, lasts at most those frames and
    // `descent_frames` of the new window before the new path is in force alone. The rise
    // ends on noise within 16 maximum windows (the test's own limit).
    let mut draws = Noise::new();
    for rate in rates() {
        for used in [false, true] {
            for (from, to) in [(0.5_f64, 5.0_f64), (1.5_f64, 3.0_f64), (4.9_f64, 5.0_f64)] {
                let (old, new) = (frames_of_duration(from, rate), frames_of_duration(to, rate));
                let maximum = frames_of_duration(5.0_f64, rate);
                let context = format!("{} Hz, {from} to {to} ms, used {used}", rate.hz());
                let mut limiter = limiter_at(rate, 5.0_f64, from, used, &mut draws);
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
                // Each within the rounding of both computations ([`weights_apart`]).
                for length in [1, old, new, maximum] {
                    assert!(
                        (limiter.peaks[length - 1].raw() - hann_peak(length)).abs()
                            <= weights_apart(length, hann_peak(length)),
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
                        descent <= new - old + descent_frames(new),
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
    // farther than the new window. Within the roundings of both: the limiter's need within
    // `4.0001u` of the tests' (`2.5001u` and `1.5u` of `1 - c/p + E`, the ceiling the same
    // value: [`step_slack`]), each weight within `48u / (D + 1)` of the exact one
    // (`computes_the_largest_hann_weight_of_every_window_up_to_the_longest`), and each
    // product one rounding, far below a weight's change from one `D` to the next.
    for rate in rates() {
        for (most, from, to) in [
            (5.0_f64, 1.5_f64, 5.0_f64),
            (5.0_f64, 1.5_f64, 3.0_f64),
            (1.0_f64, 0.5_f64, 1.0_f64),
            (1.0_f64, 0.5_f64, 0.75_f64),
        ] {
            let maximum = frames_of_duration(most, rate);
            let (old, new) = (frames_of_duration(from, rate), frames_of_duration(to, rate));
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
                // Set after `2 × maximum` frames, the change begun after the next, the rebuild's
                // last frame `c` the `rebuild`-th after that, the need of the frame
                // `c - (maximum - 1 - D)`.
                let last = 2 * maximum + rebuild;
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
                    let within = (4.000_1_f64 * weight + 2.0_f64) * libm::exp2(-53.0_f64)
                        + need * weights_apart(depth, weight);
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
    // At every rate, in silence (no need, every reduction zero): after the frame it begins
    // after and the rebuild's frames (`rebuild_frames_of`), growing, the rise reaches the new
    // path on its first frame (both zero) and the descent from zero ends on the next, so the
    // change is over two frames later; shrinking, as soon as the new window holds no
    // reduction it rebuilt, its `new - 1` frames later (one at least), the level falling
    // from zero nowhere. Each time the gain is none, the input passed through.
    for rate in rates() {
        for (from, to) in [(0.5_f64, 5.0_f64), (5.0_f64, 0.5_f64), (1.5_f64, 3.0_f64)] {
            let rebuild = rebuild_frames_of(frames_of_duration(5.0_f64, rate));
            let new = frames_of_duration(to, rate);
            let frames = 1 + rebuild + if to > from { 2 } else { (new - 1).max(1) };
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
            // Past the maximum, so that no shrink waits for the release set when it was built.
            for _ in 0..2 * frames_of_duration(5.0_f64, rate) {
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
                frames_of_duration(to, rate),
                "{context}: in force"
            );
        }
    }
}

/// One part of a limiter's state, as [`each_stored`] gives it.
enum Stored<'a> {
    /// A value it stores through `math::kept`, or derives only from such values by an exact
    /// maximum or minimum: zero or at least `2^-100` (`math::LEAST_KEPT`). The stage's
    /// values, the release stages, the rebuild's cap and fast stage, the share applied and
    /// what is left of the way.
    Kept(Binary64),
    /// A ring of such values and the index it writes next: the needs, and each path's
    /// released reductions.
    KeptRing(&'a [Binary64], usize),
    /// A path's hold: the step each need entered on, and the need, the newest need when it
    /// was written.
    Hold(&'a [(u64, Binary64)]),
    /// A value derived from kept ones and proven zero or normal (each path's Hann sums, in
    /// force and accumulated: `spec/measurements.md`, "The true-peak limiter", step 7), or one
    /// fixed when it is built, set or a path prepared (a path's turn, scale and peak, the
    /// ceiling, the margin, the release's and the crossfade's shares, the settings).
    Derived(Binary64),
    /// The largest Hann weight of every window, fixed when it is built.
    Peaks(&'a [Binary64]),
    /// A path's phasor, fixed when the path is prepared
    /// (`turns_no_phasor_part_below_two_to_the_minus_55_up_to_the_longest_window`).
    Phasor(&'a [(Binary64, Binary64)]),
    /// A count, a position, a step or a flag.
    Count(u64),
}

/// Gives `visit` every part of the state of `limiter` ([`Stored`]), listed from one
/// destructuring of it, its paths, its rebuild and its change's stage with no field left
/// out, so that a field added to any of them does not compile until it is listed here: the
/// one list the laws of its values read ([`kept_state`], [`written_values`],
/// [`control_state`]). Its signal path (its interpolator, the history of its input and its
/// delay, values of `S`) apart.
#[expect(
    clippy::too_many_lines,
    reason = "one destructuring of every field of the limiter's control path"
)]
fn each_stored<'a, S: Sample>(limiter: &'a TruePeakLimiter<S>, visit: &mut impl FnMut(Stored<'a>)) {
    let count = |value: usize| Stored::Count(u64::try_from(value).expect("small"));
    let TruePeakLimiter {
        rate: _,
        channels,
        maximum,
        maximum_ms,
        wanted,
        settings,
        taps,
        delay,
        filter: _,
        history: _,
        history_at,
        delayed: _,
        position,
        needed,
        needed_at,
        peaks,
        step,
        ceiling,
        margin,
        keep_fast,
        slow_attack,
        held,
        release_from,
        path,
        previous,
        change,
        rebuild_frames,
        rebuild,
        applied,
        mode,
        protected,
        given_binary32,
        remaining,
        keep_mode,
    } = limiter;
    let super::Rebuild {
        longer,
        run,
        unheld,
        turns,
        terms,
        cap: rebuild_cap,
        fast: rebuild_fast,
        offset,
        partial,
    } = rebuild;
    for value in [*applied, *remaining, *rebuild_cap, *rebuild_fast] {
        visit(Stored::Kept(value));
    }
    visit(Stored::KeptRing(needed, *needed_at));
    for value in [
        *maximum_ms,
        settings.ceiling(),
        settings.lookahead(),
        settings.release(),
        settings.adaptivity(),
        *ceiling,
        *margin,
        *keep_fast,
        *slow_attack,
        *keep_mode,
    ] {
        visit(Stored::Derived(value));
    }
    match *held {
        Some((fast, slow)) => {
            visit(Stored::Count(1));
            visit(Stored::Derived(fast));
            visit(Stored::Derived(slow));
        }
        None => visit(Stored::Count(0)),
    }
    for value in [partial.sum, partial.real, partial.imaginary] {
        visit(Stored::Derived(value));
    }
    visit(Stored::Peaks(peaks));
    for value in [
        *offset,
        *maximum,
        *wanted,
        *taps,
        *delay,
        *history_at,
        *position,
        *needed_at,
        *rebuild_frames,
        *run,
        *unheld,
        *turns,
        *terms,
    ] {
        visit(count(value));
    }
    for flag in [
        *longer,
        *mode == LimiterMode::Protected,
        *protected,
        *given_binary32,
    ] {
        visit(Stored::Count(u64::from(flag)));
    }
    visit(Stored::Count(u64::from(*channels)));
    visit(Stored::Count(*step));
    visit(Stored::Count(*release_from));
    let glided = |glide: Option<super::Glide>, seen: &mut dyn FnMut(Stored<'a>)| match glide {
        Some(super::Glide {
            step: glide_step,
            last,
        }) => {
            seen(Stored::Count(1));
            seen(Stored::Kept(glide_step));
            seen(Stored::Kept(last));
        }
        None => seen(Stored::Count(0)),
    };
    match *change {
        Some(super::Stage::Pending) => visit(Stored::Count(5)),
        Some(super::Stage::Rebuilding(left)) => {
            visit(Stored::Count(1));
            visit(count(left));
        }
        Some(super::Stage::Rising {
            rising,
            cap,
            budget,
            glide,
        }) => {
            visit(Stored::Count(2));
            for value in [rising, cap, budget] {
                visit(Stored::Kept(value));
            }
            glided(glide, visit);
        }
        Some(super::Stage::Shrinking {
            scaling,
            level,
            fall,
            glide,
        }) => {
            visit(Stored::Count(3));
            visit(count(scaling));
            visit(Stored::Kept(level));
            visit(Stored::Kept(fall));
            glided(glide, visit);
        }
        Some(super::Stage::Descending { level, fall, glide }) => {
            visit(Stored::Count(4));
            visit(Stored::Kept(level));
            visit(Stored::Kept(fall));
            glided(glide, visit);
        }
        None => visit(Stored::Count(0)),
    }
    for each in [path, previous] {
        let super::Path {
            lookahead,
            queue,
            front,
            queued,
            fast,
            slow,
            released,
            released_at,
            taken,
            nonzero,
            sums,
            next,
            swap_in,
            phasor,
            turn,
            scale,
            peak,
        } = each;
        visit(Stored::Kept(*fast));
        visit(Stored::Kept(*slow));
        visit(Stored::KeptRing(released, *released_at));
        visit(Stored::Hold(queue));
        for crate::math::HannSums {
            sum,
            real,
            imaginary,
        } in [sums, next]
        {
            for value in [*sum, *real, *imaginary] {
                visit(Stored::Derived(value));
            }
        }
        for value in [turn.0, turn.1, *scale, *peak] {
            visit(Stored::Derived(value));
        }
        visit(Stored::Phasor(phasor));
        for value in [
            *lookahead,
            *front,
            *queued,
            *released_at,
            *nonzero,
            *swap_in,
        ] {
            visit(count(value));
        }
        visit(Stored::Count(taken.unwrap_or(u64::MAX)));
    }
}

/// Whether `value` is zero or normal: what a host that flushes subnormal numbers to zero
/// reads as the same number.
fn zero_or_normal(value: Binary64) -> bool {
    value.raw() == 0.0_f64 || value.raw().abs() >= f64::MIN_POSITIVE
}

/// Whether `value` is zero or at least `2^-533` in magnitude, the least the control path
/// computes by the derivation of `spec/measurements.md` ("The true-peak limiter", step 7),
/// written apart: far above the subnormal numbers, so that no host reads it as another.
fn zero_or_above_the_least_computed(value: Binary64) -> bool {
    value.raw() == 0.0_f64 || value.raw().abs() >= libm::exp2(-533.0_f64)
}

/// Whether `value` is zero or at least `2^-100`, the least value the control path keeps
/// (`math::LEAST_KEPT`), written apart.
fn zero_or_kept(value: Binary64) -> bool {
    value.raw() == 0.0_f64 || value.raw().abs() >= libm::exp2(-100.0_f64)
}

/// Checks that every value `limiter` stores through `math::kept` ([`Stored::Kept`], its
/// rings and its holds) is zero or at least `2^-100` ([`zero_or_kept`]): none that a host
/// flushing subnormal numbers would read as zero, stalling a stage or miscounting a window's
/// non-zero reductions, nor any from which it could compute one.
fn kept_state(limiter: &TruePeakLimiter<Binary64>, context: &str) {
    let check = |value: Binary64| {
        assert!(
            zero_or_kept(value),
            "{context}: {value:?} stored, below the least value kept, at {:?}",
            limiter.change
        );
    };
    each_stored(limiter, &mut |part| match part {
        Stored::Kept(value) => check(value),
        Stored::KeptRing(ring, _) => ring.iter().copied().for_each(check),
        Stored::Hold(hold) => hold.iter().for_each(|(_, need)| check(*need)),
        Stored::Derived(_) | Stored::Peaks(_) | Stored::Phasor(_) | Stored::Count(_) => {}
    });
}

#[test]
fn keeps_every_value_of_a_change_zero_or_kept_in_a_release_tail() {
    // At every rate, growing and shrinking while the release stages hold `2^-98` (the tail
    // of a long release, the slowest, so that it stays there through the change): each step
    // of a stage, the reduction times the largest Hann weight of the longer window (the
    // rise's, the new one growing, and the descent's after a freeze, the old one shrinking:
    // 24 frames or more in these pairs, its weight below a quarter, asserted from the
    // limiter's own table), is below `2^-100`, the least value the control path keeps.
    // Every value the change stores must be zero or at least that on every frame, so that no
    // host computes a subnormal number from it (`spec/engine-design.md`, decision 11), and the
    // change over within its bounds (`constant_change_frames`, a growth's rise reaching the
    // new path at once on such a reduction).
    for rate in rates() {
        let maximum = frames_of_duration(5.0_f64, rate);
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
            let tail = Binary64::new(libm::exp2(-98.0_f64));
            (limiter.path.fast, limiter.path.slow) = (tail, tail);
            for _ in 0..2 * maximum {
                let _: f64 = gain_of(&mut limiter, 0.5_f64);
            }
            assert!(
                limiter.path.smoothed().raw() >= libm::exp2(-100.0_f64),
                "{context}: a reduction in force"
            );
            limiter.set(made(to)).expect("within the maximum");
            let (old, new) = (frames_of_duration(from, rate), frames_of_duration(to, rate));
            let weight = limiter
                .peaks
                .get(old.max(new) - 1)
                .expect("a window up to the maximum")
                .raw();
            assert!(weight < 0.25_f64, "{context}: a weight of {weight}");
            let cap = constant_change_frames(maximum, old, new);
            let mut frames = 0;
            while limiter.change.is_some() {
                let _: f64 = gain_of(&mut limiter, 0.5_f64);
                kept_state(&limiter, &format!("{context}, frame {frames}"));
                frames += 1;
                assert!(frames <= cap, "{context}: still changing");
            }
        }
    }
}

/// Each value a frame of a change stores through `math::kept`, as [`stored_by_the_design`]
/// names it: the stage begun from the reduction in force (growing, the rise; shrinking, the
/// level and its fall), and on each frame the rise, the descent's level and fall where the
/// rise reaches the new path, a glide's step and the reduction it follows, a shrink's and a
/// descent's level, and the rise a shrink ends in.
const SITES: [&str; 12] = [
    "rise from the reduction in force",
    "level from the reduction in force",
    "fall from the reduction in force",
    "rise",
    "level where the rise ends",
    "fall where the rise ends",
    "glide's step",
    "reduction a glide follows",
    "shrink's level",
    "descent's level",
    "rise a shrink ends in",
    "cap of the rise a shrink ends in",
];

/// The least value the control path keeps, `2^-100`, and below it zero, written apart.
fn kept_apart(value: f64) -> f64 {
    if value < libm::exp2(-100.0_f64) {
        0.0_f64
    } else {
        value
    }
}

/// How far `value` lies above `below`, zero when not above, written apart.
fn excess_apart(value: f64, below: f64) -> f64 {
    if value - below > 0.0_f64 {
        value - below
    } else {
        0.0_f64
    }
}

/// What the paths of a frame of a change give its stage, read by the test after the frame:
/// the new path's reduction, the old path's, the new path's hold, release stages and largest
/// Hann weight, and the frames between the two windows.
struct Frame {
    /// The new path's reduction.
    newest: f64,
    /// The old path's.
    oldest: f64,
    /// The new path's hold.
    held: f64,
    /// Its fast and slow release stages.
    stages: (f64, f64),
    /// Its largest Hann weight.
    peak: f64,
    /// The frames between the two windows, at least one.
    between: usize,
}

/// The stage a frame of a change at `stage` makes, from the paths of that frame `frame`,
/// computed from the design (`spec/engine-design.md`, decision 12) apart from the code, and
/// each value it stores before `math::kept`, with its site ([`SITES`]); none when the change
/// ends on it (a rebuild's last frame apart, whose stage [`stored_by_the_design`] reads).
#[expect(
    clippy::too_many_lines,
    reason = "one arm per stage of a change, as `TruePeakLimiter::changing` has"
)]
fn stored_by_the_design(
    stage: super::Stage,
    frame: &Frame,
    sites: &mut Vec<(usize, f64)>,
) -> Option<super::Stage> {
    let binary = Binary64::new;
    let spread = 1.0_f64 / f64::from(u32::try_from(frame.between).expect("few"));
    let (newest, oldest) = (frame.newest, frame.oldest);
    let mut keep = |site: usize, value: f64| {
        sites.push((site, value));
        binary(kept_apart(value))
    };
    match stage {
        super::Stage::Rising {
            rising,
            cap,
            budget,
            glide,
        } => {
            let cap = larger(cap.raw(), frame.held);
            let (step, last) = glide.map_or_else(
                || {
                    let gap = excess_apart(newest, rising.raw()) * spread;
                    (keep(6, gap).raw(), keep(7, newest).raw())
                },
                |glide| (glide.step.raw(), glide.last.raw()),
            );
            let follow = smaller_of(
                kept_apart(cap * frame.peak),
                kept_apart(excess_apart(newest, last) + step),
            );
            let next = rising.raw() + larger(budget.raw(), follow);
            let rose = if next.to_bits() == rising.raw().to_bits() {
                newest
            } else {
                smaller_of(newest, next)
            };
            let reduction = larger(oldest, rose);
            Some(if rose < newest {
                super::Stage::Rising {
                    rising: keep(3, rose),
                    cap: binary(cap),
                    budget,
                    glide: Some(super::Glide {
                        step: binary(step),
                        last: keep(7, newest),
                    }),
                }
            } else {
                super::Stage::Descending {
                    level: keep(4, reduction),
                    fall: keep(5, reduction * frame.peak),
                    glide: None,
                }
            })
        }
        super::Stage::Shrinking {
            scaling,
            level,
            fall,
            glide,
        } => {
            let gap = keep(6, excess_apart(level.raw(), newest) * spread).raw();
            let (step, last) = glide.map_or_else(
                || (gap, keep(7, oldest).raw()),
                |glide| (larger(glide.step.raw(), gap), glide.last.raw()),
            );
            let lowered =
                level.raw() - smaller_of(fall.raw(), kept_apart(excess_apart(last, oldest) + step));
            let next = keep(8, lowered).raw();
            let level = if next.to_bits() == level.raw().to_bits() {
                0.0_f64
            } else {
                next
            };
            let reduction = smaller_of(oldest, larger(newest, level));
            let scaling = scaling.saturating_sub(1);
            if scaling > 0 || level > newest {
                Some(super::Stage::Shrinking {
                    scaling,
                    level: binary(level),
                    fall,
                    glide: Some(super::Glide {
                        step: binary(step),
                        last: keep(7, oldest),
                    }),
                })
            } else if newest <= oldest {
                None
            } else {
                Some(super::Stage::Rising {
                    rising: keep(10, reduction),
                    cap: keep(
                        11,
                        larger(larger(frame.stages.0, frame.stages.1), frame.held),
                    ),
                    budget: Binary64::ZERO,
                    glide: None,
                })
            }
        }
        super::Stage::Descending { level, fall, glide } => {
            let (step, last) = glide.map_or_else(
                || {
                    let gap = excess_apart(level.raw(), newest) * spread;
                    (keep(6, gap).raw(), keep(7, newest).raw())
                },
                |glide| (glide.step.raw(), glide.last.raw()),
            );
            let lowered =
                level.raw() - smaller_of(fall.raw(), kept_apart(excess_apart(last, newest) + step));
            let next = keep(9, lowered).raw();
            let level = if next.to_bits() == level.raw().to_bits() {
                0.0_f64
            } else {
                next
            };
            (level > newest).then(|| super::Stage::Descending {
                level: binary(level),
                fall,
                glide: Some(super::Glide {
                    step: binary(step),
                    last: keep(7, newest),
                }),
            })
        }
        super::Stage::Pending | super::Stage::Rebuilding(_) => None,
    }
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one law over each store of a change: the tail, the changes and the sites share their state"
)]
fn keeps_every_value_of_a_change_zero_or_kept_at_a_tails_end() {
    // At every rate, growing, shrinking and near-equal, a burst (peaks of 2.0 for twice the
    // maximum), then a real tail (0.25, which needs nothing) released by 10 ms with no
    // adaptivity, run for 80 releases (`e^-80`, below `2^-115`): the change made on a dense
    // stride of the frames where the path's release stages lie in `(0, 2^-90)`, within a few
    // releases of `2^-100` (the least value the control path keeps; the frames found by a
    // first run, up to 48 of them spread over that stretch). On every frame of each change,
    // the stage the limiter stores is the one the design gives from the paths' reductions of
    // the frame, computed apart ([`stored_by_the_design`]), value for value, and every value
    // it stores is zero or at least `2^-100` (`kept_state`). Each site of a store
    // ([`SITES`]) met a value in `(0, 2^-100)`, which it stored as zero, at least once at
    // some rate (counted from the design's values before `math::kept`), but the cap of the
    // rise a shrink ends in, a maximum of values each zero or at least `2^-100` (release
    // stages and a hold), so never in it. A store without its `math::kept` keeps that value
    // (finding T05-1 of the targeted review of the live lookahead change, block 2).
    const TRIED: usize = 96;
    let mut met = [0_u32; SITES.len()];
    for rate in rates() {
        let maximum = frames_of_duration(5.0_f64, rate);
        let burst = 2 * maximum;
        let tail = frames_of_duration(800.0_f64, rate);
        for (from, to) in [
            (0.5_f64, 5.0_f64),
            (5.0_f64, 0.5_f64),
            (1.5_f64, 3.0_f64),
            (3.0_f64, 1.5_f64),
            (5.0_f64, 4.9_f64),
            (1.5_f64, 0.5_f64),
            (0.5_f64, 0.6_f64),
        ] {
            let context = format!("{} Hz, {from} to {to} ms", rate.hz());
            let made = |lookahead: f64| settings(-1.0_f64, lookahead, 10.0_f64, 0.0_f64);
            let built = || {
                TruePeakLimiter::<Binary64>::new(
                    rate,
                    1,
                    value(5.0_f64),
                    super::Detector::Standard,
                    made(from),
                    LimiterMode::Protected,
                )
                .expect("a limiter")
            };
            let peak = |frame: usize| if frame < burst { 2.0_f64 } else { 0.25_f64 };
            // Release stages within a few releases of `2^-100`, or a mean in force below it.
            let near = |limiter: &TruePeakLimiter<Binary64>| {
                let most = larger(limiter.path.fast.raw(), limiter.path.slow.raw());
                let mean = limiter.path.smoothed().raw();
                (most > 0.0_f64 && most < libm::exp2(-85.0_f64))
                    || (mean > 0.0_f64 && mean < libm::exp2(-95.0_f64))
            };
            // The frames of the tail's end, from a first run.
            let mut first = built();
            let ends: Vec<usize> = (0..burst + tail)
                .filter(|frame| {
                    let _: f64 = gain_of(&mut first, peak(*frame));
                    near(&first)
                })
                .collect();
            assert!(
                !ends.is_empty(),
                "{context}: release stages near the least kept"
            );
            let stride = ends.len().div_ceil(TRIED);
            let chosen: Vec<usize> = ends.iter().copied().step_by(stride).collect();
            let (old, new) = (frames_of_duration(from, rate), frames_of_duration(to, rate));
            let cap = constant_change_frames(maximum, old, new);
            let between = old.abs_diff(new).max(1);
            let mut limiter = built();
            for frame in 0..=*chosen.last().expect("a frame") {
                let _: f64 = gain_of(&mut limiter, peak(frame));
                if !chosen.contains(&frame) {
                    continue;
                }
                assert!(near(&limiter), "{context}, frame {frame}: the same run");
                let mut changed = limiter.clone();
                changed.set(made(to)).expect("within the maximum");
                let mut frames = 0;
                while changed.change.is_some() {
                    let stage = changed.change.expect("a change");
                    let _: f64 = gain_of(&mut changed, 0.25_f64);
                    let here = format!("{context}, set after frame {frame}, frame {frames}");
                    kept_values(&changed, &here);
                    let mut sites = Vec::new();
                    if stage == super::Stage::Rebuilding(1) {
                        // The change begun from the reduction in force: the old path's.
                        let before = changed.previous.smoothed().raw();
                        sites.push((usize::from(new <= old), before));
                        match changed.change {
                            Some(super::Stage::Rising { rising, glide, .. }) => {
                                assert_eq!(rising.raw(), kept_apart(before), "{here}: the rise");
                                assert_eq!(glide, None, "{here}");
                            }
                            Some(super::Stage::Shrinking {
                                scaling,
                                level,
                                fall,
                                glide,
                            }) => {
                                let product = before * changed.previous.peak.raw();
                                sites.push((2, product));
                                assert_eq!(
                                    (scaling, level.raw(), fall.raw(), glide),
                                    (new - 1, kept_apart(before), kept_apart(product), None),
                                    "{here}: the shrink"
                                );
                            }
                            other => panic!("{here}: {other:?} begun"),
                        }
                    } else if !matches!(stage, super::Stage::Pending | super::Stage::Rebuilding(_))
                    {
                        let observed = Frame {
                            newest: changed.path.smoothed().raw(),
                            oldest: changed.previous.smoothed().raw(),
                            held: changed.path.held().raw(),
                            stages: (changed.path.fast.raw(), changed.path.slow.raw()),
                            peak: changed.path.peak.raw(),
                            between,
                        };
                        let designed = stored_by_the_design(stage, &observed, &mut sites);
                        match designed {
                            Some(designed) => assert_eq!(changed.change, Some(designed), "{here}"),
                            None => assert_eq!(changed.change, None, "{here}: over"),
                        }
                    }
                    for (site, value) in sites {
                        met[site] += u32::from(value > 0.0_f64 && value < libm::exp2(-100.0_f64));
                    }
                    frames += 1;
                    assert!(frames <= cap, "{context}: still changing");
                }
            }
        }
    }
    // Never in it, derived: the rise stored while it runs is the rise before it plus a step,
    // both zero or at least `2^-100` (the step the larger of the budget and of the smaller of
    // two kept values), the step not zero (a rise that does not move reaches the new path);
    // the cap of the rise a shrink ends in is the largest of the new path's release stages and
    // hold, each zero or at least `2^-100`; and the reduction a rise that runs on follows, the
    // new path's, lies above that rise, so the site of the reduction a glide follows meets
    // such values where a glide begins and where a shrink or a descent stores it. Met by the crafted states of
    // `keeps_the_level_of_a_descent_and_the_rise_a_shrink_ends_in_zero_or_kept`, which no
    // tail reaches here: a descent's level lowered into `(0, 2^-100)`, and the rise a shrink
    // ends in from an old path's reduction in it.
    for (site, count) in SITES.iter().zip(met) {
        match *site {
            "rise" | "cap of the rise a shrink ends in" => {
                assert_eq!(count, 0, "{site}: a sum or a maximum of kept values");
            }
            "descent's level" | "rise a shrink ends in" => {}
            _ => assert!(
                count > 0,
                "{site}: no value below the least kept met ({met:?})"
            ),
        }
    }
}

#[test]
fn keeps_the_level_of_a_descent_and_the_rise_a_shrink_ends_in_zero_or_kept() {
    // Two stores no tail of `keeps_every_value_of_a_change_zero_or_kept_at_a_tails_end`
    // reaches with a value below `2^-100`, from crafted states of a limiter shrunk from 5 to
    // 0.5 ms in silence, at every rate:
    // - a descent at `1.5 × 2^-100`, falling by `2^-100` (its fall and its glide's step), the
    //   paths at zero: lowered to `2^-101`, below the least value kept, it is zero, so the
    //   change ends on that frame, where a level stored as is would descend one frame more;
    // - a shrink's last frame (one rebuilt reduction left, its level zero), both paths' fast
    //   stages at `2^-99`: the new window, shorter, weighs the newest reduction more, so the
    //   new path's reduction is above the old one's, which lies in `(0, 2^-100)`, and the
    //   change ends in a rise from it, stored as zero.
    let least = libm::exp2(-100.0_f64);
    for rate in rates() {
        let context = format!("{} Hz", rate.hz());
        let mut limiter = TruePeakLimiter::<Binary64>::new(
            rate,
            1,
            value(5.0_f64),
            super::Detector::Standard,
            settings(-1.0_f64, 5.0_f64, 100.0_f64, 0.5_f64),
            LimiterMode::Protected,
        )
        .expect("a limiter");
        for _ in 0..2 * frames_of_duration(5.0_f64, rate) {
            let _: f64 = gain_of(&mut limiter, 0.25_f64);
        }
        limiter
            .set(settings(-1.0_f64, 0.5_f64, 100.0_f64, 0.5_f64))
            .expect("within the maximum");
        while !matches!(limiter.change, Some(super::Stage::Shrinking { .. })) {
            let _: f64 = gain_of(&mut limiter, 0.25_f64);
        }
        let mut descending = limiter.clone();
        descending.change = Some(super::Stage::Descending {
            level: Binary64::new(1.5_f64 * least),
            fall: Binary64::new(least),
            glide: Some(super::Glide {
                step: Binary64::new(least),
                last: Binary64::ZERO,
            }),
        });
        let _: f64 = gain_of(&mut descending, 0.25_f64);
        assert_eq!(
            (descending.path.smoothed(), descending.change),
            (Binary64::ZERO, None),
            "{context}: the descent lowered below the least kept, over"
        );
        let mut shrinking = limiter;
        shrinking.change = Some(super::Stage::Shrinking {
            scaling: 1,
            level: Binary64::ZERO,
            fall: Binary64::ZERO,
            glide: None,
        });
        let tail = Binary64::new(libm::exp2(-99.0_f64));
        (shrinking.path.fast, shrinking.previous.fast) = (tail, tail);
        let _: f64 = gain_of(&mut shrinking, 0.25_f64);
        let (newest, oldest) = (
            shrinking.path.smoothed().raw(),
            shrinking.previous.smoothed().raw(),
        );
        assert!(
            oldest > 0.0_f64 && oldest < least && newest > oldest,
            "{context}: the old path's reduction {oldest} in (0, 2^-100), below the new one's {newest}"
        );
        assert!(
            matches!(
                shrinking.change,
                Some(super::Stage::Rising { rising, .. }) if rising == Binary64::ZERO
            ),
            "{context}: a rise from zero: {:?}",
            shrinking.change
        );
    }
}

#[test]
fn applies_a_reduction_zero_or_kept_through_a_crossfade_in_a_release_tail() {
    // At every rate, a switchable limiter whose release stages hold `2^-98` (the tail of a
    // long release, the slowest, so that it stays there), crossfaded to protected for 20 ms
    // (the crossfade's time constant, its share applied rising from 0 to `1 - 1/e`), then to
    // raw for 40 ms (falling to `(1 - 1/e)/e²`, below a quarter): the smoothed reduction
    // times a share below a quarter is below `2^-100`, the least value the control path
    // keeps. Each frame's reduction must be zero or at least that (`spec/engine-design.md`,
    // decision 11); the product would have been below it on some frame of each crossfade,
    // and some reduction is applied.
    for rate in rates() {
        let context = format!("{} Hz", rate.hz());
        let mut limiter = TruePeakLimiter::<Binary64>::new(
            rate,
            1,
            value(5.0_f64),
            super::Detector::Standard,
            settings(-1.0_f64, 1.5_f64, 1_000.0_f64, 0.0_f64),
            LimiterMode::Switchable,
        )
        .expect("a limiter");
        let tail = Binary64::new(libm::exp2(-98.0_f64));
        (limiter.path.fast, limiter.path.slow) = (tail, tail);
        for (protected, milliseconds) in [(true, 20.0_f64), (false, 40.0_f64)] {
            limiter.set_protected(protected);
            let (mut subnormal, mut applied) = (false, false);
            for frame in 0..frames_of_duration(milliseconds, rate) {
                let reduction = limiter.reduction(value(0.5_f64));
                assert!(
                    zero_or_kept(reduction),
                    "{context}, toward protected {protected}, frame {frame}: {reduction:?}"
                );
                let product = limiter.path.smoothed().raw() * limiter.applied.raw();
                subnormal |= product > 0.0_f64 && product < libm::exp2(-100.0_f64);
                applied |= reduction > Binary64::ZERO;
            }
            assert!(
                subnormal && applied,
                "{context}, toward protected {protected}: a share below the least kept, a reduction"
            );
        }
    }
}

#[test]
fn releases_its_slow_stage_to_exactly_zero_by_values_zero_or_kept() {
    // At every rate, the release stages at `2^-90` (a release tail) and a release of 10 ms,
    // the needs zero: the slow stage closes `a` of its gap per frame, `a` about one over the
    // release in frames, so it falls below `2^-100`, the least value the control path keeps,
    // within `ln(2^10)` releases. Every value it stores must be zero or at least `2^-100`,
    // and every step it takes normal, so that no host computes a subnormal number from it
    // (`spec/engine-design.md`, decision 11), and it ends at exactly zero within ten
    // releases, never creeping on below.
    for rate in rates() {
        let context = format!("{} Hz", rate.hz());
        let mut limiter = TruePeakLimiter::<Binary64>::new(
            rate,
            1,
            value(5.0_f64),
            super::Detector::Standard,
            settings(-1.0_f64, 1.5_f64, 10.0_f64, 0.0_f64),
            LimiterMode::Protected,
        )
        .expect("a limiter");
        let tail = Binary64::new(libm::exp2(-90.0_f64));
        (limiter.path.fast, limiter.path.slow) = (tail, tail);
        for frame in 0..frames_of_duration(100.0_f64, rate) {
            let before = limiter.path.slow;
            let _: f64 = gain_of(&mut limiter, 0.5_f64);
            let after = limiter.path.slow;
            assert!(
                zero_or_kept(after) && zero_or_normal(Binary64::new(before.raw() - after.raw())),
                "{context}, frame {frame}: {before:?} to {after:?}"
            );
        }
        assert_eq!(
            (limiter.path.fast, limiter.path.slow),
            (Binary64::ZERO, Binary64::ZERO),
            "{context}: released to zero"
        );
    }
}

/// A result computed as on a host that flushes subnormal numbers to zero, the flush seen in
/// force by the arithmetic itself before and after the computation: built only by
/// [`flushing`], so that no caller takes a result as flushed from the mode word alone.
struct Flushed<R>(R);

/// `body` run as on a host that flushes subnormal numbers to zero (`fpenv`'s `FLUSHING`),
/// through [`seen_flushing`].
fn flushing<R>(body: impl FnOnce() -> R) -> Flushed<R> {
    seen_flushing(fpenv::host::FLUSHING, body)
}

/// `body` run as on a host of the mode `host`, the one call of `as_host` in this file: the
/// forced mode is test support, which Clippy refuses anywhere else (`clippy.toml`), so this
/// expectation also proves the lint fires. Panics unless flush-to-zero reads back in the mode
/// word and the arithmetic flushes, a subnormal operand and a subnormal result both taken as
/// zero in binary64 (observed by `SubnormalArithmetic::observe`), before the body and after
/// it, so that a runner whose register reads back without flushing, or code moved out of the
/// forced mode, fails here.
#[expect(
    clippy::disallowed_methods,
    reason = "the host differential runs the control path under a flushing host on purpose, through this helper only"
)]
fn seen_flushing<R>(host: fpenv::host::Word, body: impl FnOnce() -> R) -> Flushed<R> {
    let (set, (before, result, after)) = fpenv::host::as_host(host, || {
        let before = crate::SubnormalArithmetic::observe().binary64;
        let result = body();
        (
            before,
            result,
            crate::SubnormalArithmetic::observe().binary64,
        )
    });
    assert_eq!(
        set & fpenv::host::FLUSH_TO_ZERO,
        fpenv::host::FLUSH_TO_ZERO,
        "flush-to-zero in the mode word"
    );
    for (when, seen) in [("before", before), ("after", after)] {
        assert_eq!(
            (seen.operands, seen.results),
            (false, false),
            "subnormals flushed {when} the body"
        );
    }
    Flushed(result)
}

#[test]
#[should_panic(expected = "flush-to-zero in the mode word")]
fn takes_no_result_as_flushed_under_a_host_that_keeps_subnormals() {
    // The positive control of [`flushing`]'s observation: a host mode that rounds up and
    // keeps subnormal numbers gives no `Flushed` result.
    let Flushed(()) = seen_flushing(fpenv::host::ROUNDING_UP, || ());
}

/// The state of `limiter` as bits ([`Stored`]): every value, each ring and hold whole, and
/// every count, so that a host that computes any of them differently is seen on the frame it
/// does.
fn control_state<S: Sample>(limiter: &TruePeakLimiter<S>) -> Vec<u64> {
    let mut bits = Vec::new();
    each_stored(limiter, &mut |part| match part {
        Stored::Kept(value) | Stored::Derived(value) => bits.push(value.raw().to_bits()),
        Stored::KeptRing(ring, _) | Stored::Peaks(ring) => {
            bits.extend(ring.iter().map(|value| value.raw().to_bits()));
        }
        Stored::Hold(hold) => bits.extend(
            hold.iter()
                .flat_map(|(entered, need)| [*entered, need.raw().to_bits()]),
        ),
        Stored::Phasor(phasor) => bits.extend(
            phasor
                .iter()
                .flat_map(|(cos, sin)| [cos.raw().to_bits(), sin.raw().to_bits()]),
        ),
        Stored::Count(count) => bits.push(count),
    });
    bits
}

/// Frames of [`differs_on_no_host_through_a_release_tail`] at `rate`: peaks of 2.0 for twice
/// the maximum of 5 ms, then 0.25, which needs nothing, for `tail` releases of 10 ms, then
/// 0.25 for four maxima, peaks of 2.0 for two and 0.25 for two more: the peak of each frame,
/// and the frame `at` releases into the tail, at which the change or the crossfade is made,
/// if any.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "frame counts of a few hundred thousand, far from overflow"
)]
fn tail_peaks(rate: SampleRate, tail: f64, at: Option<f64>) -> (Vec<f64>, Option<usize>) {
    let maximum = frames_of_duration(5.0_f64, rate);
    let burst = 2 * maximum;
    let second = burst + frames_of_duration(10.0_f64 * tail, rate) + 4 * maximum;
    let peaks = (0..second + 4 * maximum)
        .map(|frame| {
            if frame < burst || (second..second + burst).contains(&frame) {
                2.0_f64
            } else {
                0.25_f64
            }
        })
        .collect();
    (
        peaks,
        at.map(|at| burst + frames_of_duration(10.0_f64 * at, rate)),
    )
}

/// Whether some value `limiter` keeps ([`Stored::Kept`], its rings and holds) lies in
/// `(0, 2^-90)`: a state near the least value kept, `2^-100`, and not all zero.
fn near_the_least_kept<S: Sample>(limiter: &TruePeakLimiter<S>) -> bool {
    let near = |value: &Binary64| value.raw() > 0.0_f64 && value.raw() < libm::exp2(-90.0_f64);
    let mut found = false;
    each_stored(limiter, &mut |part| match part {
        Stored::Kept(value) => found |= near(&value),
        Stored::KeptRing(ring, _) => found |= ring.iter().any(near),
        Stored::Hold(hold) => found |= hold.iter().any(|(_, need)| near(need)),
        Stored::Derived(_) | Stored::Peaks(_) | Stored::Phasor(_) | Stored::Count(_) => {}
    });
    found
}

/// Runs the control path of two limiters alike, one as a host that flushes subnormal
/// numbers ([`flushing`]) and one in IEEE arithmetic (observed on each frame), both of `mode`
/// at `rate` with a lookahead of `from` of a 5 ms maximum, a release of 10 ms and no
/// adaptivity, over [`tail_peaks`] of `tail` releases, `first` applied to each before the
/// first frame and `then` `at` releases into the tail, if given (with, there, a value of the
/// state near the least value kept), each command run under the flushing host's mode for the
/// flushing limiter, as a host runs its commands; panics at the first frame whose gain
/// or state ([`control_state`]) differs, naming it.
fn alike_on_both_hosts(
    rate: SampleRate,
    (mode, from): (LimiterMode, f64),
    (tail, at): (f64, Option<f64>),
    first: impl Fn(&mut TruePeakLimiter<Binary64>),
    then: impl Fn(&mut TruePeakLimiter<Binary64>),
    context: &str,
) {
    let built = || {
        TruePeakLimiter::<Binary64>::new(
            rate,
            1,
            value(5.0_f64),
            super::Detector::Standard,
            settings(-1.0_f64, from, 10.0_f64, 0.0_f64),
            mode,
        )
        .expect("a limiter")
    };
    let (mut flushed, mut ieee) = (built(), built());
    // The state's rings, which every frame compares, sized by the maximum: asserted against
    // the frames counted apart before any frame runs.
    assert_eq!(
        (flushed.maximum, ieee.maximum),
        (
            frames_of_duration(5.0_f64, rate),
            frames_of_duration(5.0_f64, rate)
        ),
        "{context}: the maximum, counted apart"
    );
    // Each command run as the host runs it, the flushing one in its mode too.
    let Flushed(()) = flushing(|| first(&mut flushed));
    first(&mut ieee);
    let (peaks, change) = tail_peaks(rate, tail, at);
    for (frame, peak) in peaks.into_iter().enumerate() {
        if change == Some(frame) {
            assert!(
                near_the_least_kept(&ieee),
                "{context}: a state near the least value kept when it changes"
            );
            let Flushed(()) = flushing(|| then(&mut flushed));
            then(&mut ieee);
        }
        let Flushed(flushed_gain) = flushing(|| gain_of(&mut flushed, peak));
        assert!(
            crate::SubnormalArithmetic::observe().is_ieee(),
            "{context}: the other host in IEEE arithmetic"
        );
        assert_eq!(
            flushed_gain.to_bits(),
            gain_of(&mut ieee, peak).to_bits(),
            "{context}, frame {frame}: the gain"
        );
        assert!(
            control_state(&flushed) == control_state(&ieee),
            "{context}, frame {frame}: the control state, {:?} flushing, {:?} not",
            (flushed.path.fast, flushed.path.slow, flushed.change),
            (ieee.path.fast, ieee.path.slow, ieee.change)
        );
    }
    assert_eq!(
        format!("{flushed:?}"),
        format!("{ieee:?}"),
        "{context}: every field at the end"
    );
}

#[test]
fn differs_on_no_host_through_a_release_tail() {
    // A host that flushes subnormal numbers to zero, seen flushing ([`flushing`]), computes
    // every gain and every value of the limiter's state as one in IEEE arithmetic, frame by
    // frame, since the control path keeps no value it stores below `2^-100`, the least it
    // keeps (`math::kept`), and its Hann sums are proven zero or normal
    // (`spec/engine-design.md`, decision 11; `spec/measurements.md`, "The true-peak limiter",
    // step 7); each run then a burst. In a tail every reduction gives a gain of exactly 1,
    // so the gains alone could not show a difference: the state does.
    // - A release run 707 releases (`e^-707`, about `2^-1020`) with no change, where a release
    //   stage that kept every value would decay into the subnormal numbers, which a flushing
    //   host reads as zero: a decay without `math::kept` fails it (seen on 2026-11-18).
    // - A live lookahead change (growing and shrinking) and a crossfade between raw and
    //   protected (each way), each made `ln(r / 2^-98)` releases into the tail (about 67.3,
    //   `r = 1 - c / 2`, the burst's need, where the reduction nears `2^-98`), so that they
    //   run on a state near the least value kept, not all zero (asserted), then 20 releases
    //   more. Their stages compute each value from values zero or at least `2^-100`, by sums,
    //   differences and products by weights of at least `2^-11` and shares kept, which cannot
    //   reach the subnormal numbers from there: no cheat of theirs makes a host differ within
    //   a change, so this part shows the changes alike on a state near the threshold, and
    //   their values' floor rests on step 7's derivation and the laws of zero or normal
    //   values (`computes_no_subnormal_value_through_a_release_to_zero`).
    // At the two lowest rates of the test matrix, an integer number of frames per millisecond
    // and not (the laws of the same values run every rate).
    let none = |_: &mut TruePeakLimiter<Binary64>| ();
    let near = libm::log(libm::exp2(98.0_f64) * (1.0_f64 - CEILING / 2.0_f64));
    for rate in rates().into_iter().take(2) {
        alike_on_both_hosts(
            rate,
            (LimiterMode::Protected, 1.5_f64),
            (707.0_f64, None),
            none,
            none,
            &format!("{} Hz, a release run 707 releases", rate.hz()),
        );
        for (from, to) in [(0.5_f64, 5.0_f64), (5.0_f64, 0.5_f64)] {
            alike_on_both_hosts(
                rate,
                (LimiterMode::Protected, from),
                (near + 20.0_f64, Some(near)),
                none,
                |limiter| {
                    limiter
                        .set(settings(-1.0_f64, to, 10.0_f64, 0.0_f64))
                        .expect("within the maximum");
                },
                &format!("{} Hz, from {from} to {to} ms", rate.hz()),
            );
        }
        for protected in [true, false] {
            alike_on_both_hosts(
                rate,
                (LimiterMode::Switchable, 1.5_f64),
                (near + 20.0_f64, Some(near)),
                |limiter| limiter.set_protected(!protected),
                |limiter| limiter.set_protected(protected),
                &format!("{} Hz, toward protected {protected}", rate.hz()),
            );
        }
    }
}

/// Gives `check` each value of `limiter`'s state ([`Stored`]) a frame can write: every value
/// but those of its rings, and the newest of each ring it writes a frame (the needs, each path's released
/// reductions; a hold's values are needs, each the newest when written). The rings fixed when
/// it is built or a path is prepared are proven apart
/// (`turns_no_phasor_part_below_two_to_the_minus_55_up_to_the_longest_window`).
fn written_values<S: Sample>(limiter: &TruePeakLimiter<S>, mut check: impl FnMut(Binary64)) {
    each_stored(limiter, &mut |part| match part {
        Stored::Kept(value) | Stored::Derived(value) => check(value),
        Stored::KeptRing(ring, at) => check(
            ring.get(
                at.checked_sub(1)
                    .unwrap_or_else(|| ring.len().saturating_sub(1)),
            )
            .copied()
            .expect("a ring"),
        ),
        Stored::Hold(_) | Stored::Peaks(_) | Stored::Phasor(_) | Stored::Count(_) => {}
    });
}

/// [`computes_no_subnormal_value_through_a_release_to_zero`] in `S`, at a maximum lookahead
/// of `most` milliseconds and every rate: from 0.5 ms, grown to `most` 20 releases into the
/// tail, shrunk back at 30, grown again where the reduction nears `2^-98` (with a value of
/// the state below `2^-90` there, asserted), each set starting a change
/// (asserted) but at a maximum of 0.5 ms, where no lookahead can change; crossfaded to raw
/// at 40 releases and back at 50.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "frame counts of a few hundred thousand, far from overflow"
)]
fn no_subnormal_value_through_a_release_to_zero<S: Sample>(most: f64) {
    // Where the reduction nears `2^-98`, computed apart: the slow stage, the larger after a
    // few releases at this adaptivity, rises toward the burst's need `r = 1 - c / 2` while a
    // window holds it, from twice to three times the maximum (about `1 - e^-(2.5 × most / 10)`
    // of it, its release 10 ms), then falls by a release per 10 ms: `ln(s / 2^-98)` releases
    // into the tail, within a fraction of one either way, inside the stretch of about
    // `ln 2^10` releases where the state lies in `(0, 2^-90)`.
    let reached = (1.0_f64 - CEILING / 2.0_f64) * -libm::expm1(-0.25_f64 * most);
    let near = libm::log(libm::exp2(98.0_f64) * reached);
    for rate in rates() {
        let context = format!(
            "{} Hz, {most} ms at most, binary{}",
            rate.hz(),
            S::PRECISION.bits()
        );
        let made = |lookahead: f64| settings(-1.0_f64, lookahead, 10.0_f64, 0.5_f64);
        let mut limiter = TruePeakLimiter::<S>::new(
            rate,
            1,
            value(most),
            super::Detector::Standard,
            made(0.5_f64),
            LimiterMode::Switchable,
        )
        .expect("a limiter");
        limiter.set_protected(true);
        let (burst, release) = (
            2 * frames_of_duration(most, rate),
            frames_of_duration(10.0_f64, rate),
        );
        let changing = most > 0.5_f64;
        let near = frames_of_duration(10.0_f64 * near, rate);
        for frame in 0..burst + 80 * release {
            let tail = frame.checked_sub(burst);
            for (at, lookahead) in [(20 * release, most), (30 * release, 0.5_f64), (near, most)] {
                if tail == Some(at) {
                    if at == near {
                        assert!(near_the_least_kept(&limiter), "{context}: near 2^-100");
                    }
                    limiter.set(made(lookahead)).expect("within the maximum");
                    assert_eq!(
                        limiter.change.is_some(),
                        changing,
                        "{context}: a change to {lookahead} ms begun"
                    );
                }
            }
            if tail == Some(40 * release) {
                limiter.set_protected(false);
            }
            if tail == Some(50 * release) {
                limiter.set_protected(true);
            }
            let _: f64 = gain_in(
                &mut limiter,
                if tail.is_none() { 2.0_f64 } else { 0.25_f64 },
            );
            written_values(&limiter, |value| {
                assert!(
                    zero_or_above_the_least_computed(value),
                    "{context}, frame {frame}: {value:?}, below 2^-533"
                );
            });
        }
        // The positive control: the release ran to zero, its stages and sums with it, and
        // the change is over.
        let path = &limiter.path;
        assert_eq!(
            (path.fast, path.slow, path.sums),
            (Binary64::ZERO, Binary64::ZERO, crate::math::HannSums::ZERO),
            "{context}: released to zero"
        );
        assert_eq!(limiter.change, None, "{context}: the change over");
        // A tail met deeper than any release reaches, its stages set at `2^-1012`, whose
        // weights and turns would give subnormal sums: zero from its first frame on.
        let deep = Binary64::new(libm::exp2(-1_012.0_f64));
        (limiter.path.fast, limiter.path.slow) = (deep, deep);
        for frame in 0..2 * release {
            let _: f64 = gain_in(&mut limiter, 0.25_f64);
            written_values(&limiter, |value| {
                assert!(
                    zero_or_above_the_least_computed(value),
                    "{context}, deep tail, frame {frame}: {value:?}, below 2^-533"
                );
            });
        }
    }
}

#[test]
fn computes_no_subnormal_value_through_a_release_to_zero() {
    // A burst over the ceiling, then its release, of 10 ms, run 80 releases to zero (`e^-80`,
    // below `2^-115`, so below `2^-100`, the least value the control path keeps), with live
    // lookahead changes in it (a growth 20 releases in, a shrink back at 30, a growth again
    // where the reduction nears `2^-98`) and a crossfade to raw at 40 and back at 50: every
    // value a frame can write to the state ([`written_values`]) is zero or at least
    // `2^-533`, the least step 7 derives, far above the subnormal numbers, at every
    // rate, at maxima of 0.5 ms (no lookahead change possible there: crossfades only), 0.625
    // ms (the smallest with a change at every rate, one frame more than 0.5 ms at 8 kHz), 1
    // and 5 ms, in both precisions; and so is every value of a tail met at `2^-1012`
    // (`spec/measurements.md`, "The true-peak limiter", step 7).
    for most in [0.5_f64, 0.625_f64, 1.0_f64, 5.0_f64] {
        no_subnormal_value_through_a_release_to_zero::<Binary64>(most);
        no_subnormal_value_through_a_release_to_zero::<Binary32>(most);
    }
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one change followed through each of its stages and the next one's"
)]
fn takes_a_lookahead_set_during_a_change_once_it_is_over() {
    // At every rate, on a constant need (peaks of 2.0, settled): from 1.5 to 0.5 ms, then 3
    // and 5 ms set while that change runs, the first while its new path is rebuilt, the
    // second while it shrinks. The change to 0.5 ms runs to its end (the frame it begins
    // after, its rebuild, then as long as the new window holds a reduction it rebuilt, its
    // `short - 1`, at least one frame, both paths settled on the same need, or, where their
    // roundings leave the new one above the old, a rise to it and a descent), and the last
    // lookahead asked for (5 ms) begins on its last frame: its rebuild then, or the wait for
    // the life of the 0.5 ms path to span the 5 ms window if later, a wait counted from the
    // frames the test ran (`long - lived - 1` frames past that last frame, `lived` the frames
    // of the 0.5 ms path's life then, its `short - 1` rebuilt reductions and one release a
    // frame), the one in between never in force. The rise, from the reduction in force,
    // which the new path rebuilt at that need too, reaches it on its first frame, and the
    // descent from there is over within the frames of a glide between the windows and a
    // descent ([`constant_change_frames`]).
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
            frames_of_duration(0.5_f64, rate),
            frames_of_duration(1.5_f64, rate),
            frames_of_duration(5.0_f64, rate),
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
        for _ in 0..=rebuild / 2 {
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
        assert!(
            matches!(limiter.change, Some(super::Stage::Shrinking { .. })),
            "{context}: shrinking: {:?}",
            limiter.change
        );
        set(&mut limiter, 5.0_f64);
        // The shrink's frames: as long as the new window holds a rebuilt reduction.
        let shrinking = (short - 1).max(1);
        for frame in 1..shrinking {
            let _: f64 = gain_of(&mut limiter, 2.0_f64);
            assert!(
                matches!(limiter.change, Some(super::Stage::Shrinking { .. })),
                "{context}, frame {frame}: shrinking: {:?}",
                limiter.change
            );
        }
        // Over on its last frame, or after a rise to the new path where the two paths'
        // roundings leave it above the old one, the last lookahead asked for begun on the
        // change's last frame, its rebuild waiting for the 0.5 ms path's life to span the
        // 5 ms window.
        let mut after = shrinking - 1;
        let cap = constant_change_frames(long, middle, short);
        while matches!(
            limiter.change,
            Some(
                super::Stage::Shrinking { .. }
                    | super::Stage::Rising { .. }
                    | super::Stage::Descending { .. }
            )
        ) {
            let _: f64 = gain_of(&mut limiter, 2.0_f64);
            after += 1;
            assert!(after <= cap, "{context}: still toward 0.5 ms");
        }
        let lived = after + short - 1;
        let waited = rebuild.max(long - lived - 1);
        assert_eq!(
            limiter.change,
            Some(super::Stage::Rebuilding(waited)),
            "{context}: the last asked for, begun on the change's last frame"
        );
        assert_eq!(limiter.wanted, long, "{context}: 5 ms asked for");
        for _ in 0..waited {
            assert_eq!(limiter.path.lookahead, short, "{context}: still 0.5 ms");
            let _: f64 = gain_of(&mut limiter, 2.0_f64);
        }
        assert_eq!(limiter.path.lookahead, long, "{context}: 5 ms taken over");
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
        let left = constant_change_frames(long, short, long) - 2 - rebuild;
        let descent = until_over(&mut limiter, 2.0_f64, left, &context);
        assert!(descent >= 1, "{context}: descended");
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
/// of `maximum` frames on a constant need or none, computed apart: the frame it begins after
/// (the one after its set), its rebuild, then, shrinking, the new window's `new - 1` rebuilt
/// reductions leaving it one per frame (at least one frame), or longer, the level glides over
/// the gap the two paths' roundings leave between them, at most the frames between the two
/// windows `G` and a descent ([`descent_frames`]) of the old window, then, the new path above
/// the old one by those roundings, a growth's frames; growing, the rise, which reaches the new
/// path at once on such a need, and the descent from there, at most `G` frames and a descent
/// of the new window. The bound holds for a change whose rebuild waits for
/// neither a path's life nor a release, as each caller's does.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "frame counts of a few thousand, far from overflow"
)]
fn constant_change_frames(maximum: usize, old: usize, new: usize) -> usize {
    let between = old.abs_diff(new).max(1);
    let growing = 1 + between + descent_frames(new);
    let after = if new > old {
        growing
    } else {
        (new - 1).max(1).max(between + descent_frames(old)) + growing
    };
    1 + rebuild_frames_of(maximum) + after
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
        let maximum = frames_of_duration(5.0_f64, rate);
        let context = format!("{} Hz", rate.hz());
        let (short, middle) = (
            frames_of_duration(0.5_f64, rate),
            frames_of_duration(1.0_f64, rate),
        );
        let mut limiter = TruePeakLimiter::<Binary64>::new(
            rate,
            1,
            value(5.0_f64),
            super::Detector::Standard,
            unlike_defaults(0.5_f64),
            LimiterMode::Protected,
        )
        .expect("a limiter");
        for _ in 0..frames_of_duration(100.0_f64, rate) {
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
        for _ in 0..frames_of_duration(300.0_f64, rate) + 2 * maximum + 2 {
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
fn waits_to_rebuild_a_growths_window_from_the_releases_of_the_old_paths_own_life() {
    // At every rate, the two paths used by turns: 1 ms on loud noise, grown to 2 ms on it;
    // then peaks under the ceiling (no need) with the slowest release, so that every stage
    // still holds the loud releases; shrunk to 0.5 ms (the path of 1 ms taken up again), then
    // grown to 5 ms as soon as that change is over, while that path's life, its rebuilt
    // reductions and one release a frame since, is shorter than the new window. The growth
    // waits until the life spans the new window: its rebuild ends once the life holds the
    // `maximum - 1` reductions it rebuilds, `maximum - short - after - 1` frames after the
    // frame it begins after, the one after its set (`after` the frames from the shrink's
    // takeover to the set, counted by the test), its
    // rebuild's own frames when more, the old path alone in force meanwhile, its gain that of
    // a copy left unchanged, bit for bit. On the rebuild every hold of the new path is zero
    // (no need in its windows), so each reduction it rebuilds is exactly the old path's
    // released reduction of that frame (its rebuilt reductions, then its releases, recorded
    // here frame by frame): never a release of the path's earlier life, nor a zero for a
    // frame before its life. A growth that does not wait, or waits a frame short, fails.
    let mut draws = Noise::new();
    for rate in rates() {
        let maximum = frames_of_duration(5.0_f64, rate);
        let rebuild = rebuild_frames_of(maximum);
        let context = format!("{} Hz", rate.hz());
        let (short, two) = (
            frames_of_duration(0.5_f64, rate),
            frames_of_duration(2.0_f64, rate),
        );
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
        for _ in 0..=rebuild {
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
        // The growth begins on the frame after its set, the life one release longer then.
        let lived = after + short - 1;
        let wait = rebuild.max(maximum - short - after - 1);
        assert!(wait > rebuild, "{context}: a growth that waits");
        // Read once its shrink is over, its rebuilt reductions scaled: the oldest first.
        let mut life: Vec<f64> = (0..lived)
            .rev()
            .map(|back| limiter.path.released_before(back).raw())
            .collect();
        let mut unchanged = limiter.clone();
        limiter.set(made(5.0_f64)).expect("within the maximum");
        for frame in 0..=wait {
            assert!(
                matches!(
                    limiter.change,
                    Some(super::Stage::Rebuilding(_) | super::Stage::Pending)
                ),
                "{context}, frame {frame} of the wait: rebuilding"
            );
            assert_eq!(
                gain_of(&mut limiter, 0.5_f64).to_bits(),
                gain_of(&mut unchanged, 0.5_f64).to_bits(),
                "{context}, frame {frame} of the wait: the old path's gain"
            );
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
            life.len() >= maximum - 1,
            "{context}: the new window within the old path's life"
        );
        assert!(
            life.iter().all(|release| *release > 0.0_f64),
            "{context}: the old path still releasing"
        );
        // The reduction rebuilt for the frame `back` before the rebuild's last: the old
        // path's release of that frame, the newest of `life` that last frame's.
        for back in 0..maximum - 1 {
            assert_eq!(
                limiter.path.released_before(back).raw().to_bits(),
                life[life.len() - 1 - back].to_bits(),
                "{context}: the reduction rebuilt {back} frames before the change"
            );
        }
    }
}

/// How far a limiter's reduction may lie, by rounding alone, from that of another limiter on
/// the same peaks whose exact Hann mean it equals, at a maximum lookahead of `maximum` frames
/// (`spec/measurements.md`, "The true-peak limiter", steps 4 and 5; `u = 2^-53`): each control
/// path's value within `ρ` of its exact mean ([`rounding_of`], the longest window's), each
/// gain `1 - r` within `u/2`, and each test's `1 - gain` within `u/2`, for both: `2ρ + 2u`.
fn beside_slack(maximum: usize) -> f64 {
    2.0_f64 * rounding_of(maximum) + 2.0_f64 * libm::exp2(-53.0_f64)
}

#[test]
fn keeps_a_growth_set_at_its_paths_life_boundary_above_limiters_always_at_its_lookaheads() {
    // At every rate, with two releases and adaptivities away from the defaults: 50 ms of
    // loud peaks (4.0), then peaks under the ceiling (0.5), the reduction released; 10 ms
    // into the release the lookahead changes to 1 ms (from 0.5 ms, growing, or from 2 ms,
    // shrinking), then to 5 ms as soon as that change is over, while the path in force has
    // lived less than the new window (the regime asserted, its life counted from the frames
    // the test ran: the first change taken over on the frame after its rebuild, which follows
    // the frame after its set, from a path in force since the limiter was built). Beside it, limiters always at 1 and
    // 5 ms on the same peaks. In a release every path releases the same reductions whatever
    // its window (its hold drops when the loud needs leave the oldest end of its window,
    // `maximum + 1` frames back at every lookahead), so a growth that starts over its
    // window as if it had always run keeps the reduction at or above the smaller of theirs
    // on every frame, waiting, rising (at least the old path) or descending (at least the
    // new one), less both paths' rounding `2ρ` and `u` for each gain and its difference from
    // one (`spec/measurements.md`, "The true-peak limiter", steps 4 and 5). A growth that
    // rebuilt the frames of its window before that life as zero releases descends onto a
    // mean far below both: by more than 8 dB here.
    for rate in rates() {
        let maximum = frames_of_duration(5.0_f64, rate);
        let rebuild = rebuild_frames_of(maximum);
        let slack = beside_slack(maximum);
        for (release, adaptivity) in [(1_000.0_f64, 0.0_f64), (300.0_f64, 0.25_f64)] {
            for from in [0.5_f64, 2.0_f64] {
                let context = format!(
                    "{} Hz, release {release} ms, adaptivity {adaptivity}, from {from} ms",
                    rate.hz()
                );
                let made = |lookahead: f64| settings(-1.0_f64, lookahead, release, adaptivity);
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
                let (mut limiter, mut short, mut long) =
                    (build(from), build(1.0_f64), build(5.0_f64));
                let loud = frames_of_duration(50.0_f64, rate);
                let mut frame = 0;
                let run = |changing: &mut TruePeakLimiter<Binary64>,
                           at_one: &mut TruePeakLimiter<Binary64>,
                           at_five: &mut TruePeakLimiter<Binary64>,
                           at: usize| {
                    let peak = if at < loud { 4.0_f64 } else { 0.5_f64 };
                    (
                        1.0_f64 - gain_of(changing, peak),
                        1.0_f64 - gain_of(at_one, peak),
                        1.0_f64 - gain_of(at_five, peak),
                    )
                };
                while frame < loud + frames_of_duration(10.0_f64, rate) {
                    let _: (f64, f64, f64) = run(&mut limiter, &mut short, &mut long, frame);
                    frame += 1;
                }
                limiter.set(made(1.0_f64)).expect("within the maximum");
                let mut since_set = 0;
                while limiter.change.is_some() {
                    let _: (f64, f64, f64) = run(&mut limiter, &mut short, &mut long, frame);
                    frame += 1;
                    since_set += 1;
                    assert!(
                        since_set <= constant_change_frames(maximum, maximum, 1),
                        "{context}: still changing to 1 ms"
                    );
                }
                // The 1 ms path's life: its `L - 1` rebuilt reductions, then one release per
                // frame from its change's first frame.
                let lived = since_set - 1 - rebuild + frames_of_duration(1.0_f64, rate) - 1;
                assert!(
                    lived + 1 + rebuild < maximum - 1,
                    "{context}: the new window reaches before the life"
                );
                limiter.set(made(5.0_f64)).expect("within the maximum");
                let cap = maximum + rebuild + 17 * maximum + 1 + descent_frames(maximum);
                for after in 0..cap {
                    let (reduction, at_short, at_long) =
                        run(&mut limiter, &mut short, &mut long, frame);
                    frame += 1;
                    let least = if at_short < at_long {
                        at_short
                    } else {
                        at_long
                    };
                    assert!(
                        reduction >= least - slack,
                        "{context}, frame {after} after the growth was set: a reduction of \
                         {reduction}, {at_short} always at 1 ms and {at_long} at 5 ms"
                    );
                }
                assert_eq!(limiter.change, None, "{context}: over");
            }
        }
    }
}

/// The lookaheads the random law of live settings draws at a maximum of `most` milliseconds at
/// `rate`, in milliseconds: close pairs, a frame apart near the maximum, and the maximum,
/// those within it (`spec/measurements.md`, "The true-peak limiter", step 6's laws).
fn drawn_lookaheads(most: f64, rate: SampleRate) -> Vec<f64> {
    let frame = 1_000.0_f64 / f64::from(rate.hz());
    let mut drawn: Vec<f64> = [
        0.5_f64, 0.6_f64, 1.0_f64, 1.2_f64, 2.0_f64, 3.5_f64, 4.8_f64, 4.9_f64,
    ]
    .into_iter()
    .filter(|lookahead| *lookahead < most)
    .collect();
    for back in [2.0_f64, 1.0_f64] {
        let lookahead = most - back * frame;
        if lookahead >= 0.5_f64 {
            drawn.push(lookahead);
        }
    }
    drawn.push(most);
    drawn
}

/// A draw in `[0, 1)` of `draws`, from its top 32 bits.
fn unit(draws: &mut crate::tests::splitmix::Draws) -> f64 {
    f64::from(u32::try_from(draws.next() >> 32_u32).expect("32 bits")) * libm::exp2(-32.0_f64)
}

/// One of `values`, drawn.
fn pick<T: Copy>(draws: &mut crate::tests::splitmix::Draws, values: &[T]) -> T {
    let at = draws
        .next()
        .checked_rem(u64::try_from(values.len()).expect("few"))
        .and_then(|at| usize::try_from(at).ok())
        .expect("an index");
    values[at]
}

/// What a run of the random law of live settings sets: every setting, the lookahead only,
/// or the lookahead and the release through tails that reach the least value kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drawn {
    /// Every live setting, the mode toggled in a switchable limiter.
    Everything,
    /// The lookahead only.
    Lookahead,
    /// The lookahead and a release of 10 ms, over bursts and silences of 0.75 to 1.25 s by
    /// turns.
    Tails,
}

/// The regimes the random law reaches, counted by the test from its own record of its sets
/// and of the frames it observes each stage change on: never read from the limiter's own
/// steps or takeovers.
#[derive(Debug, Default, Clone, Copy)]
struct Reached {
    /// Changes begun, growing and shrinking.
    growths: u32,
    /// Ditto, shrinking.
    shrinks: u32,
    /// Growths whose rebuild waited for the life of the path in force past their own frames.
    waits: u32,
    /// Shrinks whose rebuild waited for the release in force past their own frames.
    replays: u32,
    /// Releases asked for while a shrink ran, held.
    held: u32,
    /// Shrinks that ended in a rise, the new path above the old.
    risen: u32,
    /// Changes whose first canonical swap is not on their first frame.
    off_phase: u32,
    /// Lookaheads asked for and dropped before they began, set back between two frames.
    dropped: u32,
    /// Sets made at each stage: pending, rebuilding, rising, shrinking, descending.
    sets: [u32; 5],
    /// Frames of a change while the mode crossfades.
    crossfading: u32,
    /// Frames, among every sixteenth, with a value of the state in `(0, 2^-90)`.
    near: u32,
    /// Frames the own step law was checked on.
    stepped: u32,
    /// Frames above the largest of the fixed limiters at the lookaheads used, beyond the
    /// rounding: what a path in force carries from an earlier window, held to its derived
    /// bound ([`hann_mean`]).
    carried: u32,
    /// The largest such excess found, and the excess the derived bound allowed on its frame.
    excess: (f64, f64),
}

/// The Hann mean of a window of `length` frames over the values `newest_first` gives, from
/// the newest (0) back: `Σ (1 - cos(2π (k + 1) / (L + 1))) / (L + 1) × v(k)`, computed apart
/// in binary64 with libm's cosine, its own rounding within `(L + 3)u` of its exact value
/// times the largest value, below the paths' rounding `ρ` (`rounding_of`).
///
/// It bounds what a path in force carries from an earlier window over a sequence of changes
/// (`spec/measurements.md`, "The true-peak limiter", step 6): every path's released
/// reductions are at most those of the fixed limiter at the longest window used so far,
/// given the same needs and releases (each hold within that window's, each release stage
/// and rebuilt reduction a maximum, minimum or decay of such values), so the reduction, at
/// most the larger of the two paths' Hann means, is at most the largest Hann mean, over
/// the windows used, of that limiter's released reductions.
fn hann_mean(newest_first: impl Fn(usize) -> f64, length: usize) -> f64 {
    let span = f64::from(u32::try_from(length.saturating_add(1)).expect("a window"));
    (0..length)
        .map(|back| {
            let turn = f64::from(u32::try_from(back.saturating_add(1)).expect("a window"));
            (1.0_f64 - libm::cos(core::f64::consts::TAU * turn / span)) / span * newest_first(back)
        })
        .sum()
}

#[test]
fn computes_a_hann_mean_from_its_weights() {
    // Computed apart: one frame weighs its value whole; two frames, `(1 - cos(2π/3))/3` and
    // `(1 - cos(4π/3))/3`, a half each; three frames, a quarter, a half and a quarter.
    let one = |_: usize| 3.0_f64;
    assert_eq!(hann_mean(one, 1).to_bits(), 3.0_f64.to_bits(), "one frame");
    let two = |back: usize| [1.0_f64, 0.0_f64][back];
    assert!(
        (hann_mean(two, 2) - 0.5_f64).abs() <= 1e-15_f64,
        "two frames"
    );
    let three = |back: usize| [2.0_f64, 4.0_f64, 8.0_f64][back];
    assert!(
        (hann_mean(three, 3) - 4.5_f64).abs() <= 1e-14_f64,
        "three frames: a quarter of 2, a half of 4, a quarter of 8"
    );
    let ones = |_: usize| 1.0_f64;
    assert!(
        (hann_mean(ones, 1_920) - 1.0_f64).abs() <= 2_000.0_f64 * libm::exp2(-52.0_f64),
        "the weights of the longest window sum to one"
    );
}

/// The stage `stage` as an index of [`Reached::sets`], none when no change runs.
const fn stage_index(stage: Option<super::Stage>) -> Option<usize> {
    match stage {
        None => None,
        Some(super::Stage::Pending) => Some(0),
        Some(super::Stage::Rebuilding(_)) => Some(1),
        Some(super::Stage::Rising { .. }) => Some(2),
        Some(super::Stage::Shrinking { .. }) => Some(3),
        Some(super::Stage::Descending { .. }) => Some(4),
    }
}

/// The glide a stage at `stage` could move by on a frame that left the new path at `newest`,
/// at most, computed from the design apart (`spec/engine-design.md`, decision 12): the one it
/// stores, or, on its first frame, the gap from its value to the new path spread over the
/// `between` frames between the two windows, the larger.
fn glide_at(stage: Option<super::Stage>, newest: f64, between: usize) -> f64 {
    let spread = |gap: f64| larger(0.0_f64, gap) / f64::from(u32::try_from(between).expect("few"));
    match stage {
        Some(super::Stage::Rising { rising, glide, .. }) => larger(
            glide.map_or(0.0_f64, |glide| glide.step.raw()),
            spread(newest - rising.raw()),
        ),
        Some(
            super::Stage::Shrinking { level, glide, .. }
            | super::Stage::Descending { level, glide, .. },
        ) => larger(
            glide.map_or(0.0_f64, |glide| glide.step.raw()),
            spread(level.raw() - newest),
        ),
        _ => 0.0_f64,
    }
}

/// One run of the random law of live settings
/// ([`stays_between_limiters_always_at_its_lookaheads_while_its_lookahead_changes`]), at
/// `rate` and a maximum of `most` milliseconds, `frames` frames of the material drawn from
/// `draws`, setting what `drawn` says, protected or switchable, beside a limiter always at
/// each lookahead it may draw: its counts of the regimes reached.
#[expect(
    clippy::too_many_lines,
    reason = "one law over every live setting: the draws, the record of Rule 1 and the checks share their state"
)]
#[expect(
    clippy::arithmetic_side_effects,
    reason = "frame counts of a few hundred thousand and reductions below two, far from overflow"
)]
fn stays_within_fixed_limiters_through(
    rate: SampleRate,
    most: f64,
    (setting, switchable, frames): (Drawn, bool, usize),
    draws: &mut crate::tests::splitmix::Draws,
) -> Reached {
    const RELEASES: [f64; 5] = [10.0_f64, 30.0_f64, 100.0_f64, 300.0_f64, 1_000.0_f64];
    const ADAPTIVITIES: [f64; 5] = [0.0_f64, 0.25_f64, 0.5_f64, 0.75_f64, 1.0_f64];
    const CEILINGS: [f64; 6] = [-20.0_f64, -6.0_f64, -2.0_f64, -1.0_f64, -0.1_f64, 0.0_f64];
    let maximum = frames_of_duration(most, rate);
    let (rounding, unit_round) = (rounding_of(maximum), libm::exp2(-53.0_f64));
    let slack = beside_slack(maximum);
    let rebuild = rebuild_frames_of(maximum);
    let lookaheads = drawn_lookaheads(most, rate);
    let releases: &[f64] = if setting == Drawn::Tails {
        &[10.0_f64]
    } else {
        &RELEASES
    };
    let context = format!(
        "{} Hz, {most} ms at most, {setting:?}, switchable {switchable}",
        rate.hz()
    );
    let mode = if switchable {
        LimiterMode::Switchable
    } else {
        LimiterMode::Protected
    };
    // The settings asked for: ceiling, lookahead, release, adaptivity.
    let mut asked = (
        pick(draws, &CEILINGS),
        pick(draws, &lookaheads),
        pick(draws, releases),
        pick(draws, &ADAPTIVITIES),
    );
    let built = |lookahead: f64, (ceiling, _, release, adaptivity): (f64, f64, f64, f64)| {
        TruePeakLimiter::<Binary64>::new(
            rate,
            1,
            value(most),
            super::Detector::Standard,
            settings(ceiling, lookahead, release, adaptivity),
            mode,
        )
        .expect("a limiter")
    };
    let mut limiter = built(asked.1, asked);
    // One limiter always at each lookahead it may draw, by its frames.
    let mut always: Vec<(usize, f64, TruePeakLimiter<Binary64>)> = Vec::new();
    for lookahead in &lookaheads {
        let window = frames_of_duration(*lookahead, rate);
        if always.iter().all(|(count, _, _)| *count != window) {
            always.push((window, *lookahead, built(*lookahead, asked)));
        }
    }
    // The test's own record (Rule 1, decision 12): the lookahead in force and the change
    // running (its new lookahead, whether it shrinks), the release in force and the one held,
    // the step of the first frame computed with it, and the step of the takeover of the path
    // in force.
    let window = |lookahead: f64| frames_of_duration(lookahead, rate);
    let mut in_force = window(asked.1);
    let mut running: Option<(usize, bool)> = None;
    let mut release = (asked.2, asked.3);
    let mut held: Option<(f64, f64)> = None;
    let mut release_from = 1_u64;
    let mut taken: Option<u64> = None;
    let mut used = vec![in_force];
    let mut protected = !switchable;
    let mut reached = Reached::default();
    // The needs of the last `2 × maximum + 2` frames, computed apart, for coverage.
    let mut needs = vec![0.0_f64; 2 * maximum + 2];
    let (mut segment, mut level, mut floor, mut every) = (0, 0.0_f64, 0.0_f64, 1_u64);
    let mut kind = 0_u8;
    // The reduction of the frame before, fully protected, and the paths that ran on it, by
    // their lookaheads, with their reductions.
    let mut last: Option<(f64, Vec<(usize, f64)>)> = None;
    for step in 1..=u64::try_from(frames).expect("frames") {
        let frame = usize::try_from(step).expect("frames") - 1;
        // The material: segments of a floor, bursts, dense passages, silences and sustains.
        let amplitude = libm::exp(asked.0 * core::f64::consts::LN_10 / 20.0_f64);
        if segment == 0 {
            // Tails: a burst, then a silence, by turns.
            kind = if setting == Drawn::Tails {
                if kind == 1 { 3_u8 } else { 1_u8 }
            } else {
                pick(draws, &[0_u8, 1, 1, 2, 3, 4])
            };
            floor = amplitude
                * pick(
                    draws,
                    &[0.0_f64, 0.3_f64, 0.9_f64, 1.05_f64, 1.5_f64, 3.0_f64],
                );
            level = amplitude * pick(draws, &[1.2_f64, 2.0_f64, 4.0_f64, 16.0_f64, 100.0_f64]);
            segment = match kind {
                3 if setting == Drawn::Tails => {
                    frames_of_duration(pick(draws, &[750.0_f64, 1_000.0_f64, 1_250.0_f64]), rate)
                }
                3 => {
                    1 + usize::try_from(draws.next() % u64::try_from(8 * maximum).expect("few"))
                        .expect("few")
                }
                1 => {
                    1 + usize::try_from(draws.next() % u64::try_from(3 * maximum).expect("few"))
                        .expect("few")
                }
                _ => {
                    maximum
                        + usize::try_from(draws.next() % u64::try_from(16 * maximum).expect("few"))
                            .expect("few")
                }
            };
            if unit(draws) < 0.2_f64 || frame == 0 {
                let cadence = pick(draws, &[64_usize, 16, 4, 1]);
                every = u64::try_from((maximum / cadence).max(1)).expect("few")
                    * if unit(draws) < 0.2_f64 { 4 } else { 1 };
            }
        }
        segment -= 1;
        let peak = match kind {
            0 => floor * (0.5_f64 + 0.5_f64 * unit(draws)),
            1 if unit(draws) < 0.7_f64 => level * (0.6_f64 + 0.4_f64 * unit(draws)),
            2 if unit(draws) < 0.25_f64 => level * unit(draws),
            1 | 2 => floor * unit(draws),
            3 => 0.0_f64,
            _ => floor,
        };
        // The commands before this frame: one to three sets, and a mode toggled.
        if draws.next().is_multiple_of(every) {
            let count = if unit(draws) < 0.15_f64 {
                2 + draws.next() % 2
            } else {
                1
            };
            for _ in 0..count {
                if let Some(at) = stage_index(limiter.change) {
                    reached.sets[at] += 1;
                }
                if setting != Drawn::Tails
                    && unit(draws)
                        < if setting == Drawn::Lookahead {
                            1.0_f64
                        } else {
                            0.6_f64
                        }
                {
                    asked.1 = pick(draws, &lookaheads);
                }
                if setting == Drawn::Tails {
                    asked.1 = pick(draws, &lookaheads);
                }
                if setting == Drawn::Everything {
                    if unit(draws) < 0.35_f64 {
                        asked.2 = pick(draws, releases);
                    }
                    if unit(draws) < 0.35_f64 {
                        asked.3 = pick(draws, &ADAPTIVITIES);
                    }
                    if unit(draws) < 0.15_f64 {
                        asked.0 = pick(draws, &CEILINGS);
                    }
                }
                limiter
                    .set(settings(asked.0, asked.1, asked.2, asked.3))
                    .expect("within the maximum");
                // Rule 1: the release asked in force from the next frame, but while a shrink
                // runs (rebuilt or under way), where it waits for its end; the one in force
                // asked again drops the one held.
                let wanted = (asked.2, asked.3);
                if wanted == release {
                    held = None;
                } else if running.is_some_and(|(_, shrinking)| shrinking) {
                    held = Some(wanted);
                    reached.held += 1;
                } else {
                    release = wanted;
                    release_from = step;
                }
                for (_, lookahead, reference) in &mut always {
                    reference
                        .set(settings(asked.0, *lookahead, release.0, release.1))
                        .expect("within the maximum");
                }
            }
        }
        if switchable
            && draws
                .next()
                .is_multiple_of(u64::try_from(30 * maximum).expect("few"))
        {
            protected = !protected;
            limiter.set_protected(protected);
            for (_, _, reference) in &mut always {
                reference.set_protected(protected);
            }
        }
        // The frame, beside every limiter always at a lookahead.
        let stage = limiter.change;
        let fully = limiter.applied == Binary64::ONE;
        // The windows of the paths that may run on this frame, by the test's own record: the
        // one in force and, while a change runs, its new one.
        let running_windows = [Some(in_force), running.map(|(new, _)| new)];
        let windows = (limiter.path.lookahead, limiter.previous.lookahead);
        let reduction = 1.0_f64 - gain_of(&mut limiter, peak);
        let theirs: Vec<(usize, f64)> = always
            .iter_mut()
            .map(|(count, _, reference)| (*count, 1.0_f64 - gain_of(reference, peak)))
            .collect();
        let here = format!(
            "{context}, frame {frame}, from {stage:?} to {:?}",
            limiter.change
        );
        // The ceiling the test's needs read, computed apart, the limiter's.
        let ceiling = libm::exp(asked.0 * core::f64::consts::LN_10 / 20.0_f64);
        assert_eq!(
            limiter.ceiling.raw().to_bits(),
            ceiling.to_bits(),
            "{here}: the ceiling"
        );
        needs.rotate_left(1);
        *needs.last_mut().expect("needs") = need_above(peak, ceiling, margin_of(maximum));
        // What the test observes of the change: its end, then its beginning, on this frame.
        let ended = matches!(
            stage,
            Some(super::Stage::Shrinking { .. } | super::Stage::Descending { .. })
        ) && matches!(limiter.change, None | Some(super::Stage::Rebuilding(_)));
        if ended {
            let (new, _) = running.take().expect("a change running");
            in_force = new;
            if let Some(wanted) = held.take() {
                release = wanted;
                release_from = step + 1;
                for (_, lookahead, reference) in &mut always {
                    reference
                        .set(settings(asked.0, *lookahead, release.0, release.1))
                        .expect("within the maximum");
                }
            }
        }
        if matches!(stage, Some(super::Stage::Shrinking { .. }))
            && matches!(limiter.change, Some(super::Stage::Rising { .. }))
        {
            reached.risen += 1;
        }
        if stage == Some(super::Stage::Pending) && limiter.change.is_none() {
            reached.dropped += 1;
        }
        if stage == Some(super::Stage::Rebuilding(1)) {
            taken = Some(step);
        }
        if let Some(super::Stage::Rebuilding(left)) = limiter.change
            && !matches!(stage, Some(super::Stage::Rebuilding(_)))
        {
            // Begun on this frame, toward the lookahead asked for last: its rebuild's frames,
            // or a growth's wait for the life of the path in force, or a shrink's for the
            // release in force (decision 12), computed apart.
            let new = window(asked.1);
            let shrinking = new < in_force;
            let wait = taken.map_or(0, |taken| {
                usize::try_from(
                    (taken + u64::try_from(new).expect("few"))
                        .saturating_sub(step + u64::try_from(in_force).expect("few")),
                )
                .expect("few")
            });
            let replay = if shrinking {
                usize::try_from(
                    (release_from + u64::try_from(in_force - new).expect("few") - 1)
                        .saturating_sub(step),
                )
                .expect("few")
            } else {
                0
            };
            assert_eq!(
                left,
                rebuild.max(wait).max(replay),
                "{here}: the rebuild's frames"
            );
            reached.waits += u32::from(!shrinking && wait > rebuild);
            reached.replays += u32::from(shrinking && replay > rebuild.max(wait));
            reached.growths += u32::from(!shrinking);
            reached.shrinks += u32::from(shrinking);
            let first = step + u64::try_from(left).expect("few") + 1;
            reached.off_phase += u32::from((first - 1) % u64::try_from(new).expect("few") != 0);
            running = Some((new, shrinking));
            if !used.contains(&new) {
                used.push(new);
            }
        }
        // Rule 1 as the test records it is the limiter's: its release that of the limiters
        // given the release on the frames the test applies it.
        assert_eq!(
            (limiter.keep_fast, limiter.slow_attack),
            (always[0].2.keep_fast, always[0].2.slow_attack),
            "{here}: the release in force"
        );
        // Both sides of the law, on every frame, every stage: within the smallest and the
        // largest of the limiters always at the lookaheads used so far.
        let used_theirs: Vec<f64> = theirs
            .iter()
            .filter(|(count, _)| used.contains(count))
            .map(|(_, theirs)| *theirs)
            .collect();
        // The upper side allows what a path in force carries from an earlier window beyond
        // the envelope, at most the largest Hann mean, over the windows of the paths that ran
        // on the frame, of the released reductions of the fixed limiter at the longest
        // lookahead used, times its share applied, within its own rounding (`hann_mean`,
        // derived), counted.
        let largest = used_theirs.iter().copied().fold(0.0_f64, larger);
        let mut envelope = used_theirs.clone();
        if reduction > largest + slack {
            let longest = used.iter().copied().max().expect("a lookahead used");
            let (_, _, widest) = always
                .iter()
                .find(|(count, _, _)| *count == longest)
                .expect("a fixed limiter at each lookahead used");
            let derived = running_windows
                .into_iter()
                .flatten()
                .map(|length| hann_mean(|back| widest.path.released_before(back).raw(), length))
                .fold(0.0_f64, larger)
                * widest.applied.raw();
            reached.carried += 1;
            if reduction - largest > reached.excess.0 {
                reached.excess = (reduction - largest, derived - largest);
            }
            envelope.push(derived + rounding);
        }
        within_theirs(
            reduction,
            &envelope,
            slack,
            &format!(
                "{here}, windows {:?}, used {used:?}, theirs {theirs:?}",
                (limiter.path.lookahead, limiter.previous.lookahead)
            ),
        );
        if stage.is_some() && limiter.applied != Binary64::ONE && limiter.applied != Binary64::ZERO
        {
            reached.crossfading += 1;
        }
        // Coverage, protected: at least the needs of the samples it applies to, less the
        // control path's rounding and `6.0001u` (as `swept_through_a_change`).
        if !switchable {
            let held_need = needs[needs.len() - maximum - 2..=needs.len() - maximum]
                .iter()
                .copied()
                .fold(0.0_f64, larger);
            assert!(
                reduction >= held_need - rounding - 6.000_1_f64 * unit_round,
                "{here}: reduced by {reduction}, {held_need} needed"
            );
        }
        // The design's own step law, fully protected on both frames: each step at most the
        // larger step of the two paths' own reductions (of a path that ran on both frames),
        // the rise's budget, the stage's glide, and both paths' rounding with the gains'
        // (decision 12; the rounding also holds a stage that no longer moves and reaches the
        // path at once, by less than the frames between the windows times its rounding).
        // Each path's reduction from its sums, by the window it ran over: a change begun on
        // the frame prepares the old path for the next one, its sums left as they were.
        let mean = |path: &super::Path, length: usize| {
            let sums = path.sums;
            larger(
                0.0_f64,
                (sums.sum.raw() - sums.real.raw())
                    * (1.0_f64 / f64::from(u32::try_from(length + 1).expect("few"))),
            )
        };
        let mut ran = vec![if stage == Some(super::Stage::Rebuilding(1)) {
            (windows.0, mean(&limiter.previous, windows.0))
        } else {
            (windows.0, mean(&limiter.path, windows.0))
        }];
        if matches!(
            stage,
            Some(
                super::Stage::Rising { .. }
                    | super::Stage::Shrinking { .. }
                    | super::Stage::Descending { .. }
            )
        ) {
            ran.push((windows.1, mean(&limiter.previous, windows.1)));
        }
        let protected_now = limiter.applied == Binary64::ONE;
        if let Some((was, ran_before)) = &last
            && fully
            && protected_now
        {
            let moved = ran
                .iter()
                .filter_map(|(lookahead, now)| {
                    ran_before
                        .iter()
                        .find(|(then, _)| then == lookahead)
                        .map(|(_, then)| (now - then).abs())
                })
                .fold(0.0_f64, larger);
            let budget = match stage {
                Some(super::Stage::Rising { budget, .. }) => budget.raw(),
                _ => 0.0_f64,
            };
            let between = limiter
                .path
                .lookahead
                .abs_diff(limiter.previous.lookahead)
                .max(1);
            let glide = glide_at(stage, limiter.path.smoothed().raw(), between);
            let allowed = moved + budget + glide + slack;
            let taken_step = (reduction - was).abs();
            assert!(
                taken_step <= allowed,
                "{here}: a step of {taken_step}, {allowed} allowed ({moved} moved, budget \
                 {budget}, glide {glide})"
            );
            reached.stepped += 1;
        }
        last = protected_now.then_some((reduction, ran));
        // No stored value below the least kept, while a change runs.
        if limiter.change.is_some() || stage.is_some() {
            kept_values(&limiter, &here);
        }
        if frame % 16 == 0 && near_the_least_kept(&limiter) {
            reached.near += 1;
        }
    }
    reached
}

/// Checks that every value of the stages and release stages of `limiter` stored through
/// `math::kept` ([`Stored::Kept`]) is zero or at least `2^-100` ([`zero_or_kept`]): its rings
/// apart, which the laws of a release to zero check.
fn kept_values(limiter: &TruePeakLimiter<Binary64>, context: &str) {
    each_stored(limiter, &mut |part| {
        if let Stored::Kept(value) = part {
            assert!(
                zero_or_kept(value),
                "{context}: {value:?} stored, below the least value kept"
            );
        }
    });
}

/// `more` added to `total`, count by count.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "counts of a few hundred thousand, far from overflow"
)]
fn add_reached(total: &mut Reached, more: Reached) {
    for (sum, count) in [
        (&mut total.growths, more.growths),
        (&mut total.shrinks, more.shrinks),
        (&mut total.waits, more.waits),
        (&mut total.replays, more.replays),
        (&mut total.held, more.held),
        (&mut total.risen, more.risen),
        (&mut total.off_phase, more.off_phase),
        (&mut total.dropped, more.dropped),
        (&mut total.crossfading, more.crossfading),
        (&mut total.near, more.near),
        (&mut total.stepped, more.stepped),
        (&mut total.carried, more.carried),
    ] {
        *sum += count;
    }
    if more.excess.0 > total.excess.0 {
        total.excess = more.excess;
    }
    for (sum, count) in total.sets.iter_mut().zip(more.sets) {
        *sum += count;
    }
}

#[test]
fn stays_between_limiters_always_at_its_lookaheads_while_its_lookahead_changes() {
    // At every rate, at maxima of 0.5 ms (no lookahead change: the release, the ceiling and
    // the mode only), 0.625 (the smallest with a change at every rate), 1 and 5 ms, over
    // seeded random sequences of every live setting ([`stays_within_fixed_limiters_through`]:
    // one to three sets at one frame, on one frame in `M/64` to `4M`, so that every stage
    // receives sets; lookaheads of close pairs, a frame apart and at the maximum; every
    // release, adaptivity and ceiling; the mode toggled in switchable runs; bursts, dense
    // passages, sustains, silences, and release tails that reach `2^-100`), beside a limiter
    // always at each lookahead it may draw, given the same ceilings and modes on the same
    // frames and the release on the frame the limiter applies it, which the test computes
    // from Rule 1 (decision 12: held while a shrink runs, in force from the frame after it
    // ends) and its own record of the changes' ends (asserted: the limiter's release is
    // theirs on every frame), and its own record of every change's rebuild (its frames, a
    // growth's wait and a shrink's, computed apart and asserted):
    // - both sides of the law, on every frame, every stage: the reduction within the smallest
    //   and the largest of theirs at the lookaheads used so far, both paths' rounding `2ρ` and
    //   `u` for each gain and its difference from one allowed (`spec/measurements.md`, "The
    //   true-peak limiter", step 6: proven for a change from a path that always ran, held
    //   here over sequences). Measured on 2026-11-19 in the model of the design: 0 frames
    //   outside, of 22.2 million;
    // - coverage, protected: every reduction at least the needs of the samples it applies
    //   to, computed apart, less `ρ` and `6.0001u`;
    // - the design's own step law, fully protected: each step at most the larger step of
    //   the paths that ran on both frames, plus the rise's budget and the stage's glide;
    // - no value of a stage stored below `2^-100`.
    // The control counts each regime from the test's own record, each at least its floor at
    // every rate, so that a change of the draws that left one out fails (each floor half the
    // least count measured on 2026-11-19 over the rates, or below: from 1 shrink ending in a
    // rise at 44.1 and 48 kHz to 6,859 sets while shrinking at 384 kHz). The upper side over
    // a sequence allows what a path in force carries from an earlier window, at most its
    // derived bound ([`hann_mean`]), the frames above the envelope counted.
    const FLOORS: [(&str, u32); 15] = [
        ("growths", 700),
        ("shrinks", 700),
        ("growths that wait", 9),
        ("shrinks that wait for the release", 13),
        ("releases held", 340),
        ("shrinks that end in a rise", 1),
        ("changes off the canonical phase", 1_300),
        ("lookaheads dropped", 30),
        ("sets while pending", 190),
        ("sets while rebuilding", 1_000),
        ("sets while rising", 250),
        ("sets while shrinking", 2_800),
        ("sets while descending", 430),
        ("frames near the least kept", 20),
        ("frames of a change while the mode crossfades", 120),
    ];
    let mut draws = crate::tests::splitmix::Draws::new();
    for rate in rates() {
        let mut total = Reached::default();
        for most in [0.5_f64, 0.625_f64, 1.0_f64, 5.0_f64] {
            let maximum = frames_of_duration(most, rate);
            for (drawn, switchable) in [
                (Drawn::Everything, false),
                (Drawn::Everything, true),
                (Drawn::Lookahead, false),
            ] {
                add_reached(
                    &mut total,
                    stays_within_fixed_limiters_through(
                        rate,
                        most,
                        (drawn, switchable, 48 * maximum),
                        &mut draws,
                    ),
                );
            }
        }
        add_reached(
            &mut total,
            stays_within_fixed_limiters_through(
                rate,
                0.625_f64,
                (Drawn::Tails, false, frames_of_duration(2_000.0_f64, rate)),
                &mut draws,
            ),
        );
        let counts = [
            total.growths,
            total.shrinks,
            total.waits,
            total.replays,
            total.held,
            total.risen,
            total.off_phase,
            total.dropped,
            total.sets[0],
            total.sets[1],
            total.sets[2],
            total.sets[3],
            total.sets[4],
            total.near,
            total.crossfading,
        ];
        for ((name, floor), count) in FLOORS.into_iter().zip(counts) {
            assert!(
                count >= floor,
                "{} Hz: {count} {name}, below {floor} ({total:?})",
                rate.hz()
            );
        }
        assert!(total.stepped > 0, "{} Hz: {total:?}", rate.hz());
    }
}

/// The blocks of `N` frames the heap test of audio-host's engine runs in each of its five
/// phases on a limiter of a 2 ms maximum lookahead and the precise detector at `rate`
/// (`renders_through_a_limited_stage_without_a_heap_operation_in_blocks_of`): its latency,
/// counted apart, and four blocks.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "frame counts of a few thousand, far from overflow"
)]
fn heap_phase_blocks<const N: usize>(rate: SampleRate) -> usize {
    latency_of(2.0_f64, rate, super::Detector::Precise).div_ceil(N) + 4
}

/// The phases of audio-host's heap test of live changes, replayed on the limiter alone in
/// blocks of `N` frames at every rate: its patch (`LOUD` there: sines of 440, 1,000 and
/// 3,000 Hz summed on the first channel, the first two on the second, each at full scale), a
/// stage of a 2 ms maximum and the precise detector at the default settings, five phases
/// of its blocks ([`heap_phase_blocks`]), the lookahead set to 0.5 ms after the first, and to
/// 2 ms after the second phase's first block, while that shrink runs where the block is
/// shorter than it, so that the growth begins as it ends, waiting for the life of the path
/// it put in force; the mode switched after the third phase and the fourth. Every lookahead
/// change must be over before the five phases end, its last frames counted by the heap law
/// there, in `S` (a binary64 render given as binary32 limits in binary64); the crossfades do
/// not arrive within them, and are counted to their arrival apart
/// ([`arrives_at_each_crossfade_within_audio_hosts_heap_frames`]). Whether the growth's
/// rebuild was seen waiting past its own frames, at each rate.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "frame counts of a few hundred thousand, far from overflow"
)]
fn ends_each_change_within_the_heap_tests_phases_in_blocks_of<S: Sample, const N: usize>()
-> Vec<bool> {
    let mut waits = Vec::new();
    for rate in rates() {
        let context = format!(
            "{} Hz, blocks of {N} in binary{}",
            rate.hz(),
            S::PRECISION.bits()
        );
        let mut limiter = TruePeakLimiter::<S>::new(
            rate,
            2,
            value(2.0_f64),
            super::Detector::Precise,
            LimiterSettings::DEFAULT,
            LimiterMode::Switchable,
        )
        .expect("a limiter");
        assert_eq!(
            limiter.latency(),
            latency_of(2.0_f64, rate, super::Detector::Precise),
            "{context}: counted apart"
        );
        let hz = f64::from(rate.hz());
        let rebuild = rebuild_frames_of(frames_of_duration(2.0_f64, rate));
        let mut frame = 0_u32;
        let mut waited = false;
        let mut blocks_of = |stage: &mut TruePeakLimiter<S>, count: usize| {
            for _ in 0..count {
                let mut blocks = [[S::ZERO; N]; 2];
                let [left, right] = &mut blocks;
                for (first, second) in left.iter_mut().zip(right.iter_mut()) {
                    let at = f64::from(frame);
                    let sine =
                        |frequency: f64| libm::sin(core::f64::consts::TAU * frequency * at / hz);
                    *first = of::<S>(sine(440.0_f64) + sine(1_000.0_f64) + sine(3_000.0_f64));
                    *second = of::<S>(sine(440.0_f64) + sine(1_000.0_f64));
                    frame += 1;
                }
                let before = stage.change;
                stage.apply(&mut blocks);
                // A growth begun on this block's frames as the shrink ended: its rebuild's
                // frames left, at most its own on the block's last frame, more if it waits.
                if let Some(super::Stage::Rebuilding(remaining)) = stage.change
                    && stage.rebuild.longer
                    && !matches!(before, Some(super::Stage::Rebuilding(_)))
                {
                    waited |= remaining > rebuild;
                }
            }
        };
        let phase = heap_phase_blocks::<N>(rate);
        blocks_of(&mut limiter, phase);
        limiter
            .set(
                LimiterSettings::DEFAULT
                    .with_lookahead(value(0.5_f64))
                    .expect("in range"),
            )
            .expect("within the maximum");
        blocks_of(&mut limiter, 1);
        limiter
            .set(
                LimiterSettings::DEFAULT
                    .with_lookahead(value(2.0_f64))
                    .expect("in range"),
            )
            .expect("within the maximum");
        blocks_of(&mut limiter, phase - 1);
        blocks_of(&mut limiter, phase);
        limiter.set_protected(true);
        blocks_of(&mut limiter, phase);
        limiter.set_protected(false);
        blocks_of(&mut limiter, phase);
        assert_eq!(
            limiter.change, None,
            "{context}: every lookahead change over"
        );
        assert_eq!(
            limiter.path.lookahead,
            frames_of_duration(2.0_f64, rate),
            "{context}: the last lookahead in force"
        );
        waits.push(waited);
    }
    waits
}

/// The positive control of audio-host's heap test of the crossfades
/// (`crossfades_to_each_arrival_without_a_heap_operation`), in `S`: at every rate, a
/// switchable limiter crossfaded to protected, then back to raw, each for its frames there,
/// computed apart (`crossfade_frames` there, `⌈0.76 × rate⌉`: what is left of the way, `e^-38`
/// by then, below `2^-54`, where `1 - share` rounds to 1), arrives exactly, its share 1, then
/// 0 with nothing left of the way, so that the arrival is counted; not after `⌊0.74 × rate⌋`
/// frames (`e^-37` left, above `2^-54`), so that the frames are not loose by much.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "frame counts of a few hundred thousand, far from overflow"
)]
fn arrives_at_each_crossfade_within_audio_hosts_heap_frames_in<S: Sample>() {
    for rate in rates() {
        let context = format!("{} Hz in binary{}", rate.hz(), S::PRECISION.bits());
        let mut limiter = TruePeakLimiter::<S>::new(
            rate,
            1,
            value(1.5_f64),
            super::Detector::Standard,
            LimiterSettings::DEFAULT,
            LimiterMode::Switchable,
        )
        .expect("a limiter");
        let hz = usize::try_from(rate.hz()).expect("a rate");
        let (arrived, before) = ((76 * hz).div_ceil(100), 74 * hz / 100);
        for protected in [true, false] {
            limiter.set_protected(protected);
            let target = if protected {
                Binary64::ONE
            } else {
                Binary64::ZERO
            };
            for frame in 1..=arrived {
                let _: f64 = gain_in(&mut limiter, 0.5_f64);
                if frame == before {
                    assert_ne!(limiter.applied, target, "{context}: not arrived yet");
                }
            }
            assert_eq!(limiter.applied, target, "{context}: arrived, {protected}");
            if !protected {
                assert_eq!(limiter.remaining, Binary64::ZERO, "{context}: nothing left");
            }
        }
    }
}

#[test]
fn arrives_at_each_crossfade_within_audio_hosts_heap_frames() {
    arrives_at_each_crossfade_within_audio_hosts_heap_frames_in::<Binary64>();
    arrives_at_each_crossfade_within_audio_hosts_heap_frames_in::<Binary32>();
}

#[test]
fn ends_each_change_within_the_heap_tests_phases() {
    // The positive control of audio-host's heap law of live changes: at every engine block,
    // every change it makes runs to its end inside the frames whose heap operations it counts;
    // and at the finest engine block, its growth waits for the life of the path the shrink put
    // in force at some rate (seen at a block's end, more frames left than its rebuild's), so
    // that the heap law counts a wait's frames too (where a block holds the whole shrink, the
    // growth is set after it, from a path that lived long enough).
    let finest = ends_each_change_within_the_heap_tests_phases_in_blocks_of::<Binary64, 16>();
    assert!(finest.iter().any(|waited| *waited), "a wait: {finest:?}");
    drop(ends_each_change_within_the_heap_tests_phases_in_blocks_of::<Binary32, 16>());
    drop(ends_each_change_within_the_heap_tests_phases_in_blocks_of::<Binary64, 32>());
    drop(ends_each_change_within_the_heap_tests_phases_in_blocks_of::<Binary32, 32>());
    drop(ends_each_change_within_the_heap_tests_phases_in_blocks_of::<Binary64, 64>());
    drop(ends_each_change_within_the_heap_tests_phases_in_blocks_of::<Binary32, 64>());
    drop(ends_each_change_within_the_heap_tests_phases_in_blocks_of::<Binary64, 128>());
    drop(ends_each_change_within_the_heap_tests_phases_in_blocks_of::<Binary32, 128>());
    drop(ends_each_change_within_the_heap_tests_phases_in_blocks_of::<Binary64, 256>());
    drop(ends_each_change_within_the_heap_tests_phases_in_blocks_of::<Binary32, 256>());
    drop(ends_each_change_within_the_heap_tests_phases_in_blocks_of::<Binary64, 512>());
    drop(ends_each_change_within_the_heap_tests_phases_in_blocks_of::<Binary32, 512>());
    drop(ends_each_change_within_the_heap_tests_phases_in_blocks_of::<Binary64, 1024>());
    drop(ends_each_change_within_the_heap_tests_phases_in_blocks_of::<Binary32, 1024>());
}

/// The gain of the control path for a frame whose peak is `peak`, in binary64: 1 when it
/// gives none (no reduction).
fn gain_of(limiter: &mut TruePeakLimiter<Binary64>, peak: f64) -> f64 {
    limiter.gain(value(peak)).map_or(1.0_f64, Binary64::raw)
}

/// The gain of the control path of `limiter`, in `S`, for a frame whose peak is `peak` (a
/// value of `S`), in binary64: 1 when it gives none (no reduction).
fn gain_in<S: Sample>(limiter: &mut TruePeakLimiter<S>, peak: f64) -> f64 {
    limiter
        .gain(of::<S>(peak))
        .map_or(1.0_f64, |gain| gain.to_binary64().raw())
}

/// The smaller of `first` and `second`.
fn smaller_of(first: f64, second: f64) -> f64 {
    if second < first { second } else { first }
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

/// The bursts the control path is driven with while its lookahead changes: each its width
/// in frames (0 for twice the maximum lookahead), its peak, the floor of peaks around it,
/// and whether it runs at a maximum lookahead above [`DENSE`] frames too (one of each
/// floor and width, so that the sweep there stays within seconds). Below the ceiling, the
/// floor needs nothing; 1.25 and 1.0 a sustained reduction, which the burst rises from; 16
/// a need near the whole signal.
const BURSTS: [(usize, f64, f64, bool); 6] = [
    (1, 2.0_f64, 0.5_f64, true),
    (10, 2.0_f64, 0.5_f64, false),
    (0, 2.0_f64, 0.5_f64, false),
    (10, 16.0_f64, 0.5_f64, false),
    (10, 4.0_f64, 1.25_f64, true),
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

/// A lookahead changed live from `from` to `to` milliseconds (a maximum of `most`), at every rate,
/// while the control path holds each burst of [`BURSTS`], the change made at every depth of
/// [`change_frames`], from twice the maximum before the burst arrives to past its last need:
/// every frame's reduction covers the needs of the samples it applies to, and each one-frame
/// step of the gain is at most the attack the change allows, `need × max(hp(new), hp(D))`
/// growing and `need × hp(new)` shrinking, plus the rounding its derivation gives for that
/// attack and the windows it weighs ([`step_slack`]: `2ρ` and from `3u` to about `20u`),
/// with `2^-24` more in binary32, whose gains are rounded toward zero in binary32.
///
/// - `need` is the burst's, `1 - c / peak + E`, or a louder passage's before the sets when it
///   is larger (the most its release holds when the change starts), `hp(L)` the largest Hann
///   weight of a window of `L` frames ([`hann_peak`]) and `D` the frames from the change's
///   first frame to the first that needs the burst's first need (`onset + maximum - 1`, the
///   needs covered at a frame `n` being those of `n - maximum - 1` to `n - maximum + 1`), the
///   term dropped when `D < 1`; all computed apart, as are the margin `E` ([`margin_of`]) and
///   `ρ` ([`rounding_of`]).
/// - The change starts [`rebuild_frames_of`] frames after it is set, or after a growth's wait
///   for the life of the path in force ([`swept_through_a_change`]), the old path alone in
///   force until then: through those frames, every step at most the old window's attack of
///   that need, as before a change.
/// - Covered: each reduction, `1 - gain`, at least the largest of those needs less `ρ` and
///   the roundings of the gain and of the needs, `6.0001u`.
/// - Before the change, every step at most the old window's attack of that need.
/// - The change over by the end of each run: `maximum + 2 × longer + 64` frames past the
///   burst's first frame or the change, the later (the burst's attack complete by then).
/// - The burst limited: some gain below `1 - need / 2`.
///
/// The control path alone (`TruePeakLimiter::gain`, a frame's peak in, its gain out), the
/// detector and the delay apart: the frames before a change are the same at every depth,
/// run once, and each depth branches from them by a copy of the limiter, changed and run.
fn steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes<S: Sample>(
    most: f64,
    from: f64,
    to: f64,
) {
    let mut waits = 0_u32;
    for rate in rates() {
        let maximum = frames_of_duration(most, rate);
        for (width, level, floor, everywhere) in BURSTS {
            if maximum > DENSE && !everywhere {
                continue;
            }
            let width = if width == 0 {
                maximum.saturating_mul(2)
            } else {
                width
            };
            let waited = swept_through_a_change::<S>(
                rate,
                most,
                (from, to),
                (width, level, floor, 0.0_f64),
                Prior::Fresh,
            );
            assert!(!waited, "fresh: no wait");
        }
        // From a used limiter too, on one burst of each kind (the burst over a sustained
        // reduction, and a need near the whole signal over a sustained floor): the prior change
        // long over, and just over when the sweep's sets begin.
        waits = waits.saturating_add(u32::from(swept_through_a_change::<S>(
            rate,
            most,
            (from, to),
            (10, 4.0_f64, 1.25_f64, 0.0_f64),
            Prior::JustOver,
        )));
        let waited = swept_through_a_change::<S>(
            rate,
            most,
            (from, to),
            (maximum.saturating_mul(2), 16.0_f64, 1.0_f64, 0.0_f64),
            Prior::LongOver,
        );
        assert!(!waited, "long over: no wait");
        // A burst over a floor that needs nothing, after a louder passage whose release holds
        // a reduction above the burst's need when the change starts: the attack is that
        // reduction's, not the burst's need's.
        let released = swept_through_a_change::<S>(
            rate,
            most,
            (from, to),
            (10, 2.0_f64, 0.5_f64, 16.0_f64),
            Prior::Fresh,
        );
        assert!(!released, "fresh: no wait");
    }
    // A growth by a millisecond or more, set just after the prior change, waits at some rate:
    // the regime of the wait reached.
    if to - from >= 1.0_f64 {
        assert!(waits > 0, "{from} to {to} ms: growths that wait");
    }
}

/// What a sweep of [`swept_through_a_change`] does before its sets: nothing, from a limiter
/// built at its first lookahead; or one change to that lookahead from a limiter built at
/// another ([`prior_lookahead`]), set on the first frame and long over when the sweep's sets
/// begin, or set so that it is over just before they begin, while the path it put in force
/// has lived less than a growth's new window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Prior {
    /// No change before the sweep's sets.
    Fresh,
    /// A change on the first frame, long over.
    LongOver,
    /// A change over just before the first set.
    JustOver,
}

/// The lookahead a used sweep's limiter is built at before its change to `from` milliseconds
/// (a maximum of `most`): half of it when that is a lookahead, a growth to `from`; else
/// `from + 0.1`, a shrink by a few frames. Either change is short on a constant need: its
/// rebuild and two frames growing, the old window, one frame and a short descent shrinking.
fn prior_lookahead(from: f64, most: f64) -> f64 {
    let half = from / 2.0_f64;
    if half >= 0.5_f64 {
        half
    } else {
        assert!(
            from + 0.1_f64 <= most,
            "{from} of {most} ms: room to shrink from"
        );
        from + 0.1_f64
    }
}

/// The default ceiling's amplitude in `S`, and what rounding each gain toward zero in `S`
/// adds to one step of two gains: in binary64, `CEILING` and nothing; in binary32, the
/// ceiling rounded toward zero in binary32 (`0.891_250_9_f32` is `10^(-1/20)` rounded to
/// the nearest, below it), and less than one binary32 ulp of a gain, at most `2^-24` for
/// gains up to 1 (each gain lowered by less than its ulp).
fn ceiling_and_gain_rounding<S: Sample>() -> (f64, f64) {
    match S::PRECISION {
        crate::Precision::Binary64 => (CEILING, 0.0_f64),
        crate::Precision::Binary32 => (f64::from(0.891_250_9_f32), libm::exp2(-24.0_f64)),
    }
}

/// What a one-frame step of the gain may add, by rounding, to the attack `attack` a live
/// lookahead change allows (`need × attack`, the need and the weights computed by the tests),
/// its weights those of windows of `shortest` frames or more, with `ρ` `rounding` and
/// `toward_zero` what a binary32 gain's rounding toward zero adds ([`ceiling_and_gain_rounding`]),
/// derived in `spec/measurements.md`, "The true-peak limiter", step 6, with `u = 2^-53` and
/// every need and reduction below 1:
/// - `2ρ`: a path's value on each of the two frames, within `ρ` of its exact Hann mean (a
///   stage's step is no path's value, and its reduction at most `ρ` above the need, `ρ × attack`);
/// - `u`: the two gains `1 - r`, each within `u/2` (exact from `r = 1/2` on);
/// - `u/2`: a stage's sum or difference (the rise, the descent), a value below 1;
/// - `attack × u`: a stage's product by a weight;
/// - `4.0001u × attack`: the limiter's need against the tests', `2.5001u` (two roundings of
///   the share, then the margin's sum) and `1.5u` (three roundings), the ceiling the same;
/// - `2 × hp(L) × e(L)` for the windows of `shortest` frames or more, the limiter's weight
///   and the tests' each within `e(L)` of it ([`weight_rounding`]), the largest at `shortest`
///   or the one after (each parity non-increasing);
/// - `1.5u`: the tests' own product and sum of the allowance, and their difference of two
///   gains;
///
/// all times `1 + 2^-40`, above every second-order term and the roundings of this sum.
fn step_slack(rounding: f64, toward_zero: f64, attack: f64, shortest: usize) -> f64 {
    let error = |length: usize| hann_peak(length) * weight_rounding(length);
    let weights = larger(error(shortest), error(shortest.saturating_add(1)));
    let units = 3.0_f64 + 5.000_1_f64 * attack + 2.0_f64 * weights;
    (2.0_f64 * rounding + units * libm::exp2(-53.0_f64) + toward_zero)
        * (1.0_f64 + libm::exp2(-40.0_f64))
}

#[test]
fn bounds_a_live_changes_step_by_its_derivation() {
    // Computed apart (exact fractions, π to 40 digits), in units of 2^-53: at one frame, the
    // weights of one and two frames erring by `1 × (2/1.5 + 2.0001)` and
    // `0.5 × ((3.0001π² × 4/9 + 2)/1.5 + 2.0001)`, the second the larger; at four frames, by
    // `hp(4) × ((3.0001π² × 6/25 + 2)/1.5 + 2.0001)` and `(1/3)(2/1.5 + 2.0001)`, the first.
    let unit = libm::exp2(-53.0_f64);
    let grown = 1.0_f64 + libm::exp2(-40.0_f64);
    for (rounding, toward_zero, attack, shortest, units) in [
        (
            46.0_f64,
            0.0_f64,
            1.0_f64,
            1,
            92.0_f64 + 3.0_f64 + 5.000_1_f64 + 12.106_707_455_913_536_f64,
        ),
        (
            10_323.0_f64,
            libm::exp2(-24.0_f64),
            0.5_f64,
            4,
            20_646.0_f64 + 3.0_f64 + 2.500_05_f64 + 5.840_231_448_402_801_f64,
        ),
    ] {
        let expected = (units * unit + toward_zero) * grown;
        let slack = step_slack(rounding * unit, toward_zero, attack, shortest);
        assert!(
            (slack - expected).abs() <= libm::exp2(-40.0_f64) * expected,
            "{shortest} frames: {slack} against {expected}"
        );
    }
    // The precondition of `weight_rounding`: the weight next to the largest, of the index
    // one further from half a turn, below it by `(1 - cos(2π / (L + 1))) / (L + 1)` or more,
    // above `2^-33`, up to 1,920 frames (5 ms at 384 kHz, the longest).
    for length in 2_u32..=1_920 {
        let span = f64::from(length + 1);
        let weight = |index: u32| {
            (1.0_f64 - libm::cos(core::f64::consts::TAU * f64::from(index) / span)) / span
        };
        let index = length.div_ceil(2);
        let next = if length.is_multiple_of(2) {
            index - 1
        } else {
            index + 1
        };
        assert!(
            weight(index) - weight(next) > libm::exp2(-33.0_f64),
            "{length} frames"
        );
    }
}

/// One rate and burst of
/// [`steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes`]: the
/// lookahead changed from `from` to `to` milliseconds, around `width` frames of peaks of
/// `level` over a floor of `floor`, after `prior`, and after peaks of `louder` from the first
/// frame until the needs of the loudest leave every window before the first set (none when
/// zero): its release then holds a reduction up to its need when the change starts, so the
/// attack the change allows is the larger of the two needs, `N`, times the weights below. After a prior change, the path in force
/// took over its rebuild's frames after that change was set (from the path in force since the
/// limiter was built, which never waits), and a growth set at the step `s` waits until that
/// path's life spans its window, `taken + new - old - s` frames when more than its rebuild's
/// (`taken` the step of the takeover; decision 12), all computed apart; the old path alone in
/// force through the wait, its steps held to its own attack and every frame covered.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "frame counts of a few thousand and reductions below two, far from overflow"
)]
#[expect(
    clippy::too_many_lines,
    reason = "one sweep: the frames before its sets and each set's run share its closures"
)]
fn swept_through_a_change<S: Sample>(
    rate: SampleRate,
    most: f64,
    (from, to): (f64, f64),
    (width, level, floor, louder): (usize, f64, f64, f64),
    prior: Prior,
) -> bool {
    let (ceiling, toward_zero) = ceiling_and_gain_rounding::<S>();
    let maximum = frames_of_duration(most, rate);
    let (old, new) = (frames_of_duration(from, rate), frames_of_duration(to, rate));
    let context = format!(
        "{} Hz, {from} to {to} of {most} ms in binary{}, {width} frames of {level} over \
         {floor} after {louder}, {prior:?}",
        rate.hz(),
        S::PRECISION.bits()
    );
    assert_ne!(old, new, "{context}: two lookaheads");
    let longer = old.max(new);
    let (margin, rounding) = (margin_of(maximum), rounding_of(maximum));
    let need = |peak: f64| need_above(peak, ceiling, margin);
    // The floor settled long before the earliest change.
    let onset = 4 * maximum + 128;
    let frames = change_frames(onset, width, maximum, (old, new));
    let first = *frames.first().expect("frames");
    let peak = |frame: usize| {
        if (onset..onset + width).contains(&frame) {
            level
        } else if frame + maximum + 2 < first {
            larger(floor, louder)
        } else {
            floor
        }
    };
    let burst = need(level);
    let largest = larger(burst, need(louder));
    let rebuild = rebuild_frames_of(maximum);
    let unchanged =
        largest * hann_peak(old) + step_slack(rounding, toward_zero, hann_peak(old), old);
    // The needs of frames `frame - maximum - 1` to `frame - maximum + 1`, those before the
    // first frame none. A path's value within `(43L - 2)u` of its exact Hann mean, below `ρ`;
    // the gain within `u/2`, rounding toward zero in binary32 only raising `1 - gain`; the
    // limiter's need within `4.0001u` of the tests' ([`step_slack`]); the tests' difference
    // `1 - gain` and their two below one rounding each, `1.5u`: `6.0001u` past `ρ`.
    let covered = |frame: usize, gain: f64| {
        let held = (frame + 1).checked_sub(maximum).map_or(0.0_f64, |newest| {
            (newest.saturating_sub(2)..=newest)
                .map(|at| need(peak(at)))
                .fold(0.0_f64, larger)
        });
        assert!(
            1.0_f64 - gain >= held - rounding - 6.000_1_f64 * libm::exp2(-53.0_f64),
            "{context}: frame {frame} reduced by {}, {held} needed",
            1.0_f64 - gain
        );
    };
    let built = if prior == Prior::Fresh {
        from
    } else {
        prior_lookahead(from, most)
    };
    let before_from = frames_of_duration(built, rate);
    let at_first = || {
        TruePeakLimiter::<S>::new(
            rate,
            1,
            value(most),
            super::Detector::Standard,
            settings(-1.0_f64, built, 100.0_f64, 0.5_f64),
            LimiterMode::Protected,
        )
        .expect("a limiter")
    };
    // The frames the prior change lasts when set after `set`, from the frame after it to its
    // end, at most `constant_change_frames` on the floor, asserted before they size anything.
    let bound = constant_change_frames(maximum, before_from, old);
    let lasts = |set: usize| {
        let mut dry = at_first();
        for frame in 0..=set {
            let _: f64 = gain_in(&mut dry, peak(frame));
        }
        dry.set(settings(-1.0_f64, from, 100.0_f64, 0.5_f64))
            .expect("within the maximum");
        let mut lasted = 0;
        while dry.change.is_some() {
            lasted += 1;
            assert!(
                lasted <= bound,
                "{context}: the prior change within {bound} frames"
            );
            let _: f64 = gain_in(&mut dry, peak(set + lasted));
        }
        lasted
    };
    // The frame the prior change is set after, and the step its path takes over on.
    let prior_set = match prior {
        Prior::Fresh => None,
        Prior::LongOver => Some(0),
        // Over by the first set's frame, set as late as its length found from the earliest
        // such frame allows when it is then over by it too, the earliest otherwise.
        Prior::JustOver => {
            let earliest = first - 1 - bound;
            let later = first - 1 - lasts(earliest);
            Some(if later + lasts(later) < first {
                later
            } else {
                earliest
            })
        }
    };
    // The frame after the set, then the rebuild's frames: its last frame's step.
    let taken = prior_set.map(|set| set + 2 + rebuild);
    let mut limiter = at_first();
    // The ceiling the tests' needs read ([`step_slack`]).
    assert_eq!(
        limiter.ceiling.raw().to_bits(),
        ceiling.to_bits(),
        "{context}"
    );
    let mut before = Vec::new();
    let mut waited = false;
    let mut next = frames.iter().copied().peekable();
    for frame in 0..=*frames.last().expect("frames") {
        let gain = gain_in(&mut limiter, peak(frame));
        covered(frame, gain);
        before.push(gain);
        if prior_set == Some(frame) {
            assert_ne!(before_from, old, "{context}: a prior change");
            limiter
                .set(settings(-1.0_f64, from, 100.0_f64, 0.5_f64))
                .expect("within the maximum");
        }
        if next.next_if_eq(&frame).is_none() {
            continue;
        }
        if frame == first {
            assert_eq!(
                limiter.change, None,
                "{context}: the prior change over, set after frame {prior_set:?}"
            );
        }
        let mut changed = limiter.clone();
        changed
            .set(settings(-1.0_f64, to, 100.0_f64, 0.5_f64))
            .expect("within the maximum");
        // The frame it begins after and the rebuild, the old path alone in force, then `D`,
        // from the change's first frame.
        assert_eq!(
            changed.rebuild_frames, rebuild,
            "{context}: the rebuild's frames"
        );
        // Begun on the frame after the set, `frame + 1`, of the step `frame + 2`.
        let wait = taken.map_or(0, |taken| (taken + new).saturating_sub(frame + 2 + old));
        waited |= wait > rebuild;
        let rebuilding = rebuild.max(wait) + 1;
        let start = frame + 1 + rebuilding;
        let depth = (onset + maximum).checked_sub(start);
        let (attack, shortest) = attack_of(depth, (old, new));
        let allowed = largest * attack + step_slack(rounding, toward_zero, attack, shortest);
        let end = frame.max(onset) + maximum + 2 * longer + 64 + rebuilding;
        let mut gains = vec![gain];
        for at in frame + 1..=end {
            assert_eq!(
                matches!(
                    changed.change,
                    Some(super::Stage::Pending | super::Stage::Rebuilding(_))
                ),
                at < start,
                "{context}, set after frame {frame}: rebuilding on frame {at}, the change's \
                 first {start}"
            );
            let after = gain_in(&mut changed, peak(at));
            covered(at, after);
            gains.push(after);
        }
        // Through the rebuild, the old path's own steps; from the frame before the change.
        let (through, changing) = (&gains[..=rebuilding], &gains[rebuilding..]);
        steps_within(
            through,
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
                .any(|reduced| *reduced < 1.0_f64 - burst / 2.0_f64),
            "{context}, after frame {frame}: the burst limited"
        );
    }
    steps_within(&before, unchanged, &format!("{context}, before a change"));
    waited
}

/// The attack per unit of need a live change from `old` to `new` frames allows, the burst's
/// first need `depth` frames from the change's first frame (none when it came before), and
/// the shortest window whose weight it uses: `max(hp(new), hp(D))` growing with `D ≥ 1`,
/// `hp(new)` otherwise ([`hann_peak`]).
fn attack_of(depth: Option<usize>, (old, new): (usize, usize)) -> (f64, usize) {
    match depth {
        Some(depth) if new > old && depth >= 1 => {
            (larger(hann_peak(new), hann_peak(depth)), new.min(depth))
        }
        _ => (hann_peak(new), new),
    }
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
    steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes::<Binary64>(
        5.0_f64, 0.5_f64, 5.0_f64,
    );
}

#[test]
fn steps_within_the_attack_of_each_depth_and_covers_growing_live_from_1_5_to_5_ms() {
    steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes::<Binary64>(
        5.0_f64, 1.5_f64, 5.0_f64,
    );
}

#[test]
fn steps_within_the_attack_of_each_depth_and_covers_growing_live_from_0_5_to_1_5_ms() {
    steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes::<Binary64>(
        5.0_f64, 0.5_f64, 1.5_f64,
    );
}

#[test]
fn steps_within_the_attack_of_each_depth_and_covers_growing_live_from_3_to_4_8_ms() {
    steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes::<Binary64>(
        5.0_f64, 3.0_f64, 4.8_f64,
    );
}

#[test]
fn steps_within_the_attack_of_each_depth_and_covers_growing_live_from_2_5_to_5_ms() {
    steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes::<Binary64>(
        5.0_f64, 2.5_f64, 5.0_f64,
    );
}

#[test]
fn steps_within_the_attack_of_each_depth_and_covers_growing_live_from_4_9_to_5_ms() {
    steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes::<Binary64>(
        5.0_f64, 4.9_f64, 5.0_f64,
    );
}

#[test]
fn steps_within_the_new_attack_and_covers_shrinking_live_from_5_to_0_5_ms() {
    steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes::<Binary64>(
        5.0_f64, 5.0_f64, 0.5_f64,
    );
}

#[test]
fn steps_within_the_new_attack_and_covers_shrinking_live_from_5_to_1_5_ms() {
    steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes::<Binary64>(
        5.0_f64, 5.0_f64, 1.5_f64,
    );
}

#[test]
fn steps_within_the_new_attack_and_covers_shrinking_live_from_1_5_to_0_5_ms() {
    steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes::<Binary64>(
        5.0_f64, 1.5_f64, 0.5_f64,
    );
}

#[test]
fn steps_within_the_new_attack_and_covers_shrinking_live_from_4_8_to_3_ms() {
    steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes::<Binary64>(
        5.0_f64, 4.8_f64, 3.0_f64,
    );
}

#[test]
fn steps_within_the_new_attack_and_covers_shrinking_live_from_5_to_2_5_ms() {
    steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes::<Binary64>(
        5.0_f64, 5.0_f64, 2.5_f64,
    );
}

#[test]
fn steps_within_the_new_attack_and_covers_shrinking_live_from_5_to_4_9_ms() {
    steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes::<Binary64>(
        5.0_f64, 5.0_f64, 4.9_f64,
    );
}

#[test]
fn steps_within_the_attack_of_each_depth_and_covers_growing_live_from_0_5_to_1_ms_of_1_ms() {
    // A maximum of 1 ms, rebuilt in one frame up to 15 frames (8 and 11.025 kHz).
    steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes::<Binary64>(
        1.0_f64, 0.5_f64, 1.0_f64,
    );
}

#[test]
fn steps_within_the_attack_of_each_depth_and_covers_growing_live_from_0_6_to_0_8_ms_of_1_ms() {
    steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes::<Binary64>(
        1.0_f64, 0.6_f64, 0.8_f64,
    );
}

#[test]
fn steps_within_the_new_attack_and_covers_shrinking_live_from_1_to_0_5_ms_of_1_ms() {
    steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes::<Binary64>(
        1.0_f64, 1.0_f64, 0.5_f64,
    );
}

#[test]
fn steps_within_the_new_attack_and_covers_shrinking_live_from_0_8_to_0_6_ms_of_1_ms() {
    steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes::<Binary64>(
        1.0_f64, 0.8_f64, 0.6_f64,
    );
}

#[test]
fn steps_within_the_attack_of_each_depth_and_covers_growing_live_from_1_5_to_3_ms_in_binary32() {
    // Each gain rounded toward zero in binary32: one binary32 ulp more per step.
    steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes::<Binary32>(
        5.0_f64, 1.5_f64, 3.0_f64,
    );
}

#[test]
fn steps_within_the_new_attack_and_covers_shrinking_live_from_3_to_1_5_ms_in_binary32() {
    steps_within_the_attack_of_each_depth_and_covers_when_its_lookahead_changes::<Binary32>(
        5.0_f64, 3.0_f64, 1.5_f64,
    );
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
        let maximum = frames_of_duration(5.0_f64, rate);
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
                .saturating_add(frames_of_duration(1.5_f64, rate).saturating_mul(2))
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
                // The premise of the slow stage's one-pole in the limiter's proof
                // (`spec/measurements.md`, "The true-peak limiter", step 2): its step below a
                // half.
                assert!(
                    limiter.slow_attack.raw() < 0.5_f64,
                    "slow below a half, {what}"
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

/// Peaks of the release scenario of the targeted review of the live lookahead change: 50 ms
/// of peaks of 4.0, `loud` frames ([`frames_of_duration`]), then peaks of 0.5 (under the
/// ceiling, needing nothing), the reduction released; the peak of `frame`.
const fn released_after_loud(loud: usize, frame: usize) -> f64 {
    if frame < loud { 4.0_f64 } else { 0.5_f64 }
}

/// The reductions `1 - gain` of `limiter` and of each of `references` for the next frame,
/// all on the peak `peak`.
fn reductions_beside(
    limiter: &mut TruePeakLimiter<Binary64>,
    references: &mut [TruePeakLimiter<Binary64>],
    peak: f64,
) -> (f64, Vec<f64>) {
    let reduction = 1.0_f64 - gain_of(limiter, peak);
    let theirs = references
        .iter_mut()
        .map(|reference| 1.0_f64 - gain_of(reference, peak))
        .collect();
    (reduction, theirs)
}

/// Checks that `reduction` lies within the smallest and the largest of `theirs`, `slack`
/// outside either allowed, saying where.
fn within_theirs(reduction: f64, theirs: &[f64], slack: f64, context: &str) {
    let least = theirs.iter().copied().fold(f64::INFINITY, smaller_of);
    let most = theirs.iter().copied().fold(0.0_f64, larger);
    assert!(
        reduction >= least - slack && reduction <= most + slack,
        "{context}: a reduction of {reduction}, the fixed limiters' from {least} to {most}"
    );
}

#[test]
fn rebuilds_a_shrink_only_from_frames_run_under_the_release_in_force() {
    // At every rate, the release scenario ([`released_after_loud`]): 10 ms into the release the
    // lookahead shrinks from 5 to 0.5 ms and the release changes from 10 to 1,000 ms or back,
    // in the same set or `k` frames after a set of the release alone (`k` 1, half the frames
    // between the two windows and one less than them, `run = old - new`). A shrink rebuilds
    // its fast stage by replaying the `run` frames before its rebuild's last with the release
    // in force: every one of them must have run under it, so its rebuild lasts until the
    // first of them ran under the release asked, `max(rebuild, run - 1 - k)` frames from the
    // frame after the set (its change begins there), counted apart, and on every frame, from
    // the first set to eight maxima after the change, the reduction lies between those of two
    // fixed limiters at 5 and 0.5 ms given the release on the same frames (no shrink runs
    // when it is set, so it is in force from the next frame), within `beside_slack`. A replay
    // under the new release of frames that ran under the old one rewrites their release: 5.3
    // dB of gain below both fixed limiters, or 1.8 dB above them (finding T15-1 of the
    // targeted review of the live lookahead change, block 2).
    for rate in rates() {
        let loud = frames_of_duration(50.0_f64, rate);
        let maximum = frames_of_duration(5.0_f64, rate);
        let (old, new) = (maximum, frames_of_duration(0.5_f64, rate));
        let run = old - new;
        let rebuild = rebuild_frames_of(maximum);
        let slack = beside_slack(maximum);
        for (from, to) in [(10.0_f64, 1_000.0_f64), (1_000.0_f64, 10.0_f64)] {
            for after in [0, 1, run / 2, run - 1] {
                let context = format!(
                    "{} Hz, release {from} to {to} ms, the shrink {after} frames after",
                    rate.hz()
                );
                let made =
                    |lookahead: f64, release: f64| settings(-1.0_f64, lookahead, release, 0.5_f64);
                let build = |lookahead: f64| {
                    TruePeakLimiter::<Binary64>::new(
                        rate,
                        1,
                        value(5.0_f64),
                        super::Detector::Standard,
                        made(lookahead, from),
                        LimiterMode::Protected,
                    )
                    .expect("a limiter")
                };
                let mut limiter = build(5.0_f64);
                let mut references = [build(5.0_f64), build(0.5_f64)];
                let set_at = frames_of_duration(60.0_f64, rate);
                let mut frame = 0;
                while frame < set_at {
                    drop(reductions_beside(
                        &mut limiter,
                        &mut references,
                        released_after_loud(loud, frame),
                    ));
                    frame += 1;
                }
                // The release first, then the shrink `after` frames later, the references
                // given the release with it.
                let release = |changed: &mut TruePeakLimiter<Binary64>, lookahead: f64| {
                    changed
                        .set(made(lookahead, to))
                        .expect("within the maximum");
                };
                release(&mut limiter, 5.0_f64);
                release(&mut references[0], 5.0_f64);
                release(&mut references[1], 0.5_f64);
                for _ in 0..after {
                    let (reduction, theirs) = reductions_beside(
                        &mut limiter,
                        &mut references,
                        released_after_loud(loud, frame),
                    );
                    within_theirs(
                        reduction,
                        &theirs,
                        slack,
                        &format!("{context}, frame {frame}"),
                    );
                    frame += 1;
                }
                release(&mut limiter, 0.5_f64);
                let (mut rebuilding, mut over) = (0, None);
                let cap = run + rebuild + constant_change_frames(maximum, old, new) + 1;
                for since in 0..cap + 8 * maximum {
                    let (reduction, theirs) = reductions_beside(
                        &mut limiter,
                        &mut references,
                        released_after_loud(loud, frame),
                    );
                    within_theirs(
                        reduction,
                        &theirs,
                        slack,
                        &format!("{context}, frame {frame}"),
                    );
                    rebuilding +=
                        usize::from(matches!(limiter.change, Some(super::Stage::Rebuilding(_))));
                    if over.is_none() && limiter.change.is_none() {
                        over = Some(since);
                    }
                    frame += 1;
                }
                assert!(
                    over.is_some_and(|over| over <= cap),
                    "{context}: the shrink over within {cap} frames"
                );
                assert_eq!(
                    rebuilding,
                    rebuild.max((run - 1).saturating_sub(after)),
                    "{context}: the rebuild's frames"
                );
            }
        }
    }
}

#[test]
fn takes_lookaheads_set_between_two_frames_as_the_last_one_alone() {
    // At every rate, from a used limiter at 1.5 ms on loud noise, two lookaheads set between
    // the same two frames, growing then shrinking, shrinking then growing, and out and back
    // to 1.5 ms, both ways: the limiter computes exactly as one given the last of them alone
    // (none, when it is the lookahead in force), every gain and stage equal bit for bit after
    // every frame, and every value of its state ([`control_state`]) after every maximum's
    // frames and the last, through the change and four maxima after it; as every other live
    // setting, the last one set between two frames is
    // the one that acts. A change begun at the first set runs before the last one's
    // (finding T02-1 of the targeted review of the live lookahead change, block 2).
    let mut draws = Noise::new();
    for rate in rates() {
        let maximum = frames_of_duration(5.0_f64, rate);
        let cap = rebuild_frames_of(maximum) + 18 * maximum + descent_frames(maximum) + 4;
        for (first, last) in [
            (5.0_f64, 0.5_f64),
            (0.5_f64, 5.0_f64),
            (5.0_f64, 1.5_f64),
            (0.5_f64, 1.5_f64),
        ] {
            let context = format!("{} Hz, 1.5 to {first} then {last} ms", rate.hz());
            let mut twice = limiter_at(rate, 5.0_f64, 1.5_f64, true, &mut draws);
            for _ in 0..2 * maximum {
                let _: f64 = gain_of(&mut twice, 4.0_f64 * draws.next().abs());
            }
            let mut once = twice.clone();
            for lookahead in [first, last] {
                twice
                    .set(settings(-1.0_f64, lookahead, 100.0_f64, 0.5_f64))
                    .expect("within the maximum");
            }
            once.set(settings(-1.0_f64, last, 100.0_f64, 0.5_f64))
                .expect("within the maximum");
            let mut over = 0;
            for frame in 0..cap + 4 * maximum {
                let peak = 4.0_f64 * draws.next().abs();
                assert_eq!(
                    gain_of(&mut twice, peak).to_bits(),
                    gain_of(&mut once, peak).to_bits(),
                    "{context}, frame {frame}: the gain"
                );
                // The whole state once a maximum, and on the change's last frame: a state that
                // differs once differs from then on, both run on the same peaks.
                if frame % maximum == 0 || frame + 1 == cap + 4 * maximum {
                    assert!(
                        control_state(&twice) == control_state(&once),
                        "{context}, frame {frame}: the state, {:?} against {:?}",
                        twice.change,
                        once.change
                    );
                }
                assert_eq!(
                    twice.change, once.change,
                    "{context}, frame {frame}: the stage"
                );
                over += usize::from(once.change.is_none());
            }
            assert!(
                over >= 4 * maximum,
                "{context}: the change over within {cap}"
            );
        }
    }
}

#[test]
fn computes_after_a_silence_the_bits_of_a_limiter_always_at_its_lookahead() {
    // At every rate, from below and from above: a limiter at 1.5 ms changed live on loud
    // noise (to 0.5 ms or 5 ms, through 3.5 ms, each change run to its end), then a silence
    // long enough to release every value to zero (100 releases of 10 ms, past the 69 that
    // take any reduction below `2^-100`), then loud noise again; beside it, a limiter always at
    // its last lookahead on the same peaks, with the same release. Once both states are zero
    // (asserted), the two compute alike: every gain, and every value of their paths (release
    // stages, Hann sums in force and accumulated, the position of the next exact sums, the
    // count of non-zero reductions), equal bit for bit on every frame of four maxima of noise.
    // A path that swaps its exact sums in on a phase its change left, rather than on the
    // phase of a path that always ran, rounds every later reduction otherwise (finding T02-2
    // of the targeted review of the live lookahead change, block 2). The positive control:
    // some change took over on a frame off that phase, `(t - 1) mod L ≠ 0`, `t` the first
    // frame in force, counted by the test.
    let mut draws = Noise::new();
    let mut off_phase = 0_u32;
    for rate in rates() {
        let maximum = frames_of_duration(5.0_f64, rate);
        let cap = rebuild_frames_of(maximum) + 18 * maximum + descent_frames(maximum) + 4;
        for route in [[3.5_f64, 0.5_f64], [3.5_f64, 5.0_f64]] {
            let last = route[1];
            let context = format!("{} Hz, 1.5 to {route:?} ms", rate.hz());
            let made = |lookahead: f64| settings(-1.0_f64, lookahead, 10.0_f64, 0.0_f64);
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
            let (mut changed, mut always) = (build(1.5_f64), build(last));
            let mut frame = 0_usize;
            let mut step = |ours: &mut TruePeakLimiter<Binary64>,
                            theirs: &mut TruePeakLimiter<Binary64>,
                            peak: f64| {
                frame += 1;
                (gain_of(ours, peak), gain_of(theirs, peak), frame)
            };
            for _ in 0..2 * maximum {
                let peak = 4.0_f64 * draws.next().abs();
                let _: (f64, f64, usize) = step(&mut changed, &mut always, peak);
            }
            for lookahead in route {
                changed.set(made(lookahead)).expect("within the maximum");
                let window = frames_of_duration(lookahead, rate);
                let mut frames = 0;
                loop {
                    let peak = 4.0_f64 * draws.next().abs();
                    let (_, _, at) = step(&mut changed, &mut always, peak);
                    frames += 1;
                    assert!(frames <= cap, "{context}: still changing to {lookahead} ms");
                    if changed.path.lookahead == window
                        && !matches!(changed.change, Some(super::Stage::Rebuilding(_)))
                    {
                        // The first frame in force: the frame just computed.
                        off_phase += u32::from((at - 1) % window != 0);
                        break;
                    }
                }
                while changed.change.is_some() {
                    let _: (f64, f64, usize) =
                        step(&mut changed, &mut always, 4.0_f64 * draws.next().abs());
                    frames += 1;
                    assert!(frames <= cap, "{context}: still changing to {lookahead} ms");
                }
            }
            for _ in 0..frames_of_duration(1_000.0_f64, rate) {
                let _: (f64, f64, usize) = step(&mut changed, &mut always, 0.5_f64);
            }
            let zero = |limiter: &TruePeakLimiter<Binary64>| {
                let path = &limiter.path;
                (path.fast, path.slow, path.sums, path.next, path.nonzero)
                    == (
                        Binary64::ZERO,
                        Binary64::ZERO,
                        crate::math::HannSums::ZERO,
                        crate::math::HannSums::ZERO,
                        0,
                    )
            };
            assert!(
                zero(&changed) && zero(&always),
                "{context}: every value released to zero"
            );
            for at in 0..4 * maximum {
                let peak = 4.0_f64 * draws.next().abs();
                let (theirs, ours, _) = step(&mut changed, &mut always, peak);
                let here = format!("{context}, frame {at} after the silence");
                assert_eq!(theirs.to_bits(), ours.to_bits(), "{here}: the gain");
                let state = |limiter: &TruePeakLimiter<Binary64>| {
                    let path = &limiter.path;
                    (
                        path.fast,
                        path.slow,
                        path.sums,
                        path.next,
                        path.swap_in,
                        path.nonzero,
                    )
                };
                assert_eq!(state(&changed), state(&always), "{here}: the path");
            }
        }
    }
    assert!(off_phase > 0, "a change taken over off the canonical phase");
}

/// The release in force of `limiter`: the share its fast stage keeps per frame and the one
/// of the gap its slow stage closes.
const fn release_of(limiter: &TruePeakLimiter<Binary64>) -> (Binary64, Binary64) {
    (limiter.keep_fast, limiter.slow_attack)
}

#[test]
fn holds_a_release_set_while_a_shrink_runs_until_it_ends() {
    // At every rate, on loud noise, a release (from 100 ms and an adaptivity of 0.5 to 10 ms
    // and none) set at each stage of a change, its lookahead the one asked for (decision 12,
    // Rule 1): while a shrink runs (5 to 0.5 ms), rebuilt or shrinking, it is held, the
    // release in force unchanged on every frame until the change ends, then in force from
    // the frame after, that of a limiter built with it, bit for bit, the frames counted by
    // the test; set back to the one in force meanwhile, it is dropped, never in force. While
    // a growth runs (0.5 to 5 ms), rebuilt, rising or descending, or while a lookahead is
    // only asked for, it is in force from the next frame, since a growth replays no frame.
    // A shrink whose rebuild replays frames that ran under another release than the one it
    // replays them with rewrites their release (finding T15-1 of the targeted review of the
    // live lookahead change, block 2).
    let mut draws = Noise::new();
    for rate in rates() {
        let maximum = frames_of_duration(5.0_f64, rate);
        let made = |lookahead: f64, release: f64, adaptivity: f64| {
            settings(-1.0_f64, lookahead, release, adaptivity)
        };
        let build = |lookahead: f64, release: f64, adaptivity: f64| {
            TruePeakLimiter::<Binary64>::new(
                rate,
                1,
                value(5.0_f64),
                super::Detector::Standard,
                made(lookahead, release, adaptivity),
                LimiterMode::Protected,
            )
            .expect("a limiter")
        };
        let old = release_of(&build(5.0_f64, 100.0_f64, 0.5_f64));
        let new = release_of(&build(5.0_f64, 10.0_f64, 0.0_f64));
        assert_ne!(old, new, "two releases");
        // Each stage the release is set at: the change's lookaheads, whether it shrinks, and
        // whether the release is asked back before the change ends.
        let stages: [(&str, f64, f64, bool); 7] = [
            ("pending", 5.0_f64, 0.5_f64, false),
            ("rebuilding", 5.0_f64, 0.5_f64, false),
            ("shrinking", 5.0_f64, 0.5_f64, false),
            ("shrinking", 5.0_f64, 0.5_f64, true),
            ("rebuilding", 0.5_f64, 5.0_f64, false),
            ("rising", 0.5_f64, 5.0_f64, false),
            ("descending", 0.5_f64, 5.0_f64, false),
        ];
        for (at, from, to, back) in stages {
            let context = format!(
                "{} Hz, {from} to {to} ms, set while {at}, back {back}",
                rate.hz()
            );
            let shrinking = to < from;
            let mut limiter = build(from, 100.0_f64, 0.5_f64);
            for _ in 0..4 * maximum {
                let _: f64 = gain_of(&mut limiter, 4.0_f64 * draws.next().abs());
            }
            limiter
                .set(made(to, 100.0_f64, 0.5_f64))
                .expect("within the maximum");
            // Run to the stage, at most 16 maximum windows (the test's own limit).
            let reached = |stage: Option<super::Stage>| match at {
                "pending" => matches!(stage, Some(super::Stage::Pending)),
                "rebuilding" => matches!(stage, Some(super::Stage::Rebuilding(_))),
                "rising" => matches!(stage, Some(super::Stage::Rising { .. })),
                "shrinking" => matches!(stage, Some(super::Stage::Shrinking { .. })),
                _ => matches!(stage, Some(super::Stage::Descending { .. })),
            };
            let mut frames = 0;
            while !reached(limiter.change) {
                let _: f64 = gain_of(&mut limiter, 4.0_f64 * draws.next().abs());
                frames += 1;
                assert!(frames <= 16 * maximum, "{context}: never reached");
            }
            limiter
                .set(made(to, 10.0_f64, 0.0_f64))
                .expect("within the maximum");
            if !shrinking || at == "pending" {
                assert_eq!(release_of(&limiter), new, "{context}: in force at once");
                continue;
            }
            assert_eq!(release_of(&limiter), old, "{context}: held");
            if back {
                limiter
                    .set(made(to, 100.0_f64, 0.5_f64))
                    .expect("within the maximum");
            }
            // Held, frame by frame, until the frame the change ends on.
            let mut held = 0;
            loop {
                let running = limiter.change.is_some();
                let _: f64 = gain_of(&mut limiter, 4.0_f64 * draws.next().abs());
                held += 1;
                assert!(held <= 32 * maximum, "{context}: still changing");
                if running && limiter.change.is_none() {
                    break;
                }
                assert_eq!(release_of(&limiter), old, "{context}, frame {held}: held");
            }
            assert_eq!(
                release_of(&limiter),
                if back { old } else { new },
                "{context}: after the {held} frames of the change, in force"
            );
        }
    }
}

/// The largest one-frame step of `reductions`.
fn largest_move(reductions: &[f64]) -> f64 {
    largest_step(reductions).1
}

#[test]
fn glides_a_change_in_a_release_no_faster_than_the_fixed_limiters_and_its_gap() {
    // At every rate, in a release ([`released_after_loud`]), a change set 1, 10, 30 and 100 ms
    // into it, from 1.5 to 0.5, from 5 to 0.5 and from 1.5 to 5 ms (a maximum of 5 ms), and a
    // drag of 24 sets between 0.5 and 5 ms, one per eighth of a maximum, at 8 and 48 kHz:
    // beside two limiters always at the change's two lookaheads (the drag's), the largest
    // one-frame move of the reduction, from the set to eight maxima after the change, is at
    // most twice the larger of theirs over the same frames (the maintainer's criterion (3),
    // 2026-11-19) plus the largest gap between them on a stage's first frame spread over the
    // frames between the windows, and both paths' rounding with the gains' (`beside_slack`).
    // The design bounds each step by the paths' own moves and the glide (the random law's own
    // step law); against the fixed limiters' moves, measured on 2026-11-19, from 0.9 to 1.65
    // times theirs (8 kHz, 1.5 to 0.5 ms). A change that moves by the attack of the whole
    // gap at once, as one sized for the worst case does, moves by up to 22 times as much
    // (finding T15-2 of the targeted review of the live lookahead change, block 2).
    for rate in rates() {
        let loud = frames_of_duration(50.0_f64, rate);
        let maximum = frames_of_duration(5.0_f64, rate);
        let slack = beside_slack(maximum);
        let mut runs: Vec<(Vec<f64>, Vec<f64>, &str)> = vec![
            (
                vec![1.5_f64, 0.5_f64],
                vec![1.0_f64, 10.0_f64, 30.0_f64, 100.0_f64],
                "single",
            ),
            (
                vec![5.0_f64, 0.5_f64],
                vec![1.0_f64, 10.0_f64, 30.0_f64, 100.0_f64],
                "single",
            ),
            (
                vec![1.5_f64, 5.0_f64],
                vec![1.0_f64, 10.0_f64, 30.0_f64, 100.0_f64],
                "single",
            ),
        ];
        if [8_000, 48_000].contains(&rate.hz()) {
            runs.push((vec![0.5_f64, 5.0_f64], vec![1.0_f64], "drag"));
        }
        for (pair, intos, kind) in runs {
            let (from, to) = (pair[0], pair[1]);
            let between = frames_of_duration(from, rate).abs_diff(frames_of_duration(to, rate));
            let build = |lookahead: f64| {
                TruePeakLimiter::<Binary64>::new(
                    rate,
                    1,
                    value(5.0_f64),
                    super::Detector::Standard,
                    settings(-1.0_f64, lookahead, 100.0_f64, 0.5_f64),
                    LimiterMode::Protected,
                )
                .expect("a limiter")
            };
            // The frames before each set run once, each change from a copy of them.
            let (mut base, mut fixed_pair) = (build(from), [build(from), build(to)]);
            let mut frame = 0;
            for into in intos {
                let context = format!(
                    "{} Hz, {from} to {to} ms {into} ms into the release, {kind}",
                    rate.hz()
                );
                let set_at = frames_of_duration(50.0_f64 + into, rate);
                while frame < set_at {
                    drop(reductions_beside(
                        &mut base,
                        &mut fixed_pair,
                        released_after_loud(loud, frame),
                    ));
                    frame += 1;
                }
                let (mut limiter, mut references) = (base.clone(), fixed_pair.clone());
                let sets = if kind == "drag" { 24 } else { 1 };
                let cadence = (maximum / 8).max(1);
                let (mut ours, mut theirs) = (Vec::new(), [Vec::new(), Vec::new()]);
                let (mut gap, mut stage) = (0.0_f64, None);
                let (mut at, mut over) = (set_at, None);
                let cap =
                    set_at + sets * cadence + constant_change_frames(maximum, 1, maximum) * 16;
                while over.is_none_or(|over| at < over + 8 * maximum) {
                    if at < set_at + sets * cadence && (at - set_at).is_multiple_of(cadence) {
                        let lookahead = pair[((at - set_at) / cadence + 1) % 2];
                        limiter
                            .set(settings(-1.0_f64, lookahead, 100.0_f64, 0.5_f64))
                            .expect("within the maximum");
                    }
                    let (reduction, both) = reductions_beside(
                        &mut limiter,
                        &mut references,
                        released_after_loud(loud, at),
                    );
                    // A stage's first frame: the gap between the two fixed limiters then.
                    let now = stage_index(limiter.change);
                    if now != stage {
                        gap = larger(gap, (both[0] - both[1]).abs());
                        stage = now;
                    }
                    ours.push(reduction);
                    theirs[0].push(both[0]);
                    theirs[1].push(both[1]);
                    if over.is_none() && at >= set_at + sets * cadence && limiter.change.is_none() {
                        over = Some(at);
                    }
                    at += 1;
                    assert!(at <= cap, "{context}: still changing");
                }
                let fixed = larger(largest_move(&theirs[0]), largest_move(&theirs[1]));
                let allowed =
                    2.0_f64 * fixed + gap / f64::from(u32::try_from(between).expect("few")) + slack;
                let moved = largest_move(&ours);
                assert!(
                    moved <= allowed,
                    "{context}: a move of {moved}, the fixed limiters' {fixed}, {allowed} allowed"
                );
            }
        }
    }
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one criterion over random material: the material and the checks share their state"
)]
fn keeps_a_change_from_a_settled_limiter_between_its_two_fixed_limiters() {
    // The maintainer's criteria (2026-11-19) for a change from a limiter that never changed
    // (no state carried from an earlier window), at every rate, at maxima of 1 and 5 ms, on
    // random material (a floor, bursts, dense passages, silences), a random ceiling, the
    // lookahead drawn as the random law draws it ([`drawn_lookaheads`]), the change alone or
    // with a new release and adaptivity in the same set (a preset, whose release is in force
    // from the next frame: no shrink runs then), beside two limiters always at the change's
    // two lookaheads given the same commands: (1) the ceiling, every reduction at least the
    // needs of the samples it applies to (less `ρ` and `6.0001u`, as the random law); (2) on
    // every frame from the set to twelve maxima after the change, the reduction between
    // theirs, within `beside_slack` (`spec/measurements.md`, "The true-peak limiter", step 6:
    // proven while the change's stages run, held after them). Criterion (3), each step at
    // most twice theirs, holds in the design's own form, the paths' own moves plus the
    // budget and the glide, which the random law asserts on every frame
    // ([`stays_between_limiters_always_at_its_lookaheads_while_its_lookahead_changes`]).
    const CEILINGS: [f64; 4] = [-20.0_f64, -6.0_f64, -1.0_f64, 0.0_f64];
    let mut draws = crate::tests::splitmix::Draws::new();
    for rate in rates() {
        for most in [1.0_f64, 5.0_f64] {
            let maximum = frames_of_duration(most, rate);
            let (rounding, slack) = (rounding_of(maximum), beside_slack(maximum));
            let lookaheads = drawn_lookaheads(most, rate);
            for preset in [false, true] {
                for _ in 0_u8..4 {
                    let ceiling = pick(&mut draws, &CEILINGS);
                    let from = pick(&mut draws, &lookaheads);
                    let mut to = pick(&mut draws, &lookaheads);
                    if frames_of_duration(to, rate) == frames_of_duration(from, rate) {
                        to = if from < most { most } else { 0.5_f64 };
                    }
                    let release = if preset {
                        (30.0_f64, 0.25_f64)
                    } else {
                        (100.0_f64, 0.5_f64)
                    };
                    let context = format!(
                        "{} Hz, {from} to {to} of {most} ms at {ceiling} dBTP, preset {preset}",
                        rate.hz()
                    );
                    let build = |lookahead: f64| {
                        TruePeakLimiter::<Binary64>::new(
                            rate,
                            1,
                            value(most),
                            super::Detector::Standard,
                            settings(ceiling, lookahead, 100.0_f64, 0.5_f64),
                            LimiterMode::Protected,
                        )
                        .expect("a limiter")
                    };
                    let mut limiter = build(from);
                    let mut references = [build(from), build(to)];
                    let amplitude = libm::exp(ceiling * core::f64::consts::LN_10 / 20.0_f64);
                    let mut needs = vec![0.0_f64; 2 * maximum + 2];
                    let set_at = maximum * (2 + usize::try_from(draws.next() % 18).expect("few"));
                    let (mut segment, mut kind, mut floor, mut level) = (0, 0_u8, 0.0_f64, 0.0_f64);
                    let (mut frame, mut over) = (0_usize, None);
                    let cap = set_at + 64 * maximum;
                    while over.is_none_or(|over| frame < over + 12 * maximum) {
                        if segment == 0 {
                            kind = pick(&mut draws, &[0_u8, 1, 1, 2, 3]);
                            floor = amplitude
                                * pick(
                                    &mut draws,
                                    &[0.0_f64, 0.3_f64, 0.9_f64, 1.05_f64, 1.5_f64, 3.0_f64],
                                );
                            level = amplitude
                                * pick(&mut draws, &[1.2_f64, 2.0_f64, 4.0_f64, 16.0_f64]);
                            segment = 1 + usize::try_from(
                                draws.next() % u64::try_from(4 * maximum).expect("few"),
                            )
                            .expect("few");
                        }
                        segment -= 1;
                        let peak = match kind {
                            0 => floor * (0.5_f64 + 0.5_f64 * unit(&mut draws)),
                            1 if unit(&mut draws) < 0.7_f64 => {
                                level * (0.6_f64 + 0.4_f64 * unit(&mut draws))
                            }
                            2 if unit(&mut draws) < 0.25_f64 => level * unit(&mut draws),
                            3 => 0.0_f64,
                            _ => floor * unit(&mut draws),
                        };
                        if frame == set_at {
                            limiter
                                .set(settings(ceiling, to, release.0, release.1))
                                .expect("within the maximum");
                            for (reference, lookahead) in references.iter_mut().zip([from, to]) {
                                reference
                                    .set(settings(ceiling, lookahead, release.0, release.1))
                                    .expect("within the maximum");
                            }
                        }
                        let (reduction, theirs) =
                            reductions_beside(&mut limiter, &mut references, peak);
                        needs.rotate_left(1);
                        *needs.last_mut().expect("needs") =
                            need_above(peak, amplitude, margin_of(maximum));
                        let held = needs[needs.len() - maximum - 2..=needs.len() - maximum]
                            .iter()
                            .copied()
                            .fold(0.0_f64, larger);
                        let here = format!("{context}, frame {frame}, set after {set_at}");
                        assert!(
                            reduction >= held - rounding - 6.000_1_f64 * libm::exp2(-53.0_f64),
                            "{here}: reduced by {reduction}, {held} needed"
                        );
                        if frame >= set_at {
                            within_theirs(reduction, &theirs, slack, &here);
                        }
                        if over.is_none() && frame > set_at && limiter.change.is_none() {
                            over = Some(frame);
                        }
                        frame += 1;
                        assert!(frame <= cap, "{context}: still changing");
                    }
                }
            }
        }
    }
}
