//! The output stage's true-peak limiter (`spec/engine-design.md`, decision 12): a component
//! of the core, not a box, so that no patch can bypass it and it acts on what is delivered.
//!
//! # Fixed when built, live after
//!
//! Its **maximum lookahead** is fixed when it is built: it sizes the delay lines and sets
//! the latency, `maximum lookahead + Detector::delay` frames, declared
//! ([`TruePeakLimiter::latency`]) and never changed while it renders, so that no setting
//! moves the signal in time and a host compensates it once. Changing it means building the
//! limiter again.
//!
//! Every other setting changes **live**, the ceiling held across each change (a lowered
//! ceiling over the maximum lookahead after it, within the latency: the samples leaving the
//! delay until then keep the needs computed for the old one, so every sample is at or below
//! the old ceiling over the maximum lookahead from the change, and at or below the new one
//! from then on, `lowers_its_ceiling_live_after_its_maximum_lookahead_*`; a raised one at
//! once): the ceiling,
//! the release and its adaptivity ([`TruePeakLimiter::set`]), which the window held and the
//! Hann smoothing or the release carry to the gain; the mode
//! ([`TruePeakLimiter::set_protected`]), by a crossfade of the reduction (in raw, its output
//! is its input delayed by the latency, bit for bit); and the lookahead itself up to the
//! maximum (the output stays delayed by the maximum; only the window the limiter looks
//! ahead through changes), by a **crossfade of two control paths**. The path in force runs
//! on alone while a new one, over the new window, fills that window with reductions of its
//! own (as many frames as it holds, less one); then the two are mixed, the new one's share
//! rising by the Hann weights of the longer window, until it is in force alone. Each path
//! covers every sample leaving the delay by itself and the mix never lies below the smaller
//! of the two, so the ceiling holds across the change
//! (`holds_the_ceiling_when_its_lookahead_changes_live_in_each_precision`); per frame, each
//! path moves the reduction as its own attack does, and the mix by at most the gap between
//! them times the longer window's largest Hann weight
//! (`steps_its_gain_by_at_most_an_attack_when_its_lookahead_grows_live`, `..._shrinks_live`).
//! A lookahead set while a change runs starts when it is over, the last one asked for: in
//! force within `4 × maximum` frames. The second path costs a second hold, release and Hann
//! window per frame while a change runs, about 5% more, and a frame of no change within 7
//! instructions of one path's (`spec/benchmarks.md`, "Two control paths for a live
//! lookahead").
//!
//! # How it limits
//!
//! Per frame, linked across channels (one gain, so that the image does not move):
//! 1. **Detect**: the true peak of each channel through its [`Detector`]'s interpolator
//!    (BS.1770's own, or a windowed one), the largest over the channels;
//! 2. **Need**: the reduction that peak needs to stay at the ceiling, plus a margin derived
//!    from the maximum lookahead, exactly zero at or below it;
//! 3. **Hold**: the largest need over the lookahead window (a sliding maximum), so that the
//!    reduction is in place before the peak leaves the delay;
//! 4. **Release**: two stages, fast after an isolated transient and slow on a sustained
//!    level, mixed by the adaptivity (its distortion and pumping in the bass are measured
//!    nowhere yet: roadmap #24 step 114);
//! 5. **Smooth**: a Hann window as long as the lookahead (the attack's shape), from sliding
//!    sums refreshed exactly once per window, so that rounding never accumulates;
//! 6. **Apply** the gain of the frame, `1 - reduction` rounded toward zero in the sample
//!    type, to each channel's delayed input; while the reduction is zero, the input is
//!    passed through unchanged, bit for bit.
//!
//! The peak estimate, the delay and the gain's product run in the sample type `S`; the
//! control path, from each need to the gain, in binary64 whatever `S`, the ceiling rounded
//! toward zero in `S`: with the margin, every sample is at or below the ceiling, with no
//! slack, in both precisions and at every finite level (`spec/engine-design.md`, decision
//! 12; the proof in `spec/measurements.md`, "The true-peak limiter").

#![cfg_attr(not(test), forbid(clippy::arithmetic_side_effects))]

use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

use crate::host::Raw;
use crate::math::limiter::{
    amplitude, fast_release, frames, interpolator, keep, margin, scale, step, turned,
};
use crate::math::{
    HannSums, TRUE_PEAK_FILTER, approached, complement, decayed, gain, hann_step, larger,
    larger_reduction, mixed, reduction_needed, share, true_peak, with_gain,
};
use crate::{Binary32, Binary64, Block, MAX_CHANNELS, Sample, SampleRate};

/// How precisely the limiter estimates the true peak: the interpolator it oversamples each
/// channel with.
///
/// `spec/engine-design.md`, decision 12, states each detector's largest under-read band by
/// band, computed from its coefficients by the limiter's measurements.
///
/// A setting fixed when the limiter is built, since its interpolator's length sets part of
/// the latency ([`Detector::delay`]).
///
/// Not exhaustive: another interpolator may be added without breaking a caller, which
/// lists them through [`Detector::ALL`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Detector {
    /// ITU-R BS.1770's own interpolator, 4 phases of 12 taps ([`TRUE_PEAK_FILTER`]): the
    /// cheapest; it rolls off near half the rate, where it reads a peak much lower.
    Standard,
    /// 4 phases of 32 taps, a Kaiser-windowed sinc (β 2.5). The default.
    Precise,
    /// 8 phases of 128 taps (β 5): the most precise near half the rate, for offline renders
    /// and mastering.
    Mastering,
}

