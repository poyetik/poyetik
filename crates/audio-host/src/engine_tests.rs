//! Tests of the safe engine the C interface drives.

#![forbid(unsafe_code)]

extern crate std;

use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use audio_boxes::REGISTRY;
use audio_core::host::Raw;
use audio_core::{
    BLOCK_SIZE, Binary32, Binary64, FirstReplaced, Plan, RenderFormat, Renderer, Replaced,
    SampleRate,
};

use super::{Blocks, Engine, LimiterStage, Output};
use crate::ENGINE_BLOCK;

/// A 440 Hz sine on both channels.
const SINE: &str = r#"{"format":1,"nodes":[{"id":1,"box":"osc~"},{"id":2,"box":"dac~"}],
    "connections":[{"from":[1,0],"to":[2,0]},{"from":[1,0],"to":[2,1]}]}"#;

/// The bits of `blocks`, interleaved by channel, widened to 64 bits.
fn interleaved<const N: usize>(blocks: Blocks<'_, N>) -> Vec<u64> {
    let mut bits = Vec::new();
    for frame in 0..N {
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
        Plan::<Binary64, Box<dyn audio_core::Object<Binary64>>>::build(
            patch,
            &REGISTRY,
            rate,
            audio_core::ExecutionMode::Offline,
        )
        .expect("a plan")
    };
    match format {
        RenderFormat::Binary64 => Renderer::<_, _, f64>::new(wide())
            .render(frames)
            .expect("small render")
            .iter()
            .map(|sample| sample.to_bits())
            .collect(),
        RenderFormat::Binary64AsBinary32 => Renderer::<_, _, f32>::new(wide())
            .render(frames)
            .expect("small render")
            .iter()
            .map(|sample| u64::from(sample.to_bits()))
            .collect(),
        RenderFormat::Binary32 => Renderer::<_, _, f32>::new(
            Plan::<Binary32, Box<dyn audio_core::Object<Binary32>>>::build(
                patch,
                &REGISTRY,
                rate,
                audio_core::ExecutionMode::Offline,
            )
            .expect("a plan"),
        )
        .render(frames)
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
    assert_eq!(
        audio_core::ENGINE_BLOCKS,
        [16, 32, 64, 128, 256, 512, 1024],
        "every engine block a build offers, each rendered below"
    );
    for text in [SINE, SUMMED] {
        // At every engine block a module is built for: the renderer's samples (the
        // engine-block law, decision 2).
        renders_like_the_renderer::<16>(text);
        renders_like_the_renderer::<32>(text);
        renders_like_the_renderer::<64>(text);
        renders_like_the_renderer::<128>(text);
        renders_like_the_renderer::<256>(text);
        renders_like_the_renderer::<512>(text);
        renders_like_the_renderer::<1024>(text);
    }
}

#[test]
fn renders_in_the_largest_engine_block_like_the_renderer() {
    // Blocks of `MAX_ENGINE_BLOCK` frames (8 MiB in binary64), the engine's buffers made on
    // the heap: a debug build renders them on a test thread's 2 MiB. One block of each
    // format at one rate, the matrix's other rates at the smaller sizes above.
    const LARGEST: usize = audio_core::MAX_ENGINE_BLOCK;
    let patch = audio_patch::parse_patch(SINE).expect("valid patch");
    let rate = SampleRate::new(44_100).expect("a valid rate");
    for format in RenderFormat::ALL {
        let mut engine = Engine::<LARGEST>::new(SINE.as_bytes(), rate.hz(), format, Output::Raw)
            .expect("valid patch");
        assert_eq!(
            interleaved(engine.process()),
            expected(&patch, rate, format, LARGEST),
            "{format}, bit-identical in blocks of {LARGEST}"
        );
    }
}

/// Renders `text` in every format at every rate of the matrix, in blocks of `N` frames, as
/// the renderer does in its default blocks.
fn renders_like_the_renderer<const N: usize>(text: &str) {
    let patch = audio_patch::parse_patch(text).expect("valid patch");
    for format in RenderFormat::ALL {
        for rate in audio_testkit::matrix::sample_rates() {
            let mut engine = Engine::<N>::new(text.as_bytes(), rate.hz(), format, Output::Raw)
                .expect("valid patch");
            assert_eq!(engine.channels(), 2, "stereo");
            // At least 640 frames and three blocks, whole blocks: every engine block
            // crosses two of its boundaries.
            let blocks = 640_usize.div_ceil(N).max(3);
            let mut actual = Vec::new();
            for _ in 0..blocks {
                actual.extend(interleaved(engine.process()));
            }
            assert_eq!(
                actual,
                expected(&patch, rate, format, blocks.saturating_mul(N)),
                "{format}, bit-identical at {} Hz in blocks of {N}",
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
        let mut engine =
            Engine::<ENGINE_BLOCK>::new(SUBNORMAL_SINE.as_bytes(), 8_000, format, Output::Raw)
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
        let mut engine = Engine::<ENGINE_BLOCK>::new(SINE.as_bytes(), 48_000, format, Output::Raw)
            .expect("valid patch");
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
    // At every engine block a module is built for.
    renders_without_a_heap_operation_in_blocks_of::<16>();
    renders_without_a_heap_operation_in_blocks_of::<32>();
    renders_without_a_heap_operation_in_blocks_of::<64>();
    renders_without_a_heap_operation_in_blocks_of::<128>();
    renders_without_a_heap_operation_in_blocks_of::<256>();
    renders_without_a_heap_operation_in_blocks_of::<512>();
    renders_without_a_heap_operation_in_blocks_of::<1024>();
}

/// Renders 100 blocks of `N` frames of a sine in every format at every rate with no heap
/// operation, from the first block of a fresh engine.
fn renders_without_a_heap_operation_in_blocks_of<const N: usize>() {
    for format in RenderFormat::ALL {
        for rate in audio_testkit::matrix::sample_rates() {
            let mut engine = Engine::<N>::new(SINE.as_bytes(), rate.hz(), format, Output::Raw)
                .expect("valid patch");
            let operations = audio_testkit::heap_operations_during(|| {
                for _ in 0_u16..100_u16 {
                    let _: Blocks<'_, N> = engine.process();
                }
            });
            assert_eq!(
                operations,
                Ok(((), 0)),
                "{format}: 100 blocks of {N} frames at {} Hz",
                rate.hz()
            );
        }
    }
}

#[test]
fn rejects_text_that_is_not_utf8() {
    for format in RenderFormat::ALL {
        let error = Engine::<ENGINE_BLOCK>::new(&[0xff], 48_000, format, Output::Raw)
            .expect_err("invalid UTF-8");
        assert!(error.starts_with("patch is not UTF-8: "), "{error}");
    }
}

#[test]
fn rejects_an_unsupported_sample_rate() {
    let error =
        Engine::<ENGINE_BLOCK>::new(SINE.as_bytes(), 0, RenderFormat::Binary64, Output::Raw)
            .expect_err("rate 0");
    assert_eq!(
        error, "invalid sample rate: 0 Hz (supported: 8000 to 384000 Hz)",
        "the rate and the range"
    );
}

#[test]
fn rejects_an_unreadable_patch() {
    let error = Engine::<ENGINE_BLOCK>::new(b"{}", 48_000, RenderFormat::Binary32, Output::Raw)
        .expect_err("no format");
    assert_eq!(
        error, "invalid patch: missing field `format` at line 1 column 2",
        "explained"
    );
}

#[test]
fn rejects_an_invalid_plan_in_every_format() {
    let unknown = r#"{"format":1,"nodes":[{"id":1,"box":"nope~"}],"connections":[]}"#;
    for format in RenderFormat::ALL {
        let error = Engine::<ENGINE_BLOCK>::new(unknown.as_bytes(), 48_000, format, Output::Raw)
            .expect_err("unknown box");
        assert!(error.contains("nope~"), "{format}: {error}");
    }
}

#[test]
fn reports_what_its_stage_replaced_as_it_was_given() {
    // No built-in box gives a non-finite sample: each format's stage is fed one directly,
    // after a first block, counted in that block, frame 64 + 3: as computed, or, given as
    // binary32, a finite binary64 value infinite once rounded, as given.
    let frame = u32::try_from(BLOCK_SIZE).expect("a frame count") + 3;
    for format in RenderFormat::ALL {
        let mut engine = Engine::<ENGINE_BLOCK>::new(SINE.as_bytes(), 48_000, format, Output::Raw)
            .expect("a sine");
        let _first = engine.process();
        assert_eq!(
            engine.replaced(),
            Replaced::default(),
            "{format}: a sine replaces nothing"
        );
        match &mut engine.rendering {
            super::Rendering::Binary64 { stage, .. } => {
                let _ = stage.apply(|blocks| {
                    blocks.fill(<Binary64 as audio_core::Sample>::SILENCE);
                    blocks[0][3] = Binary64::from_raw(f64::NAN);
                });
            }
            super::Rendering::Binary64AsBinary32 { stage, .. } => {
                let given = stage.apply_narrowing(|blocks| {
                    blocks.fill(<Binary64 as audio_core::Sample>::SILENCE);
                    blocks[0][3] = Binary64::from_raw(1.0e300_f64);
                });
                assert_eq!(given[0][3].bits(), 0, "replaced as given");
            }
            super::Rendering::Binary32 { stage, .. } => {
                let _ = stage.apply(|blocks| {
                    blocks.fill(<Binary32 as audio_core::Sample>::SILENCE);
                    blocks[0][3] = Binary32::from_raw(f32::NAN);
                });
            }
        }
        assert_eq!(
            engine.replaced(),
            Replaced::new(
                1,
                Some(FirstReplaced {
                    frame: u64::from(frame),
                    channel: 0
                })
            )
            .expect("one sample, its first"),
            "{format}: counted at the frame it was computed"
        );
    }
}

/// A loud stereo patch: sines of 440, 1,000 and 3,000 Hz summed on the first channel, the
/// first two on the second, well above the default ceiling, so that a protected stage
/// limits. audio-core's `ends_each_change_within_the_heap_tests_phases` replays this signal,
/// with the stage and the phases of
/// [`renders_through_a_limited_stage_without_a_heap_operation_in_blocks_of`], on the limiter
/// alone: a change of either changes that test the same.
const LOUD: &str = r#"{"format":1,"nodes":[{"id":1,"box":"osc~"},{"id":2,"box":"osc~","params":{"frequency":1000}},
    {"id":3,"box":"osc~","params":{"frequency":3000}},{"id":4,"box":"dac~"}],
    "connections":[{"from":[1,0],"to":[4,0]},{"from":[2,0],"to":[4,0]},{"from":[3,0],"to":[4,0]},
    {"from":[1,0],"to":[4,1]},{"from":[2,0],"to":[4,1]}]}"#;

/// A protected output with a maximum lookahead of 2 ms and the default settings.
fn protected() -> Output {
    Output::Protected(limiter_stage())
}

/// A limiter's stage with a maximum lookahead of 2 ms and the default settings.
const fn limiter_stage() -> LimiterStage {
    LimiterStage {
        maximum_lookahead: Binary64::new(2.0),
        detector: audio_core::Detector::Precise,
        limiter: audio_core::LimiterSettings::DEFAULT,
    }
}

#[test]
fn builds_a_limiter_stage_in_the_order_of_its_fields() {
    let settings = audio_core::LimiterSettings::DEFAULT
        .with_lookahead(Binary64::new(1.0))
        .expect("a lookahead in range");
    assert_eq!(
        LimiterStage::new(
            Binary64::new(2.5),
            audio_core::Detector::Mastering,
            settings
        ),
        LimiterStage {
            maximum_lookahead: Binary64::new(2.5),
            detector: audio_core::Detector::Mastering,
            limiter: settings,
        }
    );
}

/// The latency of a protected stage with `detector` and a maximum lookahead of `maximum`
/// milliseconds at `rate`, counted apart: the maximum lookahead in frames
/// ([`audio_testkit::frames_of_duration`]) and the detector's delay
/// ([`audio_testkit::detector_delay`]). A test sizes its work from an engine's latency only
/// once it equals this.
fn latency_of(maximum: f64, rate: SampleRate, detector: audio_core::Detector) -> usize {
    audio_testkit::frames_of_duration(maximum, rate)
        .saturating_add(audio_testkit::detector_delay(detector))
}

/// What the native renderer gives for `frames` frames of `patch` at `rate` in `format`
/// through a protected stage of `limiting`, widened to 64 bits.
fn expected_protected(
    patch: &audio_core::Patch,
    rate: SampleRate,
    format: RenderFormat,
    frames: usize,
    limiting: LimiterStage,
) -> Vec<u64> {
    fn stage<S: audio_core::Sample, D: audio_core::host::Delivered<S>>(
        rate: SampleRate,
        channels: usize,
        limiting: LimiterStage,
    ) -> audio_core::OutputStage<S, D> {
        audio_core::OutputStage::limited(
            audio_core::TruePeakLimiter::new(
                rate,
                u16::try_from(channels).expect("few channels"),
                limiting.maximum_lookahead,
                limiting.detector,
                limiting.limiter,
                audio_core::LimiterMode::Protected,
            )
            .expect("a limiter"),
        )
    }
    match format {
        RenderFormat::Binary32 => {
            let plan = Plan::<Binary32, Box<dyn audio_core::Object<Binary32>>>::build(
                patch,
                &REGISTRY,
                rate,
                audio_core::ExecutionMode::Offline,
            )
            .expect("a plan");
            let channels = plan.channels();
            Renderer::with_stage(plan, stage::<_, f32>(rate, channels, limiting))
                .expect("the plan's channels")
                .render(frames)
                .expect("small render")
                .iter()
                .map(|sample| u64::from(sample.to_bits()))
                .collect()
        }
        wide @ (RenderFormat::Binary64 | RenderFormat::Binary64AsBinary32) => {
            let plan = Plan::<Binary64, Box<dyn audio_core::Object<Binary64>>>::build(
                patch,
                &REGISTRY,
                rate,
                audio_core::ExecutionMode::Offline,
            )
            .expect("a plan");
            let channels = plan.channels();
            if wide == RenderFormat::Binary64 {
                Renderer::with_stage(plan, stage::<_, f64>(rate, channels, limiting))
                    .expect("the plan's channels")
                    .render(frames)
                    .expect("small render")
                    .iter()
                    .map(|sample| sample.to_bits())
                    .collect()
            } else {
                Renderer::with_stage(plan, stage::<_, f32>(rate, channels, limiting))
                    .expect("the plan's channels")
                    .render(frames)
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
                Engine::<ENGINE_BLOCK>::new(LOUD.as_bytes(), rate.hz(), format, protected())
                    .expect("valid patch");
            let blocks = 40_usize;
            let mut actual = Vec::new();
            for _ in 0..blocks {
                actual.extend(interleaved(engine.process()));
            }
            assert_eq!(
                actual,
                expected_protected(&patch, rate, format, blocks * BLOCK_SIZE, limiter_stage()),
                "{format}, bit-identical at {} Hz",
                rate.hz()
            );
            assert_eq!(
                engine.latency(),
                latency_of(2.0_f64, rate, audio_core::Detector::Precise),
                "{format}: 2 ms and the precise delay at {} Hz",
                rate.hz()
            );
        }
    }
}

#[test]
fn renders_through_a_limited_stage_without_a_heap_operation() {
    // At every engine block a module is built for.
    renders_through_a_limited_stage_without_a_heap_operation_in_blocks_of::<16>();
    renders_through_a_limited_stage_without_a_heap_operation_in_blocks_of::<32>();
    renders_through_a_limited_stage_without_a_heap_operation_in_blocks_of::<64>();
    renders_through_a_limited_stage_without_a_heap_operation_in_blocks_of::<128>();
    renders_through_a_limited_stage_without_a_heap_operation_in_blocks_of::<256>();
    renders_through_a_limited_stage_without_a_heap_operation_in_blocks_of::<512>();
    renders_through_a_limited_stage_without_a_heap_operation_in_blocks_of::<1024>();
}

/// Renders blocks of `N` frames through a switchable stage, changed and switched live, in
/// every format at every rate with no heap operation.
fn renders_through_a_limited_stage_without_a_heap_operation_in_blocks_of<const N: usize>() {
    for format in RenderFormat::ALL {
        for rate in audio_testkit::matrix::sample_rates() {
            let switchable = Output::Switchable(limiter_stage());
            let mut engine = Engine::<N>::new(LOUD.as_bytes(), rate.hz(), format, switchable)
                .expect("valid patch");
            let latency = engine.latency();
            assert_eq!(
                latency,
                latency_of(2.0_f64, rate, audio_core::Detector::Precise),
                "{format}: counted apart"
            );
            // Each phase spans the latency and four blocks more: every frame of the delay
            // and of the lookahead's window is written, and leaves, in each. audio-core's
            // `ends_each_change_within_the_heap_tests_phases` shows, on `LOUD`'s signal and
            // this stage, that every lookahead change made below ends within these phases, so
            // that its last frames are counted: its `heap_phase_blocks`, the order of the
            // changes and the stage there change with these. The crossfades between the modes
            // do not arrive within them: `crossfades_to_each_arrival_without_a_heap_operation`
            // counts each to its arrival.
            let blocks = latency.div_ceil(N).saturating_add(4);
            let rendered = |rendering: &mut Engine<N>, count: usize| {
                for _ in 0..count {
                    let _: Blocks<'_, N> = rendering.process();
                }
            };
            let phase = |rendering: &mut Engine<N>| rendered(rendering, blocks);
            let operations = audio_testkit::heap_operations_during(|| {
                phase(&mut engine);
                // Changed live: a shorter lookahead starts a second control path beside the old
                // one, which runs on.
                let shorter = audio_core::LimiterSettings::DEFAULT
                    .with_lookahead(Binary64::new(0.5))
                    .expect("in range");
                assert_eq!(engine.set_limiter(shorter), Ok(()), "{format}: live");
                rendered(&mut engine, 1);
                // Then, one block later, while the shrink runs where the block is shorter than
                // it, the longest it was built for: a longer lookahead, from the needs kept,
                // begun as the shrink ends, waiting for the life of its path.
                let longer = audio_core::LimiterSettings::DEFAULT
                    .with_lookahead(Binary64::new(2.0))
                    .expect("in range");
                assert_eq!(engine.set_limiter(longer), Ok(()), "{format}: longer");
                rendered(&mut engine, blocks.saturating_sub(1));
                phase(&mut engine);
                // Toward protected, then back to raw, each through its crossfade.
                assert_eq!(engine.set_protected(true), Ok(()), "{format}: protected");
                phase(&mut engine);
                assert_eq!(engine.set_protected(false), Ok(()), "{format}: raw");
                phase(&mut engine);
            });
            assert_eq!(
                operations,
                Ok(((), 0)),
                "{format}: {} blocks of {N} frames in each of five phases at {} Hz",
                blocks,
                rate.hz()
            );
        }
    }
}

/// The frames a crossfade between the modes takes to arrive at most, at `rate`, computed
/// apart: what is left of its way, 1 at its start, decays by `exp(-1 / (20 ms × rate))` per
/// frame (its time constant of 20 ms) and arrives once it is at most `2^-54`, below which
/// `1 - share` rounds to 1, after `54 × ln 2 × 20 ms`, 0.7486 s of frames; 0.76 s of them
/// cover that and every rounding of the decay (each frame's, relative `2^-52`, over less than
/// 300,000 frames). audio-core's `arrives_at_each_crossfade_within_audio_hosts_heap_frames`
/// shows each crossfade arrived after these frames, not after 0.74 s of them, at every rate.
fn crossfade_frames(rate: u32) -> usize {
    usize::try_from(rate)
        .expect("a rate")
        .saturating_mul(76)
        .div_ceil(100)
}

#[test]
fn crossfades_to_each_arrival_without_a_heap_operation() {
    // At every rate and in every format, at the engine block the playground ships: a
    // switchable stage crossfaded to protected, then back to raw, each run to its arrival
    // ([`crossfade_frames`]), from the first block of a fresh engine, with no heap operation.
    // A stage of the standard detector, the cheapest, and a maximum of the default lookahead:
    // the crossfade acts on the reduction, whatever the detector and the window.
    let stage = LimiterStage {
        maximum_lookahead: Binary64::new(1.5),
        detector: audio_core::Detector::Standard,
        limiter: audio_core::LimiterSettings::DEFAULT,
    };
    for format in RenderFormat::ALL {
        for rate in audio_testkit::matrix::sample_rates() {
            let mut engine = Engine::<ENGINE_BLOCK>::new(
                LOUD.as_bytes(),
                rate.hz(),
                format,
                Output::Switchable(stage),
            )
            .expect("valid patch");
            let blocks = crossfade_frames(rate.hz()).div_ceil(ENGINE_BLOCK);
            let operations = audio_testkit::heap_operations_during(|| {
                for protected in [true, false] {
                    assert_eq!(
                        engine.set_protected(protected),
                        Ok(()),
                        "{format}: switched"
                    );
                    for _ in 0..blocks {
                        let _: Blocks<'_, ENGINE_BLOCK> = engine.process();
                    }
                }
            });
            assert_eq!(
                operations,
                Ok(((), 0)),
                "{format}: {blocks} blocks of {ENGINE_BLOCK} frames per crossfade at {} Hz",
                rate.hz()
            );
        }
    }
}

/// The modes and their names as the interfaces write them, as literals: a reorder of
/// [`super::OutputMode::ALL`] would change the codes of the C interface silently otherwise.
#[test]
fn lists_every_output_mode() {
    use super::OutputMode;
    assert_eq!(
        OutputMode::ALL.map(OutputMode::name),
        ["raw", "protected", "switchable"],
        "names, in the order of the codes"
    );
    assert_eq!(
        OutputMode::ALL,
        [
            OutputMode::Raw,
            OutputMode::Protected,
            OutputMode::Switchable
        ],
        "every mode"
    );
}

#[test]
fn builds_a_switchable_stage_raw_first_with_the_latency_of_its_limiter() {
    let mut switchable = Engine::<ENGINE_BLOCK>::new(
        LOUD.as_bytes(),
        48_000,
        RenderFormat::Binary64,
        Output::Switchable(limiter_stage()),
    )
    .expect("valid");
    let mut raw =
        Engine::<ENGINE_BLOCK>::new(LOUD.as_bytes(), 48_000, RenderFormat::Binary64, Output::Raw)
            .expect("valid");
    assert_eq!(
        switchable.latency(),
        96 + 16,
        "2 ms at 48 kHz, kept while raw"
    );
    let lead = switchable.latency() * switchable.channels();
    let delayed: Vec<u64> = (0_u8..8_u8)
        .flat_map(|_| interleaved(switchable.process()))
        .collect();
    let straight: Vec<u64> = (0_u8..8_u8)
        .flat_map(|_| interleaved(raw.process()))
        .collect();
    assert_eq!(
        delayed[lead..],
        straight[..straight.len() - lead],
        "raw first: delayed bit for bit"
    );
}

#[test]
fn changes_its_limiter_live_and_refuses_without_one() {
    let mut raw =
        Engine::<ENGINE_BLOCK>::new(LOUD.as_bytes(), 48_000, RenderFormat::Binary32, Output::Raw)
            .expect("valid");
    assert_eq!(raw.latency(), 0, "raw: no latency");
    let refusal = Err("the output stage is raw: it has no limiter to set".to_owned());
    assert_eq!(
        raw.set_limiter(audio_core::LimiterSettings::DEFAULT),
        refusal,
        "no limiter to set"
    );
    assert_eq!(raw.set_protected(false), refusal, "nor to switch");
    for format in RenderFormat::ALL {
        let mut engine = Engine::<ENGINE_BLOCK>::new(LOUD.as_bytes(), 48_000, format, protected())
            .expect("valid");
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
            engine.set_protected(true),
            Ok(()),
            "{format}: protected, as it was"
        );
    }
    let wrong = Output::Protected(LimiterStage {
        maximum_lookahead: Binary64::new(9.0),
        detector: audio_core::Detector::Precise,
        limiter: audio_core::LimiterSettings::DEFAULT,
    });
    assert_eq!(
        Engine::<ENGINE_BLOCK>::new(LOUD.as_bytes(), 48_000, RenderFormat::Binary64, wrong)
            .map(|_| ()),
        Err("limiter maximum lookahead 9 ms is out of range (0.5 to 5 ms)".to_owned()),
        "a maximum out of range"
    );
}

#[test]
fn switches_only_a_switchable_stage_to_raw() {
    let locked =
        Err("the output stage is protected: only a switchable stage is switched to raw".to_owned());
    let no_limiter = Err("the output stage is raw: it has no limiter to set".to_owned());
    let switchable = Output::Switchable(limiter_stage());
    for format in RenderFormat::ALL {
        let build = |output| {
            Engine::<ENGINE_BLOCK>::new(LOUD.as_bytes(), 48_000, format, output).expect("valid")
        };
        let mut raw = build(Output::Raw);
        assert_eq!(
            raw.set_protected(true),
            no_limiter,
            "{format}: raw, toward protected"
        );
        assert_eq!(
            raw.set_protected(false),
            no_limiter,
            "{format}: raw, toward raw"
        );
        // A protected stage told to go raw renders on bit for bit as one never told.
        let mut protected_told = build(protected());
        let mut untouched = build(protected());
        assert_eq!(
            protected_told.set_protected(false),
            locked,
            "{format}: protected, toward raw"
        );
        assert_eq!(
            protected_told.set_protected(true),
            Ok(()),
            "{format}: protected, as it is"
        );
        // Past the latency of 112 frames (2 ms at 48 kHz and the precise detector's 16) and
        // the second of a crossfade, 48,000 frames: a crossfade started would show.
        for block in 0..800_u16 {
            assert_eq!(
                interleaved(protected_told.process()),
                interleaved(untouched.process()),
                "{format}: block {block}, still protected"
            );
        }
        let mut switched = build(switchable);
        assert_eq!(
            switched.set_protected(true),
            Ok(()),
            "{format}: switchable, toward protected"
        );
        assert_eq!(
            switched.set_protected(false),
            Ok(()),
            "{format}: switchable, toward raw"
        );
    }
}

/// A conformance case as the engine renders it: its name, its patch, its rate, its frames
/// and its output stage.
struct Case {
    /// Its file name, in `conformance/cases/` and `conformance/patches/`.
    name: alloc::string::String,
    /// Its patch, as the engine reads it.
    patch: Vec<u8>,
    /// Its sample rate, in hertz.
    rate: u32,
    /// The frames rendered.
    frames: usize,
    /// Its output stage: raw, or protected with its lookahead (also the maximum) and detector,
    /// the limiter's other settings the defaults, as `audio-cli` renders it.
    output: Output,
}

/// The output stage of a case's `output` JSON object, raw when it has none: a raw one
/// carries no setting, a protected one its lookahead and detector.
fn case_output(output: Option<&serde_json::Value>) -> Output {
    let Some(output) = output else {
        return Output::Raw;
    };
    if output["mode"].as_str() == Some("raw") {
        assert_eq!(
            output.as_object().map(serde_json::Map::len),
            Some(1),
            "a raw output carries no setting"
        );
        return Output::Raw;
    }
    assert_eq!(
        output["mode"].as_str(),
        Some("protected"),
        "a mode of the case format"
    );
    let lookahead = Binary64::from_raw(output["lookahead"].as_f64().expect("a lookahead"));
    let name = output["detector"].as_str();
    let detector = audio_core::Detector::ALL
        .into_iter()
        .find(|detector| Some(detector.name()) == name)
        .expect("a detector of the case format");
    Output::Protected(LimiterStage {
        maximum_lookahead: lookahead,
        detector,
        limiter: audio_core::LimiterSettings::DEFAULT
            .with_lookahead(lookahead)
            .expect("a valid lookahead"),
    })
}

/// Every case of the conformance corpus, by name.
fn corpus() -> Vec<Case> {
    let cases = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/cases");
    let patches =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/patches");
    let mut names: Vec<alloc::string::String> = std::fs::read_dir(&cases)
        .expect("the corpus")
        .map(|entry| {
            entry
                .expect("a case")
                .file_name()
                .into_string()
                .expect("a UTF-8 name")
        })
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let case: serde_json::Value =
                serde_json::from_slice(&std::fs::read(cases.join(&name)).expect("a case file"))
                    .expect("valid JSON");
            Case {
                patch: std::fs::read(patches.join(&name)).expect("its patch"),
                rate: u32::try_from(case["sample_rate"].as_u64().expect("a rate"))
                    .expect("a 32-bit rate"),
                frames: usize::try_from(case["frames"].as_u64().expect("frames"))
                    .expect("frames in memory"),
                output: case_output(case.get("output")),
                name,
            }
        })
        .collect()
}

