//! Tests of an engine instance and its patch loading.

#![forbid(unsafe_code)]

use alloc::boxed::Box;
use alloc::vec::Vec;

use audio_core::host::Raw;
use audio_core::{BLOCK_SIZE, RenderFormat};

use super::Instance;
use crate::Blocks;
use crate::ENGINE_BLOCK;

/// The first channel of `blocks`, widened to 64 bits.
fn first(blocks: Blocks<'_, ENGINE_BLOCK>) -> Vec<u64> {
    match blocks {
        Blocks::Binary64(channels) => channels[0].iter().map(|sample| sample.bits()).collect(),
        Blocks::Binary32(channels) => channels[0].iter().map(|sample| sample.bits()).collect(),
    }
}

/// The blocks of a binary32 render, or `None` for binary64.
fn narrow(blocks: Blocks<'_, ENGINE_BLOCK>) -> Option<&[audio_core::Block<audio_core::Binary32>]> {
    match blocks {
        Blocks::Binary64(_) => None,
        Blocks::Binary32(channels) => Some(channels),
    }
}

/// A 440 Hz sine on one channel.
const MONO: &[u8] =
    br#"{"format":1,"nodes":[{"id":1,"box":"osc~"},{"id":2,"box":"dac~"}],"connections":[{"from":[1,0],"to":[2,0]}]}"#;

fn loaded() -> Instance {
    let mut instance = Instance::new();
    instance.input(MONO.len()).copy_from_slice(MONO);
    assert!(instance.load(48_000, 1), "{}", instance.message());
    instance
}

/// The format codes of the C interface, written as literals: a reorder of
/// [`RenderFormat::ALL`] would change them silently otherwise.
#[test]
fn maps_each_format_code_of_the_interface() {
    assert_eq!(super::format_of(0), Some(RenderFormat::Binary64), "0");
    assert_eq!(
        super::format_of(1),
        Some(RenderFormat::Binary64AsBinary32),
        "1"
    );
    assert_eq!(super::format_of(2), Some(RenderFormat::Binary32), "2");
    assert_eq!(super::format_of(3), None, "3");
    assert_eq!(super::format_of(u32::MAX), None, "the largest code");
}

/// The mode and detector codes of the C interface, written as literals: a reorder of
/// [`crate::OutputMode::ALL`] or [`audio_core::Detector::ALL`] would change them silently
/// otherwise.
#[test]
fn maps_each_mode_and_detector_code_of_the_interface() {
    use crate::OutputMode;
    use audio_core::Detector;
    assert_eq!(super::mode_of(0), Some(OutputMode::Raw), "0");
    assert_eq!(super::mode_of(1), Some(OutputMode::Protected), "1");
    assert_eq!(super::mode_of(2), Some(OutputMode::Switchable), "2");
    assert_eq!(super::mode_of(3), None, "3");
    assert_eq!(super::mode_of(u32::MAX), None, "the largest mode code");
    assert_eq!(super::detector_of(0), Some(Detector::Standard), "0");
    assert_eq!(super::detector_of(1), Some(Detector::Precise), "1");
    assert_eq!(super::detector_of(2), Some(Detector::Mastering), "2");
    assert_eq!(super::detector_of(3), None, "3");
    assert_eq!(
        super::detector_of(u32::MAX),
        None,
        "the largest detector code"
    );
}

#[test]
fn starts_empty() {
    let mut instance = Instance::default();
    assert_eq!(instance.channels(), 0, "no engine");
    assert!(
        matches!(instance.process(), Blocks::Binary32([])),
        "no blocks"
    );
    assert_eq!(instance.message(), "", "no error");
}

