//! One loaded patch, rendered one block at a time in its render format.

#![forbid(unsafe_code)]

use alloc::string::{String, ToString};

use audio_boxes::{EnginePlan, REGISTRY};
use audio_core::host::Delivered;

use crate::ENGINE_BLOCK;

use audio_core::{
    Binary32, Binary64, Block, Detector, LimiterError, LimiterMode, LimiterSettings, OutputStage,
    Plan, RenderFormat, Replaced, Sample, SampleRate, TruePeakLimiter,
};

/// The plan of a patch and its output stage, which holds its output blocks, in one render
/// format.
#[derive(Debug)]
enum Rendering<const N: usize> {
    /// Computed and given in binary64.
    Binary64 {
        /// The execution plan.
        plan: EnginePlan<Binary64, N>,
        /// The output stage, in the computing precision, holding the last rendered block,
        /// one per output channel.
        stage: OutputStage<Binary64, f64, N>,
    },
    /// Computed in binary64, given in binary32.
    Binary64AsBinary32 {
        /// The execution plan.
        plan: EnginePlan<Binary64, N>,
        /// The output stage, in the computing precision, before the rounding, holding the
        /// last rendered block as computed and as given.
        stage: OutputStage<Binary64, f32, N>,
    },
    /// Computed and given in binary32.
    Binary32 {
        /// The execution plan.
        plan: EnginePlan<Binary32, N>,
        /// The output stage, in the computing precision, holding the last rendered block,
        /// one per output channel.
        stage: OutputStage<Binary32, f32, N>,
    },
}

/// What the output stage of an engine does (`spec/engine-design.md`, decision 12), each at
/// the index of its code in the C interface ([`crate::Instance::output`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OutputMode {
    /// Raw, without a limiter: non-finite samples replaced, nothing else, no latency.
    Raw,
    /// Protected by the true-peak limiter, from the first block, and never switched to raw
    /// (its stage refuses, `audio_core::LimiterMode::Protected`, and so does
    /// [`Engine::set_protected`]).
    Protected,
    /// With the limiter, raw from the first block, protected when the host switches it
    /// ([`Engine::set_protected`]): the latency of a protected stage, kept while raw.
    Switchable,
}

impl OutputMode {
    /// Every mode, in the order of their codes in the C interface: a new mode is added here,
    /// and every list derived from it (codes, names, messages) follows.
    pub const ALL: [Self; 3] = [Self::Raw, Self::Protected, Self::Switchable];

    /// Its name, as the interfaces and the cases write it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Protected => "protected",
            Self::Switchable => "switchable",
        }
    }
}

/// What an output stage with the true-peak limiter is built with beside its mode.
///
/// The maximum lookahead and the detector fix the limiter's latency for the life of the
/// engine; the limiter's settings change live.
///
/// The limiter may gain a setting built with it, so that it is built by
/// [`LimiterStage::new`], never by a literal outside this crate, which a new field would
/// break:
///
/// ```compile_fail,E0639
/// use audio_core::{Binary64, Detector, LimiterSettings};
/// let _ = audio_host::LimiterStage {
///     maximum_lookahead: Binary64::new(5.0),
///     detector: Detector::Precise,
///     limiter: LimiterSettings::DEFAULT,
/// };
/// ```
///
/// ```
/// use audio_core::{Binary64, Detector, LimiterSettings};
/// let stage =
///     audio_host::LimiterStage::new(Binary64::new(5.0), Detector::Precise, LimiterSettings::DEFAULT);
/// assert_eq!(stage.detector, Detector::Precise);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct LimiterStage {
    /// The maximum lookahead, in milliseconds.
    pub maximum_lookahead: Binary64,
    /// The limiter's peak detector.
    pub detector: Detector,
    /// The limiter's settings.
    pub limiter: LimiterSettings,
}

impl LimiterStage {
    /// The stage of a limiter of `maximum_lookahead` milliseconds at most, with `detector`,
    /// starting with `limiter`'s settings; checked when an engine builds it
    /// ([`Engine::new`]).
    #[must_use]
    pub const fn new(
        maximum_lookahead: Binary64,
        detector: Detector,
        limiter: LimiterSettings,
    ) -> Self {
        Self::made(maximum_lookahead, detector, limiter)
    }

