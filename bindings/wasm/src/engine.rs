//! One loaded patch, rendered one block at a time in its render format.

#![forbid(unsafe_code)]

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use engine_core::{Binary32, Binary64, Block, Plan, RenderFormat, Sample, SampleRate};
use engine_objects::{EnginePlan, REGISTRY};

/// The plan of a patch and its output buffers, in one render format.
#[derive(Debug)]
enum Rendering {
    /// Computed and given in binary64.
    Binary64 {
        /// The execution plan.
        plan: EnginePlan<Binary64>,
        /// The last rendered block, one per output channel.
        out: Vec<Block<Binary64>>,
    },
    /// Computed in binary64, given in binary32.
    Binary64AsBinary32 {
        /// The execution plan.
        plan: EnginePlan<Binary64>,
        /// The last rendered block, one per output channel, as computed.
        out: Vec<Block<Binary64>>,
        /// The same block rounded to binary32: what the host is given.
        given: Vec<Block<Binary32>>,
    },
    /// Computed and given in binary32.
    Binary32 {
        /// The execution plan.
        plan: EnginePlan<Binary32>,
        /// The last rendered block, one per output channel.
        out: Vec<Block<Binary32>>,
    },
}

/// The planar blocks of the last render, in the format of the render: one block per output
/// channel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Blocks<'engine> {
    /// Binary64 samples, 8 bytes each.
    Binary64(&'engine [Block<Binary64>]),
    /// Binary32 samples, 4 bytes each.
    Binary32(&'engine [Block<Binary32>]),
}

impl Blocks<'_> {
    /// The address of the first sample, as bytes.
    #[must_use]
    pub const fn as_ptr(&self) -> *const u8 {
        match self {
            Self::Binary64(blocks) => blocks.as_ptr().cast(),
            Self::Binary32(blocks) => blocks.as_ptr().cast(),
        }
    }
}

/// A validated patch ready to render in one render format, with its output buffers.
#[derive(Debug)]
pub struct Engine {
    /// Number of output channels.
    channels: usize,
    /// The plan and its buffers.
    rendering: Rendering,
}

/// Builds the plan of `patch` at `rate`, computing in `S`.
fn plan<S: Sample>(patch: &engine_core::Patch, rate: SampleRate) -> Result<EnginePlan<S>, String> {
    Plan::build(patch, &REGISTRY, rate).map_err(|error| error.to_string())
}

impl Engine {
    /// Reads a patch (UTF-8 text in the patch format) and builds its plan at `rate` hertz,
    /// rendering in `format`.
    ///
    /// # Errors
    ///
    /// Returns a message when the text is not UTF-8, the sample rate is not supported,
    /// or the patch is invalid.
    pub fn new(patch: &[u8], rate: u32, format: RenderFormat) -> Result<Self, String> {
        let text = core::str::from_utf8(patch)
            .map_err(|error| alloc::format!("patch is not UTF-8: {error}"))?;
        let rate = SampleRate::new(rate).map_err(|error| error.to_string())?;
        let parsed = engine_patch::parse_patch(text).map_err(|error| error.to_string())?;
        let (channels, rendering) = match format {
            RenderFormat::Binary64 => {
                let plan = plan::<Binary64>(&parsed, rate)?;
                let channels = plan.channels();
                (
                    channels,
                    Rendering::Binary64 {
                        plan,
                        out: vec![Binary64::SILENCE; channels],
                    },
                )
            }
            RenderFormat::Binary64AsBinary32 => {
                let plan = plan::<Binary64>(&parsed, rate)?;
                let channels = plan.channels();
                (
                    channels,
                    Rendering::Binary64AsBinary32 {
                        plan,
                        out: vec![Binary64::SILENCE; channels],
                        given: vec![Binary32::SILENCE; channels],
                    },
                )
            }
            RenderFormat::Binary32 => {
                let plan = plan::<Binary32>(&parsed, rate)?;
                let channels = plan.channels();
                (
                    channels,
                    Rendering::Binary32 {
                        plan,
                        out: vec![Binary32::SILENCE; channels],
                    },
                )
            }
        };
        Ok(Self {
            channels,
            rendering,
        })
    }

    /// Number of output channels.
    #[must_use]
    pub const fn channels(&self) -> usize {
        self.channels
    }

    /// Renders the next block and returns it, one block per output channel, in the render
    /// format: a binary64 render given as binary32 is rounded sample by sample
    /// ([`Sample::from_binary64`]).
    ///
    /// Does not use the heap (no allocation or release).
    pub fn process(&mut self) -> Blocks<'_> {
        match &mut self.rendering {
            Rendering::Binary64 { plan, out } => {
                plan.process(out);
                Blocks::Binary64(out)
            }
            Rendering::Binary64AsBinary32 { plan, out, given } => {
                plan.process(out);
                for (narrow, wide) in given.iter_mut().zip(out.iter()) {
                    for (sample, value) in narrow.iter_mut().zip(wide) {
                        *sample = Binary32::from_binary64(*value);
                    }
                }
                Blocks::Binary32(given)
            }
            Rendering::Binary32 { plan, out } => {
                plan.process(out);
                Blocks::Binary32(out)
            }
        }
    }
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