#[test]
fn loads_in_the_format_of_its_code() {
    for (code, format) in (0_u32..).zip(RenderFormat::ALL) {
        let mut instance = Instance::new();
        instance.input(MONO.len()).copy_from_slice(MONO);
        assert!(instance.load(8_000, code), "{}", instance.message());
        let mut engine =
            crate::Engine::<ENGINE_BLOCK>::new(MONO, 8_000, format, crate::Output::Raw)
                .expect("valid");
        assert_eq!(
            first(instance.process()),
            first(engine.process()),
            "{format}"
        );
    }
    for code in [3_u32, u32::MAX] {
        let mut instance = Instance::new();
        instance.input(MONO.len()).copy_from_slice(MONO);
        assert!(!instance.load(48_000, code), "{code} is no format");
        assert_eq!(
            instance.message(),
            alloc::format!(
                "unknown render format {code} (0 binary64, 1 binary64-as-binary32, 2 binary32)"
            ),
            "explained"
        );
        assert_eq!(instance.channels(), 0, "nothing loaded");
    }
}

#[test]
fn input_is_exactly_the_requested_zeroed_length() {
    let mut instance = Instance::new();
    instance.input(4).copy_from_slice(b"abcd");
    assert_eq!(instance.input(2), &[0, 0], "shrunk and zeroed");
    assert_eq!(instance.input(3), &[0, 0, 0], "grown and zeroed");
}

#[test]
fn loads_and_renders() {
    let mut instance = loaded();
    assert_eq!(instance.channels(), 2, "dac~ has two inlets");
    assert_eq!(instance.message(), "", "no error");
    let mut wide = Instance::new();
    wide.input(MONO.len()).copy_from_slice(MONO);
    assert!(wide.load(48_000, 0), "{}", wide.message());
    assert_eq!(narrow(wide.process()), None, "binary64 blocks");
    let blocks = narrow(instance.process()).expect("binary32 blocks");
    assert_eq!(blocks.len(), 2, "one block per channel");
    assert!(
        blocks[0].iter().any(|&sample| sample.raw() != 0.0),
        "sound on the left"
    );
    assert!(
        blocks[1].iter().all(|&sample| sample.raw() == 0.0),
        "silence on the right"
    );
}

#[test]
fn loads_at_the_requested_rate() {
    let patch =
        audio_patch::parse_patch(core::str::from_utf8(MONO).expect("UTF-8")).expect("valid patch");
    for rate in audio_testkit::matrix::sample_rates() {
        let mut instance = Instance::new();
        instance.input(MONO.len()).copy_from_slice(MONO);
        assert!(instance.load(rate.hz(), 0), "{}", instance.message());
        let expected = audio_core::Renderer::<_, _, f64>::new(
            audio_core::Plan::<
                audio_core::Binary64,
                Box<dyn audio_core::Object<audio_core::Binary64>>,
            >::build(
                &patch,
                &audio_boxes::REGISTRY,
                rate,
                audio_core::ExecutionMode::Offline,
            )
            .expect("valid plan"),
        )
        .render(BLOCK_SIZE)
        .expect("one block");
        let left: Vec<u64> = expected
            .iter()
            .step_by(2)
            .map(|sample| sample.to_bits())
            .collect();
        assert_eq!(
            first(instance.process()),
            left,
            "the first block at {} Hz",
            rate.hz()
        );
    }
}

#[test]
fn renders_without_a_heap_operation() {
    for code in 0_u32..3 {
        let mut instance = Instance::new();
        instance.input(MONO.len()).copy_from_slice(MONO);
        assert!(instance.load(48_000, code), "{}", instance.message());
        let operations = audio_testkit::heap_operations_during(|| {
            for _ in 0_u16..100_u16 {
                let _: Blocks<'_, ENGINE_BLOCK> = instance.process();
            }
        });
        assert_eq!(operations, Ok(((), 0)), "format {code}: 100 blocks");
    }
    let mut empty = Instance::new();
    let none = audio_testkit::heap_operations_during(|| {
        let _: Blocks<'_, ENGINE_BLOCK> = empty.process();
    });
    assert_eq!(none, Ok(((), 0)), "nothing loaded");
}

