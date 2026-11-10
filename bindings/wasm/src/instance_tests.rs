//! Tests of an engine instance and its patch loading.

#![forbid(unsafe_code)]

use engine_core::host::Raw;
use engine_core::{BLOCK_SIZE, RenderFormat};

use super::Instance;
use crate::Blocks;

/// The first channel of `blocks`, widened to 64 bits.
fn first(blocks: Blocks<'_>) -> Vec<u64> {
    match blocks {
        Blocks::Binary64(channels) => channels[0].iter().map(|sample| sample.bits()).collect(),
        Blocks::Binary32(channels) => channels[0].iter().map(|sample| sample.bits()).collect(),
    }
}

/// The blocks of a binary32 render, or `None` for binary64.
fn narrow(blocks: Blocks<'_>) -> Option<&[engine_core::Block<engine_core::Binary32>]> {
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

#[test]
fn starts_empty() {
    let mut instance = Instance::default();
    assert_eq!(instance.channels(), 0, "no engine");
    assert!(instance.process().is_empty(), "no blocks");
    assert_eq!(instance.message(), "", "no error");
}

#[test]
fn loads_in_the_format_of_its_code() {
    for (code, format) in (0_u32..).zip(RenderFormat::ALL) {
        let mut instance = Instance::new();
        instance.input(MONO.len()).copy_from_slice(MONO);
        assert!(instance.load(8_000, code), "{}", instance.message());
        let mut engine = crate::Engine::new(MONO, 8_000, format).expect("valid");
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
                "unknown render format: {code} (supported: 0 binary64, 1 binary64-as-binary32, 2 binary32)"
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
        engine_patch::parse_patch(core::str::from_utf8(MONO).expect("UTF-8")).expect("valid patch");
    for rate in engine_testkit::matrix::sample_rates() {
        let mut instance = Instance::new();
        instance.input(MONO.len()).copy_from_slice(MONO);
        assert!(instance.load(rate.hz(), 0), "{}", instance.message());
        let expected = engine_core::Renderer::new(
            engine_core::Plan::<engine_core::Binary64>::build(
                &patch,
                &engine_objects::REGISTRY,
                rate,
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
        let operations = engine_testkit::heap_operations_during(|| {
            for _ in 0_u16..100_u16 {
                let _: Blocks<'_> = instance.process();
            }
        });
        assert_eq!(operations, Ok(((), 0)), "format {code}: 100 blocks");
    }
    let mut empty = Instance::new();
    let none = engine_testkit::heap_operations_during(|| {
        let _: Blocks<'_> = empty.process();
    });
    assert_eq!(none, Ok(((), 0)), "nothing loaded");
}

#[test]
fn a_failed_load_unloads_and_explains() {
    let mut instance = loaded();
    instance.input(1).copy_from_slice(b"x");
    assert!(!instance.load(48_000, 1), "invalid patch");
    assert_eq!(instance.channels(), 0, "unloaded");
    assert!(instance.process().is_empty(), "no blocks");
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
        engine_patch::MAX_MESSAGE_CHARS,
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
