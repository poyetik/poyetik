//! Tests of the true-peak limiter.

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use super::{DETECTOR_DELAY, LimiterError, LimiterSettings, Setting, TruePeakLimiter};
use crate::host::Raw;
use crate::math::tests::splitmix::Draws;
use crate::{BLOCK_SIZE, Binary32, Binary64, Block, Sample, SampleRate};

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

/// A sample drawn uniformly in [-amplitude, amplitude), in binary64 then in `S`.
fn drawn<S: Sample>(draws: &mut Draws, amplitude: f64) -> S {
    // 52 random bits in the fraction of a number of [1, 2): uniform there.
    let unit = f64::from_bits((0x3ff_u64 << 52_u32) | (draws.next() >> 12_u32));
    of::<S>((unit - 1.5_f64) * 2.0_f64 * amplitude)
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
/// half a frame.
fn frames_at(milliseconds: f64, rate: SampleRate) -> usize {
    let exact = milliseconds * f64::from(rate.hz()) / 1_000.0_f64;
    let count = (0_u32..=u32::MAX)
        .find(|count| f64::from(*count) + 0.5_f64 > exact)
        .expect("a count");
    usize::try_from(count).expect("small").max(1)
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
        assert!(with(index, low).is_ok(), "{setting:?} at its low end");
        assert!(with(index, high).is_ok(), "{setting:?} at its high end");
        for refused in [low.next_down(), high.next_up(), f64::NAN, f64::INFINITY] {
            assert_eq!(
                with(index, refused)
                    .map(|_| ())
                    .map_err(|error| match error {
                        LimiterError::OutOfRange(named, raw) => (named, raw.bits()),
                        LimiterError::AboveMaximum { .. } => panic!("out of range"),
                    }),
                Err((setting, refused.to_bits())),
                "{setting:?} refuses {refused}"
            );
        }
    }
    let rate = SampleRate::new(48_000).expect("a rate");
    for (maximum, accepted) in [
        (0.5_f64, true),
        (5.0_f64, true),
        (0.5_f64.next_down(), false),
        (5.0_f64.next_up(), false),
    ] {
        let built = TruePeakLimiter::<Binary64>::new(
            rate,
            2,
            value(maximum),
            settings(-1.0_f64, 0.5_f64, 100.0_f64, 0.5_f64),
            true,
        );
        assert_eq!(
            built.is_ok(),
            accepted,
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
        settings(-1.0_f64, 2.0_f64, 100.0_f64, 0.5_f64),
        true,
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
            settings(-1.0_f64, 2.0_f64, 100.0_f64, 0.5_f64),
            true
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
fn declares_its_latency_from_the_maximum_lookahead_at_every_rate() {
    for rate in rates() {
        for maximum in [0.5_f64, 1.5_f64, 5.0_f64] {
            let limiter = TruePeakLimiter::<Binary64>::new(
                rate,
                2,
                value(maximum),
                settings(-1.0_f64, 0.5_f64, 100.0_f64, 0.5_f64),
                true,
            )
            .expect("a limiter");
            assert_eq!(
                limiter.latency(),
                frames_at(maximum, rate) + DETECTOR_DELAY,
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

/// The law below the ceiling, in `S`: at every rate, a signal whose true peak stays below
/// the ceiling leaves delayed, bit for bit.
fn changes_no_bit_below_the_ceiling<S: Sample + Raw>() {
    let mut draws = Draws::new();
    for rate in rates() {
        for channels in [1, 2, 3] {
            // Uniform in ±0.4_f64: each phase of the interpolator sums at most 2.02_f64 times the
            // largest sample (the sum of its taps' magnitudes), so the true peak stays below
            // 0.81_f64, under the default ceiling of -1 dBTP (0.891_f64).
            let input = signal::<S>(12, channels, |_, _| drawn(&mut draws, 0.4_f64));
            let mut limiter = TruePeakLimiter::<S>::new(
                rate,
                channels,
                value(5.0_f64),
                LimiterSettings::DEFAULT,
                true,
            )
            .expect("a limiter");
            let output = limited(&mut limiter, &input);
            assert!(
                delayed_exactly(&input, &output, limiter.latency(), channels),
                "{channels} channels at {} Hz",
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
/// 384 kHz, 1,920 frames, plus the detector's 6), so that every rate gives limited output.
const LOUD_BLOCKS: usize = 4_096 / BLOCK_SIZE;

/// Loud test signals, in `S`: noise, then sines from low to near half the rate, each well
/// above the ceiling.
fn loud<S: Sample>(channels: usize, draws: &mut Draws) -> Vec<Vec<Vec<Block<S>>>> {
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
        .fold(0.0_f64, |largest, magnitude| {
            if magnitude > largest {
                magnitude
            } else {
                largest
            }
        })
}

/// The ceiling held, in `S`: on loud signals, no output sample above the ceiling but by the
/// rounding `slack` (relative).
fn holds_the_ceiling<S: Sample + Raw>(slack: f64) {
    let mut draws = Draws::new();
    // 10^(-1/20), the default ceiling of -1 dBTP as an amplitude, computed apart.
    let ceiling = 0.891_250_938_133_745_6_f64;
    for rate in rates() {
        for channels in [1, 2] {
            for input in loud::<S>(channels, &mut draws) {
                let mut limiter = TruePeakLimiter::<S>::new(
                    rate,
                    channels,
                    value(5.0_f64),
                    LimiterSettings::DEFAULT,
                    true,
                )
                .expect("a limiter");
                let output = limited(&mut limiter, &input);
                let peak = largest(&output, 0, channels);
                assert!(
                    limiter.latency() < LOUD_BLOCKS * BLOCK_SIZE,
                    "limited output"
                );
                assert!(
                    peak <= ceiling * (1.0_f64 + slack),
                    "{peak} above {ceiling} at {} Hz, {channels} channels",
                    rate.hz()
                );
                assert!(
                    peak > ceiling * 0.5_f64,
                    "it does not silence the signal: {peak}"
                );
            }
        }
    }
}

#[test]
fn holds_the_ceiling_in_each_precision() {
    holds_the_ceiling::<Binary64>(1e-9_f64);
    holds_the_ceiling::<Binary32>(1e-4_f64);
}

/// Raw, in `S`: even a loud signal leaves delayed, bit for bit.
fn passes_everything_raw<S: Sample + Raw>() {
    let mut draws = Draws::new();
    for rate in rates() {
        for input in loud::<S>(2, &mut draws) {
            let mut limiter =
                TruePeakLimiter::<S>::new(rate, 2, value(5.0_f64), LimiterSettings::DEFAULT, false)
                    .expect("a limiter");
            let output = limited(&mut limiter, &input);
            assert!(
                delayed_exactly(&input, &output, limiter.latency(), 2),
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
    let mut draws = Draws::new();
    let rate = SampleRate::new(48_000).expect("a rate");
    let loud_input = signal::<S>(100, 2, |_, _| drawn(&mut draws, 4.0_f64));
    let mut limiter =
        TruePeakLimiter::<S>::new(rate, 2, value(1.5_f64), LimiterSettings::DEFAULT, true)
            .expect("a limiter");
    let _limited = limited(&mut limiter, &loud_input);
    limiter.set_protected(false);
    assert!(!limiter.protected(), "raw from now on");
    // 20 ms constant, snapping below 2^-24: about 17 time constants, 333 ms; a second is
    // 750 blocks at 48 kHz.
    let fading = signal::<S>(750, 2, |_, _| drawn(&mut draws, 4.0_f64));
    let _faded = limited(&mut limiter, &fading);
    let after = signal::<S>(50, 2, |_, _| drawn(&mut draws, 4.0_f64));
    let output = limited(&mut limiter, &after);
    let latency = limiter.latency();
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
fn holds_the_ceiling_when_its_lookahead_changes_live() {
    let mut draws = Draws::new();
    // 10^(-1/20), the default ceiling of -1 dBTP as an amplitude, computed apart.
    let ceiling = 0.891_250_938_133_745_6_f64;
    for rate in rates() {
        let mut limiter = TruePeakLimiter::<Binary64>::new(
            rate,
            2,
            value(5.0_f64),
            settings(-1.0_f64, 5.0_f64, 100.0_f64, 0.5_f64),
            true,
        )
        .expect("a limiter");
        let mut peak = 0.0_f64;
        for lookahead in [5.0_f64, 0.5_f64, 3.0_f64, 1.5_f64] {
            limiter
                .set(settings(-1.0_f64, lookahead, 100.0_f64, 0.5_f64))
                .expect("within the maximum");
            let input = signal::<Binary64>(10, 2, |_, _| drawn(&mut draws, 4.0_f64));
            let output = limited(&mut limiter, &input);
            let this = largest(&output, 0, 2);
            if this > peak {
                peak = this;
            }
        }
        assert!(
            peak <= ceiling * (1.0_f64 + 1e-9_f64),
            "{peak} at {} Hz",
            rate.hz()
        );
    }
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
fn holds_the_ceiling_once_protected_again<S: Sample + Raw>(slack: f64) {
    let mut draws = Draws::new();
    // 10^(-1/20), the default ceiling as an amplitude, computed apart.
    let ceiling = 0.891_250_938_133_745_6_f64;
    let rate = SampleRate::new(48_000).expect("a rate");
    let mut limiter =
        TruePeakLimiter::<S>::new(rate, 2, value(1.5_f64), LimiterSettings::DEFAULT, false)
            .expect("a limiter");
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
    let peak = largest(&protected, limiter.latency(), 2);
    assert!(
        peak <= ceiling * (1.0_f64 + slack),
        "{peak} once protected again"
    );
}

#[test]
fn holds_the_ceiling_once_protected_again_in_each_precision() {
    holds_the_ceiling_once_protected_again::<Binary64>(1e-9_f64);
    holds_the_ceiling_once_protected_again::<Binary32>(1e-4_f64);
}

/// Live settings, in `S`: a lookahead above the maximum refused, the settings in force
/// kept; a new release with the same lookahead taken as it is.
fn changes_its_settings_live<S: Sample>() {
    let rate = SampleRate::new(96_000).expect("a rate");
    let mut limiter = TruePeakLimiter::<S>::new(
        rate,
        2,
        value(3.0_f64),
        settings(-1.0_f64, 3.0_f64, 100.0_f64, 0.5_f64),
        true,
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
    let one = of::<S>(1.0_f64);
    let mut draws = Draws::new();
    let mut limiter =
        TruePeakLimiter::<S>::new(rate, 1, value(1.5_f64), LimiterSettings::DEFAULT, true)
            .expect("a limiter");
    assert_eq!(limiter.applied, one, "protected from the first frame");
    for (protect, end) in [(false, S::ZERO), (true, one)] {
        limiter.set_protected(protect);
        let _block = limited(
            &mut limiter,
            &signal::<S>(1, 1, |_, _| drawn(&mut draws, 4.0_f64)),
        );
        assert!(
            limiter.applied > S::ZERO && limiter.applied < one,
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
/// are computed exactly once per window, the frames between counted from 0 to its length
/// less one.
fn keeps_its_windows_exact_and_small<S: Sample>() {
    let rate = SampleRate::new(48_000).expect("a rate");
    let mut limiter =
        TruePeakLimiter::<S>::new(rate, 1, value(1.5_f64), LimiterSettings::DEFAULT, true)
            .expect("a limiter");
    let window = limiter.lookahead;
    assert_eq!(window, 72, "1.5 ms at 48 kHz");
    let mut counts = Vec::new();
    for block in 0_u8..20 {
        let _block = limited(&mut limiter, &signal::<S>(1, 1, |_, _| of::<S>(2.0_f64)));
        counts.push(limiter.since_exact);
        // Once the window has seen only the constant (its 74 frames, the 12 of the
        // interpolator before them): two blocks.
        if block >= 2 {
            assert_eq!(limiter.queued, 1, "one need held after block {block}");
        }
    }
    assert!(
        counts.iter().all(|count| *count < window),
        "refreshed within each window: {counts:?}"
    );
    assert!(
        counts.iter().any(|count| *count > 0),
        "slid between refreshes: {counts:?}"
    );
}

#[test]
fn keeps_its_windows_exact_and_small_in_each_precision() {
    keeps_its_windows_exact_and_small::<Binary64>();
    keeps_its_windows_exact_and_small::<Binary32>();
}