#[test]
fn reports_what_the_loaded_engine_replaced_or_nothing() {
    let mut empty = Instance::new();
    let none = empty.replaced_report();
    assert!(core::ptr::eq(none, empty.report.as_ptr()), "its own words");
    assert_eq!(empty.report, [0; 4], "nothing loaded: four zeros");
    let mut instance = loaded();
    instance.report = [9; 4];
    let given = instance.replaced_report();
    assert!(
        core::ptr::eq(given, instance.report.as_ptr()),
        "its own words"
    );
    assert_eq!(
        instance.report, [0; 4],
        "a sine replaces nothing, rewritten when asked"
    );
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
        super::report_words(
            Replaced::new(
                3,
                Some(FirstReplaced {
                    frame: 0x0000_0002_0000_0007,
                    channel: 1,
                })
            )
            .expect("a count and a first sample that agree")
        ),
        [3, 7, 2, 1],
        "the frame split in its low and high words"
    );
    assert_eq!(
        super::report_words(
            Replaced::new(
                u64::MAX,
                Some(FirstReplaced {
                    frame: u64::MAX,
                    channel: usize::MAX,
                })
            )
            .expect("a count and a first sample that agree")
        ),
        [u32::MAX, u32::MAX, u32::MAX, u32::MAX],
        "the count and the channel saturate, every bit of the frame kept"
    );
}

#[test]
fn a_failed_load_unloads_and_explains() {
    let mut instance = loaded();
    instance.input(1).copy_from_slice(b"x");
    assert!(!instance.load(48_000, 1), "invalid patch");
    assert_eq!(instance.channels(), 0, "unloaded");
    assert!(
        matches!(instance.process(), Blocks::Binary32([])),
        "no blocks"
    );
    assert_eq!(
        instance.message(),
        "invalid patch: expected value at line 1 column 1",
        "explained"
    );
}

#[test]
fn explains_in_one_escaped_bounded_line() {
    let mut instance = Instance::new();
    let hostile =
        br#"{"format":1,"nodes":[{"id":1,"box":"x\u001b[31m\nsecond\u202e"}],"connections":[]}"#;
    instance.input(hostile.len()).copy_from_slice(hostile);
    assert!(!instance.load(48_000, 1), "unknown box");
    assert_eq!(
        instance.message(),
        "box 1: unknown box type \"x\\u{1b}[31m\\u{a}second\\u{202e}\"",
        "a box type from the patch, escaped"
    );
    let field = br#"{"format":1,"nodes":[],"connections":[],"a\u001b\n":1}"#;
    instance.input(field.len()).copy_from_slice(field);
    assert!(!instance.load(48_000, 1), "unknown field");
    assert!(
        instance
            .message()
            .starts_with("invalid patch: unknown field `a\\u{1b}\\u{a}`"),
        "a field name echoed by the JSON reader, escaped: {}",
        instance.message()
    );
    let long = alloc::format!(
        r#"{{"format":1,"nodes":[{{"id":1,"box":"{}"}}],"connections":[]}}"#,
        "y".repeat(5_000)
    );
    instance.input(long.len()).copy_from_slice(long.as_bytes());
    assert!(!instance.load(48_000, 1), "unknown box");
    assert_eq!(
        instance.message().chars().count(),
        audio_patch::MAX_MESSAGE_CHARS,
        "bounded: {}",
        instance.message()
    );
    assert!(instance.message().ends_with('…'), "marked as cut");
}

#[test]
fn a_successful_load_clears_the_previous_error() {
    let mut instance = Instance::new();
    assert!(!instance.load(48_000, 1), "empty input");
    instance.input(MONO.len()).copy_from_slice(MONO);
    assert!(instance.load(48_000, 1), "valid");
    assert_eq!(instance.message(), "", "cleared");
}

/// Loads `MONO` at 48 kHz in binary32, returning whether it succeeded.
fn load_mono(instance: &mut Instance) -> bool {
    instance.input(MONO.len()).copy_from_slice(MONO);
    instance.load(48_000, 2)
}

