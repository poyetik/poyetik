//! The output stage (`spec/engine-design.md`, decision 12): after the plan, before delivery
//! and before any hash, every non-finite sample is replaced by zero and reported, then,
//! when the stage protects, the true-peak limiter ([`TruePeakLimiter`]) keeps the signal
//! under its ceiling.

#![cfg_attr(not(test), forbid(clippy::arithmetic_side_effects))]

use crate::host::{Delivered, Raw};
use crate::math::all_finite;
use crate::{Binary32, Binary64, Block, Sample, TruePeakLimiter};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Replaced {
    /// How many samples were replaced, saturating at `u64::MAX`.
    pub count: u64,
    /// The first sample replaced; `None` while none was.
    pub first: Option<FirstReplaced>,
}

impl Replaced {
    /// Nothing replaced yet.
    const NONE: Self = Self {
        count: 0,
        first: None,
    };

    /// Counts one more sample replaced, at `at`, keeping the earliest.
    fn note(&mut self, at: FirstReplaced) {
        self.count = self.count.saturating_add(1);
        self.first = Some(self.first.map_or(at, |first| first.min(at)));
    }
}

/// The output stage of a render, live or offline, computing in `S`: one for each stream of
/// output blocks, applied to every block the plan produces, in order.
///
/// Every non-finite sample (NaN or infinite, decided from its bits) is replaced by
/// [`Sample::ZERO`] and reported ([`OutputStage::replaced`]); every finite sample keeps its
/// bits. A raw stage ([`OutputStage::new`]) does nothing else and has no latency; a stage
/// with a limiter ([`OutputStage::limited`]) then limits, protected or raw as its limiter
/// is set, and delays every sample by the limiter's latency ([`OutputStage::latency`]). No
/// heap: it runs on the audio thread.
#[derive(Debug, Clone)]
pub struct OutputStage<S: Sample> {
    /// The frame of the next block's first sample, as the plan computes it.
    frame: u64,
    /// What it replaced so far.
    replaced: Replaced,
    /// The limiter, when the stage has one.
    limiter: Option<TruePeakLimiter<S>>,
}

impl<S: Sample> Default for OutputStage<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S: Sample> OutputStage<S> {
    /// A raw stage at the first frame, having replaced nothing: no limiter, no latency (the
    /// native renderer's, conformance's and plugins' default).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            frame: 0,
            replaced: Replaced::NONE,
            limiter: None,
        }
    }

    /// A stage at the first frame that limits through `limiter` once its non-finite samples
    /// are replaced (the playground's default, protected).
    #[must_use]
    pub const fn limited(limiter: TruePeakLimiter<S>) -> Self {
        Self {
            frame: 0,
            replaced: Replaced::NONE,
            limiter: Some(limiter),
        }
    }

    /// Its latency, in frames: its limiter's ([`TruePeakLimiter::latency`]), 0 without one.
    #[must_use]
    pub fn latency(&self) -> usize {
        self.limiter.as_ref().map_or(0, TruePeakLimiter::latency)
    }

    /// The channels of its limiter ([`TruePeakLimiter::channels`]), which a plan it applies
    /// to must have; `None` for a raw stage, which applies to any.
    #[must_use]
    pub fn channels(&self) -> Option<u16> {
        self.limiter.as_ref().map(TruePeakLimiter::channels)
    }

    /// Whether it applied no block yet: at the first frame.
    pub(crate) const fn is_unused(&self) -> bool {
        self.frame == 0
    }

    /// Its limiter, to change its settings or its mode live; `None` for a raw stage.
    pub const fn limiter(&mut self) -> Option<&mut TruePeakLimiter<S>> {
        self.limiter.as_mut()
    }

    /// Applies the stage to the next block of every channel (`blocks[channel]`): replaces
    /// and reports its non-finite samples, then limits it when the stage has a limiter.
    ///
    /// A block is first checked whole, in one tight pass; only a block holding a non-finite
    /// sample is then replaced and counted, so that a render without any pays the check
    /// alone.
    ///
    /// Compiles for an engine block `N` of 1 to [`crate::MAX_ENGINE_BLOCK`] frames only
    /// (decision 2), neither 0 nor one more:
    ///
    /// ```compile_fail,E0080
    /// use audio_core::{Binary32, Binary64, OutputStage};
    /// OutputStage::<Binary64>::new().apply::<0>(&mut []);
    /// ```
    ///
    /// ```compile_fail,E0080
    /// use audio_core::{Binary32, Binary64, OutputStage};
    /// OutputStage::<Binary64>::new().apply::<1_048_577>(&mut []);
    /// ```
    #[expect(
        clippy::inline_always,
        reason = "inlined in every caller, the check of each block costs fewer instructions \
                  per sample and the module is smaller: measured in spec/benchmarks.md, \
                  The output stage"
    )]
    #[inline(always)]
    pub fn apply<const N: usize>(&mut self, blocks: &mut [Block<S, N>]) {
        crate::block::engine_block::<N>();
        for (channel, block) in blocks.iter_mut().enumerate() {
            if !all_finite(block) {
                self.replace(block, channel);
            }
        }
        if let Some(limiter) = &mut self.limiter {
            limiter.apply(blocks);
        }
        self.advance::<N>();
    }

    /// Counts one more block, of `N` frames.
    fn advance<const N: usize>(&mut self) {
        self.frame = self
            .frame
            .saturating_add(u64::try_from(N).unwrap_or(u64::MAX));
    }

    /// Counts one more sample replaced, the sample at `offset` in the block applied last, of
    /// channel `channel`: one its host was given as infinite though it was finite when
    /// computed, by a rounding to a narrower format (a binary64 value beyond the largest
    /// binary32 one, [`Delivered::replace_overflowed`](crate::host::Delivered::replace_overflowed)), which the host
    /// replaced by zero as it gave it.
    ///
    /// Its frame is the one the plan computed it at: the stage delayed it by its latency.
    pub(crate) fn note_given<const N: usize>(&mut self, offset: usize, channel: usize) {
        crate::block::engine_block::<N>();
        // The block applied last starts a block before the next one: computed here, on the
        // cold path, rather than kept by every block.
        let block = self
            .frame
            .saturating_sub(u64::try_from(N).unwrap_or(u64::MAX));
        let offset = u64::try_from(offset).unwrap_or(u64::MAX);
        let latency = u64::try_from(self.latency()).unwrap_or(u64::MAX);
        self.replaced.note(FirstReplaced {
            frame: block.saturating_add(offset).saturating_sub(latency),
            channel,
        });
    }

    /// Replaces and counts the infinite samples of `block`, of channel `channel`, as given.
    #[cold]
    fn replace_given_block<G: Sample, const N: usize>(
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

    /// Replaces every non-finite sample of `block`, of channel `channel`, and counts it.
    #[cold]
    fn replace<const N: usize>(&mut self, block: &mut Block<S, N>, channel: usize) {
        let mut frame = self.frame;
        for sample in block.iter_mut() {
            if !sample.is_finite() {
                *sample = S::ZERO;
                self.replaced.note(FirstReplaced { frame, channel });
            }
            frame = frame.saturating_add(1);
        }
    }

    /// What it replaced since it was built.
    #[must_use]
    pub const fn replaced(&self) -> Replaced {
        self.replaced
    }
}

