//! Tests of the C interface, at the pointer level: they use `unsafe` under the lifting of
//! the FFI module that declares them, and run under Miri.

use alloc::vec::Vec;

use audio_boxes::REGISTRY;
use audio_core::{
    BLOCK_SIZE, Binary32, Binary64, Plan, RenderFormat, Renderer, SampleRate, SubnormalArithmetic,
    SubnormalsKept,
};

use super::{
    ABI_VERSION, engine_abi_version, engine_block_size, engine_channels, engine_input,
    engine_latency, engine_limiter, engine_load, engine_max_patch_bytes, engine_message_len,
    engine_message_ptr, engine_output, engine_process, engine_protect, engine_replaced,
    engine_subnormals, subnormals_code,
};

/// Writes `bytes` through the exported input buffer, as JavaScript does.
fn write(bytes: &[u8]) {
    let pointer = engine_input(bytes.len());
    // SAFETY: `engine_input` returned a buffer of exactly `bytes.len()` writable
    // bytes, which cannot overlap `bytes`, and nothing else runs until `load`.
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer, bytes.len()) };
}

/// Reads what the output stage replaced through the exported interface, as JavaScript
/// does: four words.
fn replaced() -> [u32; 4] {
    let pointer = engine_replaced();
    // SAFETY: `engine_replaced` returns the address of four initialized, aligned `u32`
    // words, unchanged until the next call.
    let words = unsafe { core::slice::from_raw_parts(pointer, 4) };
    [words[0], words[1], words[2], words[3]]
}

/// Reads the exported error message, as JavaScript does.
fn message() -> Vec<u8> {
    let (pointer, len) = (engine_message_ptr(), engine_message_len());
    // SAFETY: the message is `len` initialized bytes at `pointer`, unchanged until
    // the next load.
    unsafe { core::slice::from_raw_parts(pointer, len) }.to_vec()
}

/// Renders one block through the exported interface and copies its bytes, planar:
/// `width` bytes per sample, as JavaScript reads them.
fn block_bytes(width: usize) -> Vec<u8> {
    let pointer = engine_process();
    let len = engine_channels()
        .checked_mul(engine_block_size())
        .and_then(|samples| samples.checked_mul(width))
        .expect("small block");
    // SAFETY: the block is `channels × block size` initialized samples of `width` bytes
    // at `pointer`, unchanged until the next call.
    unsafe { core::slice::from_raw_parts(pointer, len) }.to_vec()
}

/// Renders one block of a binary32 format through the exported interface, planar.
fn block() -> Vec<f32> {
    block_bytes(4)
        .as_chunks::<4>()
        .0
        .iter()
        .map(|bytes| f32::from_le_bytes(*bytes))
        .collect()
}

/// Renders one block of binary64 through the exported interface, planar, as bits.
fn wide_block() -> Vec<u64> {
    block_bytes(8)
        .as_chunks::<8>()
        .0
        .iter()
        .map(|bytes| u64::from_le_bytes(*bytes))
        .collect()
}

/// The codes of the render formats of the C interface, written here as facts: compared with
/// `RenderFormat::ALL` (whose order is the codes) by `follows_the_protocol`, so that a
/// format added there fails until it is listed here.
const FORMAT_CODES: [(u32, RenderFormat); 3] = [
    (0, RenderFormat::Binary64),
    (1, RenderFormat::Binary64AsBinary32),
    (2, RenderFormat::Binary32),
];