/// The limiter's default settings as the C interface takes them: -1 dBTP, 1.5 ms, 100 ms,
/// 0.5, written as literals.
const DEFAULT: [f64; 4] = [-1.0_f64, 1.5_f64, 100.0_f64, 0.5_f64];

/// Stores the output stage of `mode`, `maximum` and `detector` codes with the limiter's
/// `settings` for later loads, returning whether it succeeded.
fn output(
    instance: &mut Instance,
    mode: u32,
    maximum: f64,
    detector: u32,
    settings: [f64; 4],
) -> bool {
    let [ceiling, lookahead, release, adaptivity] = settings;
    instance.output(
        mode, maximum, detector, ceiling, lookahead, release, adaptivity,
    )
}

/// Sets the loaded engine's limiter to `settings`, live, returning whether it succeeded.
fn live(instance: &mut Instance, settings: [f64; 4]) -> bool {
    let [ceiling, lookahead, release, adaptivity] = settings;
    instance.limiter(ceiling, lookahead, release, adaptivity)
}

#[test]
fn builds_every_later_load_with_the_output_set() {
    let mut instance = Instance::new();
    assert!(load_mono(&mut instance), "a raw load");
    assert_eq!(instance.latency(), 0, "raw by default: no latency");
    let mut before = 0;
    for (mode, latency) in [(1_u32, 72 + 16), (2, 72 + 16), (0, 0)] {
        assert!(output(&mut instance, mode, 1.5, 1, DEFAULT), "mode {mode}");
        assert_eq!(instance.message(), "", "no message");
        assert_eq!(
            instance.latency(),
            before,
            "the loaded engine keeps its own"
        );
        assert!(load_mono(&mut instance), "a load in mode {mode}");
        assert_eq!(
            instance.latency(),
            latency,
            "mode {mode}: 1.5 ms at 48 kHz and the detector"
        );
        before = latency;
    }
}

#[test]
fn refuses_an_output_it_cannot_build_and_keeps_the_one_set() {
    let mut instance = Instance::new();
    assert!(
        !output(&mut instance, 3, 1.5, 1, DEFAULT),
        "an unknown mode"
    );
    assert_eq!(
        instance.message(),
        "unknown output mode 3 (0 raw, 1 protected, 2 switchable)"
    );
    assert!(
        !output(&mut instance, u32::MAX, 1.5, 1, DEFAULT),
        "the largest code"
    );
    for mode in [1_u32, 2] {
        for (maximum, written) in [(0.25_f64, "0.25"), (5.5_f64, "5.5"), (f64::NAN, "NaN")] {
            assert!(
                !output(
                    &mut instance,
                    mode,
                    maximum,
                    1,
                    [-1.0_f64, 0.5_f64, 100.0_f64, 0.5_f64]
                ),
                "mode {mode}: {maximum}"
            );
            assert_eq!(
                instance.message(),
                alloc::format!(
                    "limiter maximum lookahead {written} ms is out of range (0.5 to 5 ms)"
                ),
                "mode {mode}: {maximum}"
            );
        }
        assert!(
            !output(
                &mut instance,
                mode,
                1.5,
                1,
                [1.0_f64, 1.5_f64, 100.0_f64, 0.5_f64]
            ),
            "mode {mode}: a ceiling out of range"
        );
        assert_eq!(
            instance.message(),
            "limiter ceiling 1 dBTP is out of range (-20 to 0 dBTP)"
        );
        for (settings, refusal) in [
            (
                [-1.0_f64, 1.5_f64, 9.0_f64, 0.5_f64],
                "limiter release 9 ms is out of range (10 to 1000 ms)",
            ),
            (
                [-1.0_f64, 1.5_f64, 100.0_f64, 1.5_f64],
                "limiter adaptivity 1.5 is out of range (0 to 1)",
            ),
            (
                [-1.0_f64, 0.25_f64, 100.0_f64, 0.5_f64],
                "limiter lookahead 0.25 ms is out of range (0.5 to 5 ms)",
            ),
        ] {
            assert!(
                !output(&mut instance, mode, 1.5, 1, settings),
                "mode {mode}: {refusal}"
            );
            assert_eq!(instance.message(), refusal, "mode {mode}");
        }
    }
    assert!(load_mono(&mut instance), "a load");
    assert_eq!(instance.latency(), 0, "still raw");
}

