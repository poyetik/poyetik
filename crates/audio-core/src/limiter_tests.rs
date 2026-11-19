//! Tests of the true-peak limiter.

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use super::{LimiterError, LimiterSettings, Setting, TruePeakLimiter};
use crate::LimiterMode;
use crate::host::Raw;
use crate::tests::{exact_steady, larger, margin_of};
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
            step: self.step,
            ceiling: self.ceiling,
            margin: self.margin,
            keep_fast: self.keep_fast,
            slow_attack: self.slow_attack,
            path: self.path.clone(),
            entering: self.entering,
            loud: self.loud,
            released: self.released,
            decaying: self.decaying,
            set_at: self.set_at,
            ran_with: self.ran_with,
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

/// The ceiling held, in `S`, at every rate and at maxima of 5 and 1 ms, while lookaheads are
/// set live as it limits, each held pending (asserted), longer and shorter, one after
/// another and a block apart: on two channels of noise limited near the ceiling, every
/// segment between two settings is at or below it, with no slack, and above half of it, so
/// that a reduction disturbed by a set pending fails. Where one is taken, at a rest, the
/// limiter is one always at it (`renders_from_each_rest_as_limiters_always_at_its_lookaheads`),
/// whose ceiling the sweeps of every lookahead hold.
fn holds_the_ceiling_with_lookaheads_set_while_it_limits<S: Sample + Raw>() {
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
            // Each lookahead and the frames run after it is set: the latency and four windows;
            // one block; and a window and a half of the longest.
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
                assert_eq!(
                    limiter.path.lookahead,
                    longest,
                    "{} Hz, {lookahead} of {most} ms: pending while it limits",
                    rate.hz()
                );
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
                // A copy, then a constant above every need of the noise: its exact gain, a
                // lookahead pending or not.
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
fn holds_the_ceiling_with_lookaheads_set_while_it_limits_in_each_precision() {
    holds_the_ceiling_with_lookaheads_set_while_it_limits::<Binary64>();
    holds_the_ceiling_with_lookaheads_set_while_it_limits::<Binary32>();
}

/// One part of a limiter's state, as [`each_stored`] gives it.
enum Stored<'a> {
    /// A value it stores through `math::kept`: zero or at least `2^-100`
    /// (`math::LEAST_KEPT`). The release stages, the share applied and what is left of the
    /// way.
    Kept(Binary64),
    /// A ring of such values and the index it writes next: the needs, and the path's
    /// released reductions.
    KeptRing(&'a [Binary64], usize),
    /// The path's hold: the step each need entered on, and the need, the newest need when it
    /// was written.
    Hold(&'a [(u64, Binary64)]),
    /// A value derived from kept ones and proven zero or normal (the path's Hann sums, in
    /// force and accumulated: `spec/measurements.md`, "The true-peak limiter", step 7), or one
    /// fixed when it is built, set or a window put in force (the path's turn and scale, the
    /// ceiling, the margin, the release's and the crossfade's shares, the settings).
    Derived(Binary64),
    /// The path's phasor, fixed when a window is put in force
    /// (`turns_no_phasor_part_below_two_to_the_minus_55_up_to_the_longest_window`).
    Phasor(&'a [(Binary64, Binary64)]),
    /// A count, a position, a step or a flag.
    Count(u64),
}

/// Gives `visit` every part of the state of `limiter` ([`Stored`]), listed from one
/// destructuring of it and of its path with no field left out, so that a field added to
/// either does not compile until it is listed here: the one list the laws of its values read
/// ([`written_values`], [`control_state`]). Its signal path (its interpolator, the history
/// of its input and its delay, values of `S`) apart.
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
        step,
        ceiling,
        margin,
        keep_fast,
        slow_attack,
        path,
        entering,
        loud,
        released,
        decaying,
        set_at,
        ran_with,
        applied,
        mode,
        protected,
        given_binary32,
        remaining,
        keep_mode,
    } = limiter;
    for value in [*applied, *remaining] {
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
    for value in [
        *maximum,
        *wanted,
        *taps,
        *delay,
        *history_at,
        *position,
        *needed_at,
        *entering,
    ] {
        visit(count(value));
    }
    for flag in [*mode == LimiterMode::Protected, *protected, *given_binary32] {
        visit(Stored::Count(u64::from(flag)));
    }
    visit(Stored::Count(u64::from(*channels)));
    visit(Stored::Derived(ran_with.0));
    visit(Stored::Derived(ran_with.1));
    for value in [
        *step, *loud, *released, *decaying, *set_at, ran_with.2, ran_with.3, ran_with.4,
    ] {
        visit(Stored::Count(value));
    }
    let super::Path {
        lookahead,
        queue,
        front,
        queued,
        fast,
        slow,
        released: released_ring,
        released_at,
        nonzero,
        sums,
        next,
        swap_in,
        phasor,
        turn,
        scale,
    } = path;
    visit(Stored::Kept(*fast));
    visit(Stored::Kept(*slow));
    visit(Stored::KeptRing(released_ring, *released_at));
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
    for value in [turn.0, turn.1, *scale] {
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
        Stored::KeptRing(ring, _) => {
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
/// and the frame `at` releases into the tail, at which a lookahead is set or the crossfade
/// is made, if any.
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
        Stored::Derived(_) | Stored::Phasor(_) | Stored::Count(_) => {}
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
/// or state ([`control_state`]) differs, naming it. The limiter in IEEE arithmetic, after
/// its last frame.
fn alike_on_both_hosts(
    rate: SampleRate,
    (mode, from): (LimiterMode, f64),
    (tail, at): (f64, Option<f64>),
    first: impl Fn(&mut TruePeakLimiter<Binary64>),
    then: impl Fn(&mut TruePeakLimiter<Binary64>),
    context: &str,
) -> TruePeakLimiter<Binary64> {
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
            (flushed.path.fast, flushed.path.slow, flushed.loud),
            (ieee.path.fast, ieee.path.slow, ieee.loud)
        );
    }
    assert_eq!(
        format!("{flushed:?}"),
        format!("{ieee:?}"),
        "{context}: every field at the end"
    );
    ieee
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
    // - A release run 707 releases (`e^-707`, about `2^-1020`), where a release stage that
    //   kept every value would decay into the subnormal numbers, which a flushing host reads
    //   as zero: a decay without `math::kept` fails it (seen on 2026-11-18).
    // - A lookahead set (longer and shorter) and a crossfade between raw and protected (each
    //   way), each made `ln(r / 2^-98)` releases into the tail (about 67.3, `r = 1 - c / 2`,
    //   the burst's need, where the reduction nears `2^-98`), so that they run on a state
    //   near the least value kept, not all zero (asserted), then 20 releases more: the
    //   lookahead, held pending, is taken at the limiter's rest within them (asserted).
    // At the two lowest rates of the test matrix, an integer number of frames per millisecond
    // and not (the laws of the same values run every rate).
    let none = |_: &mut TruePeakLimiter<Binary64>| ();
    let near = libm::log(libm::exp2(98.0_f64) * (1.0_f64 - CEILING / 2.0_f64));
    for rate in rates().into_iter().take(2) {
        drop(alike_on_both_hosts(
            rate,
            (LimiterMode::Protected, 1.5_f64),
            (707.0_f64, None),
            none,
            none,
            &format!("{} Hz, a release run 707 releases", rate.hz()),
        ));
        for (from, to) in [(0.5_f64, 5.0_f64), (5.0_f64, 0.5_f64)] {
            let context = format!("{} Hz, from {from} to {to} ms", rate.hz());
            let ieee = alike_on_both_hosts(
                rate,
                (LimiterMode::Protected, from),
                (near + 20.0_f64, Some(near)),
                none,
                |limiter| {
                    limiter
                        .set(settings(-1.0_f64, to, 10.0_f64, 0.0_f64))
                        .expect("within the maximum");
                    assert_ne!(
                        limiter.wanted, limiter.path.lookahead,
                        "pending: not at rest"
                    );
                },
                &context,
            );
            assert_eq!(
                ieee.path.lookahead,
                frames_of_duration(to, rate),
                "{context}: taken at its rest in the tail"
            );
        }
        for protected in [true, false] {
            drop(alike_on_both_hosts(
                rate,
                (LimiterMode::Switchable, 1.5_f64),
                (near + 20.0_f64, Some(near)),
                |limiter| limiter.set_protected(!protected),
                |limiter| limiter.set_protected(protected),
                &format!("{} Hz, toward protected {protected}", rate.hz()),
            ));
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
        Stored::Hold(_) | Stored::Phasor(_) | Stored::Count(_) => {}
    });
}

/// [`computes_no_subnormal_value_through_a_release_to_zero`] in `S`, at a maximum lookahead
/// of `most` milliseconds and every rate: from 0.5 ms, `most` set 20 releases into the tail,
/// 0.5 ms set back at 30 (the change dropped), `most` set again where the reduction nears
/// `2^-98` (with a value of the state below `2^-90` there, asserted), each held pending
/// (asserted) but at a maximum of 0.5 ms, where no lookahead can change, and taken at the
/// limiter's rest in the tail; crossfaded to raw at 40 releases and back at 50.
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
                        limiter.wanted != limiter.path.lookahead,
                        changing && lookahead > 0.5_f64,
                        "{context}: {lookahead} ms pending"
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
        // The positive control: the release ran to zero, its stages and its sums with it,
        // and the lookahead set last is in force.
        let path = &limiter.path;
        assert_eq!(
            (path.fast, path.slow, path.sums),
            (Binary64::ZERO, Binary64::ZERO, crate::math::HannSums::ZERO),
            "{context}: released to zero"
        );
        assert_eq!(
            limiter.path.lookahead,
            frames_of_duration(most, rate),
            "{context}: taken at its rest"
        );
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
    // lookaheads set in it (a longer one 20 releases in, the one in force back at 30, the
    // longer again where the reduction nears `2^-98`, taken at the limiter's rest) and a
    // crossfade to raw at 40 and back at 50: every
    // value a frame can write to the state ([`written_values`]) is zero or at least
    // `2^-533`, the least step 7 derives, far above the subnormal numbers, at every
    // rate, at maxima of 0.5 ms (no lookahead change possible there: crossfades only), 0.625
    // ms (the smallest with a longer lookahead at every rate, one frame more than 0.5 ms at
    // 8 kHz), 1
    // and 5 ms, in both precisions; and so is every value of a tail met at `2^-1012`
    // (`spec/measurements.md`, "The true-peak limiter", step 7).
    for most in [0.5_f64, 0.625_f64, 1.0_f64, 5.0_f64] {
        no_subnormal_value_through_a_release_to_zero::<Binary64>(most);
        no_subnormal_value_through_a_release_to_zero::<Binary32>(most);
    }
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

#[test]
fn closes_at_least_the_slow_share_in_its_fast_stage_at_every_rate() {
    // The premise of the rest's count (`spec/measurements.md`, "The true-peak limiter",
    // step 6), which counts the decay of both release stages from the slow stage's share
    // alone: the fast release is at most the release, exactly, and the fast stage's share,
    // one less its keep, is at least the slow stage's, at every rate, through the limiter's
    // own settings, over a grid of releases (from 10 to 1,000 ms, each tenth of a millisecond
    // up to 100 then each millisecond) at the edges of the adaptivity (0, the least subnormal,
    // `2^-54` and `2^-53` about the rounding of `1 - adaptivity` to 1, the largest number
    // below 1, and 1) and at seeded draws of both.
    let tiny = [
        0.0_f64,
        f64::from_bits(1),
        libm::exp2(-54.0_f64),
        libm::exp2(-53.0_f64),
        1.0_f64 - libm::exp2(-53.0_f64),
        1.0_f64,
    ];
    let mut draws = crate::tests::splitmix::Draws::new();
    let unit = |bits: u64| {
        f64::from(u32::try_from(bits >> 32_u32).expect("32 bits")) / 4_294_967_296.0_f64
    };
    for rate in rates() {
        let mut limiter = TruePeakLimiter::<Binary64>::new(
            rate,
            1,
            value(5.0_f64),
            super::Detector::Standard,
            LimiterSettings::DEFAULT,
            LimiterMode::Protected,
        )
        .expect("a limiter");
        let grid = (100_u32..=1_000)
            .map(|tenths| f64::from(tenths) / 10.0_f64)
            .chain((101_u32..=1_000).map(f64::from));
        let mut checked = 0_u32;
        for (release, adaptivity) in grid
            .flat_map(|release| tiny.map(|adaptivity| (release, adaptivity)))
            .chain((0_u32..4_096).map(|_| {
                let release = 10.0_f64 + 990.0_f64 * unit(draws.next());
                (release, unit(draws.next()))
            }))
        {
            limiter
                .set(settings(-1.0_f64, 1.5_f64, release, adaptivity))
                .expect("in range");
            let fast = crate::math::limiter::fast_release(
                value(release),
                value(adaptivity),
                super::FAST_RELEASE_DIVISOR,
            );
            assert!(
                fast.raw() <= release,
                "{} Hz, {release} ms at {adaptivity:e}: the fast release {} at most the release",
                rate.hz(),
                fast.raw()
            );
            assert!(
                1.0_f64 - limiter.keep_fast.raw() >= limiter.slow_attack.raw(),
                "{} Hz, {release} ms at {adaptivity:e}: the fast share {:e} under the slow {:e}",
                rate.hz(),
                1.0_f64 - limiter.keep_fast.raw(),
                limiter.slow_attack.raw()
            );
            checked += 1;
        }
        assert_eq!(checked, 10_806 + 4_096, "{} Hz: every setting", rate.hz());
    }
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
    // dBTP, then back, then the lookahead is set from 1.5 to 0.5 ms, pending while the
    // constant is limited (decision 12). A lower ceiling enters the
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

/// What [`Rest`] keeps of the last frame for the sets after it: its pace, its `decay`, its
/// `since_release` and its `never`.
type Ran = (Option<(u64, u64)>, usize, usize, bool);

/// The test's own account of when a lookahead set takes effect, written from its rule
/// (`spec/measurements.md`, "The true-peak limiter", step 6) in frames counted since what
/// starts each wait, apart from the limiter's arithmetic of steps: at rest after a frame if
/// no need was ever above zero, or if `K` frames passed since the newest that was and since
/// the release and the adaptivity in force were set, `K = round(71 / c) + 2`, `c` the slow
/// stage's share; a release and an adaptivity set count again only if they differ, in
/// milliseconds and as a share, from those the last frame ran with (the draws of the laws
/// give no two of either with the same keeps).
struct Rest {
    /// Whether no need was ever above zero.
    never: bool,
    /// The frames since the newest need above zero.
    since_need: usize,
    /// The frames since the release in force was set.
    since_release: usize,
    /// `K` of the release in force.
    decay: usize,
    /// The lookahead in force.
    in_force: usize,
    /// The lookahead asked for last.
    wanted: usize,
    /// The release and the adaptivity in force, by their bits: none before the first set.
    pace: Option<(u64, u64)>,
    /// The pace, `decay`, `since_release` and `never` the last frame ran with, once a set
    /// since it kept them.
    ran_with: Option<Ran>,
    /// The paces set at rest, which count nothing again.
    quiet_paces: usize,
}

#[expect(
    clippy::arithmetic_side_effects,
    reason = "frame counts and test values far from overflow"
)]
impl Rest {
    /// Before the first frame of a limiter at `lookahead` frames.
    const fn new(lookahead: usize) -> Self {
        Self {
            never: true,
            since_need: 0,
            since_release: 0,
            decay: 0,
            in_force: lookahead,
            wanted: lookahead,
            pace: None,
            ran_with: None,
            quiet_paces: 0,
        }
    }

    /// A set of `release` and `adaptivity`, whose slow stage closes `slow_attack`: counted
    /// again if they differ from those the last frame ran with, as that frame left them if
    /// not; set when the limiter is at rest under that frame's pace, every state is zero and
    /// stays so: that of a limiter that never needed a reduction.
    fn released(&mut self, release: f64, adaptivity: f64, slow_attack: Binary64) {
        let (pace, decay, since_release, never) =
            *self
                .ran_with
                .get_or_insert((self.pace, self.decay, self.since_release, self.never));
        self.pace = Some((release.to_bits(), adaptivity.to_bits()));
        if self.pace == pace {
            (self.decay, self.since_release, self.never) = (decay, since_release, never);
        } else {
            if !never && self.since_need >= decay && since_release >= decay {
                self.never = true;
                self.quiet_paces += 1;
            }
            let count = libm::round(71.0_f64 / slow_attack.raw()) + 2.0_f64;
            self.decay = (0_u32..=u32::MAX)
                .find(|each| f64::from(*each) >= count)
                .and_then(|count| usize::try_from(count).ok())
                .expect("a count");
            self.since_release = 0;
        }
    }

    /// Whether the limiter is at rest after the last frame.
    fn resting(&self) -> bool {
        self.never || (self.since_need >= self.decay && self.since_release >= self.decay)
    }

    /// Before a frame: the lookahead asked for put in force if at rest; whether it was.
    fn begin(&mut self) -> bool {
        let switched = self.wanted != self.in_force && self.resting();
        if switched {
            self.in_force = self.wanted;
        }
        switched
    }

    /// After a frame whose need was `need`.
    fn end(&mut self, need: Binary64) {
        if need > Binary64::ZERO {
            self.never = false;
            self.since_need = 0;
        } else {
            self.since_need = self.since_need.saturating_add(1);
        }
        self.since_release = self.since_release.saturating_add(1);
        self.ran_with = None;
    }
}

/// The need of the frame `limiter` computed last.
fn newest_need<S: Sample>(limiter: &TruePeakLimiter<S>) -> Binary64 {
    limiter.needed[super::behind(limiter.needed_at, 1, limiter.needed.len())]
}

/// The frames, at most, from the last need above zero to the rest of a limiter of
/// `maximum` frames at `rate` under a release of `release` ms set before it, counted apart:
/// `maximum + K + maximum` frames, `K` about `71 / c` frames, `c` the slow stage's share
/// `1 - e^(-1/f)` (`f` the release in frames, `c` above `1/(f + 1)`), so at most `71 (f + 1)`
/// and 3 more. A test waiting for a rest stops there.
fn rest_within(maximum: usize, rate: SampleRate, release: f64) -> usize {
    let release_frames = release * f64::from(rate.hz()) / 1_000.0_f64;
    let decay = 71.0_f64 * (release_frames + 1.0_f64) + 3.0_f64;
    let frames = (0_u32..=u32::MAX)
        .find(|count| f64::from(*count) >= decay)
        .expect("a count");
    maximum
        .saturating_mul(2)
        .saturating_add(usize::try_from(frames).expect("small"))
}

/// How loud the peaks of a frame are, over the settings the laws of a lookahead set live
/// draw (ceilings from -6 to -1 dBTP, amplitudes 0.501 to 0.891).
#[derive(Clone, Copy)]
enum Level {
    /// Above every ceiling: every need is not zero, the limiter limits on every frame.
    Loud,
    /// Above and below the ceilings, frame by frame: needs of zero between others.
    Varying,
    /// Below every ceiling: every need is zero, the limiter releases toward its rest.
    Quiet,
}

/// The peak of a frame at `level`, from `draws`: 1.5 to 2.5 loud, 0 to 2 varying, 0 to 0.4
/// quiet.
fn peak_at(level: Level, draws: &mut Noise) -> f64 {
    let magnitude = draws.next().abs();
    match level {
        Level::Loud => 1.5_f64 + magnitude,
        Level::Varying => 2.0_f64 * magnitude,
        Level::Quiet => 0.4_f64 * magnitude,
    }
}

/// A value of `values` drawn from `draws`.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "frame counts and test values far from overflow"
)]
fn pick<T: Copy>(draws: &mut crate::tests::splitmix::Draws, values: &[T]) -> T {
    let index = draws.next() % u64::try_from(values.len()).expect("few");
    values[usize::try_from(index).expect("few")]
}