    /// [`LimiterStage::new`]'s body, a function of its own so that mutation testing reaches it:
    /// cargo-mutants never mutates a function named `new`.
    const fn made(
        maximum_lookahead: Binary64,
        detector: Detector,
        limiter: LimiterSettings,
    ) -> Self {
        Self {
            maximum_lookahead,
            detector,
            limiter,
        }
    }
}

/// The output stage an engine is built with: raw, which has no limiter and so no limiter
/// setting, or with the true-peak limiter, protected or switchable, and its settings.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum Output {
    /// Raw: no limiter, no latency.
    Raw,
    /// Protected by the limiter from the first block, never switched to raw
    /// ([`OutputMode::Protected`]).
    Protected(LimiterStage),
    /// With the limiter, raw first, protected live ([`OutputMode::Switchable`]).
    Switchable(LimiterStage),
}

impl Output {
    /// The stage of this output for `channels` channels at `rate`, in `S`, in blocks of `N`
    /// frames: of exactly `channels` channels, its limiter's too (decision 12), so that its
    /// count is the plan's by construction.
    fn stage<S: Sample, D: Delivered<S>, const N: usize>(
        self,
        rate: SampleRate,
        channels: usize,
    ) -> Result<OutputStage<S, D, N>, LimiterError> {
        let (stage, mode) = match self {
            Self::Raw => return Ok(OutputStage::new(channels)),
            Self::Protected(stage) => (stage, LimiterMode::Protected),
            Self::Switchable(stage) => (stage, LimiterMode::Switchable),
        };
        TruePeakLimiter::new(
            rate,
            // A count beyond `u16` is beyond the limiter's largest too: refused as the
            // largest `u16`.
            u16::try_from(channels).unwrap_or(u16::MAX),
            stage.maximum_lookahead,
            stage.detector,
            stage.limiter,
            mode,
        )
        .map(OutputStage::limited)
    }
}

/// The planar blocks of the last render, in the format of the render: one block per output
/// channel.
#[derive(Debug, Clone, Copy, PartialEq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "one variant per sample type a host is given (decision 10), closed as `RenderFormat`"
)]
pub enum Blocks<'engine, const N: usize> {
    /// Binary64 samples, 8 bytes each.
    Binary64(&'engine [Block<Binary64, N>]),
    /// Binary32 samples, 4 bytes each.
    Binary32(&'engine [Block<Binary32, N>]),
}

impl<const N: usize> Blocks<'_, N> {
    /// The address of the first sample, as bytes.
    #[must_use]
    pub const fn as_ptr(&self) -> *const u8 {
        match self {
            Self::Binary64(blocks) => blocks.as_ptr().cast(),
            Self::Binary32(blocks) => blocks.as_ptr().cast(),
        }
    }
}

/// A validated patch ready to render in one render format, with its output buffers and
/// its output stage.
///
/// Natively, it is built, rendered and set inside `fpenv::with_default_mode`, a
/// precondition in prose until #32 step 17 (the crate's doc).
#[derive(Debug)]
pub struct Engine<const N: usize = ENGINE_BLOCK> {
    /// The plan, its buffers and its stage, which holds the output channels.
    rendering: Rendering<N>,
}

/// Builds the plan of `patch` at `rate`, computing in `S`.
fn plan<S: Sample, const N: usize>(
    patch: &audio_core::Patch,
    rate: SampleRate,
) -> Result<EnginePlan<S, N>, String> {
    Plan::build(patch, &REGISTRY, rate, audio_core::ExecutionMode::Live)
        .map_err(|error| error.to_string())
}

