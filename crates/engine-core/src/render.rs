//! Offline rendering: any number of frames, whatever the block boundaries.

use alloc::vec;
use alloc::vec::Vec;

use crate::{BLOCK_SIZE, Block, CoreError, Plan, SILENCE};

/// Renders a plan offline, frame by frame, as one continuous stream: rendering `a` then
/// `b` frames gives the same samples as rendering `a + b`.
///
/// It owns its [`Plan`] and offers no way to run [`Plan::process`] on it: a block
/// processed behind its back would reorder time (frames of a later block returned before
/// the rest of the current one). That mix does not compile:
///
/// ```compile_fail,E0599
/// fn mix(renderer: &mut engine_core::Renderer, out: &mut [engine_core::Block]) {
///     renderer.process(out);
/// }
/// ```
#[derive(Debug)]
pub struct Renderer {
    /// The plan, advanced only by this renderer.
    plan: Plan,
    /// The last block the plan produced, one per channel.
    last: Vec<Block>,
    /// Frames of `last` already returned; [`BLOCK_SIZE`] when none is left.
    used: usize,
}

impl Renderer {
    /// A renderer starting at the first frame of `plan`.
    #[must_use]
    pub fn new(plan: Plan) -> Self {
        Self {
            last: vec![SILENCE; plan.channels()],
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
    /// ([`Renderer::channels`] samples per frame).
    ///
    /// Successive calls continue exactly where the previous one stopped, whatever their
    /// lengths (also across [`Renderer::render_into`]). A plan without output renders
    /// nothing (and runs no box).
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::RenderTooLong`] when the samples cannot be held in memory.
    pub fn render(&mut self, frames: usize) -> Result<Vec<f32>, CoreError> {
        let length = frames
            .checked_mul(self.channels())
            .ok_or(CoreError::RenderTooLong(frames))?;
        let mut samples = Vec::new();
        samples
            .try_reserve_exact(length)
            .map_err(|_| CoreError::RenderTooLong(frames))?;
        samples.resize(length, 0.0);
        self.render_into(&mut samples);
        Ok(samples)
    }

    /// Renders the next frames into `out`, interleaved by channel: as many whole frames
    /// as `out` holds ([`Renderer::channels`] samples each). Samples after the last whole
    /// frame are left untouched; a plan without output writes nothing.
    ///
    /// Continues exactly where the previous render stopped, like [`Renderer::render`].
    /// Does not allocate. Copies one channel at a time, as many frames as both the
    /// current block and `out` still hold.
    pub fn render_into(&mut self, out: &mut [f32]) {
        let channels = self.channels();
        let Some(frames) = out.len().checked_div(channels) else {
            return;
        };
        let mut rest = out
            .get_mut(..frames.saturating_mul(channels))
            .unwrap_or_default();
        // Each round copies at least one frame, so it ends after at most `frames` rounds.
        while !rest.is_empty() {
            if self.used == BLOCK_SIZE {
                self.plan.process(&mut self.last);
                self.used = 0;
            }
            let count = BLOCK_SIZE
                .saturating_sub(self.used)
                .min(rest.len().checked_div(channels).unwrap_or(0));
            for (offset, block) in self.last.iter().enumerate() {
                let samples = rest.iter_mut().skip(offset).step_by(channels);
                for (sample, value) in samples.zip(block.iter().skip(self.used)).take(count) {
                    *sample = *value;
                }
            }
            self.used = self.used.saturating_add(count);
            rest = core::mem::take(&mut rest)
                .get_mut(count.saturating_mul(channels)..)
                .unwrap_or_default();
        }
    }
}