impl Detector {
    /// Every detector, from the cheapest to the most precise: the order of their codes in
    /// the C interface (`audio_host::Instance::output`), from which every list of them
    /// (codes, names, messages, the readers of cases) is derived.
    pub const ALL: [Self; 3] = [Self::Standard, Self::Precise, Self::Mastering];

    /// Its name, as the interfaces and the cases write it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Precise => "precise",
            Self::Mastering => "mastering",
        }
    }

    /// Its phases: the oversampling factor.
    #[must_use]
    pub const fn phases(self) -> usize {
        match self {
            Self::Standard | Self::Precise => 4,
            Self::Mastering => 8,
        }
    }

    /// Its taps per phase.
    #[must_use]
    pub const fn taps(self) -> usize {
        match self {
            Self::Standard => 12,
            Self::Precise => 32,
            Self::Mastering => 128,
        }
    }

    /// Frames between a sample's arrival and the estimate of the interval it opens: the
    /// detector estimates the interval in the middle of its taps, half of them behind its
    /// input.
    #[must_use]
    pub const fn delay(self) -> usize {
        self.taps() / 2
    }

    /// Its coefficients in binary64, phase by phase, tap `k` multiplying the sample `k`
    /// frames before the newest: BS.1770's table, or the windowed interpolator computed in
    /// the math layer.
    #[must_use]
    pub fn coefficients(self) -> Vec<Binary64> {
        match self {
            Self::Standard => TRUE_PEAK_FILTER
                .iter()
                .flatten()
                .map(|tap| Binary64::new(*tap))
                .collect(),
            Self::Precise => interpolator(self.phases(), self.taps(), Binary64::new(2.5)),
            Self::Mastering => interpolator(self.phases(), self.taps(), Binary64::new(5.0)),
        }
    }
}

/// A setting of the limiter, as its errors name it.
///
/// Not exhaustive: the limiter may gain a setting without breaking a caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Setting {
    /// The ceiling, in dBTP.
    Ceiling,
    /// The lookahead, in milliseconds.
    Lookahead,
    /// The maximum lookahead, in milliseconds.
    MaximumLookahead,
    /// The release, in milliseconds.
    Release,
    /// The adaptivity of the release, from 0 to 1.
    Adaptivity,
}

impl Setting {
    /// Its name and unit, as an error says it.
    const fn words(self) -> (&'static str, &'static str) {
        match self {
            Self::Ceiling => ("ceiling", " dBTP"),
            Self::Lookahead => ("lookahead", " ms"),
            Self::MaximumLookahead => ("maximum lookahead", " ms"),
            Self::Release => ("release", " ms"),
            Self::Adaptivity => ("adaptivity", ""),
        }
    }

    /// The range it accepts, both ends included.
    #[must_use]
    pub const fn range(self) -> (Binary64, Binary64) {
        match self {
            Self::Ceiling => (Binary64::new(-20.0), Binary64::new(0.0)),
            Self::Lookahead | Self::MaximumLookahead => (Binary64::new(0.5), Binary64::new(5.0)),
            Self::Release => (Binary64::new(10.0), Binary64::new(1000.0)),
            Self::Adaptivity => (Binary64::new(0.0), Binary64::new(1.0)),
        }
    }

    /// `value`, if it is a number within its range ([`Setting::range`]): how every setting
    /// is checked, and how a host checks a maximum lookahead before it stores it.
    ///
    /// # Errors
    ///
    /// Returns [`LimiterError::OutOfRange`] for a value outside its range or not a number.
    pub fn checked(self, value: Binary64) -> Result<Binary64, LimiterError> {
        let (low, high) = self.range();
        if value >= low && value <= high {
            Ok(value)
        } else {
            Err(LimiterError::OutOfRange(self, value))
        }
    }
}

/// Why a setting of the limiter was refused.
///
/// Not exhaustive: a new setting or check may bring a new refusal without breaking a caller,
/// which shows it through its [`Display`](fmt::Display).
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum LimiterError {
    /// A value outside the range of its setting ([`Setting::range`]), or not a number.
    OutOfRange(Setting, Binary64),
    /// A lookahead longer than the maximum the limiter was built with.
    AboveMaximum {
        /// The lookahead asked for, in milliseconds.
        lookahead: Binary64,
        /// The maximum, in milliseconds.
        maximum: Binary64,
    },
    /// More channels than any engine applies ([`MAX_CHANNELS`]): the count asked.
    TooManyChannels(u16),
}

impl fmt::Display for LimiterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::OutOfRange(setting, value) => {
                let (name, unit) = setting.words();
                let (low, high) = setting.range();
                write!(
                    f,
                    "limiter {name} {}{unit} is out of range ({} to {}{unit})",
                    value.raw(),
                    low.raw(),
                    high.raw()
                )
            }
            Self::AboveMaximum { lookahead, maximum } => write!(
                f,
                "limiter lookahead {} ms is above its maximum of {} ms",
                lookahead.raw(),
                maximum.raw()
            ),
            Self::TooManyChannels(count) => {
                write!(
                    f,
                    "too many limiter channels: {count} (at most {MAX_CHANNELS})"
                )
            }
        }
    }
}

impl core::error::Error for LimiterError {}