/// The bits of a path's sums, in force and accumulated.
fn sums_bits(path: &super::Path) -> [u64; 6] {
    [
        path.sums.sum.raw().to_bits(),
        path.sums.real.raw().to_bits(),
        path.sums.imaginary.raw().to_bits(),
        path.next.sum.raw().to_bits(),
        path.next.real.raw().to_bits(),
        path.next.imaginary.raw().to_bits(),
    ]
}

/// The needs a path holds, from its front: each step it entered on and the need's bits.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "frame counts and test values far from overflow"
)]
fn held_needs(path: &super::Path) -> Vec<(u64, u64)> {
    (0..path.queued)
        .map(|offset| {
            let (entered, need) = path.queue[(path.front + offset) % path.queue.len()];
            (entered, need.raw().to_bits())
        })
        .collect()
}

/// Checks that `path`, a used limiter's, holds what `always`, a limiter's always at its
/// window, holds, as far as either computes from it: its window, its release stages, its
/// count of non-zero reductions, its sums and their phase, the needs it holds and the
/// released reductions its window reads (`all` of them, or only those the next frame reads
/// leaving, the cheaper check).
fn path_as(path: &super::Path, always: &super::Path, all: bool, context: &str) {
    assert_eq!(path.lookahead, always.lookahead, "{context}: the window");
    assert_eq!(
        (path.fast.raw().to_bits(), path.slow.raw().to_bits()),
        (always.fast.raw().to_bits(), always.slow.raw().to_bits()),
        "{context}: the release stages"
    );
    assert_eq!(
        (path.nonzero, path.swap_in, sums_bits(path)),
        (always.nonzero, always.swap_in, sums_bits(always)),
        "{context}: the sums and their phase"
    );
    let reads = if all { path.lookahead } else { 1 };
    for back in path.lookahead.saturating_sub(reads)..path.lookahead {
        assert_eq!(
            path.released_before(back).raw().to_bits(),
            always.released_before(back).raw().to_bits(),
            "{context}: the reduction released {back} frames before"
        );
    }
    if all {
        assert_eq!(held_needs(path), held_needs(always), "{context}: the hold");
    } else {
        assert_eq!(
            (path.queued, path.held().raw().to_bits()),
            (always.queued, always.held().raw().to_bits()),
            "{context}: the hold"
        );
    }
}

