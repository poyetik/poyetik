//! The output stage (`spec/engine-design.md`, decision 12): after the plan, before delivery
//! and before any hash, every non-finite sample is replaced by zero and reported, then,
//! when the stage protects, the true-peak limiter ([`TruePeakLimiter`]) keeps the signal
//! under its ceiling.

#![cfg_attr(not(test), forbid(clippy::arithmetic_side_effects))]

use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

use crate::host::{Delivered, Raw};
use crate::math::all_finite;
use crate::{
    BLOCK_SIZE, Binary32, Binary64, Block, LimiterError, LimiterSettings, Sample, TruePeakLimiter,
};

/// The first sample the output stage replaced: the earliest frame, then the lowest channel
/// of that frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FirstReplaced {
    /// Its frame, counted from the first frame the stage processed.
    pub frame: u64,
    /// Its channel.
    pub channel: usize,
}

/// What the output stage replaced since it was built: how many non-finite samples (NaN or
/// infinite), and the first of them.
///
/// Its fields are private, so that a count and a first sample always agree (none while the
/// count is zero, one once it is not): a stage counts them, and [`Replaced::new`] checks
/// them. A report cannot be written by hand:
///
/// ```compile_fail,E0451
/// let forged = audio_core::Replaced { count: 1, first: None };
/// ```
///
/// while the checked constructor refuses that pair:
///
/// ```
/// assert_eq!(audio_core::Replaced::new(1, None), None);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Replaced {
    /// How many samples were replaced, saturating at `u64::MAX`.
    count: u64,
    /// The first sample replaced; `None` while none was.
    first: Option<FirstReplaced>,
}

impl Replaced {
    /// Nothing replaced yet.
    const NONE: Self = Self {
        count: 0,
        first: None,
    };

    /// A report of `count` samples replaced, the first at `first`: `None` unless they agree,
    /// no first sample for a count of zero and one for any other count.
    #[must_use]
    pub const fn new(count: u64, first: Option<FirstReplaced>) -> Option<Self> {
        if (count == 0) == first.is_none() {
            Some(Self { count, first })
        } else {
            None
        }
    }

    /// How many samples were replaced, saturating at `u64::MAX`.
    #[must_use]
    pub const fn count(self) -> u64 {
        self.count
    }

    /// The first sample replaced, the earliest frame then the lowest channel; `None` while
    /// none was.
    #[must_use]
    pub const fn first(self) -> Option<FirstReplaced> {
        self.first
    }

    /// Counts one more sample replaced, at `at`, keeping the earliest.
    fn note(&mut self, at: FirstReplaced) {
        self.count = self.count.saturating_add(1);
        self.first = Some(self.first.map_or(at, |first| first.min(at)));
    }
}

/// Why an output stage refused a change of its limiter ([`OutputStage::set_limiter`],
/// [`OutputStage::set_protected`]).
///
/// Not exhaustive: a stage may gain a setting, and with it a refusal, without breaking a
/// caller, which shows it through its [`Display`](fmt::Display).
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum StageError {
    /// The stage is raw: it has no limiter to set.
    Raw,
    /// Its limiter refused the settings ([`TruePeakLimiter::set`]); they are unchanged.
    Limiter(LimiterError),
}

impl fmt::Display for StageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Raw => write!(f, "the output stage is raw: it has no limiter to set"),
            Self::Limiter(error) => write!(f, "{error}"),
        }
    }
}

impl core::error::Error for StageError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Raw => None,
            Self::Limiter(error) => Some(error),
        }
    }
}

/// What an output stage keeps from one block to the next, its blocks apart: its report,
/// the frames it reports on and its limiter.
#[derive(Debug, Clone)]
struct Stream<S: Sample> {
    /// The frame of the next block's first sample, as the plan computes it.
    frame: u64,
    /// The frames of the render, delivered from its first: it counts only what it replaced
    /// in them ([`OutputStage::reporting_until`]); none for a stream without an end.
    until: Option<u64>,
    /// What it replaced so far.
    replaced: Replaced,
    /// The limiter, when the stage has one: it limits every block of the stage, one per
    /// channel it was built for.
    limiter: Option<TruePeakLimiter<S>>,
}