/// The settings of the limiter that change live: each within its range, checked when built.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LimiterSettings {
    /// The ceiling, in dBTP.
    ceiling: Binary64,
    /// The lookahead, in milliseconds.
    lookahead: Binary64,
    /// The release, in milliseconds.
    release: Binary64,
    /// The adaptivity of the release, from 0 (a fixed release) to 1 (fully
    /// program-dependent).
    adaptivity: Binary64,
}

impl LimiterSettings {
    /// The defaults: a ceiling of -1 dBTP (EBU R128's maximum true-peak level), a lookahead
    /// of 1.5 ms, a release of 100 ms, and an adaptivity of 0.5.
    pub const DEFAULT: Self = Self {
        ceiling: Binary64::new(-1.0),
        lookahead: Binary64::new(1.5),
        release: Binary64::new(100.0),
        adaptivity: Binary64::new(0.5),
    };

    /// The ceiling, in dBTP.
    #[must_use]
    pub const fn ceiling(self) -> Binary64 {
        self.ceiling
    }

    /// The lookahead, in milliseconds.
    #[must_use]
    pub const fn lookahead(self) -> Binary64 {
        self.lookahead
    }

    /// The release, in milliseconds.
    #[must_use]
    pub const fn release(self) -> Binary64 {
        self.release
    }

    /// The adaptivity of the release, from 0 to 1.
    #[must_use]
    pub const fn adaptivity(self) -> Binary64 {
        self.adaptivity
    }

    /// Checks that `maximum` milliseconds are a maximum lookahead, and that the lookahead of
    /// these settings is at most that maximum: the one check of both, which the limiter makes
    /// when it is built and when its settings change, and a host before it stores either.
    ///
    /// # Errors
    ///
    /// Returns [`LimiterError::OutOfRange`] for a maximum outside its range
    /// ([`Setting::MaximumLookahead`]) or not a number, then [`LimiterError::AboveMaximum`]
    /// for a lookahead above it.
    pub fn within_maximum(self, maximum: Binary64) -> Result<(), LimiterError> {
        let maximum = Setting::MaximumLookahead.checked(maximum)?;
        if self.lookahead > maximum {
            Err(LimiterError::AboveMaximum {
                lookahead: self.lookahead,
                maximum,
            })
        } else {
            Ok(())
        }
    }

    /// These settings with the lookahead `lookahead`, in milliseconds.
    ///
    /// # Errors
    ///
    /// Returns [`LimiterError::OutOfRange`] for a lookahead outside its range or not a
    /// number.
    pub fn with_lookahead(self, lookahead: Binary64) -> Result<Self, LimiterError> {
        Ok(Self {
            lookahead: Setting::Lookahead.checked(lookahead)?,
            ..self
        })
    }

    /// Settings from their values: a ceiling in dBTP, a lookahead and a release in
    /// milliseconds, an adaptivity from 0 to 1.
    ///
    /// # Errors
    ///
    /// Returns [`LimiterError::OutOfRange`] for the first value outside its range
    /// ([`Setting::range`]) or not a number, in that order.
    pub fn new(
        ceiling: Binary64,
        lookahead: Binary64,
        release: Binary64,
        adaptivity: Binary64,
    ) -> Result<Self, LimiterError> {
        Ok(Self {
            ceiling: Setting::Ceiling.checked(ceiling)?,
            lookahead: Setting::Lookahead.checked(lookahead)?,
            release: Setting::Release.checked(release)?,
            adaptivity: Setting::Adaptivity.checked(adaptivity)?,
        })
    }
}

/// How much faster the fast release stage is than the release setting, at an adaptivity of
/// 1: a tenth of it, so that a single transient leaves no audible hole.
const FAST_RELEASE_DIVISOR: Binary64 = Binary64::new(10.0);

/// The time constant of the mode crossfade, in milliseconds: long enough not to click,
/// short enough to follow a switch at once.
const MODE_FADE: Binary64 = Binary64::new(20.0);

/// The next index of a ring of `length`, after `index`.
const fn next(index: usize, length: usize) -> usize {
    let after = index.saturating_add(1);
    if after >= length { 0 } else { after }
}

/// Writes `value` at `index` of `ring`, an index the ring's arithmetic keeps in range.
fn put<T: Copy>(ring: &mut [T], index: usize, value: T) {
    ring.get_mut(index)
        .into_iter()
        .for_each(|slot| *slot = value);
}

/// The index `count` places after `index` in a ring of `length`.
fn ahead(index: usize, count: usize, length: usize) -> usize {
    index.saturating_add(count).checked_rem(length).unwrap_or(0)
}

/// The index `back` places before `index` in a ring of `length` (`back` below `length`).
const fn behind(index: usize, back: usize, length: usize) -> usize {
    match index.checked_sub(back) {
        Some(earlier) => earlier,
        None => index.saturating_add(length).saturating_sub(back),
    }
}

