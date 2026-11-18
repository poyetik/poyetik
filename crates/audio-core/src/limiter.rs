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
//! ([`OutputStage::set_protected`](crate::OutputStage::set_protected), a switchable stage's
//! only), by a crossfade of the reduction (in raw, its output
//! is its input delayed by the latency, bit for bit); and the lookahead itself up to the
//! maximum (the output stays delayed by the maximum; only the window the limiter looks
//! ahead through changes), by a **second control path started warm**. The needs of the last
//! `2 × maximum + 2` frames are kept, so that the new path starts over the new window as if
//! it had always run, from them and from what the path in force released in its own life,
//! since its own change (a growth set before that life spans the new window waits until it
//! does, since what the path held before it another window released: its change then
//! starts at most `new - old` frames after the set, in frames of the windows, or after its
//! rebuild if that ends later). It is
//! rebuilt first, over `⌈(2 × maximum + 1) / 32⌉` frames, or the wait if longer, at most
//! [`REBUILT_STEPS`] steps of each part per frame (turns of its phasor, frames of its hold,
//! its rebuilt reductions), the old path alone in force and unchanged meanwhile, so that
//! every sample is covered by it as before a change; the change
//! then starts after the rebuild's last frame exactly as a rebuild made at once on that
//! frame would start it, when no other setting changes during the rebuild
//! (`rebuilds_a_new_path_over_frames_exactly_as_at_once_on_its_last_frame`; a release or an
//! adaptivity set during it is under no test, `spec/limitations.md`). Its
//! last frame does what needs that frame's reduction in force (the rise's budget, a shorter
//! window's scale); the new window's Hann sums, accumulated over the reductions it
//! rebuilds, are completed and swapped in on the change's first frame, as a path does once
//! per window:
//! - **Growing**, it rebuilds the reductions it would have released over its window (each
//!   the larger of its own hold then and of the old path's released reduction), so that it
//!   covers from its first frame; the reduction is the larger of the old path's and of a
//!   rise from the reduction in force toward the new path, by at least the new window's
//!   attack of the largest reduction held, and faster for a need the old path has not seen,
//!   by the attack of the frames left before the first that needs it; once the rise reaches
//!   the new path, the larger of that path and of a linear descent from there.
//! - **Shrinking**, the old path runs on frozen (no new need enters it) for its window and
//!   one frame, beside the new one, whose release stages and rebuilt reductions carry
//!   nothing from beyond its window, the reductions scaled by one share so that their
//!   weighted mean is at most the reduction in force; the reduction is the larger of the
//!   two, then the same descent.
//!
//! Each path, warm or frozen, covers every sample leaving the delay by itself, and the rise
//! and the descent only ever join one through an exact maximum, so the ceiling holds
//! (`holds_the_ceiling_when_its_lookahead_changes_live_in_each_precision`, and frame by
//! frame against the needs in the tests below). Per frame, the gain moves by at most `N`
//! times `max(hp(new), hp(D))` growing, `N` the larger of the burst's need and the largest
//! reduction the control path holds when the change starts (the reduction in force, or a
//! release stage above it: the descent falls by the longer window's largest weight of the
//! reduction it began from, which a release from a louder passage can hold above the burst's
//! need), `hp(L)` the largest Hann weight of `L`
//! frames and `D` the frames from the change to the first frame that needs the burst (no
//! limiter matching the old one before the change can ramp it slower with a Hann attack),
//! and times `hp(new)` shrinking, plus the control path's rounding
//! (`steps_within_the_attack_of_each_depth_and_covers_growing_live_*`,
//! `steps_within_the_new_attack_and_covers_shrinking_live_*`; the growth's bound rests on an
//! inequality proven over every window up to the longest (audio-testkit's limiter
//! measurements,
//! `proves_a_growths_rise_stays_above_the_old_windows_attack_up_to_the_longest_window`), and is
//! proven for a single burst, simulated for several, `spec/limitations.md`). A lookahead set while
//! a change or a rebuild runs starts its rebuild when that change is over, the last one asked for:
//! a shrink lasts the old window and one frame, then at most `⌈1 / hp(old)⌉ + 1` frames of descent;
//! a growth about the new window and two frames after the largest need it holds last rose, so
//! longer while louder needs keep arriving, with no deadline. A change's work runs out of line: a
//! frame of no change costs as one path's, the set that starts a change does a fixed amount of
//! work, each frame of the rebuild at most [`REBUILT_STEPS`] steps of each of its parts, and its
//! last frame the rise's budget too (`spec/benchmarks.md`, "Hann sums accumulated over each
//! window").
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
use core::cmp::Ordering;
use core::fmt;