#[test]
fn reads_no_limiter_setting_for_a_raw_stage() {
    let mut instance = Instance::new();
    assert!(
        output(&mut instance, 1, 1.5, 1, DEFAULT),
        "protected, 1.5 ms at most"
    );
    assert!(
        output(
            &mut instance,
            0,
            9.0,
            7,
            [9.0_f64, 9.0_f64, 9.0_f64, 9.0_f64]
        ),
        "raw: neither a maximum, a detector nor a setting is read"
    );
    assert_eq!(instance.message(), "", "no message");
    assert!(load_mono(&mut instance), "a raw load");
    assert_eq!(instance.latency(), 0, "raw");
}

#[test]
fn checks_the_lookahead_against_the_maximum_of_the_stage_it_is_stored_with() {
    for mode in [1_u32, 2] {
        let mut instance = Instance::new();
        assert!(
            output(
                &mut instance,
                mode,
                2.0,
                1,
                [-1.0_f64, 2.0_f64, 100.0_f64, 0.5_f64]
            ),
            "mode {mode}: a lookahead at the maximum"
        );
        assert_eq!(instance.message(), "", "no message");
        assert!(
            !output(
                &mut instance,
                mode,
                2.0,
                1,
                [-3.0_f64, 2.0_f64.next_up(), 100.0_f64, 0.5_f64]
            ),
            "mode {mode}: one step above the maximum"
        );
        assert_eq!(
            instance.message(),
            "limiter lookahead 2.0000000000000004 ms is above its maximum of 2 ms"
        );
        assert!(
            !output(
                &mut instance,
                mode,
                2.0_f64.next_down(),
                1,
                [-1.0_f64, 2.0_f64, 100.0_f64, 0.5_f64]
            ),
            "mode {mode}: a maximum one step below the lookahead"
        );
        assert_eq!(
            instance.message(),
            "limiter lookahead 2 ms is above its maximum of 1.9999999999999998 ms"
        );
        // The stages refused were not stored: the next load builds the one of 2 ms.
        assert!(load_mono(&mut instance), "mode {mode}: a limited load");
        assert_eq!(
            instance.latency(),
            96 + 16,
            "mode {mode}: 2 ms at 48 kHz and the precise detector's 16"
        );
    }
}

/// The maintainer's decision on live settings (2026-11-16): a live lookahead is checked
/// against the maximum of the loaded engine's limiter, never against the maximum of the
/// stage stored for later loads. A change of what this test expects is a change of that
/// decision.
#[test]
fn checks_a_live_lookahead_against_the_loaded_engines_maximum_only() {
    let mut instance = Instance::new();
    assert!(output(&mut instance, 1, 2.0, 1, DEFAULT), "2 ms at most");
    assert!(load_mono(&mut instance), "a load of 2 ms at most");
    for (maximum, lookahead) in [(1.0_f64, 1.0_f64), (5.0_f64, 5.0_f64)] {
        assert!(
            output(
                &mut instance,
                1,
                maximum,
                1,
                [-1.0_f64, lookahead, 100.0_f64, 0.5_f64]
            ),
            "{maximum} ms at most for later loads"
        );
        assert!(
            live(&mut instance, [-1.0_f64, 2.0_f64, 100.0_f64, 0.5_f64]),
            "at the loaded engine's maximum, whatever is stored: {maximum} ms"
        );
        assert_eq!(instance.message(), "", "no message");
        assert!(
            !live(
                &mut instance,
                [-1.0_f64, 2.0_f64.next_up(), 100.0_f64, 0.5_f64]
            ),
            "one step above the loaded engine's maximum, whatever is stored: {maximum} ms"
        );
        assert_eq!(
            instance.message(),
            "limiter lookahead 2.0000000000000004 ms is above its maximum of 2 ms"
        );
    }
    assert!(load_mono(&mut instance), "a load of the stage stored last");
    assert_eq!(
        instance.latency(),
        240 + 16,
        "5 ms at 48 kHz and the precise detector's 16"
    );
}