impl<S: Sample> Stream<S> {
    /// At the first frame, nothing replaced, for a stream without an end, limited through
    /// `limiter` when there is one.
    const fn new(limiter: Option<TruePeakLimiter<S>>) -> Self {
        Self {
            frame: 0,
            until: None,
            replaced: Replaced::NONE,
            limiter,
        }
    }

    /// The latency of its limiter, in frames, 0 without one.
    fn latency(&self) -> usize {
        self.limiter.as_ref().map_or(0, TruePeakLimiter::latency)
    }

    /// Counts one more block, of `N` frames.
    fn advance<const N: usize>(&mut self) {
        self.frame = self
            .frame
            .saturating_add(u64::try_from(N).unwrap_or(u64::MAX));
    }

    /// Replaces and reports the non-finite samples of `blocks`, then limits them when it
    /// has a limiter: the stage's work on a block as computed.
    fn replace_then_limit<const N: usize>(&mut self, blocks: &mut [Block<S, N>]) {
        for (channel, block) in blocks.iter_mut().enumerate() {
            if !all_finite(block) {
                self.replace(block, channel);
            }
        }
        if let Some(limiter) = &mut self.limiter {
            limiter.apply(blocks);
        }
    }

    /// Counts one more sample replaced, computed at `at`, if it is delivered within the
    /// frames it reports on ([`OutputStage::reporting_until`]).
    fn count(&mut self, at: FirstReplaced) {
        let latency = u64::try_from(self.latency()).unwrap_or(u64::MAX);
        if self
            .until
            .is_none_or(|until| at.frame < until.saturating_sub(latency))
        {
            self.replaced.note(at);
        }
    }

    /// Counts one more sample replaced, the sample at `offset` in the block applied last, of
    /// `N` frames, of channel `channel`: one its host was given as infinite though it was
    /// finite when computed (`OutputStage::note_given`). Its frame is the one the plan
    /// computed it at: the stage delayed it by its latency.
    fn note_given<const N: usize>(&mut self, offset: usize, channel: usize) {
        // The block applied last starts a block before the next one: computed here, on the
        // cold path, rather than kept by every block.
        let block = self
            .frame
            .saturating_sub(u64::try_from(N).unwrap_or(u64::MAX));
        let offset = u64::try_from(offset).unwrap_or(u64::MAX);
        let latency = u64::try_from(self.latency()).unwrap_or(u64::MAX);
        self.count(FirstReplaced {
            frame: block.saturating_add(offset).saturating_sub(latency),
            channel,
        });
    }

    /// Replaces every non-finite sample of `block`, of channel `channel`, and counts it.
    #[cold]
    fn replace<const N: usize>(&mut self, block: &mut Block<S, N>, channel: usize) {
        let mut frame = self.frame;
        for sample in block.iter_mut() {
            if !sample.is_finite() {
                *sample = S::ZERO;
                self.count(FirstReplaced { frame, channel });
            }
            frame = frame.saturating_add(1);
        }
    }

    /// Replaces and counts the infinite samples of `block`, of channel `channel`, as given.
    #[cold]
    fn replace_given<G: Sample, const N: usize>(
        &mut self,
        block: &mut Block<G, N>,
        channel: usize,
    ) {
        for (offset, sample) in block.iter_mut().enumerate() {
            if !sample.is_finite() {
                *sample = G::ZERO;
                self.note_given::<N>(offset, channel);
            }
        }
    }
}

/// `channels` blocks of silence of `N` frames, made on the heap: the array of a block
/// written as a value (`vec![[S::ZERO; N]; channels]`) is first built on the stack, too
/// large for a thread's stack at the largest engine blocks. Copied from one buffer of
/// samples, whose blocks a slice of `Copy` values extends by a copy of memory.
pub(crate) fn silent_blocks<S: Sample, const N: usize>(channels: usize) -> Vec<Block<S, N>> {
    let samples = vec![S::ZERO; channels.saturating_mul(N)];
    let mut blocks = Vec::with_capacity(channels);
    blocks.extend_from_slice(samples.as_chunks::<N>().0);
    blocks
}

