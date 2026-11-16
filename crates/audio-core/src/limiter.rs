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
//! Every other setting changes **live**, without a click: the ceiling, the release and its
//! adaptivity ([`TruePeakLimiter::set`]), the lookahead itself up to the maximum (the
//! output stays delayed by the maximum; only the window the limiter looks ahead through
//! shortens), and the mode ([`TruePeakLimiter::set_protected`]), by a crossfade of the
//! reduction: in raw, its output is its input delayed by the latency, bit for bit.
//!
//! # How it limits
//!
//! Per frame, linked across channels (one gain, so that the image does not move):
//! 1. **Detect**: the true peak of each channel through its [`Detector`]'s interpolator
//!    (BS.1770's own, or a windowed one), the largest over the channels;
//! 2. **Need**: the reduction that peak needs to stay at the ceiling, exactly zero at or
//!    below it;
//! 3. **Hold**: the largest need over the lookahead window (a sliding maximum), so that the
//!    reduction is in place before the peak leaves the delay;
//! 4. **Release**: two stages, fast after an isolated transient and slow on a sustained
//!    level, mixed by the adaptivity (its distortion and pumping in the bass are measured
//!    nowhere yet: roadmap #24 step 114);
//! 5. **Smooth**: a Hann window as long as the lookahead (the attack's shape), from sliding
//!    sums refreshed exactly once per window, so that rounding never accumulates;
//! 6. **Apply** the reduction to the delayed input; while it is zero, the input is passed
//!    through unchanged, bit for bit.

#![cfg_attr(not(test), forbid(clippy::arithmetic_side_effects))]

use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

use crate::host::Raw;
use crate::math::limiter::{
    amplitude, fast_release, frames, interpolator, keep, scale, step, turned,
};
use crate::math::{
    HannSums, TRUE_PEAK_FILTER, approached, complement, decayed, larger_reduction, reduced,
    reduction_needed, share, true_peak,
};
use crate::{Binary64, Block, Sample, SampleRate};

/// How precisely the limiter estimates the true peak: the interpolator it oversamples each
/// channel with.
///
/// `spec/engine-design.md`, decision 12, states each detector's largest under-read band by
/// band, computed from its coefficients by the limiter's measurements.
///
/// A setting fixed when the limiter is built, since its interpolator's length sets part of
/// the latency ([`Detector::delay`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
            Self::Precise => interpolator(4, 32, Binary64::new(2.5)),
            Self::Mastering => interpolator(8, 128, Binary64::new(5.0)),
        }
    }
}

/// A setting of the limiter, as its errors name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

/// Why a setting of the limiter was refused.
#[derive(Debug, Clone, Copy, PartialEq)]
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
        }
    }
}