/// What a sequence of [`renders_from_each_rest_as_limiters_always_at_its_lookaheads`] reached:
/// lookaheads taken at a rest, longer and shorter, frames with one pending on which the
/// limiter at it gives another gain (taking it then would change a bit), and frames after a
/// switch on which the limiter at the old one gives another gain (the switch changed bits).
#[derive(Default)]
struct Switches {
    /// Longer lookaheads taken.
    longer: usize,
    /// Shorter ones taken.
    shorter: usize,
    /// Frames with a lookahead pending whose limiter would have given another gain.
    pending_apart: usize,
    /// Frames after a switch on which the old lookahead's limiter gives another gain.
    switched_apart: usize,
}

/// One seeded sequence of [`renders_from_each_rest_as_limiters_always_at_its_lookaheads`], in
/// `S`, at `rate`, with a maximum lookahead of `most` ms, the sequence `seed`: a limiter `A`
/// and one limiter always at each lookahead it can be set to (0.5 ms, `most`, and between),
/// fed the same peaks and the same commands, each with its own lookahead, through five
/// segments: varying, quiet, loud, quiet, varying. In each loud or varying segment, one to
/// three sets of settings drawn (lookaheads among the three, two in a row between the same
/// frames at times, the one in force at times, releases of 10, 30 and 300 ms, adaptivities
/// of 0, 0.5 and 1, ceilings of -6, -3 and -1 dBTP); each quiet segment begins with a set of
/// a release of 10 ms, its other settings drawn, the lookahead asked for kept, and runs past
/// the rest of [`Rest`] by up to a window. On every frame, `A`'s gain is, bit for bit, that
/// of the limiter always at the lookahead [`Rest`] says is in force (so `A` keeps its old
/// lookahead while one is pending and takes it on the first frame of its rest, not before
/// nor after), its bound is [`Rest`]'s, and its path holds what that limiter's holds
/// ([`path_as`]).
#[expect(
    clippy::too_many_lines,
    clippy::arithmetic_side_effects,
    reason = "one sequence of segments, each command and each frame checked where it runs; frame counts far from overflow"
)]
fn always_at_its_lookahead_from_each_rest<S: Sample>(
    rate: SampleRate,
    most: f64,
    seed: u64,
) -> Switches {
    let mut draws = crate::tests::splitmix::Draws(seed);
    let mut noise = Noise(u32::try_from(seed % 1_000).expect("small"));
    let context = format!(
        "{} Hz, {most} ms at most, binary{}, sequence {seed}",
        rate.hz(),
        S::PRECISION.bits()
    );
    let maximum = frames_of_duration(most, rate);
    let choices = [0.5_f64, 0.5_f64.midpoint(most), most];
    let windows: Vec<usize> = choices
        .iter()
        .map(|milliseconds| frames_of_duration(*milliseconds, rate))
        .collect();
    // Settings drawn at `lookahead`, their release `release` or drawn.
    let settings_drawn =
        |from: &mut crate::tests::splitmix::Draws, window: f64, chosen: Option<f64>| {
            let release = chosen.unwrap_or_else(|| pick(from, &[10.0_f64, 30.0_f64, 300.0_f64]));
            settings(
                pick(from, &[-6.0_f64, -3.0_f64, -1.0_f64]),
                window,
                release,
                pick(from, &[0.0_f64, 0.5_f64, 1.0_f64]),
            )
        };
    let initial = pick(&mut draws, &choices);
    let first = settings_drawn(&mut draws, initial, None);
    let built = |settings: LimiterSettings| {
        TruePeakLimiter::<S>::new(
            rate,
            1,
            value(most),
            super::Detector::Standard,
            settings,
            LimiterMode::Protected,
        )
        .expect("a limiter")
    };
    let mut limiter = built(first);
    let mut always: Vec<TruePeakLimiter<S>> = choices
        .iter()
        .map(|lookahead| built(first.with_lookahead(value(*lookahead)).expect("in range")))
        .collect();
    assert_eq!(limiter.maximum, maximum, "{context}: counted apart");
    let index_of = |window: usize| {
        windows
            .iter()
            .position(|each| *each == window)
            .expect("a window of the three")
    };
    let mut rest = Rest::new(frames_of_duration(first.lookahead().raw(), rate));
    rest.released(
        first.release().raw(),
        first.adaptivity().raw(),
        limiter.slow_attack,
    );
    let mut asked = first;
    let mut switches = Switches::default();
    let mut previous: Option<usize> = None;
    let set = |used: &mut TruePeakLimiter<S>,
               fixed: &mut [TruePeakLimiter<S>],
               rule: &mut Rest,
               asked_now: LimiterSettings| {
        used.set(asked_now).expect("within the maximum");
        rule.released(
            asked_now.release().raw(),
            asked_now.adaptivity().raw(),
            used.slow_attack,
        );
        for (each, window) in fixed.iter_mut().zip(choices) {
            each.set(asked_now.with_lookahead(value(window)).expect("in range"))
                .expect("within the maximum");
        }
        rule.wanted = frames_of_duration(asked_now.lookahead().raw(), rate);
    };
    let mut frame = 0_usize;
    for (segment, level) in [
        Level::Varying,
        Level::Quiet,
        Level::Loud,
        Level::Quiet,
        Level::Varying,
    ]
    .into_iter()
    .enumerate()
    {
        let quiet = matches!(level, Level::Quiet);
        // Loud or varying: its length, and the frames of its sets.
        let length = maximum
            .saturating_mul(usize::try_from(draws.next() % 4 + 1).expect("small"))
            .saturating_add(usize::try_from(draws.next() % 64).expect("small"));
        let sets: Vec<usize> = (0..=draws.next() % 3)
            .map(|_| usize::try_from(draws.next() % 64).expect("small") % length)
            .collect();
        if quiet {
            asked = settings_drawn(&mut draws, asked.lookahead().raw(), Some(10.0_f64));
            set(&mut limiter, &mut always, &mut rest, asked);
        }
        let cap = rest_within(maximum, rate, 10.0_f64).saturating_add(maximum);
        let extra = usize::try_from(draws.next()).expect("64 bits") % maximum.saturating_add(1);
        let mut rested: Option<usize> = None;
        for at in 0_usize.. {
            if quiet {
                assert!(
                    at <= cap,
                    "{context}, segment {segment}: at rest within {cap}"
                );
                if rested.is_none() && rest.wanted == rest.in_force && rest.resting() {
                    rested = Some(at);
                }
                if rested.is_some_and(|since| at >= since.saturating_add(extra)) {
                    break;
                }
            } else if at == length {
                break;
            }
            if !quiet {
                for (count, _) in sets.iter().enumerate().filter(|(_, when)| **when == at) {
                    // At times two sets between the same frames, the second the one asked,
                    // or the lookahead in force asked again.
                    if count == 0 && draws.next().is_multiple_of(4) {
                        let window = pick(&mut draws, &choices);
                        let other = settings_drawn(&mut draws, window, None);
                        set(&mut limiter, &mut always, &mut rest, other);
                    }
                    let window = if draws.next().is_multiple_of(4) {
                        choices[index_of(rest.in_force)]
                    } else {
                        pick(&mut draws, &choices)
                    };
                    asked = settings_drawn(&mut draws, window, None);
                    set(&mut limiter, &mut always, &mut rest, asked);
                }
            }
            let before = rest.in_force;
            let taken = rest.begin();
            if taken {
                if rest.in_force > before {
                    switches.longer += 1;
                } else {
                    switches.shorter += 1;
                }
                previous = Some(index_of(before));
            }
            let peak = peak_at(level, &mut noise);
            let gain = gain_in(&mut limiter, peak);
            let gains: Vec<f64> = always.iter_mut().map(|each| gain_in(each, peak)).collect();
            let here = format!("{context}, segment {segment}, frame {frame}");
            let in_force = index_of(rest.in_force);
            assert_eq!(
                gain.to_bits(),
                gains[in_force].to_bits(),
                "{here}: the gain of a limiter always at {} frames",
                rest.in_force
            );
            path_as(&limiter.path, &always[in_force].path, taken, &here);
            let wanted = index_of(rest.wanted);
            if wanted != in_force && gains[wanted].to_bits() != gain.to_bits() {
                switches.pending_apart += 1;
            }
            if previous.is_some_and(|old| old != in_force && gains[old].to_bits() != gain.to_bits())
            {
                switches.switched_apart += 1;
            }
            rest.end(newest_need(&limiter));
            frame += 1;
        }
    }
    switches
}

