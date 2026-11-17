//! Offline rendering: any number of frames, whatever the block boundaries.

#![cfg_attr(not(test), forbid(clippy::arithmetic_side_effects))]

use alloc::vec::Vec;

use crate::host::Delivered;
use crate::{BLOCK_SIZE, CoreError, Dispatch, OutputStage, Plan, Replaced, Sample};

/// Renders a plan computing in `S` offline, frame by frame, as one continuous stream.
///
/// It gives its samples as `D` ([`host::Delivered`](Delivered): `f64` or `f32` for a
/// binary64 render, `f32` for a binary32 one), a fact of the stream fixed when it is built,
/// as its output stage's ([`OutputStage`]); its plan holds its box instances as `I`, as
/// [`Plan`] says.
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
/// fn mix(renderer: &mut Renderer<Binary64, Virtual, f64>, out: &mut [Block<Binary64>]) {
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
/// fn mix(renderer: &mut Renderer<Binary64, Virtual, f64>, out: &mut [Block<Binary64>]) {
///     let _ = (renderer.channels(), out);
/// }
/// ```
///
/// The format given is its type's, never a call's: a render in another does not compile.
///
/// ```compile_fail,E0107
/// use audio_core::{Binary64, Object, Renderer};
///
/// fn wide_then_narrow(renderer: &mut Renderer<Binary64, Box<dyn Object<Binary64>>, f64>) {
///     let _ = renderer.render::<f32>(64);
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
pub struct Renderer<S: Sample, I, D: Delivered<S>, const N: usize = BLOCK_SIZE> {
    /// The plan, advanced only by this renderer.
    plan: Plan<S, I, N>,
    /// Frames of the stage's last block already returned; `N` when none is left.
    used: usize,
    /// The output stage every block passes before it is returned, which holds the last
    /// block the plan produced, one per channel.
    stage: OutputStage<S, D, N>,
    /// The frames of the render, when it has a known length ([`Renderer::lasting`]): none
    /// is rendered past them.
    length: Option<u64>,
    /// The frames rendered so far, saturating.
    rendered: u64,
}

impl<S: Sample, I: Dispatch<S, N>, D: Delivered<S>, const N: usize> Renderer<S, I, D, N> {
    /// A renderer starting at the first frame of `plan`, through a raw output stage
    /// ([`OutputStage::new`]: non-finite samples replaced, nothing limited, no latency).
    #[must_use]
    pub fn new(plan: Plan<S, I, N>) -> Self {
        let channels = plan.channels();
        Self::through(plan, OutputStage::new(channels))
    }

    /// A renderer starting at the first frame of `plan`, through `stage` (a limited one,
    /// [`OutputStage::limited`], delays every sample by its latency): a stage at its first
    /// frame, of as many channels as the plan has.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::StageChannels`] for a stage of another channel count than the
    /// plan's, and [`CoreError::UsedStage`] for a stage that applied a block already (its
    /// frames and its limiter's state belong to another stream).
    pub fn with_stage(plan: Plan<S, I, N>, stage: OutputStage<S, D, N>) -> Result<Self, CoreError> {
        if stage.channels() != plan.channels() {
            return Err(CoreError::StageChannels {
                plan: plan.channels(),
                stage: stage.channels(),
            });
        }
        if !stage.is_unused() {
            return Err(CoreError::UsedStage);
        }
        Ok(Self::through(plan, stage))
    }

    /// A renderer of `frames` frames of `plan`, through `stage`, as [`Renderer::with_stage`]
    /// checks them: it refuses to render past them ([`CoreError::PastTheEnd`]), and its
    /// stage reports on those frames only, not on what its last block computes past them or
    /// leaves in the limiter's delay, so that its report is the same at every engine block
    /// (decision 2).
    ///
    /// # Errors
    ///
    /// As [`Renderer::with_stage`].
    pub fn lasting(
        plan: Plan<S, I, N>,
        stage: OutputStage<S, D, N>,
        frames: u64,
    ) -> Result<Self, CoreError> {
        let mut renderer = Self::with_stage(plan, stage.reporting_until(frames))?;
        renderer.length = Some(frames);
        Ok(renderer)
    }