/// `value` if it is a number within the range of `setting`.
fn checked(setting: Setting, value: Binary64) -> Result<Binary64, LimiterError> {
    let (low, high) = setting.range();
    if value >= low && value <= high {
        Ok(value)
    } else {
        Err(LimiterError::OutOfRange(setting, value))
    }
}

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

    /// The lookahead, in milliseconds.
    #[must_use]
    pub const fn lookahead(self) -> Binary64 {
        self.lookahead
    }

    /// These settings with the lookahead `lookahead`, in milliseconds.
    ///
    /// # Errors
    ///
    /// Returns [`LimiterError::OutOfRange`] for a lookahead outside its range or not a
    /// number.
    pub fn with_lookahead(self, lookahead: Binary64) -> Result<Self, LimiterError> {
        Ok(Self {
            lookahead: checked(Setting::Lookahead, lookahead)?,
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
            ceiling: checked(Setting::Ceiling, ceiling)?,
            lookahead: checked(Setting::Lookahead, lookahead)?,
            release: checked(Setting::Release, release)?,
            adaptivity: checked(Setting::Adaptivity, adaptivity)?,
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
#[derive(Debug, Clone)]
pub struct TruePeakLimiter<S: Sample> {
    /// The rate it was built for.
    rate: SampleRate,
    /// The maximum lookahead, in frames.
    maximum: usize,
    /// The maximum lookahead, in milliseconds.
    maximum_ms: Binary64,
    /// The lookahead, in frames, from 1 to `maximum` (0 only while it is built).
    lookahead: usize,
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
    /// The reduction each of the last `maximum + 2` frames needs, a ring.
    needed: Vec<S>,
    /// The frame of `needed` written next.
    needed_at: usize,
    /// The sliding maximum of the needs: their step and value, a ring of
    /// `maximum + 2` (a monotonic queue, decreasing from its front).
    queue: Vec<(u64, S)>,
    /// The front of `queue`.
    front: usize,
    /// How many entries `queue` holds.
    queued: usize,
    /// The frames processed.
    step: u64,
    /// The ceiling, as a linear amplitude.
    ceiling: S,
    /// The fast release stage's reduction.
    fast: S,
    /// The slow stage's reduction.
    slow: S,
    /// The share of the fast stage's reduction kept per frame.
    keep_fast: S,
    /// The share of the gap to the held need the slow stage closes per frame.
    slow_attack: S,
    /// The released reductions of the last `maximum` frames, a ring.
    released: Vec<S>,
    /// The frame of `released` written next.
    released_at: usize,
    /// How many of the last `lookahead` released reductions are not zero.
    nonzero: usize,
    /// The Hann sums of the last `lookahead` released reductions.
    sums: HannSums<S>,
    /// Frames since `sums` were computed exactly.
    since_exact: usize,
    /// The phasor's positions for the window (`cos`, `sin` of `2π (k + 1) / (lookahead + 1)`).
    phasor: Vec<(S, S)>,
    /// The phasor's step per frame.
    turn: (S, S),
    /// One over `lookahead + 1`.
    scale: S,
    /// The share of the limiter's reduction applied: 1 protected, 0 raw, between them while
    /// crossfading.
    applied: S,
    /// Whether it is protecting (the target of `applied`).
    protected: bool,
    /// What is left of the way from `applied` to its target: it decays to exactly zero,
    /// where recomputing it from `applied` near 1 would stall on the spacing of numbers
    /// there.
    remaining: S,
    /// The share of the gap to its target `applied` keeps per frame.
    keep_mode: S,
}

impl<S: Sample> TruePeakLimiter<S> {
    /// A limiter at `rate` for `channels` channels, its delay sized for `maximum_lookahead`
    /// milliseconds and its `detector`, protecting with `settings` from its first frame, or
    /// raw.
    ///
    /// # Errors
    ///
    /// Returns [`LimiterError::OutOfRange`] for a maximum lookahead outside its range, and
    /// [`LimiterError::AboveMaximum`] for a lookahead of `settings` above it.
    pub fn new(
        rate: SampleRate,
        channels: usize,
        maximum_lookahead: Binary64,
        detector: Detector,
        settings: LimiterSettings,
        protected: bool,
    ) -> Result<Self, LimiterError> {
        let maximum_ms = checked(Setting::MaximumLookahead, maximum_lookahead)?;
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
            S::from_binary64(Binary64::ONE)
        } else {
            S::ZERO
        };
        let mut limiter = Self {
            rate,
            maximum,
            maximum_ms,
            // No window yet: the first `set` below builds it.
            lookahead: 0,
            settings,
            taps,
            delay,
            filter,
            history: vec![S::ZERO; taps.saturating_mul(2).saturating_mul(channels)],
            history_at: 0,
            delayed: vec![S::ZERO; latency.saturating_mul(channels)],
            position: 0,
            needed: vec![S::ZERO; ring],
            needed_at: 0,
            queue: vec![(0, S::ZERO); ring],
            front: 0,
            queued: 0,
            step: 0,
            ceiling: S::ZERO,
            fast: S::ZERO,
            slow: S::ZERO,
            keep_fast: S::ZERO,
            slow_attack: S::ZERO,
            released: vec![S::ZERO; maximum],
            released_at: 0,
            nonzero: 0,
            sums: HannSums::ZERO,
            since_exact: 0,
            phasor: vec![(S::ZERO, S::ZERO); maximum],
            turn: (S::ZERO, S::ZERO),
            scale: S::ZERO,
            applied,
            protected,
            remaining: S::ZERO,
            keep_mode: S::from_binary64(keep(MODE_FADE, rate)),
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

    /// The settings in force.
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
    /// next frame; a new lookahead rebuilds the window from the needs already seen, with no
    /// gap in time.
    ///
    /// # Errors
    ///
    /// Returns [`LimiterError::AboveMaximum`] for a lookahead above the maximum the limiter
    /// was built with; the settings in force are then unchanged.
    pub fn set(&mut self, settings: LimiterSettings) -> Result<(), LimiterError> {
        if settings.lookahead > self.maximum_ms {
            return Err(LimiterError::AboveMaximum {
                lookahead: settings.lookahead,
                maximum: self.maximum_ms,
            });
        }
        self.settings = settings;
        self.ceiling = S::from_binary64(amplitude(settings.ceiling));
        let fast = fast_release(settings.release, settings.adaptivity, FAST_RELEASE_DIVISOR);
        self.keep_fast = S::from_binary64(keep(fast, self.rate));
        self.slow_attack = S::from_binary64(step(keep(settings.release, self.rate)));
        let lookahead = frames(settings.lookahead, self.rate).min(self.maximum);
        if lookahead != self.lookahead {
            self.lookahead = lookahead;
            self.rebuild_window();
        }
        Ok(())
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

    /// Rebuilds what depends on the lookahead: the phasor, the sliding maximum and the Hann
    /// sums, from the needs and released reductions already seen.
    fn rebuild_window(&mut self) {
        // Only the window's first `lookahead` turns are read (`HannSums::of`, `turn`).
        for (steps, turn) in self.phasor.iter_mut().take(self.lookahead).enumerate() {
            let (cos, sin) = turned(steps.saturating_add(1), self.lookahead);
            *turn = (S::from_binary64(cos), S::from_binary64(sin));
        }
        self.turn = self.phasor.first().copied().unwrap_or((S::ZERO, S::ZERO));
        self.scale = S::from_binary64(scale(self.lookahead));
        // The sliding maximum, from the needs of its window, oldest first: the window ends
        // `maximum - lookahead` frames behind the newest need.
        self.queued = 0;
        let delay = self.maximum.saturating_sub(self.lookahead);
        for back in (0..self.lookahead.saturating_add(2)).rev() {
            let step = self
                .step
                .saturating_sub(u64::try_from(back).unwrap_or(u64::MAX));
            self.enqueue(step, self.need_before(delay.saturating_add(back)));
        }
        // The released reductions still in the window were computed over the old window,
        // which a longer lookahead now looks ahead of: each is raised to at least the need
        // held over the new window, so that every sample leaving the delay before they leave
        // the window is reduced as its peaks need.
        let held = self.held();
        let length = self.released.len();
        let newest = behind(self.released_at, 1, length);
        for back in 0..self.lookahead {
            let at = behind(newest, back, length);
            let kept = self.released.get(at).copied().unwrap_or(S::ZERO);
            put(&mut self.released, at, larger_reduction(kept, held));
        }
        self.exact_sums();
    }

    /// Computes the Hann sums of the window exactly, and counts its non-zero reductions.
    fn exact_sums(&mut self) {
        let window = (0..self.lookahead).map(|back| self.released_before(back));
        let nonzero = window.clone().filter(|value| *value != S::ZERO).count();
        let sums = if nonzero == 0 {
            HannSums::ZERO
        } else {
            HannSums::of(window, &self.phasor)
        };
        (self.nonzero, self.sums, self.since_exact) = (nonzero, sums, 0);
    }

    /// Drops what left the window of the sliding maximum, then adds the need `value` of step
    /// `step`.
    ///
    /// Dropping first keeps at most the window's `lookahead + 1` older needs, so the new one
    /// always has a free entry of the ring (`maximum + 2` long), with the lookahead at its
    /// maximum too.
    fn enqueue(&mut self, step: u64, value: S) {
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
    fn held(&self) -> S {
        self.queue
            .get(self.front)
            .map_or(S::ZERO, |(_, value)| *value)
    }

    /// The need recorded `back` frames before the newest (`back` below the ring's length).
    fn need_before(&self, back: usize) -> S {
        let length = self.needed.len();
        let newest = behind(self.needed_at, 1, length);
        self.needed
            .get(behind(newest, back, length))
            .copied()
            .unwrap_or(S::ZERO)
    }

    /// The released reduction recorded `back` frames before the newest.
    fn released_before(&self, back: usize) -> S {
        let length = self.released.len();
        let newest = behind(self.released_at, 1, length);
        self.released
            .get(behind(newest, back, length))
            .copied()
            .unwrap_or(S::ZERO)
    }

    /// Limits the next block of every channel (`blocks[channel]`, as many as it was built
    /// for), in place: each sample becomes the input of [`Self::latency`] frames before,
    /// reduced as the peaks ahead of it need. Frame by frame over the channels, so that it
    /// holds no block of its own, whatever the engine block `N`.
    pub fn apply<const N: usize>(&mut self, blocks: &mut [Block<S, N>]) {
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
                peak = larger_reduction(peak, true_peak(window, &self.filter, taps));
            }
            let reduction = self.reduction(peak);
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
                    *sample = if reduction == S::ZERO {
                        out
                    } else {
                        reduced(out, reduction)
                    };
                }
            }
            self.position = next(position, latency);
        }
    }

    /// The reduction to apply to the sample leaving the delay at the frame whose peak, over
    /// the channels, is `peak`.
    fn reduction(&mut self, peak: S) -> S {
        put(
            &mut self.needed,
            self.needed_at,
            reduction_needed(peak, self.ceiling),
        );
        self.needed_at = next(self.needed_at, self.needed.len());
        self.step = self.step.saturating_add(1);
        // The need entering the window: `maximum - lookahead` frames behind the newest.
        let entering = self.need_before(self.maximum.saturating_sub(self.lookahead));
        self.enqueue(self.step, entering);
        let held = self.held();
        self.fast = larger_reduction(held, decayed(self.fast, self.keep_fast));
        // The slow stage follows the held need through a one-pole of the release's time,
        // up and down: it rises only under a sustained need.
        self.slow = approached(self.slow, held, self.slow_attack);
        let smoothed = self.smooth(larger_reduction(self.fast, self.slow));
        self.crossfade();
        share(smoothed, self.applied)
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
            if complement(self.remaining) == S::from_binary64(Binary64::ONE) {
                self.remaining = S::ZERO;
            }
            self.remaining
        };
    }

    /// Adds `released` to the Hann window and returns the smoothed reduction.
    fn smooth(&mut self, released: S) -> S {
        let oldest = self.released_before(self.lookahead.saturating_sub(1));
        put(&mut self.released, self.released_at, released);
        self.released_at = next(self.released_at, self.released.len());
        if released != S::ZERO {
            self.nonzero = self.nonzero.saturating_add(1);
        }
        if oldest != S::ZERO {
            self.nonzero = self.nonzero.saturating_sub(1);
        }
        if self.nonzero == 0 {
            self.sums = HannSums::ZERO;
            return S::ZERO;
        }
        self.since_exact = self.since_exact.saturating_add(1);
        if self.since_exact >= self.lookahead {
            self.exact_sums();
        } else {
            self.sums = self.sums.advanced(released, oldest, self.turn);
        }
        larger_reduction(S::ZERO, self.sums.smoothed(self.scale))
    }
}

#[cfg(test)]
#[path = "limiter_tests.rs"]
mod tests;