use crate::host::Raw;
use crate::math::limiter::{
    amplitude, fast_release, frames, hann_peak, interpolator, keep, margin, scale, step, turned,
};
use crate::math::{
    HannSums, TRUE_PEAK_FILTER, approached, complement, decayed, gain, kept, larger,
    larger_reduction, lowered, quotient, raised, reduction_needed, share, smaller, stepped,
    true_peak, with_gain,
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

/// The steps of each part of a new path's rebuild per frame ([`TruePeakLimiter::rebuilt`]):
/// turns of its phasor (a cosine and a sine each), frames of its hold (one need held, and
/// for a frame rebuilt one reduction recorded or one step of the fast stage), and terms of
/// the shorter window's rebuilt reductions. A rebuild runs at most `2 × maximum + 1` frames
/// of the hold (`lookahead + 2` and `lookahead - 1` growing, `lookahead + 2` and
/// `old - lookahead` shrinking) and at most `maximum` of each other part, so
/// `⌈(2 × maximum + 1) / REBUILT_STEPS⌉` frames complete it
/// (`rebuilds_a_new_path_by_thirty_two_steps_of_each_part_per_frame` counts each part). The
/// rise's budget is not among them: the rebuild's last frame folds it whole, a product and
/// a maximum per need the old window has not seen, at most `maximum - 1`
/// ([`TruePeakLimiter::finish_rebuild`]).
const REBUILT_STEPS: usize = 32;

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
    /// The reduction each of the last `2 × maximum + 2` frames needs, a ring, which both
    /// control paths read: a path's window ends `maximum - lookahead` frames behind the
    /// newest and holds `lookahead + 2` of them; a new path rebuilt runs its hold from its
    /// window `run` frames before the rebuild's last frame (`run` below the maximum), as far
    /// back as `maximum + run + 1` needs, at most `2 × maximum` (see
    /// [`TruePeakLimiter::rebuilt`]).
    needed: Vec<Binary64>,
    /// The frame of `needed` written next.
    needed_at: usize,
    /// The largest Hann weight of a window of each length from 1 to `maximum` frames
    /// ([`hann_peak`]), computed when it is built: per unit of a need, the most a window's
    /// attack moves the reduction in a frame, which a change of the lookahead ramps by.
    peaks: Vec<Binary64>,
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
    /// The control path in force, or the new one while a lookahead changes.
    path: Path,
    /// The control path in force before a lookahead changes, while the change needs it,
    /// unused otherwise.
    previous: Path,
    /// Where a change of the lookahead stands, none while none runs.
    change: Option<Stage>,
    /// The frames a new path is rebuilt over before its change starts
    /// ([`Stage::Rebuilding`]), from the maximum: `⌈(2 × maximum + 1) / REBUILT_STEPS⌉`.
    rebuild_frames: usize,
    /// Where the rebuild of a new path stands, read only while one runs.
    rebuild: Rebuild,
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
        let ring = maximum.saturating_mul(2).saturating_add(2);
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
        let needed = vec![Binary64::ZERO; ring];
        let peaks: Vec<Binary64> = (1..=maximum).map(hann_peak).collect();
        let mut path = Path::new(maximum);
        path.prepare(wanted, peak_of(&peaks, wanted));
        let _: usize = path.turn_phasor(0, wanted);
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
            peaks,
            step: 0,
            ceiling: Binary64::ZERO,
            margin: margin(maximum),
            keep_fast: Binary64::ZERO,
            slow_attack: Binary64::ZERO,
            path,
            previous: Path::new(maximum),
            change: None,
            rebuild_frames: maximum
                .saturating_mul(2)
                .saturating_add(1)
                .div_ceil(REBUILT_STEPS),
            rebuild: Rebuild::starting(false, 0, 0),
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

    /// Its mode, fixed when it was built: whether its stage may switch it to raw.
    #[must_use]
    pub const fn mode(&self) -> LimiterMode {
        self.mode
    }

    /// Changes the settings, live: the ceiling, the release and its adaptivity act from the
    /// next frame; a new lookahead starts a control path over its window, warm, which takes
    /// over from the one in force, or after the change running, if one is (see the module's
    /// doc). A lowered ceiling holds from the maximum lookahead on, in frames after
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

    /// Starts the rebuild of a new control path toward the lookahead asked for, unless it is
    /// in force, after the last frame computed: the path in force stays in force, unchanged,
    /// while the other is rebuilt over the new window for `rebuild_frames` frames, or until
    /// the life of the path in force spans the new window ([`Path::taken`]) if that comes
    /// later ([`Self::rebuilt`]), after which the change starts as [`Self::finish_rebuild`]
    /// says ([`Stage::Rebuilding`]).
    fn begin_change(&mut self) {
        let lookahead = self.wanted;
        let longer = match lookahead.cmp(&self.path.lookahead) {
            Ordering::Equal => return,
            Ordering::Greater => true,
            Ordering::Less => false,
        };
        let old = self.path.lookahead;
        let run = if longer {
            lookahead.saturating_sub(1)
        } else {
            old.saturating_sub(lookahead)
        };
        self.previous
            .prepare(lookahead, peak_of(&self.peaks, lookahead));
        // The hold's frames, from the window it starts from to the last frame: those of its
        // window `run` frames before it, `lookahead + 2`, then the `run` rebuilt.
        self.rebuild =
            Rebuild::starting(longer, run, run.saturating_add(lookahead).saturating_add(2));
        // Its last frame no earlier than `lookahead - 2` frames into the life of the path in
        // force, that path's takeover `old - 2` frames into it: a growth set sooner waits,
        // so that every reduction it rebuilds is one that path released (none before its
        // life, which another window released); a shrink, whose window lies within that
        // life, never does.
        let wait = self.path.taken.map_or(0, |taken| {
            let last = taken.saturating_add(u64::try_from(lookahead).unwrap_or(u64::MAX));
            let now = self
                .step
                .saturating_add(u64::try_from(old).unwrap_or(u64::MAX));
            usize::try_from(last.saturating_sub(now)).unwrap_or(usize::MAX)
        });
        self.change = Some(Stage::Rebuilding(self.rebuild_frames.max(wait)));
    }

    /// One frame of the rebuild, `left` frames before its last (the frame just computed is
    /// `left` frames before it), each part at most `quota` steps and never past the frame
    /// just computed:
    /// - turns of the new path's phasor, from the last position of its window down, so that
    ///   each reduction below finds the turn of its position computed;
    /// - frames of its hold, each `ago` frames before the rebuild's last frame, its step and
    ///   its entering need those of that frame; of a frame rebuilt (`ago` below `run`),
    ///   growing, the larger of its hold and of the old path's released reduction then is
    ///   recorded at the position it holds on the change's first frame, `ago + 1`, and kept
    ///   if the largest; shrinking, the fast stage follows its hold, from the old path's
    ///   released reduction on the frame before the first rebuilt, with the release in force;
    /// - shrinking, the reductions of the window's last `lookahead - 1` frames, each the
    ///   smaller of the old path's then and `old - lookahead` frames before, recorded at the
    ///   position it holds on the change's first frame.
    fn rebuilt(&mut self, left: usize, quota: usize) {
        let new = &mut self.previous;
        let state = &mut self.rebuild;
        state.turns = new.turn_phasor(state.turns, quota);
        let lookahead = new.lookahead;
        let delay = self.maximum.saturating_sub(lookahead);
        let last = self
            .step
            .saturating_add(u64::try_from(left).unwrap_or(u64::MAX));
        // Each loop's count fixed before its first step, so that it ends whatever the quota.
        for _ in 0..state.unheld.saturating_sub(left).min(quota) {
            state.unheld = state.unheld.saturating_sub(1);
            let ago = state.unheld;
            let back = ago.saturating_sub(left);
            new.enqueue(
                last.saturating_sub(u64::try_from(ago).unwrap_or(u64::MAX)),
                need_before(&self.needed, self.needed_at, delay.saturating_add(back)),
            );
            if ago == state.run {
                state.fast = self.path.released_before(back);
            }
            if ago < state.run {
                let hold = new.held();
                if state.longer {
                    let released = larger_reduction(hold, self.path.released_before(back));
                    new.record(released, ago.saturating_add(1));
                    state.cap = larger(state.cap, released);
                } else {
                    state.fast = larger_reduction(hold, decayed(state.fast, self.keep_fast));
                }
            }
        }
        if !state.longer {
            let run = state.run;
            let total = lookahead.saturating_sub(1);
            for _ in 0..total
                .saturating_sub(state.terms)
                .saturating_sub(left)
                .min(quota)
            {
                state.terms = state.terms.saturating_add(1);
                let ago = total.saturating_sub(state.terms);
                let back = ago.saturating_sub(left);
                let ours = smaller(
                    self.path.released_before(back),
                    self.path.released_before(back.saturating_add(run)),
                );
                new.record(ours, ago.saturating_add(1));
            }
        }
    }

    /// The last frame of the rebuild, the frame just computed: what is left of it, then the
    /// change, from the reduction in force, after this frame. The path rebuilt becomes the
    /// one in force and the other the old one (see the module's doc), its sums replaced on
    /// the change's first frame by those it accumulated, completed then.
    ///
    /// - **A longer window** holds the reductions it would have released over its last
    ///   `lookahead - 1` frames (each the larger of its own hold then and of the old path's
    ///   released reduction then, every one of them within that path's life, which the
    ///   rebuild waited for, [`Path::taken`], so that the window covers what its hold covers
    ///   and a sustained reduction carries over); its release stages are the old path's. The
    ///   change rises from the reduction in force ([`Stage::Rising`]), by a budget that
    ///   ramps every need the old path has not seen, `D` frames before the first frame that
    ///   needs it, by at most the largest Hann weight of `D` frames of its excess over that
    ///   reduction, and at least the new window's attack of the largest reduction held: a
    ///   multiplication and a maximum per need, at most `maximum` of them, read in place.
    /// - **A shorter window** carries nothing from beyond itself: its fast stage is the one
    ///   [`Self::rebuilt`] followed; its slow stage is the old one's; its rebuilt reductions,
    ///   all scaled by one share, so that their weighted mean on the change's first frame is
    ///   at most the reduction in force (the share is one when it already is). The old path
    ///   runs on frozen, the needs entering it zero ([`Stage::Frozen`]).
    fn finish_rebuild(&mut self) {
        self.rebuilt(0, usize::MAX);
        let before = self.path.smoothed();
        core::mem::swap(&mut self.path, &mut self.previous);
        let (old, new, state) = (&self.previous, &mut self.path, &self.rebuild);
        let lookahead = new.lookahead;
        new.swap_in = 0;
        new.taken = Some(self.step);
        if state.longer {
            (new.fast, new.slow) = (old.fast, old.slow);
            // The needs the old path has not seen, each `D` frames from the change's first
            // frame to the first that needs it, `D` from the old window to the new and below
            // the maximum (a need of the next frame enters the new path's hold then, whose
            // attack of the cap ramps it at least as fast), each paired with `hp(D)`. The need
            // of `D` is `maximum - 1 - D` before the newest (the old path took in the one
            // `maximum - old` before it on this frame), so these are the `farthest - old + 1`
            // before the `maximum - 1 - farthest` newest, read in place.
            let farthest = lookahead.min(self.maximum.saturating_sub(1));
            let spans = farthest.saturating_add(1).saturating_sub(old.lookahead);
            let skipped = self.maximum.saturating_sub(1).saturating_sub(farthest);
            let length = self.needed.len();
            let newest = behind(self.needed_at, 1, length);
            let start = behind(
                newest,
                skipped.saturating_add(spans).saturating_sub(1),
                length,
            );
            let (wrapped, oldest) = self.needed.split_at(start.min(length));
            let peaks = self
                .peaks
                .get(old.lookahead.saturating_sub(1)..)
                .unwrap_or_default();
            // Oldest first, as `D` grows from the old window, `hp(D)` from `hp(old)` on.
            let mut budget = Binary64::ZERO;
            for (need, peak) in oldest.iter().chain(wrapped).take(spans).zip(peaks) {
                budget = larger(budget, stepped(lowered(*need, before), *peak));
            }
            // Zero below the least value kept, as every value a stage stores (decision 11):
            // the mean in force can lie below it at a release tail's end.
            self.change = Some(Stage::Rising {
                rising: kept(before),
                cap: larger(larger(old.fast, old.slow), state.cap),
                budget,
            });
        } else {
            (new.fast, new.slow) = (state.fast, old.slow);
            // The weighted sum of the reductions rebuilt on the change's first frame, at their
            // positions there (the newest's still to come), and the most it may be: the
            // reduction in force times the weights of those positions.
            let rebuilt = new.next.smoothed(new.scale);
            let newest = new.phasor.first().map_or(Binary64::ZERO, |(cos, _)| {
                share(complement(*cos), new.scale)
            });
            let room = share(complement(newest), before);
            // Zero below the least value kept, as every value a stage stores (decision 11).
            let by = kept(smaller(Binary64::ONE, quotient(room, rebuilt)));
            new.next = new.next.scaled(by);
            self.change = Some(Stage::Frozen {
                left: old.lookahead,
                scaling: lookahead.saturating_sub(1),
                by,
            });
        }
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

    /// The reduction of the frame whose peak, over the channels, is `peak`: the smoothed one
    /// times the share of it the crossfade between raw and protected applies, zero below the
    /// least value the control path keeps ([`stepped`], `math::LEAST_KEPT`): no reduction, the
    /// input passed through, on every host (decision 11).
    fn reduction(&mut self, peak: S) -> Binary64 {
        put(
            &mut self.needed,
            self.needed_at,
            reduction_needed(peak.to_binary64(), self.ceiling, self.margin),
        );
        self.needed_at = next(self.needed_at, self.needed.len());
        self.step = self.step.saturating_add(1);
        let smoothed = self.smoothed();
        self.crossfade();
        stepped(smoothed, self.applied)
    }

    /// The smoothed reduction of the frame: the path in force's, or, while the lookahead
    /// changes, what the change gives (see [`Self::changing`]).
    fn smoothed(&mut self) -> Binary64 {
        let newest = self.advanced(false);
        let Some(stage) = self.change else {
            return newest;
        };
        self.changing(newest, stage)
    }

    /// One frame of the path in force, or of the old one (`previous`), from the need
    /// entering its window: `maximum - lookahead` frames behind the newest.
    #[expect(
        clippy::inline_always,
        reason = "as `Path::advance`, the frame of a path it runs (spec/benchmarks.md, \"A path's frame inlined whole, measured again\")"
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

    /// The smoothed reduction of a frame while the lookahead changes, from `newest`, the
    /// reduction of the path in force (the old one while the new one is rebuilt, its last
    /// frame included; the new one after), at `stage`:
    /// - [`Stage::Rebuilding`]: `newest` itself, the old path alone in force, while
    ///   [`Self::rebuilt`] runs at most [`REBUILT_STEPS`] steps of each part, nothing while a
    ///   growth waits for the life of the path in force to span its window
    ///   ([`Self::begin_change`]); on its last frame [`Self::finish_rebuild`] swaps the paths
    ///   and starts the change;
    /// - [`Stage::Rising`]: the larger of the old path's and of the rise, which climbs by the
    ///   larger of its budget and the new window's attack of the largest reduction held,
    ///   never above the new path; once it reaches it, the descent;
    /// - [`Stage::Frozen`]: the larger of the new path's and of the old one's, frozen, until
    ///   the old one has released every need it saw; then the descent;
    /// - [`Stage::Descending`]: the larger of the new path's and of the descent, a linear
    ///   fall from the reduction where it began, until the new path is above it, then the
    ///   new path alone, and the next change asked for, if any.
    ///
    /// Out of line, measured (`spec/benchmarks.md`, "A warm-started path for a live
    /// lookahead"): a frame of no change then costs as one path's.
    #[inline(never)]
    fn changing(&mut self, newest: Binary64, stage: Stage) -> Binary64 {
        match stage {
            Stage::Rebuilding(left) => {
                let left = left.saturating_sub(1);
                self.change = Some(Stage::Rebuilding(left));
                if left == 0 {
                    self.finish_rebuild();
                } else {
                    self.rebuilt(left, REBUILT_STEPS);
                }
                newest
            }
            Stage::Rising {
                rising,
                cap,
                budget,
            } => {
                let oldest = self.advanced(true);
                let cap = larger(cap, self.path.held());
                let rise = larger(budget, stepped(cap, self.path.peak));
                // A rise of zero, of a cap below the least value kept over the weight, whose
                // gain is 1, reaches the new path at once rather than never.
                let rising = if rise == Binary64::ZERO {
                    newest
                } else {
                    smaller(newest, raised(rising, rise))
                };
                let reduction = larger(oldest, rising);
                // Each value stored zero below the least value kept, as the stage's frames
                // compute from means that can lie below it.
                self.change = Some(if rising < newest {
                    Stage::Rising {
                        rising: kept(rising),
                        cap,
                        budget,
                    }
                } else {
                    Stage::Descending {
                        level: kept(reduction),
                        fall: stepped(reduction, self.path.peak),
                    }
                });
                reduction
            }
            Stage::Frozen { left, scaling, by } => {
                let oldest = self.previous.advance(
                    Binary64::ZERO,
                    self.step,
                    self.keep_fast,
                    self.slow_attack,
                );
                let reduction = larger(newest, oldest);
                // The rebuilt reduction leaving the new window on the next frame scaled as its
                // sums were, one per frame, from the oldest.
                if scaling > 0 {
                    self.path.scale_leaving(by);
                }
                let scaling = scaling.saturating_sub(1);
                self.change = Some(left.checked_sub(1).map_or_else(
                    || Stage::Descending {
                        level: kept(reduction),
                        fall: stepped(reduction, self.previous.peak),
                    },
                    |left| Stage::Frozen { left, scaling, by },
                ));
                reduction
            }
            Stage::Descending { level, fall } => {
                // A fall of zero, of a level below the least value kept over the weight, whose
                // gain is 1, ends the descent at once rather than never.
                let level = if fall == Binary64::ZERO {
                    Binary64::ZERO
                } else {
                    lowered(level, fall)
                };
                if level > newest {
                    self.change = Some(Stage::Descending { level, fall });
                    return level;
                }
                self.change = None;
                self.begin_change();
                newest
            }
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

/// Where a running change of the lookahead stands (see [`TruePeakLimiter::changing`]).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Stage {
    /// The new path rebuilt, the old one alone in force, for this many frames more, the last
    /// one included: the rebuild's frames, or the wait of a growth for the life of the path
    /// in force if longer ([`TruePeakLimiter::begin_change`], [`TruePeakLimiter::rebuilt`],
    /// [`TruePeakLimiter::finish_rebuild`]).
    Rebuilding(usize),
    /// A longer window: the rise toward the new path, with the old one in force under it.
    Rising {
        /// The rise: from the reduction in force when the change started, never above the
        /// new path.
        rising: Binary64,
        /// The largest reduction held so far: the new path's holds, its rebuilt reductions
        /// and the old path's release stages when the change started.
        cap: Binary64,
        /// The rise per frame the needs the old path had not seen ask for.
        budget: Binary64,
    },
    /// A shorter window: the old path frozen for `left` frames more, the new one in force
    /// with it.
    Frozen {
        /// The frames left frozen.
        left: usize,
        /// The new path's rebuilt reductions still to scale by `by`, one per frame as each
        /// is about to leave its window.
        scaling: usize,
        /// The share the new path's rebuilt reductions and their sums are scaled by.
        by: Binary64,
    },
    /// The descent from the reduction where the new path took over, by `fall` per frame (the
    /// longer window's largest Hann weight of that reduction), the new path in force under
    /// it.
    Descending {
        /// The descent's reduction.
        level: Binary64,
        /// Its fall per frame.
        fall: Binary64,
    },
}

/// Where the rebuild of a new control path stands ([`TruePeakLimiter::rebuilt`]).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Rebuild {
    /// Whether the new window is longer than the old one.
    longer: bool,
    /// The frames whose holds it follows to the end, the last ones of its hold's frames:
    /// `lookahead - 1` growing (their reductions rebuilt), `old - lookahead` shrinking (its
    /// fast stage followed).
    run: usize,
    /// Its hold's frames still to run, the next one this many frames, less one, before the
    /// rebuild's last frame.
    unheld: usize,
    /// The turns of its phasor computed.
    turns: usize,
    /// Shrinking, the rebuilt reductions recorded so far.
    terms: usize,
    /// Growing, the largest reduction it rebuilt so far.
    cap: Binary64,
    /// Shrinking, its fast stage so far.
    fast: Binary64,
}

impl Rebuild {
    /// A rebuild not begun: `longer` or not, following `run` holds to the end, `unheld`
    /// frames of its hold to run, nothing computed yet.
    const fn starting(longer: bool, run: usize, unheld: usize) -> Self {
        Self {
            longer,
            run,
            unheld,
            turns: 0,
            terms: 0,
            cap: Binary64::ZERO,
            fast: Binary64::ZERO,
        }
    }
}

/// The largest Hann weight of a window of `length` frames, from 1 to the maximum, in the
/// table `peaks` of the limiter.
fn peak_of(peaks: &[Binary64], length: usize) -> Binary64 {
    peaks
        .get(length.saturating_sub(1))
        .copied()
        .unwrap_or(Binary64::ZERO)
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
/// window as long as it. The limiter keeps two, so that a new lookahead takes over without
/// a step.
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
    /// The step of the frame it took over on, the last of its rebuild, none for the path in
    /// force since the limiter was built (whose ring is zero before its first frame, as a
    /// path that always ran in silence would hold): its current life starts with the first
    /// of the `lookahead - 1` reductions it rebuilt then, `lookahead - 2` frames before that
    /// one, and those of `released` before it were released in an earlier life, over another
    /// window and perhaps long before, which no frame of this one released. A growth waits
    /// until the life spans its new window ([`TruePeakLimiter::begin_change`]), so that no
    /// rebuild reads them.
    taken: Option<u64>,
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
    /// The largest Hann weight of its window ([`hann_peak`]): per unit of a need, the most
    /// its attack moves the reduction in a frame.
    peak: Binary64,
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
            taken: None,
            nonzero: 0,
            sums: HannSums::ZERO,
            next: HannSums::ZERO,
            swap_in: 0,
            phasor: vec![(Binary64::ZERO, Binary64::ZERO); maximum],
            turn: (Binary64::ZERO, Binary64::ZERO),
            scale: Binary64::ZERO,
            peak: Binary64::ZERO,
        }
    }

    /// Prepares the path for a window of `lookahead` frames, whose largest Hann weight is
    /// `peak`: its sliding maximum empty, its count of non-zero reductions and the sums it
    /// accumulates zero, and a zero recorded, the reduction that leaves its window on the
    /// change's first frame, before the `lookahead - 1` it rebuilds ([`Self::record`]); its
    /// release stages, its phasor ([`Self::turn_phasor`] computes it) and the reductions it
    /// released before left as they are, none of which its window then holds.
    fn prepare(&mut self, lookahead: usize, peak: Binary64) {
        self.lookahead = lookahead;
        self.scale = scale(lookahead);
        self.peak = peak;
        self.queued = 0;
        (self.nonzero, self.next) = (0, HannSums::ZERO);
        self.remember(Binary64::ZERO);
    }

    /// Computes up to `count` more turns of the phasor of its window, from the last
    /// position of the window down, `done` of them computed already, and returns how many
    /// are computed then (at most its lookahead: only the window's first `lookahead` turns
    /// are read).
    fn turn_phasor(&mut self, done: usize, count: usize) -> usize {
        let lookahead = self.lookahead;
        let until = done.saturating_add(count).min(lookahead);
        for (steps, turn) in self
            .phasor
            .iter_mut()
            .enumerate()
            .take(lookahead.saturating_sub(done))
            .skip(lookahead.saturating_sub(until))
        {
            *turn = turned(steps.saturating_add(1), lookahead);
        }
        self.turn = self
            .phasor
            .first()
            .copied()
            .unwrap_or((Binary64::ZERO, Binary64::ZERO));
        until
    }

    /// Records `released` as a reduction its window holds when the change starts, after
    /// those recorded, at `position` then (0 the newest), counted and added to the sums it
    /// accumulates ([`HannSums::added`]).
    fn record(&mut self, released: Binary64, position: usize) {
        self.remember(released);
        if released != Binary64::ZERO {
            self.nonzero = self.nonzero.saturating_add(1);
        }
        let turn = self
            .phasor
            .get(position)
            .copied()
            .unwrap_or((Binary64::ZERO, Binary64::ZERO));
        self.next = self.next.added(released, turn);
    }

    /// Scales by `by` the reduction that leaves its window on the next frame, a rebuilt
    /// one, as its sums were scaled, zero below the least value kept ([`stepped`]), as every
    /// reduction it releases is; its count of non-zero reductions follows a reduction scaled
    /// to zero, so that it stays exact on every host.
    fn scale_leaving(&mut self, by: Binary64) {
        let length = self.released.len();
        let at = behind(
            behind(self.released_at, 1, length),
            self.lookahead.saturating_sub(1),
            length,
        );
        let leaving = self.released.get(at).copied().unwrap_or(Binary64::ZERO);
        let scaled = stepped(leaving, by);
        if leaving != Binary64::ZERO && scaled == Binary64::ZERO {
            self.nonzero = self.nonzero.saturating_sub(1);
        }
        put(&mut self.released, at, scaled);
    }

    /// Records `released` as the reduction it released on the frame after the newest
    /// recorded, without smoothing it.
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
