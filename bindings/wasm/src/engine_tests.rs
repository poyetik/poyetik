//! Tests of the safe engine the C interface drives.

#![forbid(unsafe_code)]

use alloc::vec::Vec;

use engine_core::host::Raw;
use engine_core::{BLOCK_SIZE, Binary32, Binary64, Plan, RenderFormat, Renderer, SampleRate};
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
                bits.extend(channels.iter().map(|channel| channel[frame].bits()));
            }
            Blocks::Binary32(channels) => {
                bits.extend(channels.iter().map(|channel| channel[frame].bits()));
            }
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
    let wide = || {
        Renderer::new(
            Plan::<Binary64, Box<dyn engine_core::Object<Binary64>>>::build(patch, &REGISTRY, rate)
                .expect("a plan"),
        )
    };
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
        RenderFormat::Binary32 => Renderer::new(
            Plan::<Binary32, Box<dyn engine_core::Object<Binary32>>>::build(patch, &REGISTRY, rate)
                .expect("a plan"),
        )
        .render::<f32>(frames)
        .expect("small render")
        .iter()
        .map(|sample| u64::from(sample.to_bits()))
        .collect(),
    }
}

/// Two sines summed into the left channel of one `dac~`, the first alone on the right: a
/// channel that is a sum, flushed at delivery, and one that is not.
const SUMMED: &str = r#"{"format":1,"nodes":[{"id":1,"box":"osc~"},{"id":2,"box":"osc~","params":{"frequency":1000}},
    {"id":3,"box":"dac~"}],"connections":[{"from":[1,0],"to":[3,0]},{"from":[2,0],"to":[3,0]},{"from":[1,0],"to":[3,1]}]}"#;

#[test]
fn renders_exactly_like_the_plan_in_every_format_at_every_rate() {
    for text in [SINE, SUMMED] {
        renders_like_the_renderer(text);
    }
}

/// Renders `text` in every format at every rate of the matrix, as the renderer does.
fn renders_like_the_renderer(text: &str) {
    let patch = engine_patch::parse_patch(text).expect("valid patch");
    for format in RenderFormat::ALL {
        for rate in engine_testkit::matrix::sample_rates() {
            let mut engine = Engine::new(text.as_bytes(), rate.hz(), format).expect("valid patch");
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

/// A sine at 2^-126 Hz, the smallest normal binary32 frequency, at 8 kHz on both channels
/// (the conformance case `sine-1.1754944e-38-at-8k`): its first frames, about n · 2^-136,
/// are normal in binary64 and binary32 subnormals.
const SUBNORMAL_SINE: &str = r#"{"format":1,"nodes":[{"id":1,"box":"osc~","params":{"frequency":1.1754944e-38}},
    {"id":2,"box":"dac~"}],"connections":[{"from":[1,0],"to":[2,0]},{"from":[1,0],"to":[2,1]}]}"#;

#[test]
fn delivers_no_subnormal_in_any_format() {
    // Delivery is a flush point (decision 11), after the rounding to binary32: given as
    // binary32, the first block is all +0 (the binary32 box flushes its own outputs, the
    // binary64 render is flushed once rounded); binary64 keeps its normal samples; each
    // like the renderer.
    let patch = engine_patch::parse_patch(SUBNORMAL_SINE).expect("valid patch");
    let rate = SampleRate::new(8_000).expect("a supported rate");
    for format in RenderFormat::ALL {
        let mut engine =
            Engine::new(SUBNORMAL_SINE.as_bytes(), 8_000, format).expect("valid patch");
        let actual = interleaved(engine.process());
        let zeros = actual.iter().filter(|bits| **bits == 0).count();
        assert_eq!(
            zeros,
            if format == RenderFormat::Binary64 {
                2
            } else {
                2 * BLOCK_SIZE
            },
            "{format}: +0 at frame 0 only in binary64, everywhere given as binary32"
        );
        assert_eq!(
            actual,
            expected(&patch, rate, format, BLOCK_SIZE),
            "{format}, like the renderer"
        );
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
        let count = match blocks {
            Blocks::Binary64(blocks) => blocks.len(),
            Blocks::Binary32(blocks) => blocks.len(),
        };
        assert_eq!(count, 2, "{format}: one block per channel");
        assert!(!blocks.as_ptr().is_null(), "{format}: an address");
    }
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
            let mut engine = Engine::new(SINE.as_bytes(), rate.hz(), format).expect("valid patch");
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