/// [`renders_from_each_rest_as_limiters_always_at_its_lookaheads`] in `S`: every rate, maxima
/// of 0.5, 1 and 5 ms, one sequence each; the counts of what they reached at maxima above
/// 0.5 ms (one lookahead only below), each regime reached.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "frame counts and test values far from overflow"
)]
fn renders_from_each_rest_as_limiters_always_at_its_lookaheads_in<S: Sample>() {
    let mut reached = Switches::default();
    for rate in rates() {
        for most in [0.5_f64, 1.0_f64, 5.0_f64] {
            {
                let seed = 1_u64;
                let switches = always_at_its_lookahead_from_each_rest::<S>(
                    rate,
                    most,
                    u64::from(rate.hz()) * 10 + seed,
                );
                if most.to_bits() == 0.5_f64.to_bits() {
                    assert_eq!(
                        (switches.longer, switches.shorter),
                        (0, 0),
                        "one lookahead at a maximum of 0.5 ms"
                    );
                }
                reached.longer += switches.longer;
                reached.shorter += switches.shorter;
                reached.pending_apart += switches.pending_apart;
                reached.switched_apart += switches.switched_apart;
            }
        }
    }
    assert!(
        reached.longer > 0
            && reached.shorter > 0
            && reached.pending_apart > 0
            && reached.switched_apart > 0,
        "binary{}: {} longer, {} shorter, {} pending frames apart, {} switched frames apart",
        S::PRECISION.bits(),
        reached.longer,
        reached.shorter,
        reached.pending_apart,
        reached.switched_apart
    );
}

#[test]
fn renders_from_each_rest_as_limiters_always_at_its_lookaheads() {
    // The law of a lookahead set live (`spec/engine-design.md`, decision 12;
    // `spec/measurements.md`, "The true-peak limiter", step 6): over seeded sequences of
    // every live setting and of peaks that limit, vary and release, at every rate, maxima of
    // 0.5, 1 and 5 ms, in both precisions, a used limiter gives on every frame, bit for bit,
    // the gain of a limiter always at the lookahead in force, fed the same peaks and the
    // same commands: its first one until a lookahead set takes effect, at the first frame of
    // its rest and not before, then that one, the state of its path the other's
    // (`always_at_its_lookahead_from_each_rest`). Its control: lookaheads taken longer and
    // shorter, frames with one pending on which taking it would have changed the gain, and
    // frames after a switch on which keeping the old one would have.
    renders_from_each_rest_as_limiters_always_at_its_lookaheads_in::<Binary64>();
    renders_from_each_rest_as_limiters_always_at_its_lookaheads_in::<Binary32>();
}

/// Whether every value of `path` a frame reads is zero: its hold's front, its release
/// stages, its count of non-zero reductions and its sums.
fn path_at_rest(path: &super::Path) -> bool {
    path.held() == Binary64::ZERO
        && path.fast == Binary64::ZERO
        && path.slow == Binary64::ZERO
        && path.nonzero == 0
        && sums_bits(path) == [0; 6]
}

/// [`holds_a_lookahead_pending_while_any_state_is_not_zero`] in `S`: whether, at some rate,
/// the limiter's own path was at rest on a frame where the longer window's was not.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "frame counts and test values far from overflow"
)]
fn pending_while_any_state_is_not_zero<S: Sample>() -> bool {
    let mut own_rest_too_soon = false;
    for rate in rates() {
        let context = format!("{} Hz in binary{}", rate.hz(), S::PRECISION.bits());
        let built = |lookahead: f64| {
            TruePeakLimiter::<S>::new(
                rate,
                1,
                value(5.0_f64),
                super::Detector::Standard,
                settings(-1.0_f64, lookahead, 10.0_f64, 0.0_f64),
                LimiterMode::Protected,
            )
            .expect("a limiter")
        };
        let (mut limiter, mut longer) = (built(0.5_f64), built(5.0_f64));
        let (short, long) = (
            frames_of_duration(0.5_f64, rate),
            frames_of_duration(5.0_f64, rate),
        );
        assert_eq!(limiter.maximum, long, "{context}: counted apart");
        let mut noise = Noise::new();
        // Limiting on every frame: set, it waits however long the limiting lasts.
        for frame in 0..4 * long {
            if frame == long {
                limiter
                    .set(settings(-1.0_f64, 5.0_f64, 10.0_f64, 0.0_f64))
                    .expect("within the maximum");
            }
            let peak = peak_at(Level::Loud, &mut noise);
            let _: f64 = gain_in(&mut limiter, peak);
            let _: f64 = gain_in(&mut longer, peak);
            assert_eq!(
                limiter.path.lookahead, short,
                "{context}, frame {frame}: pending"
            );
        }
        // Released: pending while any value of either path is not zero, then taken on the
        // frame after its rest, both paths at rest then.
        let cap = rest_within(long, rate, 10.0_f64);
        let mut taken = None;
        for frame in 0..cap {
            let rested = path_at_rest(&limiter.path) && path_at_rest(&longer.path);
            own_rest_too_soon |= path_at_rest(&limiter.path) && !path_at_rest(&longer.path);
            let peak = peak_at(Level::Quiet, &mut noise);
            let _: f64 = gain_in(&mut limiter, peak);
            let _: f64 = gain_in(&mut longer, peak);
            if limiter.path.lookahead == long {
                assert!(rested, "{context}, frame {frame}: taken at rest only");
                taken = Some(frame);
                break;
            }
        }
        assert!(taken.is_some(), "{context}: taken within {cap} frames");
        path_as(&limiter.path, &longer.path, true, &context);
    }
    own_rest_too_soon
}