/// The output stage of a render, live or offline: one for each stream of output blocks.
///
/// It computes in `S`, in blocks of `N` frames (the engine block, [`crate::BLOCK_SIZE`] by
/// default), applied to every block the plan produces, in order.
///
/// It owns its blocks, one per channel: the plan writes into them ([`OutputStage::apply`]),
/// so that every block it applies to is one it limits, its count fixed when it is built
/// (by its limiter's channels, [`OutputStage::limited`]). No block can be added to them:
///
/// ```compile_fail,E0277
/// use audio_core::{Binary64, OutputStage};
/// let mut stage = OutputStage::<Binary64>::new(1);
/// let _ = stage.apply(&mut [[Binary64::new(2.0); 64]; 2]);
/// ```
///
/// Every non-finite sample (NaN or infinite, decided from its bits) is replaced by
/// [`Sample::ZERO`] and reported ([`OutputStage::replaced`]); every finite sample keeps its
/// bits. A raw stage ([`OutputStage::new`]) does nothing else and has no latency; a stage
/// with a limiter ([`OutputStage::limited`]) then limits, protected or raw as its limiter
/// is set, and delays every sample by the limiter's latency ([`OutputStage::latency`]). No
/// heap after it is built: it runs on the audio thread.
#[derive(Debug, Clone)]
pub struct OutputStage<S: Sample, const N: usize = BLOCK_SIZE> {
    /// What it keeps from one block to the next, its limiter included.
    stream: Stream<S>,
    /// The last block of every channel: written by the plan, then replaced and limited in
    /// place, then read by its host until the next.
    blocks: Vec<Block<S, N>>,
}

impl<S: Sample, const N: usize> OutputStage<S, N> {
    /// A raw stage of `channels` channels at the first frame, having replaced nothing: no
    /// limiter, no latency (the native renderer's, conformance's and plugins' default).
    ///
    /// Compiles for an engine block `N` of 1 to [`crate::MAX_ENGINE_BLOCK`] frames only
    /// (decision 2), neither 0 nor one more:
    ///
    /// ```compile_fail,E0080
    /// use audio_core::{Binary64, OutputStage};
    /// let _ = OutputStage::<Binary64, 0>::new(1);
    /// ```
    ///
    /// ```compile_fail,E0080
    /// use audio_core::{Binary64, OutputStage};
    /// let _ = OutputStage::<Binary64, 1_048_577>::new(1);
    /// ```
    #[must_use]
    pub fn new(channels: usize) -> Self {
        crate::block::engine_block::<N>();
        Self {
            stream: Stream::new(None),
            blocks: silent_blocks(channels),
        }
    }

    /// A stage at the first frame that limits through `limiter` once its non-finite samples
    /// are replaced (the playground's default, protected): as many channels as `limiter`
    /// limits ([`TruePeakLimiter::channels`]).
    ///
    /// Compiles for an engine block `N` of 1 to [`crate::MAX_ENGINE_BLOCK`] frames only
    /// (decision 2), as [`OutputStage::new`] does.
    #[must_use]
    pub fn limited(limiter: TruePeakLimiter<S>) -> Self {
        crate::block::engine_block::<N>();
        Self {
            blocks: silent_blocks(usize::from(limiter.channels())),
            stream: Stream::new(Some(limiter)),
        }
    }

    /// This stage, counting only what it replaces in the first `frames` frames it delivers:
    /// for a render of `frames` frames, whose last block computes samples it never delivers
    /// (past the end, or still in the limiter's delay), so that its report is the same at
    /// every engine block (decision 2). A sample the plan computes at frame `f` is delivered
    /// at `f` plus the stage's latency ([`OutputStage::latency`]): it is counted only before
    /// `frames` less that latency. Every non-finite sample is replaced all the same. A stage
    /// is built for a stream without an end, counting everything.
    #[must_use]
    pub const fn reporting_until(mut self, frames: u64) -> Self {
        self.stream.until = Some(frames);
        self
    }

    /// Its latency, in frames: its limiter's ([`TruePeakLimiter::latency`]), 0 without one.
    #[must_use]
    pub fn latency(&self) -> usize {
        self.stream.latency()
    }

    /// Its channels: the blocks it applies to, one per channel, as many as its limiter
    /// limits when it has one. A renderer refuses a stage of another count than its plan's
    /// ([`crate::CoreError::StageChannels`]).
    #[must_use]
    pub const fn channels(&self) -> usize {
        self.blocks.len()
    }

    /// Whether it applied no block yet: at the first frame.
    pub(crate) const fn is_unused(&self) -> bool {
        self.stream.frame == 0
    }

