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
//! once): the ceiling, the release and its adaptivity ([`TruePeakLimiter::set`]), from the
//! next frame, which the window held and the Hann smoothing or the release carry to the
//! gain; the mode ([`OutputStage::set_protected`](crate::OutputStage::set_protected), a
//! switchable stage's only), by a crossfade of the reduction (in raw, its output is its
//! input delayed by the latency, bit for bit).
//!
//! **The lookahead** (up to the maximum: the output stays delayed by the maximum, only the
//! window the limiter looks ahead through changes) is held **pending** while the limiter
//! limits, and **takes effect at its next rest**, the last one set by then (any number of
//! sets acting as the last one alone, one set back to the lookahead in force dropping the
//! change). Its rest is the first frame from which its whole control state is that of a
//! limiter at rest **whatever its lookahead**: no window of any lookahead up to the maximum
//! holds a need once none of the last `maximum + 2` frames is above zero, and from then on,
//! under one release, every release stage of every such limiter fed the same signal and the
//! same commands decays by a share of itself per frame, so that it and the released
//! reductions any window reads are zero `math::limiter::rest_frames` frames (about 71 release time
//! constants) after the later of the last frame with a need and the frame before the
//! release and the adaptivity in force took effect, a release or an adaptivity set meanwhile
//! counting again. There every need of the maximum window, every hold, both release stages,
//! the released reductions any window reads, the Hann sums and the
//! reduction are zero in it and in a limiter always at the new lookahead, so that it takes
//! the new window's constants and the phase of its exact sums and **renders from then on
//! bit for bit what a limiter always at the new lookahead renders**, given the same signal
//! and the same release, adaptivity and ceiling (`spec/measurements.md`, "The true-peak
//! limiter", step 6; `renders_from_each_rest_as_limiters_always_at_its_lookaheads`), with no
//! click: the reduction is zero on both sides of the switch; until then it is pending
//! ([`TruePeakLimiter::pending_lookahead`]). Under continuous limiting it
//! waits; a limiter that never limited takes it on its next frame, and one that limits
//! rarely, as a safety limiter does, soon after its last limited frame. The rest comes a few
//! percent later than the limiter's own (`spec/limitations.md`). The frame of the switch
//! computes the new window's phasor, at
//! most `maximum` turns (`spec/benchmarks.md`, "A live lookahead taken at the limiter's
//! rest").
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
//!    sums slid frame by frame and replaced once per window by its exact sums, accumulated
//!    over that window as its reductions are released, so that rounding never accumulates
//!    and no frame sums a window at once;
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
    amplitude, fast_release, frames, interpolator, keep, margin, rest_frames, scale, step, turned,
};
use crate::math::{
    HannSums, TRUE_PEAK_FILTER, approached, complement, decayed, gain, larger, larger_reduction,
    reduction_needed, stepped, true_peak, with_gain,
};
use crate::{Binary32, Binary64, Block, LimiterMode, MAX_CHANNELS, Sample, SampleRate};

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
/// use audio_core::{Binary64, Detector, LimiterMode, LimiterSettings, SampleRate, TruePeakLimiter};
/// let rate = SampleRate::new(48_000).expect("a rate");
/// let (settings, mode) = (LimiterSettings::DEFAULT, LimiterMode::Protected);
/// let mut limiter =
///     TruePeakLimiter::<Binary64>::new(rate, 1, Binary64::new(5.0), Detector::Precise, settings, mode)
///         .expect("a limiter");
/// limiter.apply::<64>(&mut [[Binary64::new(f64::NAN); 64]]);
/// ```
///
/// Through a stage, it compiles, and the stage gives a finite sample:
///
/// ```
/// use audio_core::{
///     Binary64, Detector, LimiterMode, LimiterSettings, OutputStage, SampleRate, TruePeakLimiter,
/// };
/// let rate = SampleRate::new(48_000).expect("a rate");
/// let (settings, mode) = (LimiterSettings::DEFAULT, LimiterMode::Protected);
/// let limiter =
///     TruePeakLimiter::<Binary64>::new(rate, 1, Binary64::new(5.0), Detector::Precise, settings, mode)
///         .expect("a limiter");
/// let mut stage = OutputStage::<Binary64, f64>::limited(limiter);
/// let nan = |blocks: &mut [[Binary64; 64]]| blocks[0].fill(Binary64::new(f64::NAN));
/// for _ in 0..7 {
///     let _ = stage.apply(nan);
/// }
/// assert_eq!(stage.apply(nan)[0][0], Binary64::new(0.0), "the NaN replaced, then delayed");
/// ```
///
/// It is not `Clone`: its state is a stream's, which never leaves the stage limiting it
/// ([`OutputStage::limiter`](crate::OutputStage::limiter) shows the copy refused).
#[derive(Debug)]
pub struct TruePeakLimiter<S: Sample> {
    /// The rate it was built for.
    rate: SampleRate,
    /// The channels it was built for: the blocks it limits, one per channel.
    channels: u16,
    /// The maximum lookahead, in frames.
    maximum: usize,
    /// The maximum lookahead, in milliseconds.
    maximum_ms: Binary64,
    /// The lookahead asked for, in frames, from 1 to `maximum`: the window of `path` once it
    /// has taken effect, pending until the limiter's rest otherwise ([`Self::at_rest`]).
    wanted: usize,
    /// The settings asked for last.
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
    /// The reduction each of the last `maximum` frames needs, a ring: the path's window
    /// ends `maximum - lookahead` frames behind the newest.
    needed: Vec<Binary64>,
    /// The frame of `needed` written next.
    needed_at: usize,
    /// The frame of `needed` whose need enters the path's window on the next frame, its
    /// `maximum - lookahead`-th before the newest then: advanced with `needed_at`, and set
    /// again when a lookahead is taken.
    entering: usize,
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
    /// The control path, over the window of the lookahead in force.
    path: Path,
    /// The step of the newest need that is not zero, 0 before any: `maximum + 2` frames
    /// after it, no window of any lookahead holds a need.
    loud: u64,
    /// The step of the frame before the release and the adaptivity in force took effect (0
    /// when built): from the frame after it on, every release stage decays at their pace
    /// once no window holds a need ([`Self::at_rest`]).
    released: u64,
    /// The frames of a rest under the release and the adaptivity in force: after the later
    /// of `loud` and `released`, every state of a limiter at any lookahead up to the maximum
    /// is zero ([`rest_frames`], [`Self::at_rest`]).
    decaying: u64,
    /// The share of the limiter's reduction applied: 1 protected, 0 raw, between them while
    /// crossfading.
    applied: Binary64,
    /// Its mode, fixed when it is built: a protected one is never switched to raw.
    mode: LimiterMode,
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
    /// milliseconds and its `detector`, limiting with `settings` in `mode`: protecting from its
    /// first frame, and never switched to raw ([`LimiterMode::Protected`]), or raw first and
    /// switched both ways by its stage ([`LimiterMode::Switchable`]).
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
        mode: LimiterMode,
    ) -> Result<Self, LimiterError> {
        settings.within_maximum(maximum_lookahead)?;
        if channels > MAX_CHANNELS {
            return Err(LimiterError::TooManyChannels(channels));
        }
        let count = channels;
        let channels = usize::from(channels);
        let maximum_ms = maximum_lookahead;
        let maximum = frames(maximum_ms, rate);
        let (taps, delay) = (detector.taps(), detector.delay());
        let latency = maximum.saturating_add(delay);
        // Each phase reversed: the window it multiplies is oldest first.
        let coefficients = detector.coefficients();
        let filter = coefficients
            .chunks_exact(taps)
            .flat_map(|phase| phase.iter().rev())
            .map(|tap| S::from_binary64(*tap))
            .collect();
        let protected = mode == LimiterMode::Protected;
        let applied = if protected {
            Binary64::ONE
        } else {
            Binary64::ZERO
        };
        let wanted = frames(settings.lookahead, rate).min(maximum);
        let mut path = Path::new(maximum);
        path.start(wanted, 0);
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
            needed: vec![Binary64::ZERO; maximum],
            needed_at: 0,
            entering: behind(0, maximum.saturating_sub(wanted), maximum),
            step: 0,
            ceiling: Binary64::ZERO,
            margin: margin(maximum),
            keep_fast: Binary64::ZERO,
            slow_attack: Binary64::ZERO,
            path,
            loud: 0,
            released: 0,
            decaying: 0,
            applied,
            mode,
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

    /// The settings asked for last: their ceiling, release and adaptivity in force from the
    /// frame after they were set, their lookahead pending until the limiter's next rest if it
    /// is not the one in force (see the module's doc).
    #[must_use]
    pub const fn settings(&self) -> LimiterSettings {
        self.settings
    }

    /// The lookahead asked for last, in milliseconds, while it is pending: set while the
    /// limiter limits, it takes effect at its next rest (see the module's doc); none once it
    /// is in force, or when it counts the frames of the lookahead in force. What a host shows
    /// as "pending" until it is applied; no heap.
    #[must_use]
    pub const fn pending_lookahead(&self) -> Option<Binary64> {
        if self.wanted == self.path.lookahead {
            None
        } else {
            Some(self.settings.lookahead)
        }
    }

    /// Whether it is protecting (or crossfading toward it).
    #[must_use]
    pub const fn protected(&self) -> bool {
        self.protected
    }

    /// Its mode, fixed when it was built: whether its stage may switch it to raw.
    #[must_use]
    pub const fn mode(&self) -> LimiterMode {
        self.mode
    }

    /// Changes the settings, live: the ceiling, the release and its adaptivity act from the
    /// next frame; a new lookahead is held pending and takes effect at the limiter's next
    /// rest (see the module's doc). It stores the settings, and does no other work: any
    /// number of sets between two frames act as the last one. A lowered ceiling holds from
    /// the maximum lookahead on, in frames after the change, within the latency
    /// ([`Self::latency`]): the samples leaving the delay until then keep the needs computed
    /// for the old ceiling, at or below which they stay.
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
        let keep_fast = keep(fast, self.rate);
        let slow_attack = step(keep(settings.release, self.rate));
        if keep_fast != self.keep_fast || slow_attack != self.slow_attack {
            (self.keep_fast, self.slow_attack) = (keep_fast, slow_attack);
            self.released = self.step;
            // The slow stage's share, at most the fast stage's (`spec/measurements.md`, "The
            // true-peak limiter", step 6).
            self.decaying = rest_frames(slow_attack);
        }
        self.wanted = frames(settings.lookahead, self.rate).min(self.maximum);
        Ok(())
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

    /// Starts the crossfade toward protected (`true`) or raw (`false`), whatever its mode.
    /// Crate-private: its stage switches it, refusing a protected one told raw
    /// ([`crate::OutputStage::set_protected`]).
    pub(crate) fn set_protected(&mut self, protected: bool) {
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
        let reduction = self.reduction(peak);
        (reduction != Binary64::ZERO).then(|| S::toward_zero(gain(reduction)))
    }

    /// The reduction of the frame whose peak, over the channels, is `peak`: the path's
    /// smoothed one times the share of it the crossfade between raw and protected applies,
    /// zero below the least value the control path keeps ([`stepped`], `math::LEAST_KEPT`):
    /// no reduction, the input passed through, on every host (decision 11). The step of its
    /// need recorded if that need is not zero.
    fn reduction(&mut self, peak: S) -> Binary64 {
        let need = reduction_needed(peak.to_binary64(), self.ceiling, self.margin);
        put(&mut self.needed, self.needed_at, need);
        self.needed_at = next(self.needed_at, self.needed.len());
        self.step = self.step.saturating_add(1);
        let smoothed = self.smoothed();
        if need > Binary64::ZERO {
            self.loud = self.step;
        }
        self.crossfade();
        stepped(smoothed, self.applied)
    }

    /// The smoothed reduction of the frame: a lookahead pending taken first if the frame
    /// before left the limiter at rest ([`Self::at_rest`]), then the path's frame, from the
    /// need entering its window.
    fn smoothed(&mut self) -> Binary64 {
        if self.wanted != self.path.lookahead {
            self.at_rest();
        }
        let entering = self
            .needed
            .get(self.entering)
            .copied()
            .unwrap_or(Binary64::ZERO);
        self.entering = next(self.entering, self.needed.len());
        self.path
            .advance(entering, self.step, self.keep_fast, self.slow_attack)
    }

    /// Puts the lookahead asked for in force if the frame before this one, `before`, left
    /// the limiter at rest: it never needed a reduction, or [`rest_frames`] frames have passed
    /// since the later of its newest need and the frame before the release in force took
    /// effect, a count whose slack over the decay of every release stage covers the maximum
    /// window twice, the frames a hold and a window's released reductions outlast a need.
    /// Then, in it as in a limiter at any lookahead fed the same signal and commands, every
    /// need of the maximum window, every hold, both release stages, the released reductions
    /// any window reads, the Hann sums and the reduction are zero: the path
    /// takes the new window's constants and the phase of a path that always ran at it
    /// ([`Path::start`]), and the need entering it, so that from this frame on it renders
    /// what a limiter always at the new lookahead renders, bit for bit (`spec/measurements.md`,
    /// "The true-peak limiter", step 6).
    ///
    /// Out of line: only a frame with a lookahead pending calls it.
    #[inline(never)]
    fn at_rest(&mut self) {
        let before = self.step.saturating_sub(1);
        if self.loud == 0 || before >= self.loud.max(self.released).saturating_add(self.decaying) {
            self.path.start(self.wanted, before);
            self.entering = behind(
                self.needed_at,
                self.maximum.saturating_sub(self.wanted).saturating_add(1),
                self.needed.len(),
            );
        }
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

/// The control path of the limiter, over a window of `lookahead` frames: the largest need
/// of its window, held (a sliding maximum), released in two stages and smoothed by a Hann
/// window as long as it.
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
    /// The Hann sums of the last `lookahead` released reductions: slid frame by frame
    /// ([`HannSums::advanced`]), and replaced by `next` once per window.
    sums: HannSums<Binary64>,
    /// The exact Hann sums of the window `swap_in` frames from now, accumulated as its
    /// reductions are released, each at the position it holds then ([`HannSums::added`]).
    next: HannSums<Binary64>,
    /// The frames until `next` replaces `sums`, from `lookahead - 1` to 0: the position the
    /// reduction released now holds in that window.
    swap_in: usize,
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
            next: HannSums::ZERO,
            swap_in: 0,
            phasor: vec![(Binary64::ZERO, Binary64::ZERO); maximum],
            turn: (Binary64::ZERO, Binary64::ZERO),
            scale: Binary64::ZERO,
        }
    }

    /// Puts in force a window of `lookahead` frames after the frame `step` (0 before the
    /// first): its scale, its phasor, and the phase of its exact sums of a path always at
    /// that window, which swaps them in on the frames `s` with `(s - 1) mod lookahead = 0`
    /// (`swap_in` is `-step mod lookahead`). Every other value is left as it is: zero, at the
    /// limiter's rest, as in a path that always ran at the window (when built, every value is
    /// zero), but the released reductions no window reads and the ring positions, which
    /// change nothing it computes ([`TruePeakLimiter::at_rest`]).
    fn start(&mut self, lookahead: usize, step: u64) {
        self.lookahead = lookahead;
        self.scale = scale(lookahead);
        for (steps, turn) in self.phasor.iter_mut().enumerate().take(lookahead) {
            *turn = turned(steps.saturating_add(1), lookahead);
        }
        self.turn = self
            .phasor
            .first()
            .copied()
            .unwrap_or((Binary64::ZERO, Binary64::ZERO));
        let length = u64::try_from(lookahead).unwrap_or(u64::MAX);
        let phase = step
            .checked_rem(length)
            .and_then(|phase| usize::try_from(phase).ok())
            .unwrap_or(0);
        self.swap_in = lookahead
            .saturating_sub(phase)
            .checked_rem(lookahead)
            .unwrap_or(0);
    }

    /// Records `released` as the reduction it released on the frame after the newest
    /// recorded, without smoothing it, nor counting it among the non-zero reductions, which
    /// [`Self::smooth`] does.
    fn remember(&mut self, released: Binary64) {
        put(&mut self.released, self.released_at, released);
        self.released_at = next(self.released_at, self.released.len());
    }

    /// The smoothed reduction of its newest frame, from its sums: what [`Self::smooth`]
    /// returned last (zero once started, before a frame).
    fn smoothed(&self) -> Binary64 {
        larger_reduction(Binary64::ZERO, self.sums.smoothed(self.scale))
    }

    /// One frame of the path at the step `step`: the need `entering` its window held,
    /// released (the fast stage keeping `keep_fast` per frame, the slow one closing
    /// `slow_attack` of its gap), and smoothed; the smoothed reduction.
    #[expect(
        clippy::inline_always,
        reason = "measured, spec/benchmarks.md, \"A path's frame inlined whole, measured again\": a frame of no change costs fewer instructions inlined whole than left to the compiler, for a larger module"
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

    /// Drops what left the window of the sliding maximum, then adds the need `value` of step
    /// `step`.
    ///
    /// Dropping first keeps at most the window's `lookahead + 1` older needs, so the new one
    /// always has a free entry of the ring (`maximum + 2` long), with the lookahead at its
    /// maximum too.
    #[expect(
        clippy::inline_always,
        reason = "as `Path::advance`, which it starts (spec/benchmarks.md, \"A path's frame inlined whole, measured again\")"
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

    /// Adds `released` to the Hann window and returns the smoothed reduction: the sums slid
    /// by it, or, once per window, replaced by the exact sums accumulated over that window
    /// (`next`), which it completes; so that no frame sums a window at once, and the sums are
    /// never slid more than `lookahead - 1` times since they were exact.
    fn smooth(&mut self, released: Binary64) -> Binary64 {
        let oldest = self.released_before(self.lookahead.saturating_sub(1));
        self.remember(released);
        if released != Binary64::ZERO {
            self.nonzero = self.nonzero.saturating_add(1);
        }
        if oldest != Binary64::ZERO {
            self.nonzero = self.nonzero.saturating_sub(1);
        }
        let position = self.swap_in;
        let swapping = position == 0;
        self.swap_in = if swapping {
            self.lookahead.saturating_sub(1)
        } else {
            position.saturating_sub(1)
        };
        if self.nonzero == 0 {
            // Every reduction of the window is zero: so are its sums, and those of the next
            // window accumulated so far, whose reductions it holds.
            (self.sums, self.next) = (HannSums::ZERO, HannSums::ZERO);
            return Binary64::ZERO;
        }
        let turn = self
            .phasor
            .get(position)
            .copied()
            .unwrap_or((Binary64::ZERO, Binary64::ZERO));
        let next = self.next.added(released, turn);
        if swapping {
            (self.sums, self.next) = (next, HannSums::ZERO);
        } else {
            self.next = next;
            self.sums = self.sums.advanced(released, oldest, self.turn);
        }
        self.smoothed()
    }
}

#[cfg(test)]
#[path = "limiter_tests.rs"]
mod tests;