#[test]
fn holds_a_lookahead_pending_while_any_state_is_not_zero() {
    // At every rate, in both precisions: a limiter at 0.5 ms of a 5 ms maximum, set to 5 ms
    // while it limits on every frame, keeps 0.5 ms however long it limits; released, it keeps
    // it while its path or the path of a limiter always at 5 ms holds any value that is not
    // zero, and takes 5 ms on the frame after its rest, its path then that one's
    // (`spec/engine-design.md`, decision 12). The cheat it catches: a rest of its own path
    // alone, which comes first wherever the longer window's slow stage, rising toward each
    // need for longer, is still above zero (seen at some rate, the control).
    assert!(
        pending_while_any_state_is_not_zero::<Binary64>(),
        "its own rest before the longer window's"
    );
    assert!(
        pending_while_any_state_is_not_zero::<Binary32>(),
        "its own rest before the longer window's, binary32"
    );
}

#[test]
fn takes_a_lookahead_set_at_rest_on_the_next_frame() {
    // A limiter at rest, as a safety limiter that limits rarely is, built or released: a
    // lookahead set takes effect on its next frame, at every rate, longer and shorter, with
    // the phase of its exact sums that of a limiter always at it (`swap_in`, `-step mod L`).
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
        for (frames, lookahead) in [(0_usize, 5.0_f64), (3, 0.5_f64), (17, 2.5_f64)] {
            for _ in 0..frames {
                let _: f64 = gain_of(&mut limiter, 0.5_f64);
            }
            limiter
                .set(settings(-1.0_f64, lookahead, 100.0_f64, 0.5_f64))
                .expect("within the maximum");
            let _: f64 = gain_of(&mut limiter, 0.5_f64);
            let window = frames_of_duration(lookahead, rate);
            assert_eq!(
                limiter.path.lookahead,
                window,
                "{} Hz: {lookahead} ms on the next frame",
                rate.hz()
            );
            let step = usize::try_from(limiter.step).expect("small");
            assert_eq!(
                limiter.path.swap_in,
                (window - step % window) % window,
                "{} Hz: the phase of {lookahead} ms",
                rate.hz()
            );
        }
    }
}

/// [`takes_a_lookahead_set_on_a_used_limiter_at_rest_on_the_next_frame`] in `S`.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "frame counts and test values far from overflow"
)]
fn a_lookahead_set_on_a_used_limiter_at_rest<S: Sample>() {
    for rate in rates() {
        let context = format!("{} Hz in binary{}", rate.hz(), S::PRECISION.bits());
        let built = |lookahead: f64| {
            TruePeakLimiter::<S>::new(
                rate,
                1,
                value(5.0_f64),
                super::Detector::Standard,
                settings(-1.0_f64, lookahead, 10.0_f64, 0.5_f64),
                LimiterMode::Protected,
            )
            .expect("a limiter")
        };
        let maximum = frames_of_duration(5.0_f64, rate);
        // The limiter at 1.5 ms, and limiters always at each lookahead it is set to.
        let mut limiter = built(1.5_f64);
        let lookaheads = [5.0_f64, 2.5_f64, 0.5_f64];
        let mut always: Vec<TruePeakLimiter<S>> =
            lookaheads.iter().map(|each| built(*each)).collect();
        assert_eq!(limiter.maximum, maximum, "{context}: counted apart");
        let mut noise = Noise::new();
        let mut frames = |used: &mut TruePeakLimiter<S>,
                          fixed: &mut [TruePeakLimiter<S>],
                          count: usize,
                          level: Level|
         -> Vec<(f64, Vec<f64>)> {
            (0..count)
                .map(|_| {
                    let peak = peak_at(level, &mut noise);
                    (
                        gain_in(used, peak),
                        fixed.iter_mut().map(|each| gain_in(each, peak)).collect(),
                    )
                })
                .collect()
        };
        // Used: it limits, then rests well past its count under 10 ms.
        drop(frames(&mut limiter, &mut always, 2 * maximum, Level::Loud));
        let rest = rest_within(maximum, rate, 10.0_f64);
        drop(frames(&mut limiter, &mut always, rest, Level::Quiet));
        assert!(
            path_at_rest(&limiter.path) && always.iter().all(|each| path_at_rest(&each.path)),
            "{context}: at rest"
        );
        // Each set changes the release or the adaptivity, with the lookahead in one command
        // or before it: taken on the next frame, then the gains of the limiter always at it.
        for (index, (release, adaptivity, together)) in [
            (1_000.0_f64, 0.5_f64, true),
            (300.0_f64, 1.0_f64, false),
            (30.0_f64, 0.0_f64, true),
        ]
        .into_iter()
        .enumerate()
        {
            let lookahead = lookaheads[index];
            let with = |window: f64| settings(-1.0_f64, window, release, adaptivity);
            let asked = if together {
                lookahead
            } else {
                limiter.settings().lookahead().raw()
            };
            limiter.set(with(asked)).expect("within the maximum");
            for (each, window) in always.iter_mut().zip(lookaheads) {
                each.set(with(window)).expect("within the maximum");
            }
            if !together {
                drop(frames(&mut limiter, &mut always, 1, Level::Quiet));
                limiter.set(with(lookahead)).expect("within the maximum");
            }
            drop(frames(&mut limiter, &mut always, 1, Level::Quiet));
            assert_eq!(
                limiter.path.lookahead,
                frames_of_duration(lookahead, rate),
                "{context}: {lookahead} ms set with {release} ms and {adaptivity}, on the next frame"
            );
            path_as(&limiter.path, &always[index].path, true, &context);
            // Limiting again, then at rest under 10 ms for the next set.
            for (gain, gains) in frames(&mut limiter, &mut always, 4 * maximum, Level::Loud) {
                assert_eq!(
                    gain.to_bits(),
                    gains[index].to_bits(),
                    "{context}: the gain of a limiter always at {lookahead} ms"
                );
            }
            limiter
                .set(settings(-1.0_f64, lookahead, 10.0_f64, 0.5_f64))
                .expect("within the maximum");
            for (each, window) in always.iter_mut().zip(lookaheads) {
                each.set(settings(-1.0_f64, window, 10.0_f64, 0.5_f64))
                    .expect("within the maximum");
            }
            drop(frames(&mut limiter, &mut always, rest, Level::Quiet));
        }
    }
}

#[test]
fn takes_a_lookahead_set_on_a_used_limiter_at_rest_on_the_next_frame() {
    // A used limiter at rest, as a safety limiter is once its input needs no reduction for
    // the count of its release: a lookahead set with another release or adaptivity, in one
    // command or after it, is taken on its next frame, the pace set counting nothing again,
    // since every state of every window is zero and stays so under any pace; then it gives
    // the gains of a limiter always at that lookahead, at every rate, in both precisions
    // (`spec/engine-design.md`, decision 12).
    a_lookahead_set_on_a_used_limiter_at_rest::<Binary64>();
    a_lookahead_set_on_a_used_limiter_at_rest::<Binary32>();
}

#[test]
fn holds_a_lookahead_pending_in_a_switchable_stage_switched_to_raw_while_its_input_needs_one() {
    // A switchable limiter in raw gives its input bit for bit, yet its limiter runs on
    // unheard, so that protecting again finds the state of a limiter always at its lookahead:
    // a lookahead set while its input needs a reduction is pending as in a protected one
    // (`spec/engine-design.md`, decision 12), then taken at the count's rest, from where,
    // switched to protected, it gives the gains of a limiter always at it, at every rate.
    // The cheat it catches: a rest counted from the frames whose reduction is applied only,
    // which in raw would take it at once, where the path of a limiter always at the new
    // lookahead is not at rest (the control).
    for rate in rates() {
        let context = format!("{} Hz", rate.hz());
        let built = |lookahead: f64| {
            TruePeakLimiter::<Binary64>::new(
                rate,
                1,
                value(5.0_f64),
                super::Detector::Standard,
                settings(-1.0_f64, lookahead, 10.0_f64, 0.5_f64),
                LimiterMode::Switchable,
            )
            .expect("a limiter")
        };
        let (mut limiter, mut always) = (built(1.5_f64), built(5.0_f64));
        let maximum = frames_of_duration(5.0_f64, rate);
        assert_eq!(limiter.maximum, maximum, "{context}: counted apart");
        let mut noise = Noise::new();
        let mut moving = false;
        for frame in 0..4 * maximum {
            if frame == maximum {
                limiter
                    .set(settings(-1.0_f64, 5.0_f64, 10.0_f64, 0.5_f64))
                    .expect("within the maximum");
                // The cheat's frame: nothing applied yet, the new window's path not at rest.
                moving = !path_at_rest(&always.path);
            }
            let peak = peak_at(Level::Loud, &mut noise);
            assert_eq!(
                limiter.gain(of::<Binary64>(peak)),
                None,
                "{context}, frame {frame}: raw, its input bit for bit"
            );
            let _: f64 = gain_of(&mut always, peak);
            assert_eq!(
                limiter.pending_lookahead(),
                (frame >= maximum).then_some(value(5.0_f64)),
                "{context}, frame {frame}: pending while its input needs a reduction"
            );
        }
        assert!(moving, "{context}: the new window's path moving when set");
        let cap = rest_within(maximum, rate, 10.0_f64);
        let mut taken = None;
        for frame in 0..cap {
            let peak = peak_at(Level::Quiet, &mut noise);
            assert_eq!(limiter.gain(of::<Binary64>(peak)), None, "{context}: raw");
            let _: f64 = gain_of(&mut always, peak);
            if limiter.pending_lookahead().is_none() {
                taken = Some(frame);
                break;
            }
        }
        assert!(taken.is_some(), "{context}: taken within {cap} frames");
        path_as(&limiter.path, &always.path, true, &context);
        limiter.set_protected(true);
        always.set_protected(true);
        for frame in 0..4 * maximum {
            let peak = peak_at(Level::Loud, &mut noise);
            assert_eq!(
                gain_of(&mut limiter, peak).to_bits(),
                gain_of(&mut always, peak).to_bits(),
                "{context}, frame {frame}: protected, the gain of a limiter always at 5 ms"
            );
        }
    }
}

