//! The output stage (`spec/engine-design.md`, decision 12): after the plan, before delivery
//! and before any hash, every non-finite sample is replaced by zero and reported.

#![cfg_attr(not(test), forbid(clippy::arithmetic_side_effects))]

use crate::math::all_finite;
use crate::{BLOCK_SIZE, Block, Sample};

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
    /// Counts one more sample replaced, at `at`, keeping the earliest.
    fn note(&mut self, at: FirstReplaced) {
        self.count = self.count.saturating_add(1);
        self.first = Some(self.first.map_or(at, |first| first.min(at)));
    }
}

/// The output stage of a render, live or offline: one for each stream of output blocks,
/// applied to every block the plan produces, in order.
///
/// Every non-finite sample (NaN or infinite, decided from its bits) is replaced by
/// [`Sample::ZERO`] and reported ([`OutputStage::replaced`]); every finite sample keeps its
/// bits. No heap: it runs on the audio thread.
#[derive(Debug, Clone, Default)]
pub struct OutputStage {
    /// The frame of the next block's first sample.
    frame: u64,
    /// What it replaced so far.
    replaced: Replaced,
}

impl OutputStage {
    /// A stage at the first frame, having replaced nothing.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            frame: 0,
            replaced: Replaced {
                count: 0,
                first: None,
            },
        }
    }

    /// Applies the stage to the next block of every channel (`blocks[channel]`).
    ///
    /// A block is first checked whole, in one tight pass; only a block holding a non-finite
    /// sample is then replaced and counted, so that a render without any pays the check
    /// alone.
    #[expect(
        clippy::inline_always,
        reason = "inlined in every caller, the check of each block costs fewer instructions \
                  per sample and the module is smaller: measured in spec/benchmarks.md, \
                  The output stage"
    )]
    #[inline(always)]
    pub fn apply<S: Sample>(&mut self, blocks: &mut [Block<S>]) {
        for (channel, block) in blocks.iter_mut().enumerate() {
            if !all_finite(block) {
                self.replace(block, channel);
            }
        }
        self.frame = self
            .frame
            .saturating_add(u64::try_from(BLOCK_SIZE).unwrap_or(u64::MAX));
    }

    /// Counts one more sample replaced, the sample at `offset` in the block applied last, of
    /// channel `channel`: one its host was given as infinite though it was finite when
    /// computed, by a rounding to a narrower format (a binary64 value beyond the largest
    /// binary32 one, [`Delivered::replace_overflowed`](crate::host::Delivered::replace_overflowed)), which the host
    /// replaced by zero as it gave it.
    pub fn note_given(&mut self, offset: usize, channel: usize) {
        // The block applied last starts a block before the next one: computed here, on the
        // cold path, rather than kept by every block.
        let block = self
            .frame
            .saturating_sub(u64::try_from(BLOCK_SIZE).unwrap_or(u64::MAX));
        let offset = u64::try_from(offset).unwrap_or(u64::MAX);
        self.replaced.note(FirstReplaced {
            frame: block.saturating_add(offset),
            channel,
        });
    }

    /// Replaces every non-finite sample of `block`, of channel `channel`, and counts it.
    #[cold]
    fn replace<S: Sample>(&mut self, block: &mut Block<S>, channel: usize) {
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

#[cfg(test)]
#[path = "output_tests.rs"]
mod tests;