/// The true-peak limiter of an output stage: built for a rate, a channel count and a
/// maximum lookahead, which fix its latency; every other setting changes live (see the
/// module's doc).
///
/// No heap after it is built: it runs on the audio thread.
///
/// It limits only inside an output stage ([`crate::OutputStage::limited`]), which replaces
/// every non-finite sample before it, since its detector needs finite ones (decision 12):
/// it has no public way to limit a block of its own.
///
/// ```compile_fail,E0624
/// use audio_core::{Binary64, Detector, LimiterSettings, SampleRate, TruePeakLimiter};
/// let rate = SampleRate::new(48_000).expect("a rate");
/// let settings = LimiterSettings::DEFAULT;
/// let mut limiter =
///     TruePeakLimiter::<Binary64>::new(rate, 1, Binary64::new(5.0), Detector::Precise, settings, true)
///         .expect("a limiter");
/// limiter.apply::<64>(&mut [[Binary64::new(f64::NAN); 64]]);
/// ```
///
/// Through a stage, it compiles, and the stage gives a finite sample:
///
/// ```
/// use audio_core::{Binary64, Detector, LimiterSettings, OutputStage, SampleRate, TruePeakLimiter};
/// let rate = SampleRate::new(48_000).expect("a rate");
/// let settings = LimiterSettings::DEFAULT;
/// let limiter =
///     TruePeakLimiter::<Binary64>::new(rate, 1, Binary64::new(5.0), Detector::Precise, settings, true)
///         .expect("a limiter");
/// let mut stage = OutputStage::<Binary64, f64>::limited(limiter);
/// let nan = |blocks: &mut [[Binary64; 64]]| blocks[0].fill(Binary64::new(f64::NAN));
/// for _ in 0..7 {
///     let _ = stage.apply(nan);
/// }
/// assert_eq!(stage.apply(nan)[0][0], Binary64::new(0.0), "the NaN replaced, then delayed");
/// ```
#[derive(Debug, Clone)]
pub struct TruePeakLimiter<S: Sample> {
    /// The rate it was built for.
    rate: SampleRate,
    /// The channels it was built for: the blocks it limits, one per channel.
    channels: u16,
    /// The maximum lookahead, in frames.
    maximum: usize,
    /// The maximum lookahead, in milliseconds.
    maximum_ms: Binary64,
    /// The lookahead asked for, in frames, from 1 to `maximum`: the window of `path` once no
    /// change is running, the next change's otherwise.
    wanted: usize,
    /// The settings in force.
    settings: LimiterSettings,
    /// The detector's taps per phase.
    taps: usize,
    /// The frames the detector looks behind its input.
    delay: usize,
    /// The interpolator, in the precision `S`, phase by phase, each phase's taps in the
    /// order of a window oldest first (its last multiplying the newest sample).
    filter: Vec<S>,
    /// The last samples of each channel, twice in a row in a ring of `2 × taps`, so that
    /// the last `taps` are one slice, oldest first.
    history: Vec<S>,
    /// The position of the newest sample in each channel's ring (and `taps` after it).
    history_at: usize,
    /// The delayed input, channel by channel, each a ring of `latency` frames.
    delayed: Vec<S>,
    /// The frame of `delayed` read and written next.
    position: usize,
    /// The reduction each of the last `maximum + 2` frames needs, a ring, which both control
    /// paths read.
    needed: Vec<Binary64>,
    /// The frame of `needed` written next.
    needed_at: usize,
    /// The frames processed.
    step: u64,
    /// The ceiling, as a linear amplitude rounded toward zero in `S`, held in binary64.
    ceiling: Binary64,
    /// The margin added to every need above the ceiling (`margin`), from the maximum
    /// lookahead.
    margin: Binary64,
    /// The share of the fast stage's reduction kept per frame.
    keep_fast: Binary64,
    /// The share of the gap to the held need the slow stage closes per frame.
    slow_attack: Binary64,
    /// The control path in force, or fading in while a lookahead changes.
    path: Path,
    /// The control path fading out while a lookahead changes, unused otherwise.
    previous: Path,
    /// Where a change of the lookahead stands, none while none runs.
    change: Option<Stage>,
    /// The share of `path` in the reduction while it fades in: the Hann weights of the
    /// longer window summed so far.
    mix: Binary64,
    /// The share of the limiter's reduction applied: 1 protected, 0 raw, between them while
    /// crossfading.
    applied: Binary64,
    /// Whether it is protecting (the target of `applied`).
    protected: bool,
    /// Whether its output is given rounded to binary32 though computed in `S`, binary64
    /// ([`Self::give_binary32`]): its ceiling is then rounded toward zero in binary32.
    given_binary32: bool,
    /// What is left of the way from `applied` to its target: it decays to exactly zero,
    /// where recomputing it from `applied` near 1 would stall on the spacing of numbers
    /// there.
    remaining: Binary64,
    /// The share of the gap to its target `applied` keeps per frame.
    keep_mode: Binary64,
}