impl OutputStage<Binary64> {
    /// Applies the stage to the next block of every channel, computed in binary64
    /// (`blocks[channel]`), and gives it rounded to binary32 in `given` (`given[channel]`, as
    /// [`Delivered`] rounds it): a binary64 render given as binary32.
    ///
    /// A stage with a limiter applies itself whole before the rounding
    /// ([`OutputStage::apply`]), since the limiter's detector needs finite samples; a raw
    /// stage only counts the block, and leaves every non-finite sample to the check as given,
    /// which sees each one (the rounding keeps a non-finite sample non-finite, at the same
    /// place): one pass over the block instead of two. Then every sample the rounding left or
    /// made infinite (a finite binary64 value beyond the largest binary32 one) is replaced by
    /// zero as given, and counted at the frame the plan computed it.
    ///
    /// Compiles for an engine block `N` of 1 to [`crate::MAX_ENGINE_BLOCK`] frames only
    /// (decision 2), neither 0 nor one more:
    ///
    /// ```compile_fail,E0080
    /// use audio_core::{Binary64, OutputStage};
    /// OutputStage::<Binary64>::new().apply_narrowing::<0>(&mut [], &mut []);
    /// ```
    ///
    /// ```compile_fail,E0080
    /// use audio_core::{Binary64, OutputStage};
    /// OutputStage::<Binary64>::new().apply_narrowing::<1_048_577>(&mut [], &mut []);
    /// ```
    pub fn apply_narrowing<const N: usize>(
        &mut self,
        blocks: &mut [Block<Binary64, N>],
        given: &mut [Block<Binary32, N>],
    ) {
        crate::block::engine_block::<N>();
        if self.limiter.is_some() {
            self.apply(blocks);
        } else {
            self.advance::<N>();
        }
        for (narrow, wide) in given.iter_mut().zip(blocks.iter()) {
            for (sample, value) in narrow.iter_mut().zip(wide) {
                *sample = Binary32::from_raw(<f32 as Delivered<Binary64>>::deliver(*value));
            }
        }
        for (channel, block) in given.iter_mut().enumerate() {
            if !all_finite(block) {
                self.replace_given_block(block, channel);
            }
        }
    }
}

#[cfg(test)]
#[path = "output_tests.rs"]
mod tests;
