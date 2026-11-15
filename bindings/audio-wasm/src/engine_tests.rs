//! Tests of the safe engine the C interface drives.

#![forbid(unsafe_code)]

use alloc::vec::Vec;

use audio_boxes::REGISTRY;
use audio_core::host::Raw;
use audio_core::{BLOCK_SIZE, Binary32, Binary64, Plan, RenderFormat, Renderer, SampleRate};

use super::{Blocks, Engine, Output};

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
    patch: &audio_core::Patch,
    rate: SampleRate,
    format: RenderFormat,
    frames: usize,
) -> Vec<u64> {
    let wide = || {
        Renderer::new(
            Plan::<Binary64, Box<dyn audio_core::Object<Binary64>>>::build(patch, &REGISTRY, rate)
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
            Plan::<Binary32, Box<dyn audio_core::Object<Binary32>>>::build(patch, &REGISTRY, rate)
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
    let patch = audio_patch::parse_patch(text).expect("valid patch");
    for format in RenderFormat::ALL {
        for rate in audio_testkit::matrix::sample_rates() {
            let mut engine =
                Engine::new(text.as_bytes(), rate.hz(), format, Output::RAW).expect("valid patch");
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
    let patch = audio_patch::parse_patch(SUBNORMAL_SINE).expect("valid patch");
    let rate = SampleRate::new(8_000).expect("a supported rate");
    for format in RenderFormat::ALL {
        let mut engine = Engine::new(SUBNORMAL_SINE.as_bytes(), 8_000, format, Output::RAW)
            .expect("valid patch");
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
        let mut engine =
            Engine::new(SINE.as_bytes(), 48_000, format, Output::RAW).expect("valid patch");
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
        audio_testkit::heap_operations_during(|| drop(Box::new(0_u8))),
        Ok(((), 2)),
        "heap operations are counted: an allocation and its release"
    );
    for format in RenderFormat::ALL {
        for rate in audio_testkit::matrix::sample_rates() {
            let mut engine =
                Engine::new(SINE.as_bytes(), rate.hz(), format, Output::RAW).expect("valid patch");
            let operations = audio_testkit::heap_operations_during(|| {
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
        let error = Engine::new(&[0xff], 48_000, format, Output::RAW).expect_err("invalid UTF-8");
        assert!(error.starts_with("patch is not UTF-8: "), "{error}");
    }
}

#[test]
fn rejects_an_unsupported_sample_rate() {
    let error =
        Engine::new(SINE.as_bytes(), 0, RenderFormat::Binary64, Output::RAW).expect_err("rate 0");
    assert_eq!(
        error, "invalid sample rate: 0 Hz (supported: 8000 to 384000 Hz)",
        "the rate and the range"
    );
}

#[test]
fn rejects_an_unreadable_patch() {
    let error =
        Engine::new(b"{}", 48_000, RenderFormat::Binary32, Output::RAW).expect_err("no format");
    assert_eq!(
        error, "invalid patch: missing field `format` at line 1 column 2",
        "explained"
    );
}

#[test]
fn rejects_an_invalid_plan_in_every_format() {
    let unknown = r#"{"format":1,"nodes":[{"id":1,"box":"nope~"}],"connections":[]}"#;
    for format in RenderFormat::ALL {
        let error =
            Engine::new(unknown.as_bytes(), 48_000, format, Output::RAW).expect_err("unknown box");
        assert!(error.contains("nope~"), "{format}: {error}");
    }
}

#[test]
fn gives_what_the_output_stage_replaced_as_four_words() {
    use audio_core::{FirstReplaced, Replaced};
    assert_eq!(
        super::report_words(Replaced::default()),
        [0, 0, 0, 0],
        "nothing replaced"
    );
    assert_eq!(
        super::report_words(Replaced {
            count: 3,
            first: Some(FirstReplaced {
                frame: 0x0000_0002_0000_0007,
                channel: 1,
            }),
        }),
        [3, 7, 2, 1],
        "the frame split in its low and high words"
    );
    assert_eq!(
        super::report_words(Replaced {
            count: u64::MAX,
            first: Some(FirstReplaced {
                frame: u64::MAX,
                channel: usize::MAX,
            }),
        }),
        [u32::MAX, u32::MAX, u32::MAX, u32::MAX],
        "the count and the channel saturate, every bit of the frame kept"
    );
}

#[test]
fn reports_what_its_stage_replaced_as_it_was_given() {
    // No built-in box gives a non-finite sample: each format's stage is fed one directly,
    // after a first block: as computed (counted in the next block, frame 64 + 3) or, given
    // as binary32, as given (counted in the block given last, frame 3).
    let block = u32::try_from(BLOCK_SIZE).expect("a frame count");
    for (format, frame) in RenderFormat::ALL.into_iter().zip([block + 3, 3, block + 3]) {
        let mut engine = Engine::new(SINE.as_bytes(), 48_000, format, Output::RAW).expect("a sine");
        let _first = engine.process();
        let replaced = |reporting: &mut Engine| {
            let pointer = reporting.replaced_report();
            assert!(
                core::ptr::eq(pointer, reporting.report.as_ptr()),
                "its own words"
            );
            reporting.report
        };
        assert_eq!(
            replaced(&mut engine),
            [0, 0, 0, 0],
            "{format}: a sine replaces nothing"
        );
        match &mut engine.rendering {
            super::Rendering::Binary64 { stage, .. } => {
                let mut nan = [<Binary64 as audio_core::Sample>::SILENCE];
                nan[0][3] = Binary64::from_raw(f64::NAN);
                stage.apply(&mut nan);
            }
            super::Rendering::Binary64AsBinary32 { stage, .. } => {
                let mut given = [<Binary32 as audio_core::Sample>::SILENCE];
                given[0][3] = Binary32::from_raw(f32::INFINITY);
                stage.replace_given(&mut given);
            }
            super::Rendering::Binary32 { stage, .. } => {
                let mut nan = [<Binary32 as audio_core::Sample>::SILENCE];
                nan[0][3] = Binary32::from_raw(f32::NAN);
                stage.apply(&mut nan);
            }
        }
        assert_eq!(
            replaced(&mut engine),
            [1, frame, 0, 0],
            "{format}: counted at the frame it was computed"
        );
    }
}

/// A loud stereo patch: two sines summed on both channels, well above the default ceiling,
/// so that a protected stage limits.
const LOUD: &str = r#"{"format":1,"nodes":[{"id":1,"box":"osc~"},{"id":2,"box":"osc~","params":{"frequency":1000}},
    {"id":3,"box":"osc~","params":{"frequency":3000}},{"id":4,"box":"dac~"}],
    "connections":[{"from":[1,0],"to":[4,0]},{"from":[2,0],"to":[4,0]},{"from":[3,0],"to":[4,0]},
    {"from":[1,0],"to":[4,1]},{"from":[2,0],"to":[4,1]}]}"#;

/// A protected output with a maximum lookahead of 2 ms and the default settings.
fn protected() -> Output {
    Output {
        mode: super::OutputMode::Protected,
        maximum_lookahead: Binary64::new(2.0),
        detector: audio_core::Detector::Precise,
        settings: audio_core::LimiterSettings::DEFAULT,
    }
}

/// What the native renderer gives for `frames` frames of `patch` at `rate` in `format`
/// through a protected stage like [`protected`]'s, widened to 64 bits.
fn expected_protected(
    patch: &audio_core::Patch,
    rate: SampleRate,
    format: RenderFormat,
    frames: usize,
) -> Vec<u64> {
    fn stage<S: audio_core::Sample>(
        rate: SampleRate,
        channels: usize,
    ) -> audio_core::OutputStage<S> {
        audio_core::OutputStage::limited(
            audio_core::TruePeakLimiter::new(
                rate,
                channels,
                Binary64::new(2.0),
                audio_core::Detector::Precise,
                audio_core::LimiterSettings::DEFAULT,
                true,
            )
            .expect("a limiter"),
        )
    }
    match format {
        RenderFormat::Binary32 => {
            let plan = Plan::<Binary32, Box<dyn audio_core::Object<Binary32>>>::build(
                patch, &REGISTRY, rate,
            )
            .expect("a plan");
            let channels = plan.channels();
            Renderer::with_stage(plan, stage(rate, channels))
                .render::<f32>(frames)
                .expect("small render")
                .iter()
                .map(|sample| u64::from(sample.to_bits()))
                .collect()
        }
        wide @ (RenderFormat::Binary64 | RenderFormat::Binary64AsBinary32) => {
            let plan = Plan::<Binary64, Box<dyn audio_core::Object<Binary64>>>::build(
                patch, &REGISTRY, rate,
            )
            .expect("a plan");
            let channels = plan.channels();
            let mut renderer = Renderer::with_stage(plan, stage(rate, channels));
            if wide == RenderFormat::Binary64 {
                renderer
                    .render::<f64>(frames)
                    .expect("small render")
                    .iter()
                    .map(|sample| sample.to_bits())
                    .collect()
            } else {
                renderer
                    .render::<f32>(frames)
                    .expect("small render")
                    .iter()
                    .map(|sample| u64::from(sample.to_bits()))
                    .collect()
            }
        }
    }
}

#[test]
fn limits_exactly_like_the_renderer_through_a_protected_stage_in_every_format_at_every_rate() {
    let patch = audio_patch::parse_patch(LOUD).expect("valid patch");
    for format in RenderFormat::ALL {
        for rate in audio_testkit::matrix::sample_rates() {
            let mut engine =
                Engine::new(LOUD.as_bytes(), rate.hz(), format, protected()).expect("valid patch");
            let blocks = 40_usize;
            let mut actual = Vec::new();
            for _ in 0..blocks {
                actual.extend(interleaved(engine.process()));
            }
            assert_eq!(
                actual,
                expected_protected(&patch, rate, format, blocks * BLOCK_SIZE),
                "{format}, bit-identical at {} Hz",
                rate.hz()
            );
            assert!(engine.latency() > 0, "{format}: a latency");
        }
    }
}

#[test]
fn changes_its_limiter_live_and_refuses_without_one() {
    let mut raw =
        Engine::new(LOUD.as_bytes(), 48_000, RenderFormat::Binary32, Output::RAW).expect("valid");
    assert_eq!(raw.latency(), 0, "raw: no latency");
    let refusal = Err("the output stage is raw: it has no limiter to set".to_owned());
    assert_eq!(
        raw.set_limiter(audio_core::LimiterSettings::DEFAULT),
        refusal,
        "no limiter to set"
    );
    assert_eq!(raw.set_protected(false), refusal, "nor to switch");
    for format in RenderFormat::ALL {
        let mut engine = Engine::new(LOUD.as_bytes(), 48_000, format, protected()).expect("valid");
        assert_eq!(engine.latency(), 96 + 16, "{format}: 2 ms at 48 kHz");
        let longer = audio_core::LimiterSettings::new(
            Binary64::new(-1.0),
            Binary64::new(3.0),
            Binary64::new(100.0),
            Binary64::new(0.5),
        )
        .expect("settings");
        assert_eq!(
            engine.set_limiter(longer),
            Err("limiter lookahead 3 ms is above its maximum of 2 ms".to_owned()),
            "{format}: above the maximum"
        );
        assert_eq!(
            engine.set_limiter(audio_core::LimiterSettings::DEFAULT),
            Ok(()),
            "{format}"
        );
        assert_eq!(
            engine.set_protected(false),
            Ok(()),
            "{format}: raw from now on"
        );
    }
    let wrong = Output {
        maximum_lookahead: Binary64::new(9.0),
        ..protected()
    };
    assert_eq!(
        Engine::new(LOUD.as_bytes(), 48_000, RenderFormat::Binary64, wrong).map(|_| ()),
        Err("limiter maximum lookahead 9 ms is out of range (0.5 to 5 ms)".to_owned()),
        "a maximum out of range"
    );
}