    /// Its limiter, to read; `None` for a raw stage. Its settings and its mode change
    /// through the stage ([`OutputStage::set_limiter`], [`OutputStage::set_protected`]),
    /// never by another limiter in its place, whose channels or rate could differ:
    ///
    /// ```compile_fail,E0594
    /// use audio_core::{Binary64, Detector, LimiterSettings, OutputStage, SampleRate, TruePeakLimiter};
    /// let rate = SampleRate::new(48_000).expect("a rate");
    /// let limiter = |channels| {
    ///     TruePeakLimiter::<Binary64>::new(
    ///         rate, channels, Binary64::new(5.0), Detector::Precise, LimiterSettings::DEFAULT, true,
    ///     )
    ///     .expect("a limiter")
    /// };
    /// let mut stage = OutputStage::<Binary64>::limited(limiter(2));
    /// if let Some(inside) = stage.limiter() {
    ///     *inside = limiter(1);
    /// }
    /// ```
    #[must_use]
    pub const fn limiter(&self) -> Option<&TruePeakLimiter<S>> {
        self.stream.limiter.as_ref()
    }

    /// Changes the settings of its limiter, live ([`TruePeakLimiter::set`]).
    ///
    /// # Errors
    ///
    /// Returns [`StageError::Raw`] for a raw stage, which has no limiter, and
    /// [`StageError::Limiter`] when the limiter refuses the settings (a lookahead above its
    /// maximum), which are then unchanged.
    pub fn set_limiter(&mut self, settings: LimiterSettings) -> Result<(), StageError> {
        self.stream
            .limiter
            .as_mut()
            .ok_or(StageError::Raw)?
            .set(settings)
            .map_err(StageError::Limiter)
    }

    /// Starts the crossfade of its limiter toward protected (`true`) or raw (`false`)
    /// ([`TruePeakLimiter::set_protected`]).
    ///
    /// # Errors
    ///
    /// Returns [`StageError::Raw`] for a raw stage, which has no limiter.
    pub fn set_protected(&mut self, protected: bool) -> Result<(), StageError> {
        self.stream
            .limiter
            .as_mut()
            .ok_or(StageError::Raw)?
            .set_protected(protected);
        Ok(())
    }

    /// Applies the stage to the next block of every channel, which `produce` writes into
    /// its blocks (`blocks[channel]`, [`OutputStage::channels`] of them; the plan, as
    /// [`crate::Plan::process`]), and returns them: replaces and reports their non-finite
    /// samples, then limits them when the stage has a limiter.
    ///
    /// A block is first checked whole, in one tight pass; only a block holding a non-finite
    /// sample is then replaced and counted, so that a render without any pays the check
    /// alone. The blocks are taken as one slice once per block, handed to `produce`, then
    /// applied and returned (`spec/benchmarks.md`, "The buffers taken as slices once per
    /// block").
    #[expect(
        clippy::inline_always,
        reason = "inlined in every caller, the check of each block costs fewer instructions \
                  per sample and the module is smaller: measured in spec/benchmarks.md, \
                  The output stage"
    )]
    #[inline(always)]
    pub fn apply(&mut self, produce: impl FnOnce(&mut [Block<S, N>])) -> &mut [Block<S, N>] {
        let Self { stream, blocks } = self;
        let blocks = blocks.as_mut_slice();
        produce(blocks);
        stream.replace_then_limit(blocks);
        stream.advance::<N>();
        blocks
    }

    /// The blocks it applied to last, one per channel, as [`OutputStage::apply`] returned
    /// them (silence before the first).
    #[must_use]
    pub fn blocks(&self) -> &[Block<S, N>] {
        &self.blocks
    }

    /// Counts one more sample replaced, the sample at `offset` in the block applied last, of
    /// channel `channel`: one its host was given as infinite though it was finite when
    /// computed, by a rounding to a narrower format (a binary64 value beyond the largest
    /// binary32 one, [`crate::host::Delivered::replace_overflowed`]), which the host
    /// replaced by zero as it gave it.
    ///
    /// Its frame is the one the plan computed it at: the stage delayed it by its latency.
    pub(crate) fn note_given(&mut self, offset: usize, channel: usize) {
        self.stream.note_given::<N>(offset, channel);
    }

    /// Gives its limiter's output rounded to binary32 from now on, if `binary32` and it has
    /// one (`TruePeakLimiter::give_binary32`): its ceiling rounded toward zero in binary32.
    /// A flag, not a type: a renderer passes its delivered type's [`Delivered::NARROWS`],
    /// a constant of each of its instantiations, which one instantiation could branch on
    /// one way only; here one instantiation takes both ways (llvm-cov's summary counts a
    /// generic function's best instantiation, not the union of them).
    pub(crate) fn give_binary32_if(&mut self, binary32: bool) {
        if binary32 && let Some(limiter) = &mut self.stream.limiter {
            limiter.give_binary32();
        }
    }

    /// What it replaced since it was built.
    #[must_use]
    pub const fn replaced(&self) -> Replaced {
        self.stream.replaced
    }
}

