//! Offline rendering: any number of frames, whatever the block boundaries.

#![cfg_attr(not(test), forbid(clippy::arithmetic_side_effects))]

use alloc::vec;
use alloc::vec::Vec;

use crate::host::Delivered;
use crate::{BLOCK_SIZE, CoreError, Dispatch, OutputStage, Plan, Replaced, Sample};

/// Renders a plan computing in `S` offline, frame by frame, as one continuous stream; its
/// plan holds its box instances as `I`, as [`Plan`] says.
///
/// Rendering `a` then `b` frames gives the same samples as rendering `a + b`. Every block
/// passes the output stage ([`OutputStage`]: a non-finite sample replaced by zero and
/// reported, [`Renderer::replaced`]), then its samples are given as they are computed, or
/// rounded to binary32 for a host that takes binary32 ([`host::Delivered`](Delivered)).
///
/// It owns its [`Plan`] and offers no way to run [`Plan::process`] on it: a block
/// processed behind its back would reorder time (frames of a later block returned before
/// the rest of the current one). That mix does not compile:
///
/// ```compile_fail,E0599
/// use audio_core::{Binary64, Block, Object, Renderer};
///
/// type Virtual = Box<dyn Object<Binary64>>;
///
/// fn mix(renderer: &mut Renderer<Binary64, Virtual>, out: &mut [Block<Binary64>]) {
///     renderer.process(out);
/// }
/// ```
///
/// The same function without that call compiles, so the call is the only error above:
///
/// ```
/// use audio_core::{Binary64, Block, Object, Renderer};
///
/// type Virtual = Box<dyn Object<Binary64>>;
///
/// fn mix(renderer: &mut Renderer<Binary64, Virtual>, out: &mut [Block<Binary64>]) {
///     let _ = (renderer.channels(), out);
/// }
/// ```
///
/// Like [`Plan`], it names how its plan calls its boxes: without it, it does not compile.
///
/// ```compile_fail,E0107
/// fn channels(renderer: &audio_core::Renderer<audio_core::Binary64>) -> usize {
///     renderer.channels()
/// }
/// ```
#[derive(Debug)]
pub struct Renderer<S: Sample, I, const N: usize = BLOCK_SIZE> {
    /// The plan, advanced only by this renderer.
    plan: Plan<S, I, N>,
    /// The last block the plan produced, one per channel, in one buffer on the heap read as
    /// its blocks (no block is built on the stack, whatever the engine block).
    last: Vec<S>,
    /// Frames of `last` already returned; `N` when none is left.
    used: usize,
    /// The output stage every block passes before it is returned.
    stage: OutputStage<S>,
}

impl<S: Sample, I: Dispatch<S, N>, const N: usize> Renderer<S, I, N> {
    /// A renderer starting at the first frame of `plan`, through a raw output stage
    /// ([`OutputStage::new`]: non-finite samples replaced, nothing limited, no latency).
    #[must_use]
    pub fn new(plan: Plan<S, I, N>) -> Self {
        Self::with_stage(plan, OutputStage::new())
    }

    /// A renderer starting at the first frame of `plan`, through `stage` (a limited one,
    /// [`OutputStage::limited`], delays every sample by its latency).
    #[must_use]
    pub fn with_stage(plan: Plan<S, I, N>, stage: OutputStage<S>) -> Self {
        // A plan exists at a valid engine block only (`Plan::build`): restated where the
        // renderer is made, for the error to name it too.
        crate::block::engine_block::<N>();
        Self {
            last: vec![S::ZERO; plan.channels().saturating_mul(N)],
            plan,
            used: N,
            stage,
        }
    }

    /// The latency of its output stage, in frames ([`OutputStage::latency`]): its first
    /// frames are that many frames of silence before the plan's first.
    #[must_use]
    pub fn latency(&self) -> usize {
        self.stage.latency()
    }

    /// What the output stage replaced since the first frame: the non-finite samples, each
    /// rendered as zero. Counted when their block is computed, so it may hold samples of the
    /// current block not returned yet (less than a block ahead).
    #[must_use]
    pub const fn replaced(&self) -> Replaced {
        self.stage.replaced()
    }

    /// Number of output channels: samples per frame.
    #[must_use]
    pub const fn channels(&self) -> usize {
        self.plan.channels()
    }