/// [`takes_the_last_lookahead_set_between_two_frames_alone`] at `rate`.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "frame counts and test values far from overflow"
)]
fn the_last_lookahead_set_between_two_frames_alone(rate: SampleRate) {
    let context = format!("{} Hz", rate.hz());
    let built = || {
        TruePeakLimiter::<Binary64>::new(
            rate,
            1,
            value(1.0_f64),
            super::Detector::Standard,
            settings(-1.0_f64, 0.75_f64, 10.0_f64, 0.5_f64),
            LimiterMode::Protected,
        )
        .expect("a limiter")
    };
    // Two sets, then the second alone; one set back to the lookahead in force, then none;
    // a lookahead set, then in the tail a release and an adaptivity set and set back between
    // the same frames, then the lookahead set alone.
    let (mut twice, mut once, mut back, mut none) = (built(), built(), built(), built());
    let (mut there_and_back, mut alone) = (built(), built());
    let maximum = frames_of_duration(1.0_f64, rate);
    assert_eq!(twice.maximum, maximum, "{context}: counted apart");
    let mut noise = Noise::new();
    let cap = rest_within(maximum, rate, 10.0_f64);
    for (frame, level) in core::iter::repeat_n(Level::Varying, 2 * maximum)
        .chain(core::iter::repeat_n(Level::Quiet, cap))
        .chain(core::iter::repeat_n(Level::Loud, 2 * maximum))
        .enumerate()
    {
        if frame == maximum {
            for (limiter, lookaheads) in [
                (&mut twice, &[0.5_f64, 1.0_f64][..]),
                (&mut once, &[1.0_f64][..]),
                (&mut back, &[0.5_f64, 0.75_f64][..]),
                (&mut none, &[][..]),
            ] {
                for lookahead in lookaheads {
                    limiter
                        .set(settings(-1.0_f64, *lookahead, 10.0_f64, 0.5_f64))
                        .expect("within the maximum");
                }
            }
            for limiter in [&mut there_and_back, &mut alone] {
                limiter
                    .set(settings(-1.0_f64, 1.0_f64, 10.0_f64, 0.5_f64))
                    .expect("within the maximum");
            }
        }
        // A quarter of the way to the rest, the lookahead pending: another release, then
        // another adaptivity, each set and set back between the same frames.
        if frame == 2 * maximum + cap / 4 {
            for (release, adaptivity) in [
                (1_000.0_f64, 0.5_f64),
                (10.0_f64, 0.5_f64),
                (10.0_f64, 1.0_f64),
                (10.0_f64, 0.5_f64),
            ] {
                there_and_back
                    .set(settings(-1.0_f64, 1.0_f64, release, adaptivity))
                    .expect("within the maximum");
            }
            alone
                .set(settings(-1.0_f64, 1.0_f64, 10.0_f64, 0.5_f64))
                .expect("within the maximum");
        }
        let peak = peak_at(level, &mut noise);
        assert_eq!(
            gain_of(&mut there_and_back, peak).to_bits(),
            gain_of(&mut alone, peak).to_bits(),
            "{context}, frame {frame}: a release and an adaptivity set back, the last set alone"
        );
        assert_eq!(
            there_and_back.pending_lookahead(),
            alone.pending_lookahead(),
            "{context}, frame {frame}: pending as the last set alone"
        );
        assert_eq!(
            gain_of(&mut twice, peak).to_bits(),
            gain_of(&mut once, peak).to_bits(),
            "{context}, frame {frame}: the last set alone"
        );
        assert_eq!(
            gain_of(&mut back, peak).to_bits(),
            gain_of(&mut none, peak).to_bits(),
            "{context}, frame {frame}: a set back, none"
        );
        assert_eq!(
            back.path.lookahead,
            frames_of_duration(0.75_f64, rate),
            "{context}, frame {frame}: the lookahead in force kept"
        );
    }
    // The control: the last set taken, at the rest the quiet frames reach.
    assert_eq!(
        twice.path.lookahead,
        frames_of_duration(1.0_f64, rate),
        "{context}: the last one taken"
    );
    assert_eq!(
        there_and_back.path.lookahead,
        frames_of_duration(1.0_f64, rate),
        "{context}: taken through a release set back"
    );
}

#[test]
fn takes_the_last_lookahead_set_between_two_frames_alone() {
    // At every rate: two lookaheads set between the same frames while it limits act as the
    // second one alone, bit for bit, through its rest and after, the second taken (the
    // control); one set back to the lookahead in force acts as no set, its rest changing
    // nothing; a release and an adaptivity set and set back between the same frames while a
    // lookahead is pending act as the last set alone, its pending lookahead and its gains
    // those of a limiter given that set only, taken at the same rest (`spec/engine-design.md`,
    // decision 12).
    for rate in rates() {
        the_last_lookahead_set_between_two_frames_alone(rate);
    }
}

/// [`rests_only_where_every_window_is_at_rest`] in `S`, at `rate` and a maximum of `most`
/// ms: the frames at rest by [`Rest`].
#[expect(
    clippy::arithmetic_side_effects,
    reason = "frame counts far from overflow"
)]
fn every_window_at_rest_where_it_rests<S: Sample>(rate: SampleRate, most: f64) -> usize {
    let context = format!(
        "{} Hz, {most} ms at most, binary{}",
        rate.hz(),
        S::PRECISION.bits()
    );
    let built = |lookahead: f64| {
        TruePeakLimiter::<S>::new(
            rate,
            1,
            value(most),
            super::Detector::Standard,
            settings(-1.0_f64, lookahead, 300.0_f64, 0.0_f64),
            LimiterMode::Protected,
        )
        .expect("a limiter")
    };
    // Windows from the shortest to the maximum, the limiter at the shortest.
    let lookaheads = [
        0.5_f64,
        0.5_f64 + (most - 0.5_f64) / 4.0_f64,
        0.5_f64 + (most - 0.5_f64) / 2.0_f64,
        most,
    ];
    let mut limiter = built(0.5_f64);
    let mut windows: Vec<TruePeakLimiter<S>> = lookaheads.iter().map(|each| built(*each)).collect();
    let maximum = frames_of_duration(most, rate);
    assert_eq!(limiter.maximum, maximum, "{context}: counted apart");
    let mut rest = Rest::new(frames_of_duration(0.5_f64, rate));
    rest.released(300.0_f64, 0.0_f64, limiter.slow_attack);
    let mut noise = Noise::new();
    let mut resting = 0_usize;
    let cap = rest_within(maximum, rate, 10.0_f64).saturating_mul(2);
    // Varying under 300 ms; quiet under 10 ms, its adaptivity changed halfway to the rest
    // (the count starting again), then at rest; loud under 10 ms, an adaptivity of 1.
    let stretches = [
        (Level::Varying, 300.0_f64, 0.0_f64),
        (Level::Quiet, 10.0_f64, 0.0_f64),
        (Level::Loud, 10.0_f64, 1.0_f64),
    ];
    let mut frame = 0_usize;
    for (stretch, (level, release, adaptivity)) in stretches.into_iter().enumerate() {
        let mut changed = false;
        for each in core::iter::once(&mut limiter).chain(windows.iter_mut()) {
            let lookahead = each.settings().lookahead();
            each.set(settings(-1.0_f64, lookahead.raw(), release, adaptivity))
                .expect("within the maximum");
        }
        rest.released(release, adaptivity, limiter.slow_attack);
        let quiet = matches!(level, Level::Quiet);
        for at in 0..cap {
            if !quiet && at == 4 * maximum + 64 {
                break;
            }
            if quiet && rest.resting() && at > 4 * maximum {
                break;
            }
            // Halfway through the first quiet stretch, the adaptivity changed: the count of
            // the decay starts again.
            if stretch == 1 && !changed && rest.since_need > rest.decay / 2 + maximum {
                changed = true;
                for each in core::iter::once(&mut limiter).chain(windows.iter_mut()) {
                    let lookahead = each.settings().lookahead();
                    each.set(settings(-1.0_f64, lookahead.raw(), release, 0.25_f64))
                        .expect("within the maximum");
                }
                rest.released(release, 0.25_f64, limiter.slow_attack);
            }
            assert!(
                !quiet || at + 1 < cap,
                "{context}, stretch {stretch}: at rest within {cap} frames"
            );
            let peak = peak_at(level, &mut noise);
            let _: f64 = gain_in(&mut limiter, peak);
            for each in &mut windows {
                let _: f64 = gain_in(each, peak);
            }
            rest.end(newest_need(&limiter));
            if rest.resting() {
                resting += 1;
                for each in core::iter::once(&limiter).chain(windows.iter()) {
                    assert!(
                        path_at_rest(&each.path),
                        "{context}, frame {frame}: at rest, a window of {} frames is not",
                        each.path.lookahead
                    );
                }
            }
            frame += 1;
        }
        assert!(
            stretch != 1 || changed,
            "{context}: the adaptivity changed within the first quiet stretch"
        );
    }
    resting
}