impl<const N: usize> OutputStage<Binary64, N> {
    /// Applies the stage to the next block of every channel, computed in binary64, which
    /// `produce` writes into its blocks (as [`OutputStage::apply`]), and gives it rounded to
    /// binary32 in `given` (`given[channel]`, as [`Delivered`] rounds it): a binary64 render
    /// given as binary32.
    ///
    /// A stage with a limiter applies itself whole before the rounding
    /// ([`OutputStage::apply`]), since the limiter's detector needs finite samples, its
    /// ceiling rounded toward zero in binary32 from the first block given so, so that no
    /// sample rounds above it (decision 12); a raw
    /// stage only counts the block, and leaves every non-finite sample to the check as given,
    /// which sees each one (the rounding keeps a non-finite sample non-finite, at the same
    /// place): one pass over the block instead of two. Then the non-finite samples as given
    /// are replaced by zero and counted at the frame the plan computed them: on a raw stage,
    /// every non-finite sample, those the plan computed and those the rounding made infinite
    /// (a finite binary64 value beyond the largest binary32 one); on a limited stage, only
    /// those the rounding made infinite, the others replaced before the limiter. Each channel
    /// is rounded and checked in one turn, each sample checked as it is rounded
    /// (`spec/benchmarks.md`, "One turn per channel as given" and "The check fused with the
    /// rounding"); `given` holds one block per channel of the stage, and a block of `given`
    /// beyond them is left as it is.
    pub fn apply_narrowing(
        &mut self,
        produce: impl FnOnce(&mut [Block<Binary64, N>]),
        given: &mut [Block<Binary32, N>],
    ) {
        let Self { stream, blocks } = self;
        let blocks = blocks.as_mut_slice();
        produce(blocks);
        if let Some(limiter) = &mut stream.limiter {
            // Its ceiling in binary32, the format given (decision 12).
            limiter.give_binary32();
            stream.replace_then_limit(blocks);
        }
        stream.advance::<N>();
        for (channel, (narrow, wide)) in given.iter_mut().zip(blocks.iter()).enumerate() {
            if !narrowed_finite(narrow, wide) {
                stream.replace_given(narrow, channel);
            }
        }
    }
}

/// Rounds `wide` to binary32 into `narrow`, as [`Delivered`] gives it, and tells whether
/// every sample given is finite: the check of [`all_finite`] (each sample times zero added
/// to a sum, NaN as soon as one is not finite) made in the same pass as the rounding, on
/// the sample just rounded, so that the block is not read again (`spec/benchmarks.md`, "The
/// check fused with the rounding"). Eight at a time, each eight unrolled, as
/// [`all_finite`] reads them; the frames after the last eight follow one by one.
fn narrowed_finite<const N: usize>(
    narrow: &mut Block<Binary32, N>,
    wide: &Block<Binary64, N>,
) -> bool {
    let given =
        |value: &Binary64| Binary32::from_raw(<f32 as Delivered<Binary64>>::deliver(*value));
    let (narrow_eights, narrow_rest) = narrow.as_chunks_mut::<8>();
    let (wide_eights, wide_rest) = wide.as_chunks::<8>();
    let mut sum = Binary32::ZERO;
    for (eight, values) in narrow_eights.iter_mut().zip(wide_eights) {
        for (sample, value) in eight.iter_mut().zip(values) {
            *sample = given(value);
            sum = sample.mul_then_add(Binary32::ZERO, sum);
        }
    }
    for (sample, value) in narrow_rest.iter_mut().zip(wide_rest) {
        *sample = given(value);
        sum = sample.mul_then_add(Binary32::ZERO, sum);
    }
    sum.is_finite()
}

#[cfg(test)]
#[path = "output_tests.rs"]
mod tests;
