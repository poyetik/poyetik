//! One loaded patch, rendered one block at a time in its render format.

#![forbid(unsafe_code)]

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use engine_core::{Block, Plan, RenderFormat, Sample, SampleRate};
use engine_objects::REGISTRY;

/// The plan of a patch and its output buffers, in one render format.
#[derive(Debug)]
enum Rendering {
    /// Computed and given in binary64.
    Binary64 {
        /// The execution plan.
        plan: Plan<f64>,
        /// The last rendered block, one per output channel.
        out: Vec<Block<f64>>,
    },
    /// Computed in binary64, given in binary32.
    Binary64AsBinary32 {
        /// The execution plan.
        plan: Plan<f64>,
        /// The last rendered block, one per output channel, as computed.
        out: Vec<Block<f64>>,
        /// The same block rounded to binary32: what the host is given.
        given: Vec<Block<f32>>,
    },
    /// Computed and given in binary32.
    Binary32 {
        /// The execution plan.
        plan: Plan<f32>,
        /// The last rendered block, one per output channel.
        out: Vec<Block<f32>>,
    },
}

/// The planar blocks of the last render, in the format of the render: one block per output
/// channel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Blocks<'engine> {
    /// Binary64 samples, 8 bytes each.
    Binary64(&'engine [Block<f64>]),
    /// Binary32 samples, 4 bytes each.
    Binary32(&'engine [Block<f32>]),
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

    /// The number of blocks: one per output channel.
    #[must_use]
    pub const fn len(&self) -> usize {
        match self {
            Self::Binary64(blocks) => blocks.len(),
            Self::Binary32(blocks) => blocks.len(),
        }
    }

    /// Whether there is no block.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
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
fn plan<S: Sample>(patch: &engine_core::Patch, rate: SampleRate) -> Result<Plan<S>, String> {
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
                let plan = plan::<f64>(&parsed, rate)?;
                let channels = plan.channels();
                (
                    channels,
                    Rendering::Binary64 {
                        plan,
                        out: vec![f64::SILENCE; channels],
                    },
                )
            }
            RenderFormat::Binary64AsBinary32 => {
                let plan = plan::<f64>(&parsed, rate)?;
                let channels = plan.channels();
                (
                    channels,
                    Rendering::Binary64AsBinary32 {
                        plan,
                        out: vec![f64::SILENCE; channels],
                        given: vec![f32::SILENCE; channels],
                    },
                )
            }
            RenderFormat::Binary32 => {
                let plan = plan::<f32>(&parsed, rate)?;
                let channels = plan.channels();
                (
                    channels,
                    Rendering::Binary32 {
                        plan,
                        out: vec![f32::SILENCE; channels],
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
    /// format: a 64-bit render given as binary32 is rounded sample by sample
    /// ([`Sample::to_binary32`]).
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
                        *sample = value.to_binary32();
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
mod tests {
    use alloc::vec::Vec;

    use engine_core::{BLOCK_SIZE, Plan, RenderFormat, Renderer, SampleRate};
    use engine_objects::REGISTRY;

    use super::{Blocks, Engine};

    /// A 440 Hz sine on both channels.
    const SINE: &str = r#"{"format":1,"nodes":[{"id":1,"box":"osc~"},{"id":2,"box":"dac~"}],
        "connections":[{"from":[1,0],"to":[2,0]},{"from":[1,0],"to":[2,1]}]}"#;

    /// The bits of `blocks`, interleaved by channel, widened to 64 bits.
    fn interleaved(blocks: Blocks<'_>) -> Vec<u64> {
        let mut bits = Vec::new();
        for frame in 0..BLOCK_SIZE {
            match blocks {
                Blocks::Binary64(channels) => {
                    bits.extend(channels.iter().map(|channel| channel[frame].to_bits()));
                }
                Blocks::Binary32(channels) => bits.extend(
                    channels
                        .iter()
                        .map(|channel| u64::from(channel[frame].to_bits())),
                ),
            }
        }
        bits
    }

    /// What the renderer gives for `blocks` blocks of `patch` at `rate` in `format`, widened
    /// to 64 bits.
    fn expected(
        patch: &engine_core::Patch,
        rate: SampleRate,
        format: RenderFormat,
        frames: usize,
    ) -> Vec<u64> {
        let wide = || Renderer::new(Plan::<f64>::build(patch, &REGISTRY, rate).expect("a plan"));
        match format {
            RenderFormat::Binary64 => wide()
                .render::<f64>(frames)
                .expect("small render")
                .iter()
                .map(|sample| sample.to_bits())
                .collect(),
            RenderFormat::Binary64AsBinary32 => wide()
                .render::<f32>(frames)
                .expect("small render")
                .iter()
                .map(|sample| u64::from(sample.to_bits()))
                .collect(),
            RenderFormat::Binary32 => {
                Renderer::new(Plan::<f32>::build(patch, &REGISTRY, rate).expect("a plan"))
                    .render::<f32>(frames)
                    .expect("small render")
                    .iter()
                    .map(|sample| u64::from(sample.to_bits()))
                    .collect()
            }
        }
    }

    #[test]
    fn renders_exactly_like_the_plan_in_every_format_at_every_rate() {
        let patch = engine_patch::parse_patch(SINE).expect("valid patch");
        for format in RenderFormat::ALL {
            for rate in engine_testkit::matrix::sample_rates() {
                let mut engine =
                    Engine::new(SINE.as_bytes(), rate.hz(), format).expect("valid patch");
                assert_eq!(engine.channels(), 2, "stereo");
                let blocks = 10_usize;
                let mut actual = Vec::new();
                for _ in 0..blocks {
                    actual.extend(interleaved(engine.process()));
                }
                assert_eq!(
                    actual,
                    expected(&patch, rate, format, blocks * BLOCK_SIZE),
                    "{format}, bit-identical at {} Hz",
                    rate.hz()
                );
            }
        }
    }

    #[test]
    fn gives_each_format_in_its_own_width() {
        for (format, wide) in RenderFormat::ALL.into_iter().zip([true, false, false]) {
            let mut engine = Engine::new(SINE.as_bytes(), 48_000, format).expect("valid patch");
            let blocks = engine.process();
            assert_eq!(
                matches!(blocks, Blocks::Binary64(_)),
                wide,
                "{format}: binary64 blocks"
            );
            assert_eq!((blocks.len(), blocks.is_empty()), (2, false), "{format}");
            assert!(!blocks.as_ptr().is_null(), "{format}: an address");
        }
        let none = Blocks::Binary64(&[]);
        assert_eq!((none.len(), none.is_empty()), (0, true), "no block");
    }

    #[test]
    fn renders_without_a_heap_operation() {
        assert_eq!(
            engine_testkit::heap_operations_during(|| drop(Box::new(0_u8))),
            Ok(((), 2)),
            "heap operations are counted: an allocation and its release"
        );
        for format in RenderFormat::ALL {
            for rate in engine_testkit::matrix::sample_rates() {
                let mut engine =
                    Engine::new(SINE.as_bytes(), rate.hz(), format).expect("valid patch");
                let operations = engine_testkit::heap_operations_during(|| {
                    for _ in 0_u16..100_u16 {
                        let _: Blocks<'_> = engine.process();
                    }
                });
                assert_eq!(
                    operations,
                    Ok(((), 0)),
                    "{format}: 100 blocks at {} Hz",
                    rate.hz()
                );
            }
        }
    }

    #[test]
    fn rejects_text_that_is_not_utf8() {
        for format in RenderFormat::ALL {
            let error = Engine::new(&[0xff], 48_000, format).expect_err("invalid UTF-8");
            assert!(error.starts_with("patch is not UTF-8: "), "{error}");
        }
    }

    #[test]
    fn rejects_an_unsupported_sample_rate() {
        let error = Engine::new(SINE.as_bytes(), 0, RenderFormat::Binary64).expect_err("rate 0");
        assert_eq!(
            error, "invalid sample rate: 0 Hz (supported: 8000 to 384000 Hz)",
            "the rate and the range"
        );
    }

    #[test]
    fn rejects_an_unreadable_patch() {
        let error = Engine::new(b"{}", 48_000, RenderFormat::Binary32).expect_err("no format");
        assert_eq!(
            error, "invalid patch: missing field `format` at line 1 column 2",
            "explained"
        );
    }

    #[test]
    fn rejects_an_invalid_plan_in_every_format() {
        let unknown = r#"{"format":1,"nodes":[{"id":1,"box":"nope~"}],"connections":[]}"#;
        for format in RenderFormat::ALL {
            let error = Engine::new(unknown.as_bytes(), 48_000, format).expect_err("unknown box");
            assert!(error.contains("nope~"), "{format}: {error}");
        }
    }
}