/// The first channel of `patch`'s first block at `rate`, in `format`, loaded by its `code`
/// through the exported interface, as bits; and the same from the renderer.
fn first_channels(
    sine: &[u8],
    rate: SampleRate,
    code: u32,
    format: RenderFormat,
) -> (Vec<u64>, Vec<u64>) {
    let patch =
        audio_patch::parse_patch(core::str::from_utf8(sine).expect("UTF-8")).expect("valid patch");
    write(sine);
    assert_eq!(
        engine_load(rate.hz(), code),
        1,
        "loaded at {} Hz",
        rate.hz()
    );
    let wide = || {
        Renderer::new(
            Plan::<Binary64, Box<dyn audio_core::Object<Binary64>>>::build(&patch, &REGISTRY, rate)
                .expect("plan"),
        )
    };
    let (exported, expected): (Vec<u64>, Vec<u64>) = match format {
        RenderFormat::Binary64 => (
            wide_block(),
            wide()
                .render::<f64>(BLOCK_SIZE)
                .expect("small render")
                .iter()
                .map(|sample| sample.to_bits())
                .collect(),
        ),
        RenderFormat::Binary64AsBinary32 => (
            block()
                .iter()
                .map(|sample| u64::from(sample.to_bits()))
                .collect(),
            wide()
                .render::<f32>(BLOCK_SIZE)
                .expect("small render")
                .iter()
                .map(|sample| u64::from(sample.to_bits()))
                .collect(),
        ),
        RenderFormat::Binary32 => (
            block()
                .iter()
                .map(|sample| u64::from(sample.to_bits()))
                .collect(),
            Renderer::new(
                Plan::<Binary32, Box<dyn audio_core::Object<Binary32>>>::build(
                    &patch, &REGISTRY, rate,
                )
                .expect("plan"),
            )
            .render::<f32>(BLOCK_SIZE)
            .expect("small render")
            .iter()
            .map(|sample| u64::from(sample.to_bits()))
            .collect(),
        ),
    };
    (
        exported.into_iter().take(BLOCK_SIZE).collect(),
        expected.into_iter().step_by(2).collect(),
    )
}

/// The whole protocol, in one test: the state is global to the module.
#[test]
fn follows_the_protocol() {
    assert_eq!(engine_abi_version(), ABI_VERSION, "version");
    assert_eq!(ABI_VERSION >> 16_u32, 0x5059_u32, "identity");
    assert_eq!(ABI_VERSION & 0xFFFF_u32, 5_u32, "fifth version");
    assert_eq!(engine_block_size(), BLOCK_SIZE, "block size");
    assert_eq!(
        engine_max_patch_bytes(),
        audio_patch::MAX_PATCH_BYTES,
        "the limit of the patch reader"
    );

    write(b"not a patch");
    assert_eq!(engine_load(48_000, 1), 0, "rejected");
    assert_eq!(engine_channels(), 0, "nothing loaded");
    assert_eq!(replaced(), [0, 0, 0, 0], "nothing loaded, nothing replaced");
    let explanation = message();
    assert_eq!(
        explanation, b"invalid patch: expected ident at line 1 column 2",
        "explained"
    );
    assert!(core::str::from_utf8(&explanation).is_ok(), "UTF-8");

    let sine = br#"{"format":1,"nodes":[{"id":1,"box":"osc~"},{"id":2,"box":"dac~"}],
        "connections":[{"from":[1,0],"to":[2,0]},{"from":[1,0],"to":[2,1]}]}"#;
    write(sine);
    assert_eq!(engine_load(48_000, 3), 0, "an unknown render format");
    assert_eq!(
        message(),
        b"unknown render format 3 (0 binary64, 1 binary64-as-binary32, 2 binary32)",
        "explained"
    );
    write(sine);
    assert_eq!(engine_load(48_000, 1), 1, "loaded");
    assert_eq!(engine_message_len(), 0, "no error");
    assert_eq!(engine_channels(), 2, "stereo");

    let patch =
        audio_patch::parse_patch(core::str::from_utf8(sine).expect("UTF-8")).expect("valid patch");
    let mut renderer = Renderer::new(
        Plan::<Binary64, Box<dyn audio_core::Object<Binary64>>>::build(
            &patch,
            &REGISTRY,
            SampleRate::HZ_48000,
        )
        .expect("valid plan"),
    );
    let expected: Vec<f32> = renderer.render(2 * BLOCK_SIZE).expect("small render");
    let mut actual = Vec::new();
    for _ in 0..2_u8 {
        let planar = block();
        let (left, right) = planar.split_at(BLOCK_SIZE);
        for (left_sample, right_sample) in left.iter().zip(right) {
            actual.extend([*left_sample, *right_sample]);
        }
    }
    let bits = |samples: &[f32]| {
        samples
            .iter()
            .map(|sample| sample.to_bits())
            .collect::<Vec<_>>()
    };
    assert_eq!(bits(&actual), bits(&expected), "bit-identical");
    assert_eq!(replaced(), [0, 0, 0, 0], "a sine replaces nothing");

    // What the audio thread calls, block after block: no heap operation.
    let operations = audio_testkit::heap_operations_during(|| {
        for _ in 0_u16..100_u16 {
            let _: *const u8 = core::hint::black_box(engine_process());
            let _: usize = core::hint::black_box(engine_channels());
            let _: *const u32 = core::hint::black_box(engine_replaced());
        }
    });
    assert_eq!(
        operations,
        Ok(((), 0)),
        "engine_process does not use the heap"
    );

    // The codes are those of `RenderFormat::ALL`, every one of them.
    assert_eq!(
        FORMAT_CODES.map(|(_, format)| format),
        RenderFormat::ALL,
        "every render format, in the order of its code"
    );
    // `engine_load` hands its rate and format to the engine, at every rate of the matrix.
    for rate in audio_testkit::matrix::sample_rates() {
        for (code, format) in FORMAT_CODES {
            let (exported, reference) = first_channels(sine, rate, code, format);
            assert_eq!(exported, reference, "format {format} at {} Hz", rate.hz());
        }
    }

    follows_the_output_protocol(sine);
}