#[test]
fn sets_the_limiter_live_only_on_a_loaded_limiter() {
    let mut instance = Instance::new();
    assert!(!live(&mut instance, DEFAULT), "no engine");
    assert_eq!(instance.message(), "no engine is loaded");
    assert!(load_mono(&mut instance), "a raw load");
    assert!(
        !live(&mut instance, DEFAULT),
        "a raw engine has no limiter to set"
    );
    assert_eq!(
        instance.message(),
        "the output stage is raw: it has no limiter to set"
    );
    assert!(
        output(
            &mut instance,
            2,
            1.0,
            1,
            [-3.0_f64, 1.0_f64, 50.0_f64, 1.0_f64]
        ),
        "switchable"
    );
    assert!(load_mono(&mut instance), "a switchable load");
    assert_eq!(
        instance.latency(),
        48 + 16,
        "1 ms at 48 kHz and the precise detector's 16"
    );
    assert!(
        !live(&mut instance, [1.0_f64, 1.0_f64, 100.0_f64, 0.5_f64]),
        "a ceiling out of range"
    );
    assert_eq!(
        instance.message(),
        "limiter ceiling 1 dBTP is out of range (-20 to 0 dBTP)"
    );
    assert!(
        live(&mut instance, [-2.0_f64, 0.5_f64, 200.0_f64, 0.0_f64]),
        "live"
    );
    assert_eq!(instance.message(), "", "no message");
}

#[test]
fn switches_the_protection_of_a_loaded_limiter() {
    let mut instance = Instance::new();
    assert!(!instance.protect(1), "no engine");
    assert_eq!(instance.message(), "no engine is loaded");
    assert!(load_mono(&mut instance), "a raw load");
    assert!(!instance.protect(1), "a raw stage");
    assert_eq!(
        instance.message(),
        "the output stage is raw: it has no limiter to set"
    );
    assert!(output(&mut instance, 1, 1.5, 1, DEFAULT), "protected");
    assert!(load_mono(&mut instance), "a protected load");
    assert!(!instance.protect(0), "a protected stage stays protected");
    assert_eq!(
        instance.message(),
        "the output stage is protected: only a switchable stage is switched to raw"
    );
    assert!(instance.protect(1), "protected, as it is");
    assert_eq!(instance.message(), "", "no message");
    assert!(output(&mut instance, 2, 1.5, 1, DEFAULT), "switchable");
    assert!(load_mono(&mut instance), "a switchable load");
    assert!(instance.protect(1), "protected");
    assert!(instance.protect(0), "raw again");
    assert!(!instance.protect(2), "an unknown code");
    assert_eq!(
        instance.message(),
        "unknown protection 2 (0 raw, 1 protected)"
    );
}

/// The largest magnitude of `blocks` blocks of the loaded engine's first channel, binary32,
/// widened to binary64.
fn peak(instance: &mut Instance, blocks: usize) -> f64 {
    (0..blocks)
        .flat_map(|_| first(instance.process()))
        .map(|bits| f64::from(f32::from_bits(u32::try_from(bits).expect("binary32 bits")).abs()))
        .fold(0.0_f64, crate::tests::larger)
}