impl<S: Sample> TruePeakLimiter<S> {
    /// A limiter at `rate` for `channels` channels, its delay sized for `maximum_lookahead`
    /// milliseconds and its `detector`, protecting with `settings` from its first frame, or
    /// raw.
    ///
    /// # Errors
    ///
    /// Returns [`LimiterError::OutOfRange`] for a maximum lookahead outside its range,
    /// [`LimiterError::AboveMaximum`] for a lookahead of `settings` above it (both by
    /// [`LimiterSettings::within_maximum`]), and [`LimiterError::TooManyChannels`] for more
    /// than [`MAX_CHANNELS`] channels.
    pub fn new(
        rate: SampleRate,
        channels: u16,
        maximum_lookahead: Binary64,
        detector: Detector,
        settings: LimiterSettings,
        protected: bool,
    ) -> Result<Self, LimiterError> {
        settings.within_maximum(maximum_lookahead)?;
        if channels > MAX_CHANNELS {
            return Err(LimiterError::TooManyChannels(channels));
        }
        let count = channels;
        let channels = usize::from(channels);
        let maximum_ms = maximum_lookahead;
        let maximum = frames(maximum_ms, rate);
        let ring = maximum.saturating_add(2);
        let (taps, delay) = (detector.taps(), detector.delay());
        let latency = maximum.saturating_add(delay);
        // Each phase reversed: the window it multiplies is oldest first.
        let coefficients = detector.coefficients();
        let filter = coefficients
            .chunks_exact(taps)
            .flat_map(|phase| phase.iter().rev())
            .map(|tap| S::from_binary64(*tap))
            .collect();
        let applied = if protected {
            Binary64::ONE
        } else {
            Binary64::ZERO
        };
        let wanted = frames(settings.lookahead, rate).min(maximum);
        let needed = vec![Binary64::ZERO; ring];
        let mut path = Path::new(maximum);
        path.start(
            wanted,
            &needed,
            0,
            maximum,
            0,
            (Binary64::ZERO, Binary64::ZERO),
        );
        let mut limiter = Self {
            rate,
            channels: count,
            maximum,
            maximum_ms,
            wanted,
            settings,
            taps,
            delay,
            filter,
            history: vec![S::ZERO; taps.saturating_mul(2).saturating_mul(channels)],
            history_at: 0,
            delayed: vec![S::ZERO; latency.saturating_mul(channels)],
            position: 0,
            needed,
            needed_at: 0,
            step: 0,
            ceiling: Binary64::ZERO,
            margin: margin(maximum),
            keep_fast: Binary64::ZERO,
            slow_attack: Binary64::ZERO,
            path,
            previous: Path::new(maximum),
            change: None,
            mix: Binary64::ZERO,
            applied,
            protected,
            given_binary32: false,
            remaining: Binary64::ZERO,
            keep_mode: keep(MODE_FADE, rate),
        };
        limiter.set(settings)?;
        Ok(limiter)
    }

    /// Its latency, in frames: the maximum lookahead plus its detector's delay
    /// ([`Detector::delay`]), fixed when it was built.
    #[must_use]
    pub const fn latency(&self) -> usize {
        self.maximum.saturating_add(self.delay)
    }

    /// The channels it was built for, which it keeps: a renderer or an engine refuses a plan
    /// of another count (`CoreError::StageChannels`).
    #[must_use]
    pub const fn channels(&self) -> u16 {
        self.channels
    }

    /// The settings in force: the lookahead asked for last, which takes hold through the
    /// change it starts (see the module's doc).
    #[must_use]
    pub const fn settings(&self) -> LimiterSettings {
        self.settings
    }

    /// Whether it is protecting (or crossfading toward it).
    #[must_use]
    pub const fn protected(&self) -> bool {
        self.protected
    }

    /// Changes the settings, live: the ceiling, the release and its adaptivity act from the
    /// next frame; a new lookahead starts a control path over its window, faded in once that
    /// window holds only its own reductions, or after the change running, if one is (see the
    /// module's doc). A lowered ceiling holds from the maximum lookahead on, in frames after
    /// the change, within the latency ([`Self::latency`]): the samples leaving the delay
    /// until then keep the needs computed for the old ceiling, at or below which they stay.
    ///
    /// # Errors
    ///
    /// Returns [`LimiterError::AboveMaximum`] for a lookahead above the maximum the limiter
    /// was built with; the settings in force are then unchanged.
    pub fn set(&mut self, settings: LimiterSettings) -> Result<(), LimiterError> {
        settings.within_maximum(self.maximum_ms)?;
        self.settings = settings;
        self.ceiling = self.ceiling_of(settings.ceiling);
        let fast = fast_release(settings.release, settings.adaptivity, FAST_RELEASE_DIVISOR);
        self.keep_fast = keep(fast, self.rate);
        self.slow_attack = step(keep(settings.release, self.rate));
        self.wanted = frames(settings.lookahead, self.rate).min(self.maximum);
        if self.change.is_none() {
            self.begin_change();
        }
        Ok(())
    }

    /// Starts the change toward the lookahead asked for, unless it is in force: the path in
    /// force becomes the one fading out, and the other starts over the new window from the
    /// needs already seen and the release stages where they are, with no reduction released
    /// yet. It fades in once its window holds only reductions of its own (`Stage::Filling`).
    fn begin_change(&mut self) {
        if self.wanted == self.path.lookahead {
            return;
        }
        core::mem::swap(&mut self.path, &mut self.previous);
        let stages = (self.previous.fast, self.previous.slow);
        self.path.start(
            self.wanted,
            &self.needed,
            self.needed_at,
            self.maximum,
            self.step,
            stages,
        );
        self.mix = Binary64::ZERO;
        self.change = Some(Stage::Filling(self.wanted.saturating_sub(1)));
    }

    /// The ceiling of `decibels` as a linear amplitude rounded toward zero in the format its
    /// output is given in, held in binary64: an output at or below it in binary64 is at or
    /// below it once rounded to that format too, since rounding to the nearest keeps a value
    /// at or below a number of the format at or below it. In `S`, or in binary32 when a
    /// binary64 output is given as binary32 ([`Self::give_binary32`]), where the nearest
    /// binary32 of a binary64 output at the binary64 ceiling can lie above it.
    fn ceiling_of(&self, decibels: Binary64) -> Binary64 {
        let linear = amplitude(decibels);
        if self.given_binary32 {
            Binary32::toward_zero(linear).to_binary64()
        } else {
            S::toward_zero(linear).to_binary64()
        }
    }

