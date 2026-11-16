//! One loaded patch, rendered one block at a time in its render format.

#![forbid(unsafe_code)]

use alloc::borrow::ToOwned;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use audio_boxes::{EnginePlan, REGISTRY};

use crate::ENGINE_BLOCK;
use audio_core::host::{Delivered, Raw};
use audio_core::{
    Binary32, Binary64, Block, Detector, LimiterError, LimiterSettings, OutputStage, Plan,
    RenderFormat, Replaced, Sample, SampleRate, Setting, TruePeakLimiter,
};

/// The plan of a patch, its output buffers and its output stage, in one render format.
#[derive(Debug)]
enum Rendering<const N: usize> {
    /// Computed and given in binary64.
    Binary64 {
        /// The execution plan.
        plan: EnginePlan<Binary64, N>,
        /// The last rendered block, one per output channel.
        out: Vec<Block<Binary64, N>>,
        /// The output stage, in the computing precision.
        stage: OutputStage<Binary64>,
    },
    /// Computed in binary64, given in binary32.
    Binary64AsBinary32 {
        /// The execution plan.
        plan: EnginePlan<Binary64, N>,
        /// The last rendered block, one per output channel, as computed.
        out: Vec<Block<Binary64, N>>,
        /// The same block rounded to binary32: what the host is given.
        given: Vec<Block<Binary32, N>>,
        /// The output stage, in the computing precision, before the rounding.
        stage: OutputStage<Binary64>,
    },
    /// Computed and given in binary32.
    Binary32 {
        /// The execution plan.
        plan: EnginePlan<Binary32, N>,
        /// The last rendered block, one per output channel.
        out: Vec<Block<Binary32, N>>,
        /// The output stage, in the computing precision.
        stage: OutputStage<Binary32>,
    },
}

/// What the output stage of an engine does (`spec/engine-design.md`, decision 12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    /// Raw, without a limiter: non-finite samples replaced, nothing else, no latency.
    Raw,
    /// Protected by the true-peak limiter, from the first block.
    Protected,
    /// With the limiter, raw from the first block, protected when the host switches it
    /// ([`Engine::set_protected`]): the latency of a protected stage, kept while raw.
    Switchable,
}

/// The output stage an engine is built with: its mode, and for a stage with a limiter, its
/// maximum lookahead and its detector (fixed with the engine) and its settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Output {
    /// The mode.
    pub mode: OutputMode,
    /// The maximum lookahead, in milliseconds.
    pub maximum_lookahead: Binary64,
    /// The precision of the limiter's peak detector.
    pub detector: Detector,
    /// The limiter's settings.
    pub settings: LimiterSettings,
}

impl Output {
    /// A raw stage: no limiter, no latency.
    pub const RAW: Self = Self {
        mode: OutputMode::Raw,
        // The largest maximum lookahead (`Setting::range`).
        maximum_lookahead: Setting::MaximumLookahead.range().1,
        detector: Detector::Precise,
        settings: LimiterSettings::DEFAULT,
    };

    /// The stage of this output for `channels` channels at `rate`, in `S`.
    fn stage<S: Sample>(
        self,
        rate: SampleRate,
        channels: usize,
    ) -> Result<OutputStage<S>, LimiterError> {
        let protected = match self.mode {
            OutputMode::Raw => return Ok(OutputStage::new()),
            OutputMode::Protected => true,
            OutputMode::Switchable => false,
        };
        TruePeakLimiter::new(
            rate,
            // A count beyond `u16` is beyond the limiter's largest too: refused as the
            // largest `u16`.
            u16::try_from(channels).unwrap_or(u16::MAX),
            self.maximum_lookahead,
            self.detector,
            self.settings,
            protected,
        )
        .map(OutputStage::limited)
    }
}