/// The bits of `case` rendered in `format` by an engine of blocks of `N` frames: its
/// first `frames` frames, interleaved, widened to 64 bits.
fn render_case<const N: usize>(case: &Case, format: RenderFormat) -> Vec<u64> {
    let mut engine =
        Engine::<N>::new(&case.patch, case.rate, format, case.output).expect("a valid case");
    let mut bits = Vec::new();
    for _ in 0..case.frames.div_ceil(N) {
        bits.extend(interleaved(engine.process()));
    }
    bits.truncate(case.frames.saturating_mul(engine.channels()));
    bits
}

/// The one case rendered in blocks of `MAX_ENGINE_BLOCK` frames: the limited one, so that
/// the output stage's limiter runs at that size too. Every case at that size is too slow
/// for a debug build (a block of 2^20 frames per box, format and case).
const LARGEST_BLOCK_CASE: &str = "sine-1000-limited-at-48k.json";

#[test]
fn renders_every_conformance_case_alike_at_every_engine_block_of_the_matrix() {
    // The engine-block law (decision 2) on the corpus, through the safe engine: in every
    // format, the bits of the default engine block at every size of the matrix.
    let corpus = corpus();
    assert!(
        corpus.iter().any(|case| case.name == LARGEST_BLOCK_CASE),
        "the case rendered at the largest engine block is in the corpus"
    );
    let mut sizes = Vec::new();
    for case in &corpus {
        for format in RenderFormat::ALL {
            let default = render_case::<BLOCK_SIZE>(case, format);
            assert!(!default.is_empty(), "{}: samples", case.name);
            let mut rendered = vec![
                (1, render_case::<1>(case, format)),
                (16, render_case::<16>(case, format)),
                (32, render_case::<32>(case, format)),
                (64, render_case::<64>(case, format)),
                (128, render_case::<128>(case, format)),
                (256, render_case::<256>(case, format)),
                (441, render_case::<441>(case, format)),
                (512, render_case::<512>(case, format)),
                (1024, render_case::<1024>(case, format)),
            ];
            if case.name == LARGEST_BLOCK_CASE {
                const LARGEST: usize = audio_core::MAX_ENGINE_BLOCK;
                rendered.push((LARGEST, render_case::<LARGEST>(case, format)));
            }
            for (frames, bits) in rendered {
                assert!(
                    bits == default,
                    "{} in {format}: blocks of {frames} frames change the bits",
                    case.name
                );
                sizes.push(frames);
            }
        }
    }
    sizes.sort_unstable();
    sizes.dedup();
    assert_eq!(
        sizes,
        audio_testkit::matrix::ENGINE_BLOCK_SIZES,
        "every engine block of the test matrix"
    );
}