    /// Gives its output rounded to binary32 from now on: its ceiling rounded toward zero in
    /// binary32 ([`Self::ceiling_of`]). Called once, by an output stage that gives binary32
    /// when it is built with this limiter (`OutputStage::limited`), before any block; a
    /// binary32 limiter, whose ceiling is already a binary32 number, is
    /// unchanged.
    pub(crate) fn give_binary32(&mut self) {
        self.given_binary32 = true;
        self.ceiling = self.ceiling_of(self.settings.ceiling);
    }

    /// Starts the crossfade toward protected (`true`) or raw (`false`).
    pub fn set_protected(&mut self, protected: bool) {
        self.protected = protected;
        self.remaining = if protected {
            complement(self.applied)
        } else {
            self.applied
        };
    }

    /// Limits the next block of every channel (`blocks[channel]`, as many as it was built
    /// for), in place: each sample becomes the input of [`Self::latency`] frames before,
    /// reduced as the peaks ahead of it need. Frame by frame over the channels, so that it
    /// holds no block of its own, whatever the engine block `N`.
    ///
    /// Crate-private: only an output stage calls it, after replacing every non-finite
    /// sample (decision 12), and at an engine block it checked
    /// ([`crate::OutputStage::apply`]).
    pub(crate) fn apply<const N: usize>(&mut self, blocks: &mut [Block<S, N>]) {
        let taps = self.taps;
        let latency = self.latency();
        for frame in 0..N {
            self.history_at = next(self.history_at, taps);
            let at = self.history_at;
            // The peak of the frame, the largest over the channels.
            let mut peak = S::ZERO;
            for (history, block) in self
                .history
                .chunks_exact_mut(taps.saturating_mul(2))
                .zip(blocks.iter())
            {
                let sample = block.get(frame).copied().unwrap_or(S::ZERO);
                put(history, at, sample);
                put(history, at.saturating_add(taps), sample);
                // The last `taps` samples, oldest first: those after the newest's first copy.
                let window = history
                    .get(at.saturating_add(1)..=at.saturating_add(taps))
                    .unwrap_or_default();
                peak = larger(peak, true_peak(window, &self.filter, taps));
            }
            // One gain for the frame, in `S`, or none: the input passed through.
            let gain = self.gain(peak);
            // Each channel's delay line, from the same position.
            let position = self.position;
            for (ring, block) in self
                .delayed
                .chunks_exact_mut(latency)
                .zip(blocks.iter_mut())
            {
                // Every block spans the engine block: the frame is always found.
                for sample in Option::into_iter(block.get_mut(frame)) {
                    let out = ring.get(position).copied().unwrap_or(S::ZERO);
                    put(ring, position, *sample);
                    *sample = gain.map_or(out, |gain| with_gain(out, gain));
                }
            }
            self.position = next(position, latency);
        }
    }

    /// The gain to apply to the sample leaving the delay at the frame whose peak, over the
    /// channels, is `peak`: none while the reduction is zero (the input passed through, bit
    /// for bit), else `1 - reduction` rounded toward zero in `S`. The control path from the
    /// need to the gain runs in binary64 whatever `S` (decision 12).
    fn gain(&mut self, peak: S) -> Option<S> {
        put(
            &mut self.needed,
            self.needed_at,
            reduction_needed(peak.to_binary64(), self.ceiling, self.margin),
        );
        self.needed_at = next(self.needed_at, self.needed.len());
        self.step = self.step.saturating_add(1);
        let smoothed = self.smoothed();
        self.crossfade();
        let reduction = share(smoothed, self.applied);
        (reduction != Binary64::ZERO).then(|| S::toward_zero(gain(reduction)))
    }

    /// The smoothed reduction of the frame: the path in force's, or, while the lookahead
    /// changes, the old path's until the new one's window holds only its own reductions,
    /// then the two mixed, the new one's share rising by the longer window's Hann weights,
    /// until it is the new one's alone (see the module's doc).
    fn smoothed(&mut self) -> Binary64 {
        let newest = self.advanced(false);
        let Some(stage) = self.change else {
            return newest;
        };
        self.changing(newest, stage)
    }

