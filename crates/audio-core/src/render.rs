//! Offline rendering: any number of frames, whatever the block boundaries.

#![cfg_attr(not(test), forbid(clippy::arithmetic_side_effects))]

use alloc::vec;
use alloc::vec::Vec;

use crate::host::Delivered;
use crate::{BLOCK_SIZE, Block, CoreError, Dispatch, Plan, Sample};

/// Renders a plan computing in `S` offline, frame by frame, as one continuous stream; its
/// plan holds its box instances as `I`, as [`Plan`] says.
///
/// Rendering `a` then `b` frames gives the same samples as rendering `a + b`. Its samples
/// are given as they are computed, or rounded to binary32 for a host that takes binary32
/// ([`host::Delivered`](Delivered)).
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
pub struct Renderer<S: Sample, I> {
    /// The plan, advanced only by this renderer.
    plan: Plan<S, I>,
    /// The last block the plan produced, one per channel.
    last: Vec<Block<S>>,
    /// Frames of `last` already returned; [`BLOCK_SIZE`] when none is left.
    used: usize,
}

impl<S: Sample, I: Dispatch<S>> Renderer<S, I> {
    /// A renderer starting at the first frame of `plan`.
    #[must_use]
    pub fn new(plan: Plan<S, I>) -> Self {
        Self {
            last: vec![S::SILENCE; plan.channels()],
            plan,
            used: BLOCK_SIZE,
        }
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
    /// lengths (also across [`Renderer::render_into`]). A plan without output renders
    /// nothing (and runs no box).
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::RenderTooLong`] when the samples cannot be held in memory.
    pub fn render<D: Delivered<S>>(&mut self, frames: usize) -> Result<Vec<D>, CoreError> {
        let length = frames
            .checked_mul(self.channels())
            .ok_or(CoreError::RenderTooLong(frames))?;
        let mut samples = Vec::new();
        samples
            .try_reserve_exact(length)
            .map_err(|_| CoreError::RenderTooLong(frames))?;
        samples.resize(length, D::deliver(S::ZERO));
        self.render_into(&mut samples);
        Ok(samples)
    }

    /// Renders the next frames into `out`, interleaved by channel and given as `D`
    /// ([`host::Delivered`](Delivered)): as many whole frames as `out` holds
    /// ([`Renderer::channels`] samples each). Samples after the last whole frame are left
    /// untouched; a plan without output writes nothing.
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
            if self.used == BLOCK_SIZE {
                self.plan.process(&mut self.last);
                self.used = 0;
            }
            let count = BLOCK_SIZE
                .saturating_sub(self.used)
                .min(rest.len().checked_div(channels).unwrap_or(0));
            for (offset, block) in self.last.iter().enumerate() {
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
            }
            self.used = self.used.saturating_add(count);
            rest = core::mem::take(&mut rest)
                .get_mut(count.saturating_mul(channels)..)
                .unwrap_or_default();
        }
    }
}