#[test]
fn protects_what_it_is_told_to_and_accepts_a_lookahead_at_its_maximum() {
    // 10^(-1/20), the default ceiling as an amplitude, computed apart.
    let ceiling = f64::from(0.891_250_9_f32);
    let mut instance = Instance::new();
    assert!(
        output(&mut instance, 2, 1.5, 1, DEFAULT),
        "switchable, a lookahead at its maximum of 1.5 ms"
    );
    assert!(load_mono(&mut instance), "a switchable load");
    assert!(
        peak(&mut instance, 100) > 0.999_f64,
        "raw first: the sine at full scale"
    );
    assert!(instance.protect(1), "protected");
    // A second of crossfade at 48 kHz: 750 blocks.
    let _fading = peak(&mut instance, 750);
    assert!(
        peak(&mut instance, 100) <= ceiling,
        "under the ceiling once protected"
    );
    assert!(instance.protect(0), "raw");
    let _back = peak(&mut instance, 750);
    assert!(peak(&mut instance, 100) > 0.999_f64, "full scale again");
}

#[test]
fn the_settings_stored_with_a_stage_reach_the_audio_of_each_later_load() {
    // Every field differs from the defaults (`LimiterSettings::DEFAULT`, `DEFAULT`) and from
    // the other stages of these tests: 1 ms at most, the standard detector, -6 dBTP, 0.75 ms,
    // 50 ms, 0.25, so that a field lost on the way changes the bits.
    let stored = [-6.0_f64, 0.75_f64, 50.0_f64, 0.25_f64];
    let stage = crate::LimiterStage::new(
        audio_core::Binary64::new(1.0_f64),
        audio_core::Detector::Standard,
        audio_core::LimiterSettings::new(
            audio_core::Binary64::new(stored[0]),
            audio_core::Binary64::new(stored[1]),
            audio_core::Binary64::new(stored[2]),
            audio_core::Binary64::new(stored[3]),
        )
        .expect("settings in range"),
    );
    let mut instance = Instance::new();
    assert!(load_mono(&mut instance), "a raw load");
    assert!(
        output(&mut instance, 1, 1.0, 0, stored),
        "protected, stored while a raw engine is loaded"
    );
    for load in 0_u8..2 {
        assert!(load_mono(&mut instance), "protected load {load}");
        let mut expected = crate::Engine::<ENGINE_BLOCK>::new(
            MONO,
            48_000,
            RenderFormat::Binary32,
            crate::Output::Protected(stage),
        )
        .expect("an engine of the same stage");
        // Ten blocks: past the latency of 54 frames (1 ms at 48 kHz and the standard
        // detector's 6), through the sine's peaks (about 109 frames a period) and the gain's
        // release between them.
        for block in 0_u8..10 {
            assert_eq!(
                first(instance.process()),
                first(expected.process()),
                "load {load}, block {block}: the stored stage, bit for bit"
            );
        }
        // A live setting changes the loaded engine only: the next load builds the stored one.
        assert!(
            live(&mut instance, [-1.0_f64, 0.5_f64, 100.0_f64, 0.5_f64]),
            "load {load}: -1 dBTP and 0.5 ms live"
        );
    }
}

#[test]
fn builds_the_detector_it_is_told_and_refuses_an_unknown_one() {
    let mut instance = Instance::new();
    for mode in [1_u32, 2] {
        assert!(
            !output(&mut instance, mode, 1.5, 3, DEFAULT),
            "an unknown detector"
        );
        assert_eq!(
            instance.message(),
            "unknown detector 3 (0 standard, 1 precise, 2 mastering)"
        );
    }
    for (detector, delay) in [(0_u32, 6), (1, 16), (2, 64)] {
        assert!(
            output(&mut instance, 1, 1.5, detector, DEFAULT),
            "detector {detector}"
        );
        assert!(load_mono(&mut instance), "a protected load");
        assert_eq!(
            instance.latency(),
            72 + delay,
            "detector {detector}: 1.5 ms at 48 kHz and its delay"
        );
    }
}

