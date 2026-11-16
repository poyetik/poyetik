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
        let expected = audio_core::Renderer::new(
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
        .render::<f64>(BLOCK_SIZE)
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

#[test]
fn builds_every_later_load_with_the_output_set() {
    let mut instance = Instance::new();
    assert!(load_mono(&mut instance), "a raw load");
    assert_eq!(instance.latency(), 0, "raw by default: no latency");
    let mut before = 0;
    for (mode, latency) in [(1_u32, 72 + 16), (2, 72 + 16), (0, 0)] {
        assert!(instance.output(mode, 1.5, 1), "mode {mode}");
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
    assert!(!instance.output(3, 1.5, 1), "an unknown mode");
    assert_eq!(
        instance.message(),
        "unknown output mode 3 (0 raw, 1 protected, 2 switchable)"
    );
    assert!(!instance.output(u32::MAX, 1.5, 1), "the largest code");
    for maximum in [0.25_f64, 5.5_f64, f64::NAN] {
        assert!(!instance.output(1, maximum, 1), "{maximum}");
        assert!(
            instance.message().starts_with("limiter maximum lookahead "),
            "{}",
            instance.message()
        );
    }
    assert!(load_mono(&mut instance), "a load");
    assert_eq!(instance.latency(), 0, "still raw");
}

#[test]
fn reads_no_limiter_setting_for_a_raw_stage() {
    let mut instance = Instance::new();
    assert!(instance.output(1, 1.0, 1), "protected, 1 ms at most");
    assert!(
        instance.output(0, 9.0, 7),
        "raw: neither a maximum nor a detector is read"
    );
    assert_eq!(instance.message(), "", "no message");
    assert!(load_mono(&mut instance), "a raw load");
    assert_eq!(instance.latency(), 0, "raw");
}

#[test]
fn checks_the_settings_stored_for_later_loads_at_the_next_load() {
    let mut instance = Instance::new();
    // The default lookahead, 1.5 ms, is above a maximum of 1 ms: stored, refused at the load.
    assert!(instance.output(1, 1.0, 1), "a maximum below the lookahead");
    assert_eq!(instance.message(), "", "no message");
    assert!(
        !load_mono(&mut instance),
        "a load whose limiter cannot be built"
    );
    assert_eq!(
        instance.message(),
        "limiter lookahead 1.5 ms is above its maximum of 1 ms"
    );
    assert_eq!(instance.channels(), 0, "no engine loaded");
    assert!(
        instance.limiter(-1.0, 1.0, 100.0, 0.5),
        "a lookahead it fits"
    );
    assert!(load_mono(&mut instance), "a protected load");
    assert_eq!(
        instance.latency(),
        48 + 16,
        "1 ms at 48 kHz and the precise detector's 16"
    );
}

#[test]
fn checks_a_live_lookahead_against_the_loaded_engines_maximum() {
    let mut instance = Instance::new();
    assert!(instance.output(1, 5.0, 1), "5 ms at most");
    assert!(load_mono(&mut instance), "a protected load");
    assert!(instance.output(1, 1.0, 1), "1 ms at most for later loads");
    assert!(
        instance.limiter(-1.0, 3.0, 100.0, 0.5),
        "3 ms live: within the loaded engine's maximum"
    );
    assert_eq!(instance.message(), "", "no message");
    assert!(
        !load_mono(&mut instance),
        "3 ms is above the next load's maximum"
    );
    assert_eq!(
        instance.message(),
        "limiter lookahead 3 ms is above its maximum of 1 ms"
    );
    assert!(
        instance.limiter(-1.0, 1.0, 100.0, 0.5),
        "1 ms, no engine loaded"
    );
    assert!(load_mono(&mut instance), "a load of 1 ms at most");
    assert_eq!(
        instance.latency(),
        48 + 16,
        "1 ms at 48 kHz and the precise detector's 16"
    );
    assert!(instance.output(1, 5.0, 1), "5 ms at most for later loads");
    assert!(
        !instance.limiter(-1.0, 3.0, 100.0, 0.5),
        "3 ms live: above the loaded engine's maximum"
    );
    assert_eq!(
        instance.message(),
        "limiter lookahead 3 ms is above its maximum of 1 ms"
    );
    assert!(
        load_mono(&mut instance),
        "the settings refused were not stored"
    );
    assert_eq!(
        instance.latency(),
        240 + 16,
        "5 ms at 48 kHz and the precise detector's 16"
    );
}

#[test]
fn sets_the_limiter_live_and_for_later_loads() {
    let mut instance = Instance::new();
    assert!(
        instance.limiter(-3.0, 1.0, 50.0, 1.0),
        "settings for later loads"
    );
    assert!(instance.output(1, 1.0, 1), "a maximum the lookahead fits");
    assert!(
        !instance.limiter(1.0, 1.0, 100.0, 0.5),
        "a ceiling out of range"
    );
    assert_eq!(
        instance.message(),
        "limiter ceiling 1 dBTP is out of range (-20 to 0 dBTP)"
    );
    assert!(load_mono(&mut instance), "a protected load");
    assert_eq!(
        instance.latency(),
        48 + 16,
        "1 ms at 48 kHz and the precise detector's 16"
    );
    assert!(instance.limiter(-2.0, 0.5, 200.0, 0.0), "live");
    assert_eq!(instance.message(), "", "no message");
    assert!(instance.output(0, 5.0, 1), "raw for later loads");
    assert!(load_mono(&mut instance), "a raw load");
    assert!(
        !instance.limiter(-2.0, 4.0, 200.0, 0.0),
        "a raw engine has no limiter to set"
    );
    assert_eq!(
        instance.message(),
        "the output stage is raw: it has no limiter to set"
    );
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
    assert!(instance.output(2, 1.5, 1), "switchable");
    assert!(load_mono(&mut instance), "a switchable load");
    assert!(instance.protect(1), "protected");
    assert!(instance.protect(0), "raw again");
    assert!(!instance.protect(2), "an unknown code");
    assert_eq!(
        instance.message(),
        "unknown protection 2 (0 raw, 1 protected)"
    );
}

/// The largest magnitude of `blocks` blocks of the loaded engine's first channel, binary32.
fn peak(instance: &mut Instance, blocks: usize) -> f32 {
    (0..blocks)
        .flat_map(|_| first(instance.process()))
        .map(|bits| f32::from_bits(u32::try_from(bits).expect("binary32 bits")).abs())
        .fold(0.0_f32, |largest, magnitude| {
            if magnitude > largest {
                magnitude
            } else {
                largest
            }
        })
}

#[test]
fn protects_what_it_is_told_to_and_accepts_a_lookahead_at_its_maximum() {
    // 10^(-1/20), the default ceiling as an amplitude, computed apart.
    let ceiling = 0.891_250_9_f32;
    let mut instance = Instance::new();
    assert!(instance.output(2, 1.5, 1), "switchable, 1.5 ms at most");
    assert!(
        instance.limiter(-1.0, 1.5, 100.0, 0.5),
        "a lookahead at its maximum"
    );
    assert!(load_mono(&mut instance), "a switchable load");
    assert!(
        peak(&mut instance, 100) > 0.999,
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
    assert!(peak(&mut instance, 100) > 0.999, "full scale again");
}

#[test]
fn builds_the_detector_it_is_told_and_refuses_an_unknown_one() {
    let mut instance = Instance::new();
    assert!(!instance.output(1, 1.5, 3), "an unknown detector");
    assert_eq!(
        instance.message(),
        "unknown detector 3 (0 standard, 1 precise, 2 mastering)"
    );
    for (detector, delay) in [(0_u32, 6), (1, 16), (2, 64)] {
        assert!(instance.output(1, 1.5, detector), "detector {detector}");
        assert!(load_mono(&mut instance), "a protected load");
        assert_eq!(
            instance.latency(),
            72 + delay,
            "detector {detector}: 1.5 ms at 48 kHz and its delay"
        );
    }
}
