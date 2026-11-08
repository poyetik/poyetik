//! One loaded patch, rendered one block at a time.

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use engine_core::{Block, Plan, SILENCE, SampleRate};
use engine_objects::REGISTRY;

/// A validated patch ready to render, with its output buffers.
#[derive(Debug)]
pub struct Engine {
    /// The execution plan of the patch.
    plan: Plan,
    /// The last rendered block, one per output channel.
    out: Vec<Block>,
}

impl Engine {
    /// Reads a patch (UTF-8 text in the patch format) and builds its plan at `rate` hertz.
    ///
    /// # Errors
    ///
    /// Returns a message when the text is not UTF-8, the sample rate is not supported,
    /// or the patch is invalid.
    pub fn new(patch: &[u8], rate: u32) -> Result<Self, String> {
        let text = core::str::from_utf8(patch)
            .map_err(|error| alloc::format!("patch is not UTF-8: {error}"))?;
        let rate = SampleRate::new(rate).map_err(|error| error.to_string())?;
        let parsed = engine_patch::parse_patch(text).map_err(|error| error.to_string())?;
        let plan = Plan::build(&parsed, &REGISTRY, rate).map_err(|error| error.to_string())?;
        Ok(Self {
            out: vec![SILENCE; plan.channels()],
            plan,
        })
    }

    /// Number of output channels.
    #[must_use]
    pub const fn channels(&self) -> usize {
        self.plan.channels()
    }

    /// Renders the next block and returns it, one block per output channel.
    ///
    /// Does not allocate.
    pub fn process(&mut self) -> &[Block] {
        self.plan.process(&mut self.out);
        &self.out
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use engine_core::{BLOCK_SIZE, Plan, SampleRate};
    use engine_objects::REGISTRY;

    use super::Engine;

    /// A 440 Hz sine on both channels.
    const SINE: &str = r#"{"format":1,"nodes":[{"id":1,"box":"osc~"},{"id":2,"box":"dac~"}],
        "connections":[{"from":[1,0],"to":[2,0]},{"from":[1,0],"to":[2,1]}]}"#;

    #[test]
    fn renders_exactly_like_the_plan() {
        let mut engine = Engine::new(SINE.as_bytes(), 48_000).expect("valid patch");
        assert_eq!(engine.channels(), 2, "stereo");
        let patch = engine_patch::parse_patch(SINE).expect("valid patch");
        let mut plan = Plan::build(&patch, &REGISTRY, SampleRate::HZ_48000).expect("valid plan");
        let blocks = 10_usize;
        let expected = plan.render(blocks * BLOCK_SIZE).expect("small render");
        let mut actual = Vec::new();
        for _ in 0..blocks {
            let out = engine.process();
            for frame in 0..BLOCK_SIZE {
                actual.extend(out.iter().map(|channel| channel[frame]));
            }
        }
        let bits = |samples: &[f32]| samples.iter().map(|s| s.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&actual), bits(&expected), "bit-identical");
    }

    #[test]
    fn renders_without_allocating() {
        let ((), probe) = engine_testkit::allocations_during(|| drop(Box::new(0_u8)));
        assert_eq!(probe, 1, "allocations are counted");
        for rate in engine_testkit::matrix::sample_rates() {
            let mut engine = Engine::new(SINE.as_bytes(), rate.hz()).expect("valid patch");
            let ((), allocations) = engine_testkit::allocations_during(|| {
                for _ in 0_u16..100_u16 {
                    let _: &[engine_core::Block] = engine.process();
                }
            });
            assert_eq!(allocations, 0, "100 blocks at {} Hz", rate.hz());
        }
    }

    #[test]
    fn rejects_text_that_is_not_utf8() {
        let error = Engine::new(&[0xff], 48_000).expect_err("invalid UTF-8");
        assert!(error.starts_with("patch is not UTF-8: "), "{error}");
    }

    #[test]
    fn rejects_an_unsupported_sample_rate() {
        let error = Engine::new(SINE.as_bytes(), 0).expect_err("rate 0");
        assert!(error.contains('0'), "{error}");
    }

    #[test]
    fn rejects_an_unreadable_patch() {
        let error = Engine::new(b"{}", 48_000).expect_err("no format");
        assert!(!error.is_empty(), "explained");
    }

    #[test]
    fn rejects_an_invalid_plan() {
        let unknown = r#"{"format":1,"nodes":[{"id":1,"box":"nope~"}],"connections":[]}"#;
        let error = Engine::new(unknown.as_bytes(), 48_000).expect_err("unknown box");
        assert!(error.contains("nope~"), "{error}");
    }
}