/// The planar blocks of the last render, in the format of the render: one block per output
/// channel.
#[derive(Debug, Clone, Copy, PartialEq)]
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
    /// Number of output channels.
    channels: usize,
    /// The plan, its buffers and its stage.
    rendering: Rendering<N>,
    /// What the stage replaced, as the four words of the C interface
    /// ([`Engine::replaced_report`]).
    report: [u32; 4],
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
    pub fn new(
        patch: &[u8],
        rate: u32,
        format: RenderFormat,
        output: Output,
    ) -> Result<Self, String> {
        let text = core::str::from_utf8(patch)
            .map_err(|error| alloc::format!("patch is not UTF-8: {error}"))?;
        let rate = SampleRate::new(rate).map_err(|error| error.to_string())?;
        let parsed = audio_patch::parse_patch(text).map_err(|error| error.to_string())?;
        let stage_error = |error: LimiterError| error.to_string();
        let (channels, rendering) = match format {
            RenderFormat::Binary64 => {
                let plan = plan::<Binary64, N>(&parsed, rate)?;
                let channels = plan.channels();
                let stage = output.stage(rate, channels).map_err(stage_error)?;
                (
                    channels,
                    Rendering::Binary64 {
                        plan,
                        out: vec![[Binary64::ZERO; N]; channels],
                        stage,
                    },
                )
            }
            RenderFormat::Binary64AsBinary32 => {
                let plan = plan::<Binary64, N>(&parsed, rate)?;
                let channels = plan.channels();
                let stage = output.stage(rate, channels).map_err(stage_error)?;
                (
                    channels,
                    Rendering::Binary64AsBinary32 {
                        plan,
                        out: vec![[Binary64::ZERO; N]; channels],
                        given: vec![[Binary32::ZERO; N]; channels],
                        stage,
                    },
                )
            }
            RenderFormat::Binary32 => {
                let plan = plan::<Binary32, N>(&parsed, rate)?;
                let channels = plan.channels();
                let stage = output.stage(rate, channels).map_err(stage_error)?;
                (
                    channels,
                    Rendering::Binary32 {
                        plan,
                        out: vec![[Binary32::ZERO; N]; channels],
                        stage,
                    },
                )
            }
        };
        Ok(Self {
            channels,
            rendering,
            report: [0; 4],
        })
    }

    /// The latency of its output stage, in frames: 0 for a raw stage.
    #[must_use]
    pub fn latency(&self) -> usize {
        match &self.rendering {
            Rendering::Binary64 { stage, .. } | Rendering::Binary64AsBinary32 { stage, .. } => {
                stage.latency()
            }
            Rendering::Binary32 { stage, .. } => stage.latency(),
        }
    }

    /// Changes the settings of its limiter, live.
    ///
    /// # Errors
    ///
    /// Returns a message for a raw stage, which has no limiter, and for a lookahead above
    /// the maximum it was built with.
    pub fn set_limiter(&mut self, settings: LimiterSettings) -> Result<(), String> {
        let set = match &mut self.rendering {
            Rendering::Binary64 { stage, .. } | Rendering::Binary64AsBinary32 { stage, .. } => {
                stage.limiter().map(|limiter| limiter.set(settings))
            }
            Rendering::Binary32 { stage, .. } => {
                stage.limiter().map(|limiter| limiter.set(settings))
            }
        };
        set.map_or_else(
            || Err(NO_LIMITER.to_owned()),
            |result| result.map_err(|error| error.to_string()),
        )
    }

    /// Starts the crossfade of its limiter toward protected (`true`) or raw (`false`).
    ///
    /// # Errors
    ///
    /// Returns a message for a raw stage, which has no limiter.
    pub fn set_protected(&mut self, protected: bool) -> Result<(), String> {
        let limiter_set = match &mut self.rendering {
            Rendering::Binary64 { stage, .. } | Rendering::Binary64AsBinary32 { stage, .. } => {
                stage
                    .limiter()
                    .map(|limiter| limiter.set_protected(protected))
            }
            Rendering::Binary32 { stage, .. } => stage
                .limiter()
                .map(|limiter| limiter.set_protected(protected)),
        };
        limiter_set.ok_or_else(|| NO_LIMITER.to_owned())
    }

    /// Number of output channels.
    #[must_use]
    pub const fn channels(&self) -> usize {
        self.channels
    }

    /// The address of what the output stage replaced since the first block (the non-finite
    /// samples, each given as zero), as four `u32` words, in order: the count, saturating at
    /// `u32::MAX`; the first sample's frame, its low then its high 32 bits; its channel,
    /// saturating at `u32::MAX`; the first sample's three words 0 while none was replaced.
    /// Valid until the next call into the engine.
    pub fn replaced_report(&mut self) -> *const u32 {
        let replaced = match &self.rendering {
            Rendering::Binary64 { stage, .. } | Rendering::Binary64AsBinary32 { stage, .. } => {
                stage.replaced()
            }
            Rendering::Binary32 { stage, .. } => stage.replaced(),
        };
        self.report = report_words(replaced);
        self.report.as_ptr()
    }

    /// Renders the next block and returns it, one block per output channel, in the render
    /// format, each sample given as [`Delivered`] gives it, through the output stage
    /// ([`OutputStage`]: a non-finite sample replaced by zero and reported,
    /// [`Engine::replaced_report`], limited when the stage has a limiter, in the computing
    /// precision): a binary64 render given as binary32 passes the stage in binary64
    /// ([`OutputStage::apply_before_narrowing`]), is rounded sample by sample, then has the
    /// samples the rounding made infinite replaced too ([`OutputStage::replace_given`]);
    /// and no sample is subnormal in the format given (delivery is a flush point,
    /// `spec/engine-design.md`, decision 11). A binary64 or
    /// binary32 block is handed over in place, only its channels that are sums flushed
    /// ([`Plan::flush_sums`]), where flushing could change a bit.
    ///
    /// Does not use the heap (no allocation or release).
    pub fn process(&mut self) -> Blocks<'_, N> {
        match &mut self.rendering {
            Rendering::Binary64 { plan, out, stage } => {
                plan.process(out);
                stage.apply(out);
                plan.flush_sums(out);
                Blocks::Binary64(out)
            }
            Rendering::Binary64AsBinary32 {
                plan,
                out,
                given,
                stage,
            } => {
                plan.process(out);
                stage.apply_before_narrowing(out);
                for (narrow, wide) in given.iter_mut().zip(out.iter()) {
                    for (sample, value) in narrow.iter_mut().zip(wide) {
                        *sample = Binary32::from_raw(<f32 as Delivered<Binary64>>::deliver(*value));
                    }
                }
                // As given: a finite binary64 sample beyond the largest binary32 one is
                // infinite once rounded, and replaced too.
                stage.replace_given(given);
                Blocks::Binary32(given)
            }
            Rendering::Binary32 { plan, out, stage } => {
                plan.process(out);
                stage.apply(out);
                plan.flush_sums(out);
                Blocks::Binary32(out)
            }
        }
    }
}

/// Why a stage without a limiter cannot change one.
const NO_LIMITER: &str = "the output stage is raw: it has no limiter to set";

/// `replaced` as the four words of the C interface (`engine_replaced`), in order: the count,
/// saturating at `u32::MAX`; the first sample's frame, its low then its high 32 bits; its
/// channel, saturating at `u32::MAX`; the first sample's three words 0 while none was
/// replaced.
#[must_use]
pub(crate) fn report_words(replaced: Replaced) -> [u32; 4] {
    let saturated = |value: u64| u32::try_from(value).unwrap_or(u32::MAX);
    let (frame, channel) = replaced.first.map_or((0, 0), |first| {
        (
            first.frame,
            u64::try_from(first.channel).unwrap_or(u64::MAX),
        )
    });
    [
        saturated(replaced.count),
        saturated(frame & u64::from(u32::MAX)),
        saturated(frame >> 32_u32),
        saturated(channel),
    ]
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