#[test]
fn tells_a_lookahead_pending_while_it_limits_then_taken_at_the_rest_after() {
    // Through the instance, the transition the status is for: a sine of 3999 Hz at 8 kHz
    // (its samples above a tenth of full scale from frame 128 to 3,872 of every 4,000) limited
    // under -20 dBTP, 1 ms set from the block starting at frame 320 or after, pending; a
    // refused command (a lookahead above the maximum of 2 ms) keeps it pending; the ceiling
    // raised to 0 dBTP with a release of 10 ms from the block starting at frame 1,280 or after,
    // pending until the rest `K + 1` frames after that block starts, `K = 5,718` for 10 ms at
    // 8 kHz (`round(71/c) + 2`, `c = 1 - e^(-1/80)`, computed apart), taken in the block
    // holding that frame; none after.
    const BEATING: &[u8] = br#"{"format":1,"nodes":[{"id":1,"box":"osc~","params":{"frequency":3999}},{"id":2,"box":"dac~"}],"connections":[{"from":[1,0],"to":[2,0]}]}"#;
    let mut instance = Instance::new();
    assert!(
        output(
            &mut instance,
            1,
            2.0,
            0,
            [-20.0_f64, 1.5_f64, 100.0_f64, 0.5_f64]
        ),
        "a protected stage"
    );
    instance.input(BEATING.len()).copy_from_slice(BEATING);
    assert!(instance.load(8_000, 0), "{}", instance.message());
    let set = 320_usize.div_ceil(ENGINE_BLOCK);
    let raised = 1_280_usize.div_ceil(ENGINE_BLOCK);
    let taken = (raised * ENGINE_BLOCK + 5_718) / ENGINE_BLOCK;
    for block in 0..taken + 4 {
        if block == set {
            assert!(
                live(&mut instance, [-20.0_f64, 1.0_f64, 100.0_f64, 0.5_f64]),
                "set as it limits"
            );
            assert!(
                !live(&mut instance, [-20.0_f64, 3.0_f64, 100.0_f64, 0.5_f64]),
                "above the maximum, refused"
            );
        }
        if block == raised {
            assert!(
                live(&mut instance, [0.0_f64, 1.0_f64, 10.0_f64, 0.5_f64]),
                "raised over the signal"
            );
        }
        let _: Blocks<'_, ENGINE_BLOCK> = instance.process();
        assert_eq!(
            instance.lookahead_pending(),
            (set..taken).contains(&block),
            "block {block}"
        );
    }
}

#[test]
fn tells_whether_the_loaded_engines_lookahead_is_pending() {
    // Not with no engine, through a raw stage, nor while none is pending; while a protected
    // stage limiting `MONO`'s sine (a full scale over its -1 dBTP ceiling) waits for its rest
    // with a lookahead set, yes; not once set back. Read without the heap.
    assert!(!Instance::new().lookahead_pending(), "nothing loaded");
    assert!(!loaded().lookahead_pending(), "a raw stage");
    let mut instance = Instance::new();
    assert!(
        output(&mut instance, 1, 5.0, 0, DEFAULT),
        "a protected stage"
    );
    assert!(load_mono(&mut instance), "{}", instance.message());
    for _ in 0_u8..20 {
        let _: Blocks<'_, ENGINE_BLOCK> = instance.process();
    }
    assert!(!instance.lookahead_pending(), "none pending");
    assert!(
        live(&mut instance, [-1.0_f64, 5.0_f64, 100.0_f64, 0.5_f64]),
        "set"
    );
    let operations = audio_testkit::heap_operations_during(|| {
        for _ in 0_u8..20 {
            let _: Blocks<'_, ENGINE_BLOCK> = instance.process();
        }
        instance.lookahead_pending()
    });
    assert_eq!(
        operations,
        Ok((true, 0)),
        "pending while it limits, read without the heap"
    );
    assert!(live(&mut instance, DEFAULT), "set back");
    assert!(!instance.lookahead_pending(), "dropped");
}