#[test]
fn rests_only_where_every_window_is_at_rest() {
    // The premise of a lookahead's rest (`spec/measurements.md`, "The true-peak limiter",
    // step 6): at every rate, maxima of 0.5, 1 and 5 ms, in both precisions, on every frame
    // its rule ([`Rest`]) calls a rest, through varying, quiet and loud stretches under
    // releases of 10 and 300 ms and adaptivities of 0, 0.25 and 1, one changed within a
    // release tail (its count starting again), its own path and those of limiters always at windows from the
    // shortest to the maximum, fed the same peaks and commands, hold nothing but zeros. The
    // control: frames at rest reached in each run, after the last limited frame and the
    // count of its release.
    for rate in rates() {
        for most in [0.5_f64, 1.0_f64, 5.0_f64] {
            assert!(
                every_window_at_rest_where_it_rests::<Binary64>(rate, most) > 0,
                "{} Hz, {most} ms: at rest",
                rate.hz()
            );
            assert!(
                every_window_at_rest_where_it_rests::<Binary32>(rate, most) > 0,
                "{} Hz, {most} ms: at rest in binary32",
                rate.hz()
            );
        }
    }
}

/// [`renders_from_its_rest_the_samples_of_a_limiter_always_at_its_lookahead`] in `S`.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "frame counts and test values far from overflow"
)]
fn samples_of_a_limiter_always_at_its_lookahead<S: Sample + Raw>() {
    // The two lowest rates of the matrix (a whole number of frames per millisecond and not)
    // and 48 kHz: the gains it applies are the law's at every rate
    // (`renders_from_each_rest_as_limiters_always_at_its_lookaheads`), and the delay that
    // carries the samples to them depends on no lookahead.
    for rate in rates()
        .into_iter()
        .filter(|rate| rate.hz() <= 11_025 || rate.hz() == 48_000)
    {
        let context = format!("{} Hz in binary{}", rate.hz(), S::PRECISION.bits());
        let built = |lookahead: f64| {
            TruePeakLimiter::<S>::new(
                rate,
                2,
                value(5.0_f64),
                super::Detector::Standard,
                settings(-3.0_f64, lookahead, 10.0_f64, 1.0_f64),
                LimiterMode::Protected,
            )
            .expect("a limiter")
        };
        // The limiter set from 1.5 to 5 ms while it limits, and limiters always at each.
        let (mut limiter, mut old, mut new) = (built(1.5_f64), built(1.5_f64), built(5.0_f64));
        let maximum = frames_of_duration(5.0_f64, rate);
        assert_eq!(limiter.maximum, maximum, "{context}: counted apart");
        let mut draws = Noise::new();
        let cap = rest_within(maximum, rate, 10.0_f64);
        let (mut taken, mut apart) = (None, false);
        let frames = 3 * maximum + cap + 3 * maximum;
        for frame in 0..frames {
            if frame == maximum {
                limiter
                    .set(settings(-3.0_f64, 5.0_f64, 10.0_f64, 1.0_f64))
                    .expect("within the maximum");
            }
            // Noise four times the ceiling, then a quarter of it, then four times again.
            let amplitude = if (3 * maximum..3 * maximum + cap).contains(&frame) {
                0.25_f64
            } else {
                4.0_f64
            };
            let input: [Block<S, 1>; 2] =
                core::array::from_fn(|_| [drawn::<S>(&mut draws, amplitude)]);
            let mut outputs = [input, input, input];
            for (each, output) in [&mut limiter, &mut old, &mut new]
                .into_iter()
                .zip(outputs.iter_mut())
            {
                each.apply(output);
            }
            if taken.is_none() && limiter.path.lookahead == maximum {
                taken = Some(frame);
            }
            let expected = if taken.is_some() {
                outputs[2]
            } else {
                outputs[1]
            };
            for channel in 0..2 {
                assert_eq!(
                    outputs[0][channel][0].bits(),
                    expected[channel][0].bits(),
                    "{context}, frame {frame}, channel {channel}: the sample of a limiter \
                     always at {} ms",
                    if taken.is_some() { 5.0_f64 } else { 1.5_f64 }
                );
            }
            apart |= taken.is_some() && outputs[1][0][0].bits() != outputs[0][0][0].bits();
        }
        // The control: taken within the quiet frames, and samples the old lookahead would
        // have given otherwise after it.
        assert!(
            taken.is_some_and(|frame| frame < 3 * maximum + cap) && apart,
            "{context}: taken at {taken:?}, apart {apart}"
        );
    }
}

#[test]
fn renders_from_its_rest_the_samples_of_a_limiter_always_at_its_lookahead() {
    // The samples themselves (`spec/engine-design.md`, decision 12): at 8, 11.025 and 48 kHz,
    // in both precisions, two channels of noise four times a ceiling of -3 dBTP through a limiter
    // of a 5 ms maximum set from 1.5 to 5 ms while it limits, a quarter of it until its rest
    // and four times again after: each sample is, bit for bit, that of a limiter always at
    // 1.5 ms until the frame its lookahead is taken, and that of one always at 5 ms from
    // then on, fed the same samples and commands.
    samples_of_a_limiter_always_at_its_lookahead::<Binary64>();
    samples_of_a_limiter_always_at_its_lookahead::<Binary32>();
}

/// The blocks of `N` frames of each of audio-host's heap test phases on a limiter of a 2 ms
/// maximum lookahead and the precise detector at `rate`
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
/// blocks of `N` frames at every rate, in `S`: its loud patch (`LOUD` there: sines of 440,
/// 1,000 and 3,000 Hz summed on the first channel, the first two on the second, each at full
/// scale) through a switchable stage of a 2 ms maximum and the precise detector at the
/// default settings, five phases of its blocks ([`heap_phase_blocks`]), 0.5 ms set after the
/// first, 2 ms (the one in force) then 1 ms after the second, the mode switched after the
/// third and the fourth: each lookahead set stays pending to the end, the limiter limiting on
/// every frame, so that the heap law counts the frames that look for a rest; and its silent
/// patch (`SILENT` there) through the same stage, 0.5 ms set before its first block and
/// 2 ms after its first phase: each taken on the next frame, so that the heap law counts the
/// frames that take one.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "frame counts of a few hundred thousand, far from overflow"
)]
fn takes_each_lookahead_of_the_heap_tests_phases_in_blocks_of<S: Sample, const N: usize>() {
    for rate in rates() {
        let context = format!(
            "{} Hz, blocks of {N} in binary{}",
            rate.hz(),
            S::PRECISION.bits()
        );
        let built = || {
            TruePeakLimiter::<S>::new(
                rate,
                2,
                value(2.0_f64),
                super::Detector::Precise,
                LimiterSettings::DEFAULT,
                LimiterMode::Switchable,
            )
            .expect("a limiter")
        };
        let mut limiter = built();
        assert_eq!(
            limiter.latency(),
            latency_of(2.0_f64, rate, super::Detector::Precise),
            "{context}: counted apart"
        );
        let hz = f64::from(rate.hz());
        let mut frame = 0_u32;
        let mut blocks_of = |stage: &mut TruePeakLimiter<S>, count: usize, loud: bool| {
            for _ in 0..count {
                let mut blocks = [[S::ZERO; N]; 2];
                let [left, right] = &mut blocks;
                for (first, second) in left.iter_mut().zip(right.iter_mut()) {
                    let at = f64::from(frame);
                    let sine =
                        |frequency: f64| libm::sin(core::f64::consts::TAU * frequency * at / hz);
                    if loud {
                        *first = of::<S>(sine(440.0_f64) + sine(1_000.0_f64) + sine(3_000.0_f64));
                        *second = of::<S>(sine(440.0_f64) + sine(1_000.0_f64));
                    }
                    frame += 1;
                }
                stage.apply(&mut blocks);
            }
        };
        let with = |lookahead: f64| {
            LimiterSettings::DEFAULT
                .with_lookahead(value(lookahead))
                .expect("in range")
        };
        let phase = heap_phase_blocks::<N>(rate);
        let (default, two) = (
            frames_of_duration(1.5_f64, rate),
            frames_of_duration(2.0_f64, rate),
        );
        blocks_of(&mut limiter, phase, true);
        limiter.set(with(0.5_f64)).expect("within the maximum");
        blocks_of(&mut limiter, phase, true);
        assert_eq!(limiter.path.lookahead, default, "{context}: 0.5 ms pending");
        limiter.set(with(2.0_f64)).expect("within the maximum");
        limiter.set(with(1.0_f64)).expect("within the maximum");
        blocks_of(&mut limiter, phase, true);
        limiter.set_protected(true);
        blocks_of(&mut limiter, phase, true);
        limiter.set_protected(false);
        blocks_of(&mut limiter, phase, true);
        assert_eq!(
            (limiter.path.lookahead, limiter.wanted),
            (default, frames_of_duration(1.0_f64, rate)),
            "{context}: 1 ms pending to the end"
        );
        let mut silent = built();
        silent.set(with(0.5_f64)).expect("within the maximum");
        blocks_of(&mut silent, 1, false);
        assert_eq!(
            silent.path.lookahead,
            frames_of_duration(0.5_f64, rate),
            "{context}: 0.5 ms taken at once"
        );
        blocks_of(&mut silent, phase - 1, false);
        silent.set(with(2.0_f64)).expect("within the maximum");
        blocks_of(&mut silent, phase, false);
        assert_eq!(silent.path.lookahead, two, "{context}: 2 ms taken");
    }
}