    /// One frame of the path in force, or of the one fading out (`previous`), from the need
    /// entering its window: `maximum - lookahead` frames behind the newest.
    #[expect(
        clippy::inline_always,
        reason = "measured (spec/benchmarks.md, \"Two control paths for a live lookahead\"): inlined whole, a path costs a frame of no change within 7 instructions of one path's, 30 more left to the compiler; for a larger module"
    )]
    #[inline(always)]
    fn advanced(&mut self, previous: bool) -> Binary64 {
        let path = if previous {
            &mut self.previous
        } else {
            &mut self.path
        };
        let entering = need_before(
            &self.needed,
            self.needed_at,
            self.maximum.saturating_sub(path.lookahead),
        );
        path.advance(entering, self.step, self.keep_fast, self.slow_attack)
    }

    /// The smoothed reduction of a frame while the lookahead changes, from `newest`, the new
    /// path's, at `stage` (see [`Self::smoothed`]). Out of line, measured (`spec/benchmarks.md`, "Two
    /// control paths for a live lookahead"): a frame of no change then costs within 7
    /// instructions of one path's, a frame of a change about 5% more.
    #[inline(never)]
    fn changing(&mut self, newest: Binary64, stage: Stage) -> Binary64 {
        let (oldest, left) = match stage {
            Stage::Filling(left) => {
                let oldest = self.advanced(true);
                if let Some(left) = left.checked_sub(1) {
                    self.change = Some(Stage::Filling(left));
                    return oldest;
                }
                // Filled: the fade's first frame, the longer window's weights but its last
                // still to mix.
                let longer = self.path.lookahead.max(self.previous.lookahead);
                (oldest, longer.saturating_sub(1))
            }
            Stage::Fading(left) => (self.advanced(true), left),
        };
        let longer = core::cmp::max_by_key(&self.path, &self.previous, |path| path.lookahead);
        let Some(rest) = left.checked_sub(1) else {
            // The last weight: the new path's share is one, it alone from now on.
            self.change = None;
            self.begin_change();
            return newest;
        };
        // The weight `k = lookahead - left` of the window: its phasor's turn `k`.
        let cos = longer
            .phasor
            .get(longer.lookahead.saturating_sub(left.saturating_add(1)))
            .map_or(Binary64::ONE, |turn| turn.0);
        self.mix = hann_step(self.mix, cos, longer.scale);
        self.change = Some(Stage::Fading(rest));
        mixed(oldest, newest, self.mix)
    }

    /// Moves the share applied one frame toward its target, what is left of the way
    /// decaying. Toward protected it is exactly 1 once what is left no longer changes it;
    /// toward raw it arrives at exactly 0 once the share left no longer changes any gain
    /// (`1 - share` is 1), so that raw gives its input bit for bit again.
    fn crossfade(&mut self) {
        self.remaining = decayed(self.remaining, self.keep_mode);
        self.applied = if self.protected {
            complement(self.remaining)
        } else {
            if complement(self.remaining) == Binary64::ONE {
                self.remaining = Binary64::ZERO;
            }
            self.remaining
        };
    }
}

/// Where a running change of the lookahead stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// The new path fills its window, the old one alone in force for this many frames more.
    Filling(usize),
    /// The new path fades in, the two mixed for this many frames more, its share rising by
    /// the longer window's Hann weights, before it is in force alone.
    Fading(usize),
}

/// The need recorded `back` frames before the newest in the ring `needed`, whose next frame
/// is `needed_at` (`back` below the ring's length).
fn need_before(needed: &[Binary64], needed_at: usize, back: usize) -> Binary64 {
    let length = needed.len();
    let newest = behind(needed_at, 1, length);
    needed
        .get(behind(newest, back, length))
        .copied()
        .unwrap_or(Binary64::ZERO)
}

/// A control path of the limiter, over a window of `lookahead` frames: the largest need of
/// its window, held (a sliding maximum), released in two stages and smoothed by a Hann
/// window as long as it. The limiter keeps two, so that a new lookahead fades in.
#[derive(Debug, Clone)]
struct Path {
    /// The lookahead, in frames, from 1 to the maximum (0 only before it starts).
    lookahead: usize,
    /// The sliding maximum of the needs: their step and value, a ring of
    /// `maximum + 2` (a monotonic queue, decreasing from its front).
    queue: Vec<(u64, Binary64)>,
    /// The front of `queue`.
    front: usize,
    /// How many entries `queue` holds.
    queued: usize,
    /// The fast release stage's reduction.
    fast: Binary64,
    /// The slow stage's reduction.
    slow: Binary64,
    /// The released reductions of the last `maximum` frames, a ring.
    released: Vec<Binary64>,
    /// The frame of `released` written next.
    released_at: usize,
    /// How many of the last `lookahead` released reductions are not zero.
    nonzero: usize,
    /// The Hann sums of the last `lookahead` released reductions.
    sums: HannSums<Binary64>,
    /// Frames since `sums` were computed exactly.
    since_exact: usize,
    /// The phasor's positions for the window (`cos`, `sin` of `2π (k + 1) / (lookahead + 1)`).
    phasor: Vec<(Binary64, Binary64)>,
    /// The phasor's step per frame.
    turn: (Binary64, Binary64),
    /// One over `lookahead + 1`.
    scale: Binary64,
}

impl Path {
    /// A path for a maximum lookahead of `maximum` frames, not started.
    fn new(maximum: usize) -> Self {
        Self {
            lookahead: 0,
            queue: vec![(0, Binary64::ZERO); maximum.saturating_add(2)],
            front: 0,
            queued: 0,
            fast: Binary64::ZERO,
            slow: Binary64::ZERO,
            released: vec![Binary64::ZERO; maximum],
            released_at: 0,
            nonzero: 0,
            sums: HannSums::ZERO,
            since_exact: 0,
            phasor: vec![(Binary64::ZERO, Binary64::ZERO); maximum],
            turn: (Binary64::ZERO, Binary64::ZERO),
            scale: Binary64::ZERO,
        }
    }

    /// Starts the path over a window of `lookahead` frames at the step `step`: its phasor,
    /// its sliding maximum from the needs of the ring `needed` (whose next frame is
    /// `needed_at`, `maximum + 2` long) its window holds, its release stages at `stages`
    /// (fast, slow), and no reduction released yet.
    fn start(
        &mut self,
        lookahead: usize,
        needed: &[Binary64],
        needed_at: usize,
        maximum: usize,
        step: u64,
        stages: (Binary64, Binary64),
    ) {
        self.lookahead = lookahead;
        // Only the window's first `lookahead` turns are read (`HannSums::of`, `turn`).
        for (steps, turn) in self.phasor.iter_mut().take(lookahead).enumerate() {
            *turn = turned(steps.saturating_add(1), lookahead);
        }
        self.turn = self
            .phasor
            .first()
            .copied()
            .unwrap_or((Binary64::ZERO, Binary64::ZERO));
        self.scale = scale(lookahead);
        // The sliding maximum, from the needs of its window, oldest first: the window ends
        // `maximum - lookahead` frames behind the newest need.
        self.queued = 0;
        let delay = maximum.saturating_sub(lookahead);
        for back in (0..lookahead.saturating_add(2)).rev() {
            let at = step.saturating_sub(u64::try_from(back).unwrap_or(u64::MAX));
            self.enqueue(
                at,
                need_before(needed, needed_at, delay.saturating_add(back)),
            );
        }
        (self.fast, self.slow) = stages;
        self.released.fill(Binary64::ZERO);
        self.released_at = 0;
        (self.nonzero, self.sums, self.since_exact) = (0, HannSums::ZERO, 0);
    }