impl<const N: usize> Engine<N> {
    /// Reads a patch (UTF-8 text in the patch format) and builds its plan at `rate` hertz,
    /// rendering in `format`, through the output stage `output`.
    ///
    /// # Errors
    ///
    /// Returns a message when the text is not UTF-8, the sample rate is not supported,
    /// the patch is invalid, its plan cannot be built live (a box of an offline-only box
    /// type, `CoreError::OfflineOnly`, or of a box type this build does not hold at its
    /// engine block, `CoreError::Unbuilt`), or the output's limiter cannot be built (a
    /// maximum lookahead out of range, or below its lookahead).
    ///
    /// Compiles for an engine block `N` of 1 to `audio_core::MAX_ENGINE_BLOCK` frames only
    /// (decision 2), neither 0 nor one more, since its plan does (`audio_core::Plan::build`):
    ///
    /// ```compile_fail,E0080
    /// use audio_core::RenderFormat;
    /// use audio_host::{Engine, Output};
    /// let _ = Engine::<0>::new(b"", 48_000, RenderFormat::Binary64, Output::Raw);
    /// ```
    ///
    /// ```compile_fail,E0080
    /// use audio_core::RenderFormat;
    /// use audio_host::{Engine, Output};
    /// let _ = Engine::<1_048_577>::new(b"", 48_000, RenderFormat::Binary64, Output::Raw);
    /// ```
    ///
    /// At both ends it compiles (an empty text is no patch):
    ///
    /// ```
    /// use audio_core::{MAX_ENGINE_BLOCK, RenderFormat};
    /// use audio_host::{Engine, Output};
    /// assert!(Engine::<1>::new(b"", 48_000, RenderFormat::Binary64, Output::Raw).is_err());
    /// assert!(
    ///     Engine::<MAX_ENGINE_BLOCK>::new(b"", 48_000, RenderFormat::Binary64, Output::Raw)
    ///         .is_err()
    /// );
    /// ```
    pub fn new(
        patch: &[u8],
        rate: u32,
        format: RenderFormat,
        output: Output,
    ) -> Result<Self, String> {
        Self::made(patch, rate, format, output)
    }

    /// [`Engine::new`]'s body, a function of its own so that mutation testing reaches it:
    /// cargo-mutants never mutates a function named `new`.
    fn made(patch: &[u8], rate: u32, format: RenderFormat, output: Output) -> Result<Self, String> {
        let text = core::str::from_utf8(patch)
            .map_err(|error| alloc::format!("patch is not UTF-8: {error}"))?;
        let rate = SampleRate::new(rate).map_err(|error| error.to_string())?;
        let parsed = audio_patch::parse_patch(text).map_err(|error| error.to_string())?;
        let stage_error = |error: LimiterError| error.to_string();
        let rendering = match format {
            RenderFormat::Binary64 => {
                let plan = plan::<Binary64, N>(&parsed, rate)?;
                let stage = output.stage(rate, plan.channels()).map_err(stage_error)?;
                Rendering::Binary64 { plan, stage }
            }
            RenderFormat::Binary64AsBinary32 => {
                let plan = plan::<Binary64, N>(&parsed, rate)?;
                let stage = output.stage(rate, plan.channels()).map_err(stage_error)?;
                Rendering::Binary64AsBinary32 { plan, stage }
            }
            RenderFormat::Binary32 => {
                let plan = plan::<Binary32, N>(&parsed, rate)?;
                let stage = output.stage(rate, plan.channels()).map_err(stage_error)?;
                Rendering::Binary32 { plan, stage }
            }
        };
        Ok(Self { rendering })
    }

    /// The latency of its output stage, in frames: 0 for a raw stage.
    #[must_use]
    pub fn latency(&self) -> usize {
        match &self.rendering {
            Rendering::Binary64 { stage, .. } => stage.latency(),
            Rendering::Binary64AsBinary32 { stage, .. } => stage.latency(),
            Rendering::Binary32 { stage, .. } => stage.latency(),
        }
    }

    /// The lookahead its limiter asked for last, in milliseconds, while it is pending: a
    /// lookahead set while its input needs a reduction (a switchable stage's switched to raw
    /// too, its limiter running unheard) takes effect at its next rest
    /// (`audio_core::TruePeakLimiter::pending_lookahead`); none once in force, or for a stage
    /// built raw, which has no limiter. Does not use the heap.
    #[must_use]
    pub fn pending_lookahead(&self) -> Option<Binary64> {
        match &self.rendering {
            Rendering::Binary64 { stage, .. } => stage.pending_lookahead(),
            Rendering::Binary64AsBinary32 { stage, .. } => stage.pending_lookahead(),
            Rendering::Binary32 { stage, .. } => stage.pending_lookahead(),
        }
    }