/// The output stage's part of the protocol, after a raw load of `sine`: raw so far, then
/// protected for the next load, set live.
fn follows_the_output_protocol(sine: &[u8]) {
    assert_eq!(engine_latency(), 0, "raw: no latency");
    assert_eq!(engine_protect(1), 0, "a raw stage has no limiter");
    assert_eq!(
        message(),
        b"the output stage is raw: it has no limiter to set",
        "explained"
    );
    assert_eq!(engine_output(3, 1.5), 0, "an unknown mode");
    assert_eq!(engine_output(1, 2.0), 1, "protected, 2 ms at most");
    assert_eq!(message(), b"", "no message after a success");
    assert_eq!(
        engine_limiter(-1.0, 2.5, 100.0, 0.5),
        0,
        "above the maximum"
    );
    assert_eq!(
        engine_limiter(-2.0, 1.0, 100.0, 0.5),
        0,
        "the raw engine loaded has no limiter"
    );
    write(sine);
    assert_eq!(engine_load(48_000, 2), 1, "a protected load");
    assert_eq!(
        engine_latency(),
        96 + audio_core::DETECTOR_DELAY,
        "2 ms at 48 kHz"
    );
    assert_eq!(engine_limiter(-3.0, 0.5, 50.0, 1.0), 1, "live");
    assert_eq!(engine_protect(0), 1, "raw by a crossfade");
    assert_eq!(engine_protect(1), 1, "protected again");
    let protected_operations = audio_testkit::heap_operations_during(|| {
        for _ in 0_u16..100_u16 {
            let _: *const u8 = core::hint::black_box(engine_process());
        }
    });
    assert_eq!(
        protected_operations,
        Ok(((), 0)),
        "a protected stage does not use the heap"
    );
    assert_eq!(engine_output(0, 5.0), 1, "raw again for later loads");
}

#[test]
fn observes_ieee_subnormals_in_the_default_mode() {
    // The tests run in the processor's default mode: every subnormal kept, every bit set.
    assert_eq!(engine_subnormals(), 0b1111_u32);
}

#[test]
fn writes_each_subnormal_observation_as_its_own_bit() {
    // Every combination, against the bits written as literals.
    let kept = |operands, results| SubnormalsKept { operands, results };
    let cases = [
        ((false, false, false, false), 0b0000_u32),
        ((true, false, false, false), 0b0001_u32),
        ((false, true, false, false), 0b0010_u32),
        ((false, false, true, false), 0b0100_u32),
        ((false, false, false, true), 0b1000_u32),
        ((true, true, false, false), 0b0011_u32),
        ((false, false, true, true), 0b1100_u32),
        ((true, false, true, false), 0b0101_u32),
        ((true, true, true, true), 0b1111_u32),
    ];
    for ((operands_32, results_32, operands_64, results_64), code) in cases {
        let arithmetic = SubnormalArithmetic {
            binary32: kept(operands_32, results_32),
            binary64: kept(operands_64, results_64),
        };
        assert_eq!(subnormals_code(arithmetic), code, "{arithmetic:?}");
    }
}