    /// One frame of the path at the step `step`: the need `entering` its window held,
    /// released (the fast stage keeping `keep_fast` per frame, the slow one closing
    /// `slow_attack` of its gap), and smoothed; the smoothed reduction.
    #[expect(
        clippy::inline_always,
        reason = "measured (spec/benchmarks.md, \"Two control paths for a live lookahead\"): inlined whole, a path costs a frame of no change within 7 instructions of one path's, 30 more left to the compiler; for a larger module"
    )]
    #[inline(always)]
    fn advance(
        &mut self,
        entering: Binary64,
        step: u64,
        keep_fast: Binary64,
        slow_attack: Binary64,
    ) -> Binary64 {
        self.enqueue(step, entering);
        let held = self.held();
        self.fast = larger_reduction(held, decayed(self.fast, keep_fast));
        // The slow stage follows the held need through a one-pole of the release's time,
        // up and down: it rises only under a sustained need.
        self.slow = approached(self.slow, held, slow_attack);
        self.smooth(larger_reduction(self.fast, self.slow))
    }

    /// Computes the Hann sums of the window exactly, and counts its non-zero reductions: once
    /// per window, while it holds some ([`Self::smooth`]). Out of line, so that the per-frame
    /// work of a path inlines whole (`spec/benchmarks.md`, "Two control paths for a live
    /// lookahead").
    #[inline(never)]
    fn exact_sums(&mut self) {
        let window = (0..self.lookahead).map(|back| self.released_before(back));
        let nonzero = window
            .clone()
            .filter(|value| *value != Binary64::ZERO)
            .count();
        let sums = HannSums::of(window, &self.phasor);
        (self.nonzero, self.sums, self.since_exact) = (nonzero, sums, 0);
    }

    /// Drops what left the window of the sliding maximum, then adds the need `value` of step
    /// `step`.
    ///
    /// Dropping first keeps at most the window's `lookahead + 1` older needs, so the new one
    /// always has a free entry of the ring (`maximum + 2` long), with the lookahead at its
    /// maximum too.
    #[expect(
        clippy::inline_always,
        reason = "measured (spec/benchmarks.md, \"Two control paths for a live lookahead\"): inlined whole, a path costs a frame of no change within 7 instructions of one path's, 30 more left to the compiler; for a larger module"
    )]
    #[inline(always)]
    fn enqueue(&mut self, step: u64, value: Binary64) {
        let length = self.queue.len();
        let span = u64::try_from(self.lookahead.saturating_add(2)).unwrap_or(u64::MAX);
        while let Some(left) = self.queued.checked_sub(1) {
            let expired = self
                .queue
                .get(self.front)
                .is_some_and(|(entered, _)| step.saturating_sub(*entered) >= span);
            if !expired {
                break;
            }
            self.front = next(self.front, length);
            self.queued = left;
        }
        while let Some(last) = self.queued.checked_sub(1) {
            let back = ahead(self.front, last, length);
            if self.queue.get(back).is_some_and(|(_, kept)| *kept > value) {
                break;
            }
            self.queued = last;
        }
        put(
            &mut self.queue,
            ahead(self.front, self.queued, length),
            (step, value),
        );
        self.queued = self.queued.saturating_add(1);
    }

    /// The largest need of the window.
    fn held(&self) -> Binary64 {
        self.queue
            .get(self.front)
            .map_or(Binary64::ZERO, |(_, value)| *value)
    }

    /// The released reduction recorded `back` frames before the newest.
    fn released_before(&self, back: usize) -> Binary64 {
        let length = self.released.len();
        let newest = behind(self.released_at, 1, length);
        self.released
            .get(behind(newest, back, length))
            .copied()
            .unwrap_or(Binary64::ZERO)
    }

    /// Adds `released` to the Hann window and returns the smoothed reduction.
    fn smooth(&mut self, released: Binary64) -> Binary64 {
        let oldest = self.released_before(self.lookahead.saturating_sub(1));
        put(&mut self.released, self.released_at, released);
        self.released_at = next(self.released_at, self.released.len());
        if released != Binary64::ZERO {
            self.nonzero = self.nonzero.saturating_add(1);
        }
        if oldest != Binary64::ZERO {
            self.nonzero = self.nonzero.saturating_sub(1);
        }
        if self.nonzero == 0 {
            self.sums = HannSums::ZERO;
            return Binary64::ZERO;
        }
        self.since_exact = self.since_exact.saturating_add(1);
        if self.since_exact >= self.lookahead {
            self.exact_sums();
        } else {
            self.sums = self.sums.advanced(released, oldest, self.turn);
        }
        larger_reduction(Binary64::ZERO, self.sums.smoothed(self.scale))
    }
}

#[cfg(test)]
#[path = "limiter_tests.rs"]
mod tests;