#[test]
fn the_corpus_renders_through_every_detector_of_the_engine() {
    // A detector no case renders through has no bits proven the same natively and in
    // WebAssembly: the detectors the cases name are exactly the engine's, by name.
    let mut named: Vec<&str> = corpus()
        .iter()
        .filter_map(|case| match case.output {
            Output::Protected(stage) | Output::Switchable(stage) => Some(stage.detector.name()),
            Output::Raw => None,
        })
        .collect();
    named.sort_unstable();
    named.dedup();
    let mut engine: Vec<&str> = audio_core::Detector::ALL
        .iter()
        .map(|detector| detector.name())
        .collect();
    engine.sort_unstable();
    assert_eq!(named, engine, "the detectors of the corpus");
}

/// A patch of `count` sines, from 100 Hz up by 997 Hz each, summed into the left channel of
/// one `dac~` (a peak of nearly `count`), the first alone on the right.
fn sines(count: u32) -> alloc::string::String {
    let nodes: Vec<alloc::string::String> = (1..=count)
        .map(|id| {
            let hz = 997_u32
                .saturating_mul(id.saturating_sub(1))
                .saturating_add(100);
            alloc::format!(r#"{{"id":{id},"box":"osc~","params":{{"frequency":{hz}}}}}"#)
        })
        .collect();
    let dac = count.saturating_add(1);
    let mut cables: Vec<alloc::string::String> = (1..=count)
        .map(|id| alloc::format!(r#"{{"from":[{id},0],"to":[{dac},0]}}"#))
        .collect();
    cables.push(alloc::format!(r#"{{"from":[1,0],"to":[{dac},1]}}"#));
    alloc::format!(
        r#"{{"format":1,"nodes":[{},{{"id":{dac},"box":"dac~"}}],"connections":[{}]}}"#,
        nodes.join(","),
        cables.join(",")
    )
}

/// The largest magnitude the engine gives over `blocks` blocks, widened to binary64, and
/// the bits it gives ([`interleaved`]).
fn largest_given(engine: &mut Engine, blocks: usize) -> (f64, Vec<u64>) {
    let mut largest = 0.0_f64;
    let mut bits = Vec::new();
    for _ in 0..blocks {
        let given = engine.process();
        bits.extend(interleaved(given));
        let magnitudes: Vec<f64> = match given {
            Blocks::Binary64(channels) => channels
                .iter()
                .flatten()
                .map(|sample| sample.raw().abs())
                .collect(),
            Blocks::Binary32(channels) => channels
                .iter()
                .flatten()
                .map(|sample| f64::from(sample.raw().abs()))
                .collect(),
        };
        largest = magnitudes.into_iter().fold(largest, crate::tests::larger);
    }
    (largest, bits)
}

#[test]
fn holds_the_ceiling_with_no_slack_as_given_in_every_format_at_every_rate() {
    // The ceilings, each with its amplitude `10^(dB/20)` in binary64 computed apart (as the
    // limiter's own sweep writes them; at -20 dBTP, `exp(-20 × fl(ln 10 / 20))` rounds 2 ulps
    // below 0.1, to `0x3fb9999999999998`): every sample given is at or below it, with no
    // slack, in the format given, binary64 given as binary32 included, whose rounding to the
    // nearest binary32 could otherwise lift a sample at the ceiling above it.
    let ceilings = [
        (-20.0_f64, 0.099_999_999_999_999_98_f64),
        (-1.0_f64, 0.891_250_938_133_745_6_f64),
        (0.0_f64, 1.0_f64),
    ];
    let detectors = audio_core::Detector::ALL;
    let mut index = 0_usize;
    for format in RenderFormat::ALL {
        for rate in audio_testkit::matrix::sample_rates() {
            for count in [2_u32, 9] {
                let (decibels, amplitude) = ceilings[index.rem_euclid(ceilings.len())];
                // Each maximum lookahead.
                let maximum = [0.5_f64, 5.0_f64][index.rem_euclid(2)];
                let detector = detectors[index.div_euclid(2).rem_euclid(detectors.len())];
                let settings = audio_core::LimiterSettings::new(
                    Binary64::new(decibels),
                    Binary64::new(maximum),
                    Binary64::new(10.0),
                    Binary64::new(1.0),
                )
                .expect("settings");
                let limiting = LimiterStage {
                    maximum_lookahead: Binary64::new(maximum),
                    detector,
                    limiter: settings,
                };
                let output = Output::Protected(limiting);
                let mut engine =
                    Engine::<ENGINE_BLOCK>::new(sines(count).as_bytes(), rate.hz(), format, output)
                        .expect("a loud patch");
                let latency = engine.latency();
                assert_eq!(
                    latency,
                    latency_of(maximum, rate, detector),
                    "{format}: counted apart"
                );
                // Past the latency by a window and 1,024 frames more.
                let frames = latency.saturating_mul(2).saturating_add(1_024);
                let blocks = frames.div_ceil(ENGINE_BLOCK);
                let (peak, bits) = largest_given(&mut engine, blocks);
                assert!(
                    peak <= amplitude,
                    "{format} at {} Hz, {count} sines, {decibels} dBTP, {maximum} ms, \
                     {detector:?}: {peak} above {amplitude}",
                    rate.hz()
                );
                assert!(
                    peak >= amplitude / 2.0_f64,
                    "{format} at {} Hz: {peak}, silenced",
                    rate.hz()
                );
                // Every sample that of the core's stage with the same settings, whose gain
                // the limiter's own tests hold exactly: a setting the engine lost or changed
                // fails here, where half the ceiling cannot see a decibel.
                assert_eq!(
                    bits,
                    expected_protected(
                        &audio_patch::parse_patch(&sines(count)).expect("valid patch"),
                        rate,
                        format,
                        blocks * ENGINE_BLOCK,
                        limiting,
                    ),
                    "{format} at {} Hz, {count} sines: the renderer's bits",
                    rate.hz()
                );
                index = index.saturating_add(1);
            }
        }
    }
}