    /// Counts `frames` more frames rendered, unless they go past the length of the render.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::PastTheEnd`] when they do; nothing is then counted.
    fn take(&mut self, frames: usize) -> Result<(), CoreError> {
        let rendered = self
            .rendered
            .saturating_add(u64::try_from(frames).unwrap_or(u64::MAX));
        if let Some(length) = self.length
            && rendered > length
        {
            return Err(CoreError::PastTheEnd { length });
        }
        self.rendered = rendered;
        Ok(())
    }

    /// A renderer starting at the first frame of `plan`, through `stage`, checked by the
    /// caller.
    const fn through(plan: Plan<S, I, N>, stage: OutputStage<S, D, N>) -> Self {
        // A plan exists at a valid engine block only (`Plan::build`): restated where the
        // renderer is made, for the error to name it too.
        crate::block::engine_block::<N>();
        Self {
            plan,
            used: N,
            stage,
            length: None,
            rendered: 0,
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
    /// current block not returned yet (less than a block ahead, and the stage's latency),
    /// unless it renders a known length ([`Renderer::lasting`]), whose report is on the
    /// frames it returns only, the same at every engine block.
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
    /// Returns [`CoreError::PastTheEnd`] for frames past the length of a render of a known
    /// length ([`Renderer::lasting`]), and [`CoreError::RenderTooLong`] when the samples
    /// cannot be held in memory; nothing is rendered then.
    pub fn render(&mut self, frames: usize) -> Result<Vec<D>, CoreError> {
        // The plan's channels, read once: whether the render gives samples or only advances.
        let channels = self.plan.channels();
        let length = frames
            .checked_mul(channels)
            .ok_or(CoreError::RenderTooLong(frames))?;
        let mut samples = Vec::new();
        samples
            .try_reserve_exact(length)
            .map_err(|_| CoreError::RenderTooLong(frames))?;
        self.take(frames)?;
        samples.resize(length, D::deliver(S::ZERO));
        if channels == 0 {
            self.advance(frames);
        } else {
            self.fill(&mut samples);
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
                let _ = self.stage.apply_computed(|last| self.plan.process(last));
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
    /// Does not use the heap.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::PastTheEnd`] when `out` holds frames past the length of a render
    /// of a known length ([`Renderer::lasting`]); nothing is rendered then.
    pub fn render_into(&mut self, out: &mut [D]) -> Result<(), CoreError> {
        self.take(out.len().checked_div(self.channels()).unwrap_or(0))?;
        self.fill(out);
        Ok(())
    }

    /// Renders the next frames into `out`, as [`Renderer::render_into`] says, counted by the
    /// caller. Copies one channel at a time, as many frames as both the current block and
    /// `out` still hold.
    fn fill(&mut self, out: &mut [D]) {
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
            if self.used == N {
                let _ = self.stage.apply_computed(|last| self.plan.process(last));
                self.used = 0;
            }
            let count = N
                .saturating_sub(self.used)
                .min(rest.len().checked_div(channels).unwrap_or(0));
            let used = self.used;
            for offset in 0..channels {
                // The stage holds a block per channel of the plan (`with_stage`): each is
                // found.
                for block in Option::into_iter(self.stage.computed_blocks().get(offset)) {
                    let samples = rest.iter_mut().skip(offset).step_by(channels);
                    let values = samples.zip(block.iter().skip(used)).take(count);
                    // Decided once per channel, when the plan was built: only a sum can hold
                    // a subnormal in the plan's precision (`Delivered::deliver_unsummed`).
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
                // Given in a narrower format, a finite sample can round to infinity: the
                // stage replaces it as it is given (decision 12).
                let given = rest.iter_mut().skip(offset).step_by(channels).take(count);
                D::replace_overflowed(given, |index| {
                    self.stage.note_given(used.saturating_add(index), offset);
                });
            }
            self.used = self.used.saturating_add(count);
            rest = core::mem::take(&mut rest)
                .get_mut(count.saturating_mul(channels)..)
                .unwrap_or_default();
        }
    }
}