    /// Changes the settings of its limiter, live.
    ///
    /// # Errors
    ///
    /// Returns a message for a raw stage, which has no limiter, and for a lookahead above
    /// the maximum it was built with.
    pub fn set_limiter(&mut self, settings: LimiterSettings) -> Result<(), String> {
        match &mut self.rendering {
            Rendering::Binary64 { stage, .. } => stage.set_limiter(settings),
            Rendering::Binary64AsBinary32 { stage, .. } => stage.set_limiter(settings),
            Rendering::Binary32 { stage, .. } => stage.set_limiter(settings),
        }
        .map_err(|error| error.to_string())
    }

    /// Starts the crossfade of its limiter toward protected (`true`) or raw (`false`): a
    /// switchable stage only is switched both ways; a protected one stays protected
    /// (`spec/engine-design.md`, decision 12), told so again or refused by its stage
    /// (`OutputStage::set_protected`), whose refusal it relays.
    ///
    /// # Errors
    ///
    /// Returns the stage's refusal in words: for a raw stage, which has no limiter, and for a
    /// protected stage told to go raw, which stays protected (`audio_core::StageError`).
    pub fn set_protected(&mut self, protected: bool) -> Result<(), String> {
        match &mut self.rendering {
            Rendering::Binary64 { stage, .. } => stage.set_protected(protected),
            Rendering::Binary64AsBinary32 { stage, .. } => stage.set_protected(protected),
            Rendering::Binary32 { stage, .. } => stage.set_protected(protected),
        }
        .map_err(|error| error.to_string())
    }

    /// Number of output channels: its stage's, built for its plan's
    /// ([`OutputStage::channels`]).
    #[must_use]
    pub const fn channels(&self) -> usize {
        match &self.rendering {
            Rendering::Binary64 { stage, .. } => stage.channels(),
            Rendering::Binary64AsBinary32 { stage, .. } => stage.channels(),
            Rendering::Binary32 { stage, .. } => stage.channels(),
        }
    }

    /// What its output stage replaced since the first block: the non-finite samples, each
    /// given as zero, how many and the first ([`Replaced`]), counted as [`Engine::process`]
    /// says.
    #[must_use]
    pub const fn replaced(&self) -> Replaced {
        match &self.rendering {
            Rendering::Binary64 { stage, .. } => stage.replaced(),
            Rendering::Binary64AsBinary32 { stage, .. } => stage.replaced(),
            Rendering::Binary32 { stage, .. } => stage.replaced(),
        }
    }

    /// Renders the next block and returns it, one block per output channel, in the render
    /// format, each sample given as [`Delivered`] gives it, through the output stage
    /// ([`OutputStage`]: a non-finite sample replaced by zero and reported,
    /// [`Engine::replaced`], limited when the stage has a limiter, in the computing
    /// precision): a binary64 render given as binary32 passes the stage in binary64, is
    /// rounded sample by sample, then has the samples the rounding made infinite replaced
    /// too ([`OutputStage::apply_narrowing`]);
    /// and no sample is subnormal in the format given (delivery is a flush point,
    /// `spec/engine-design.md`, decision 11). A binary64 or
    /// binary32 block is handed over in place, only its channels that are sums flushed
    /// ([`Plan::flush_sums`]), where flushing could change a bit.
    ///
    /// Does not use the heap (no allocation or release).
    pub fn process(&mut self) -> Blocks<'_, N> {
        // The stage takes its blocks as one slice before the plan runs, as computed and as
        // given: their addresses and lengths read once per block, not again after each call
        // (`spec/benchmarks.md`, "The buffers taken as slices once per block").
        match &mut self.rendering {
            Rendering::Binary64 { plan, stage } => {
                let out = stage.apply(|out| plan.process(out));
                plan.flush_sums(out);
                Blocks::Binary64(out)
            }
            Rendering::Binary64AsBinary32 { plan, stage } => {
                // As given: a finite binary64 sample beyond the largest binary32 one is
                // infinite once rounded, and replaced too.
                Blocks::Binary32(stage.apply_narrowing(|out| plan.process(out)))
            }
            Rendering::Binary32 { plan, stage } => {
                let out = stage.apply(|out| plan.process(out));
                plan.flush_sums(out);
                Blocks::Binary32(out)
            }
        }
    }
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