#[test]
fn takes_each_lookahead_of_the_heap_tests_phases() {
    // The positive control of audio-host's heap law of live changes: at the finest and the
    // coarsest engine blocks (its phases of whole blocks only lengthen between them, and its
    // loud patch keeps the limiter limiting on every frame whatever their length), the
    // frames it counts both look for a rest with a lookahead pending (its loud patch) and
    // take one (its silent patch).
    takes_each_lookahead_of_the_heap_tests_phases_in_blocks_of::<Binary64, 16>();
    takes_each_lookahead_of_the_heap_tests_phases_in_blocks_of::<Binary32, 16>();
    takes_each_lookahead_of_the_heap_tests_phases_in_blocks_of::<Binary64, 1024>();
    takes_each_lookahead_of_the_heap_tests_phases_in_blocks_of::<Binary32, 1024>();
}

#[test]
fn tells_the_lookahead_pending_until_it_takes_it() {
    // At every rate: none pending when built nor when one set is taken; the lookahead asked
    // for, in milliseconds, while it waits for the limiter's rest, on every frame up to the
    // one that takes it (the window then in force), none from that frame on; none for a set
    // of the lookahead in force, or of another that counts its frames alike (1.5 and 1.51 ms
    // at 48 kHz, 72 frames each); a set back to the lookahead in force drops it.
    for rate in rates() {
        let context = format!("{} Hz", rate.hz());
        let with = |lookahead: f64| settings(-1.0_f64, lookahead, 10.0_f64, 0.5_f64);
        let mut limiter = TruePeakLimiter::<Binary64>::new(
            rate,
            1,
            value(5.0_f64),
            super::Detector::Standard,
            with(1.5_f64),
            LimiterMode::Protected,
        )
        .expect("a limiter");
        assert_eq!(limiter.pending_lookahead(), None, "{context}: built");
        let maximum = frames_of_duration(5.0_f64, rate);
        for _ in 0..maximum {
            let _: f64 = gain_of(&mut limiter, 2.0_f64);
        }
        limiter.set(with(5.0_f64)).expect("within the maximum");
        assert_eq!(
            limiter.pending_lookahead(),
            Some(value(5.0_f64)),
            "{context}: pending while it limits"
        );
        limiter.set(with(1.5_f64)).expect("within the maximum");
        assert_eq!(limiter.pending_lookahead(), None, "{context}: set back");
        limiter.set(with(5.0_f64)).expect("within the maximum");
        let cap = rest_within(maximum, rate, 10.0_f64);
        let mut taken = false;
        for frame in 0..cap {
            let _: f64 = gain_of(&mut limiter, 0.5_f64);
            let in_force = limiter.path.lookahead == maximum;
            assert_eq!(
                limiter.pending_lookahead(),
                (!in_force).then_some(value(5.0_f64)),
                "{context}, frame {frame}"
            );
            if in_force {
                taken = true;
                break;
            }
        }
        assert!(taken, "{context}: taken within {cap} frames");
        limiter.set(with(5.0_f64)).expect("within the maximum");
        assert_eq!(
            limiter.pending_lookahead(),
            None,
            "{context}: the one in force"
        );
    }
    let rate = SampleRate::new(48_000).expect("a rate");
    let mut limiter = TruePeakLimiter::<Binary32>::new(
        rate,
        1,
        value(5.0_f64),
        super::Detector::Standard,
        settings(-1.0_f64, 1.5_f64, 10.0_f64, 0.5_f64),
        LimiterMode::Protected,
    )
    .expect("a limiter");
    let _: f64 = gain_in(&mut limiter, 2.0_f64);
    limiter
        .set(settings(-1.0_f64, 1.51_f64, 10.0_f64, 0.5_f64))
        .expect("within the maximum");
    assert_eq!(limiter.pending_lookahead(), None, "72 frames either way");
}

#[test]
fn counts_the_decay_again_from_a_release_set_while_a_lookahead_is_pending() {
    // At every rate: 5 ms set on a limiter at 1.5 ms while it limits, then released under
    // 10 ms; halfway through the tail, its adaptivity set from 0 to 0.5 (the fast stage's
    // pace changed): the lookahead is taken where the rule says ([`Rest`]), counted from that
    // set, not on the frame the last need alone would allow (reached: the control).
    for rate in rates() {
        let context = format!("{} Hz", rate.hz());
        let with =
            |adaptivity: f64, lookahead: f64| settings(-1.0_f64, lookahead, 10.0_f64, adaptivity);
        let mut limiter = TruePeakLimiter::<Binary64>::new(
            rate,
            1,
            value(5.0_f64),
            super::Detector::Standard,
            with(0.0_f64, 1.5_f64),
            LimiterMode::Protected,
        )
        .expect("a limiter");
        let maximum = frames_of_duration(5.0_f64, rate);
        assert_eq!(limiter.maximum, maximum, "{context}: counted apart");
        let mut rest = Rest::new(frames_of_duration(1.5_f64, rate));
        rest.released(10.0_f64, 0.0_f64, limiter.slow_attack);
        let mut held_back = 0_usize;
        let cap = 2 * rest_within(maximum, rate, 10.0_f64);
        let mut changed = false;
        for frame in 0..cap {
            if frame == maximum {
                limiter
                    .set(with(0.0_f64, 5.0_f64))
                    .expect("within the maximum");
                rest.wanted = maximum;
            }
            if !changed && frame > 2 * maximum && rest.since_need > maximum + rest.decay / 2 {
                changed = true;
                limiter
                    .set(with(0.5_f64, 5.0_f64))
                    .expect("within the maximum");
                rest.released(10.0_f64, 0.5_f64, limiter.slow_attack);
            }
            if rest.wanted != rest.in_force
                && !rest.never
                && rest.since_need >= rest.decay
                && rest.since_release < rest.decay
            {
                held_back += 1;
            }
            let _: bool = rest.begin();
            let peak = if frame < 2 * maximum {
                2.0_f64
            } else {
                0.5_f64
            };
            let _: f64 = gain_of(&mut limiter, peak);
            assert_eq!(
                limiter.path.lookahead, rest.in_force,
                "{context}, frame {frame}: the lookahead in force"
            );
            rest.end(newest_need(&limiter));
            if rest.in_force == maximum {
                break;
            }
        }
        assert!(
            changed && held_back > 0 && limiter.path.lookahead == maximum,
            "{context}: changed {changed}, held back {held_back}, in force {}",
            limiter.path.lookahead
        );
    }
}

#[test]
fn counts_the_decay_again_when_either_stage_changes_its_pace() {
    // The count of a release's decay starts again, from the frame just computed, when the
    // fast stage's keep or the slow stage's share changes, and only then: a release of
    // 100 ms at an adaptivity of 0 and one of 1,000 ms at 1 have the same fast release,
    // 100 ms, and other slow stages; the same settings set again change neither. Its count
    // is `round(71 / c) + 2`, `c` the slow stage's share, written apart.
    let rate = SampleRate::new(48_000).expect("a rate");
    let mut limiter = TruePeakLimiter::<Binary64>::new(
        rate,
        1,
        value(5.0_f64),
        super::Detector::Standard,
        settings(-1.0_f64, 1.5_f64, 100.0_f64, 0.0_f64),
        LimiterMode::Protected,
    )
    .expect("a limiter");
    let expected =
        |of: &TruePeakLimiter<Binary64>| libm::round(71.0_f64 / of.slow_attack.raw()) + 2.0_f64;
    for _ in 0_u8..10 {
        let _: f64 = gain_of(&mut limiter, 0.5_f64);
    }
    let keep_fast = limiter.keep_fast;
    limiter
        .set(settings(-1.0_f64, 1.5_f64, 1_000.0_f64, 1.0_f64))
        .expect("in range");
    assert_eq!(limiter.keep_fast, keep_fast, "the same fast release");
    assert_eq!(
        (
            limiter.released,
            f64::from(u32::try_from(limiter.decaying).expect("small"))
        ),
        (10, expected(&limiter)),
        "the slow stage changed: counted again"
    );
    for _ in 0_u8..10 {
        let _: f64 = gain_of(&mut limiter, 0.5_f64);
    }
    limiter
        .set(settings(-3.0_f64, 2.5_f64, 1_000.0_f64, 1.0_f64))
        .expect("in range");
    assert_eq!(limiter.released, 10, "the same release: not counted again");
}