    /// Renders the next `frames` frames and returns them interleaved by channel
    /// ([`Renderer::channels`] samples per frame), given as `D` ([`host::Delivered`](Delivered)).
    ///
    /// Successive calls continue exactly where the previous one stopped, whatever their
    /// lengths (also across [`Renderer::render_into`]). A plan without output gives no
    /// sample, but its frames are processed: time advances and every box runs, block by
    /// block (`spec/engine-design.md`, "Patches without audio output: time advances").
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::RenderTooLong`] when the samples cannot be held in memory.
    pub fn render<D: Delivered<S>>(&mut self, frames: usize) -> Result<Vec<D>, CoreError> {
        // The plan's channels, read once: whether the render gives samples or only advances.
        let channels = self.plan.channels();
        let length = frames
            .checked_mul(channels)
            .ok_or(CoreError::RenderTooLong(frames))?;
        let mut samples = Vec::new();
        samples
            .try_reserve_exact(length)
            .map_err(|_| CoreError::RenderTooLong(frames))?;
        samples.resize(length, D::deliver(S::ZERO));
        if channels == 0 {
            self.advance(frames);
        } else {
            self.render_into(&mut samples);
        }
        Ok(samples)
    }

    /// Processes the next `frames` frames of a plan without output: every block they reach
    /// runs every box, as rendering it would, and no sample is given.
    fn advance(&mut self, frames: usize) {
        let mut left = frames;
        // Each round takes at least one frame, so `frames` rounds always suffice.
        for _ in 0..frames {
            if left == 0 {
                break;
            }
            if self.used == N {
                let last = self.last.as_chunks_mut::<N>().0;
                self.plan.process(last);
                self.stage.apply(last);
                self.used = 0;
            }
            let taken = N.saturating_sub(self.used).min(left);
            self.used = self.used.saturating_add(taken);
            left = left.saturating_sub(taken);
        }
    }

    /// Renders the next frames into `out`, interleaved by channel and given as `D`
    /// ([`host::Delivered`](Delivered)): as many whole frames as `out` holds
    /// ([`Renderer::channels`] samples each). Samples after the last whole frame are left
    /// untouched; a plan without output writes nothing, and since no frame is said, runs
    /// nothing either ([`Renderer::render`] advances it).
    ///
    /// Continues exactly where the previous render stopped, like [`Renderer::render`].
    /// Does not use the heap. Copies one channel at a time, as many frames as both the
    /// current block and `out` still hold.
    pub fn render_into<D: Delivered<S>>(&mut self, out: &mut [D]) {
        let channels = self.channels();
        let Some(frames) = out.len().checked_div(channels) else {
            return;
        };
        let mut rest = out
            .get_mut(..frames.saturating_mul(channels))
            .unwrap_or_default();
        // Each round copies at least one frame, so `frames` rounds always suffice: the bound
        // is the invariant, and it keeps a bug from looping forever.
        for _ in 0..frames {
            if rest.is_empty() {
                break;
            }
            let last = self.last.as_chunks_mut::<N>().0;
            if self.used == N {
                self.plan.process(last);
                self.stage.apply(last);
                self.used = 0;
            }
            let count = N
                .saturating_sub(self.used)
                .min(rest.len().checked_div(channels).unwrap_or(0));
            for (offset, block) in last.iter().enumerate() {
                let samples = rest.iter_mut().skip(offset).step_by(channels);
                let values = samples.zip(block.iter().skip(self.used)).take(count);
                // Decided once per channel, when the plan was built: only a sum can hold a
                // subnormal in the plan's precision (`Delivered::deliver_unsummed`).
                if self.plan.channel_is_sum(offset) {
                    for (sample, value) in values {
                        *sample = D::deliver(*value);
                    }
                } else {
                    for (sample, value) in values {
                        *sample = D::deliver_unsummed(*value);
                    }
                }
                // Given in a narrower format, a finite sample can round to infinity: the
                // stage replaces it as it is given (decision 12).
                let given = rest.iter_mut().skip(offset).step_by(channels).take(count);
                let used = self.used;
                D::replace_overflowed(given, |index| {
                    self.stage
                        .note_given::<N>(used.saturating_add(index), offset);
                });
            }
            self.used = self.used.saturating_add(count);
            rest = core::mem::take(&mut rest)
                .get_mut(count.saturating_mul(channels)..)
                .unwrap_or_default();
        }
    }
}
