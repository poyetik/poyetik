//! Tests of the C interface, at the pointer level: they use `unsafe` under the lifting of
//! the FFI module that declares them, and run under Miri.

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
    // words, valid until the next call into the module (the protocol, `ffi.rs`), and none
    // comes before this read.
    let words = unsafe { core::slice::from_raw_parts(pointer, 4) };
    [words[0], words[1], words[2], words[3]]
}

/// Reads the exported error message, as JavaScript does: right after the call it explains,
/// its length first and its address last, so that no call comes between the address and the
/// read.
fn message() -> Vec<u8> {
    let len = engine_message_len();
    let pointer = engine_message_ptr();
    // SAFETY: `engine_message_ptr` returns the address of the message's `len` initialized
    // bytes, valid until the next call into the module (the protocol, `ffi.rs`), and none
    // comes before this read.
    unsafe { core::slice::from_raw_parts(pointer, len) }.to_vec()
}

/// Renders one block through the exported interface and copies its bytes, planar:
/// `width` bytes per sample, as JavaScript reads them.
fn block_bytes(width: usize) -> Vec<u8> {
    // The length first, the address last, as for the message.
    let len = engine_channels()
        .checked_mul(engine_block_size())
        .and_then(|samples| samples.checked_mul(width))
        .expect("small block");
    let pointer = engine_process();
    // SAFETY: the block is `channels × block size` initialized samples of `width` bytes
    // at `pointer`, valid until the next call into the module (the protocol, `ffi.rs`), and
    // none comes before this read.
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
        Plan::<Binary64, Box<dyn audio_core::Object<Binary64>>>::build(
            &patch,
            &REGISTRY,
            rate,
            audio_core::ExecutionMode::Offline,
        )
        .expect("plan")
    };
    let (exported, expected): (Vec<u64>, Vec<u64>) = match format {
        RenderFormat::Binary64 => (
            wide_block(),
            Renderer::<_, _, f64>::new(wide())
                .render(BLOCK_SIZE)
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
            Renderer::<_, _, f32>::new(wide())
                .render(BLOCK_SIZE)
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
            Renderer::<_, _, f32>::new(
                Plan::<Binary32, Box<dyn audio_core::Object<Binary32>>>::build(
                    &patch,
                    &REGISTRY,
                    rate,
                    audio_core::ExecutionMode::Offline,
                )
                .expect("plan"),
            )
            .render(BLOCK_SIZE)
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
    assert_eq!(engine_block_size(), BLOCK_SIZE, "engine block");
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
    // Each live command, refused with no engine loaded: a literal's `to_owned`, made in 2.
    assert_refused(
        &[
            ("engine_limiter, no engine", || {
                engine_limiter(-2.0, 1.0, 100.0, 0.5)
            }),
            ("engine_protect, no engine", || engine_protect(1)),
        ],
        b"no engine is loaded",
        2,
    );

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
            audio_core::ExecutionMode::Offline,
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

/// The heap operations of `call`, a live command of the C interface, which the audio thread
/// calls: it must answer `answer` (1 accepted, 0 refused).
fn heap_operations_of(call: impl FnOnce() -> u32, answer: u32) -> usize {
    let (answered, operations) = audio_testkit::heap_operations_during(call).expect("counted");
    assert_eq!(answered, answer, "the answer of the command counted");
    operations
}

/// The heap operations of the refusal of a live command explained by `explanation` (ASCII)
/// and made by `making` heap operations, after an earlier refusal: until #24 step 130 writes
/// it in place, 2 per character (`safe_message` makes a `String` of each and releases it, #24
/// step 11), the safe copy's growth by doubling from 8 bytes to hold them (one allocation,
/// then a reallocation per doubling), `making` for the explanation made and released, and 1
/// for the release of the earlier message's buffer.
///
/// `making` is written by each caller as a literal, derived from how its explanation is
/// made: 2 for one allocation and its release (a literal's `to_owned`, a `Display` of one
/// piece, a `format!`, which sizes its `String` at twice its literal pieces before it
/// writes); `to_string` of a `Display` of several pieces starts from an empty `String`,
/// allocates its first piece (8 bytes at least), then reallocates each time a piece
/// overflows it, to twice its capacity or to what it needs when that is more.
fn refusal_operations(explanation: &[u8], making: usize) -> usize {
    let characters = explanation.len();
    let mut growth = 1_usize;
    let mut capacity = 8_usize;
    while capacity < characters {
        capacity = capacity.saturating_mul(2);
        growth = growth.saturating_add(1);
    }
    characters
        .saturating_mul(2)
        .saturating_add(growth)
        .saturating_add(making)
        .saturating_add(1)
}

/// A live command of the C interface, named, as the audio thread calls it.
type Command = (&'static str, fn() -> u32);

/// Calls each of `commands`, each refused with `explanation`, made by `making` heap
/// operations, after an earlier refusal, its heap operations counted exactly
/// ([`refusal_operations`]).
fn assert_refused(commands: &[Command], explanation: &[u8], making: usize) {
    for (command, call) in commands {
        assert_eq!(
            heap_operations_of(call, 0),
            refusal_operations(explanation, making),
            "{command}: refused"
        );
        assert_eq!(message(), explanation, "{command}: explained");
    }
}

/// Calls each of `commands`, each accepted with no heap operation, the message buffer of an
/// earlier refusal kept, and leaving no message.
fn assert_accepted_without_the_heap(commands: &[Command]) {
    for (command, call) in commands {
        assert_eq!(
            heap_operations_of(call, 1),
            0,
            "{command}: no heap operation"
        );
        assert_eq!(message(), b"", "{command}: no message");
    }
}

#[test]
fn counts_the_operations_of_a_refusal_as_the_safe_copy_grows() {
    // Written apart: 8 bytes hold 8 characters, 9 take a doubling.
    assert_eq!(refusal_operations(b"", 2), 4, "nothing to copy");
    assert_eq!(
        refusal_operations(b"12345678", 2),
        16 + 1 + 3,
        "8 characters"
    );
    assert_eq!(
        refusal_operations(b"123456789", 2),
        18 + 2 + 3,
        "9 characters"
    );
    assert_eq!(
        refusal_operations(&[b'a'; 73], 2),
        146 + 5 + 3,
        "73 characters: 8, 16, 32, 64, 128"
    );
    assert_eq!(
        refusal_operations(b"12345678", 5),
        16 + 1 + 6,
        "an explanation made in 5 operations"
    );
}

/// The output stage's part of the protocol, after a raw load of `sine`: raw so far, then
/// protected for the next load, set live.
fn follows_the_output_protocol(sine: &[u8]) {
    assert_eq!(engine_latency(), 0, "raw: no latency");
    assert_eq!(engine_protect(1), 0, "a raw stage has no limiter");
    let raw = b"the output stage is raw: it has no limiter to set";
    assert_eq!(message(), raw, "explained");
    // Each live command, refused, on the audio thread where the worklet calls it: the
    // deviation CLAUDE.md states, counted exactly, 105 operations for 49 characters, the
    // explanation a `Display` of one piece, made in 2.
    assert_eq!(refusal_operations(raw, 2), 105, "derived apart");
    assert_refused(
        &[
            ("engine_limiter, raw", || {
                engine_limiter(-2.0, 1.0, 100.0, 0.5)
            }),
            ("engine_protect, raw", || engine_protect(0)),
        ],
        raw,
        2,
    );
    // A code other than 0 or 1, whatever the stage: a `format!` of 40 bytes of literal
    // pieces, sized at 80 for 41 bytes, made in 2.
    assert_refused(
        &[("engine_protect, an unknown code", || engine_protect(2))],
        b"unknown protection 2 (0 raw, 1 protected)",
        2,
    );
    assert_eq!(
        engine_output(3, 1.5, 1, -1.0, 1.5, 100.0, 0.5),
        0,
        "an unknown mode"
    );
    assert_eq!(
        engine_output(1, 2.0, 1, -1.0, 2.0_f64.next_up(), 100.0, 0.5),
        0,
        "a lookahead above the maximum stored with it"
    );
    assert_eq!(
        message(),
        b"limiter lookahead 2.0000000000000004 ms is above its maximum of 2 ms",
        "explained"
    );
    assert_eq!(
        engine_output(1, 2.0, 1, -1.0, 1.5, 100.0, 0.5),
        1,
        "protected, 2 ms at most"
    );
    assert_eq!(message(), b"", "no message after a success");
    assert_eq!(
        engine_limiter(-2.0, 1.0, 100.0, 0.5),
        0,
        "the raw engine loaded has no limiter"
    );
    write(sine);
    assert_eq!(engine_load(48_000, 2), 1, "a protected load");
    assert_eq!(engine_latency(), 96 + 16, "2 ms at 48 kHz");
    // Above the loaded engine's maximum: a `Display` of several pieces, its first
    // "limiter lookahead " (18 bytes), 53 bytes in all: grown to 18, then 36 at "2" and 72
    // at " is above its maximum of ", an allocation, 2 reallocations and a release, made
    // in 4.
    assert_refused(
        &[("engine_limiter, above the maximum", || {
            engine_limiter(-1.0, 2.5, 100.0, 0.5)
        })],
        b"limiter lookahead 2.5 ms is above its maximum of 2 ms",
        4,
    );
    // A literal's `to_owned`, made in 2.
    let locked = b"the output stage is protected: only a switchable stage is switched to raw";
    assert_refused(
        &[("engine_protect, protected toward raw", || engine_protect(0))],
        locked,
        2,
    );
    // Each live command, accepted after a refusal: a lookahead shrinking and growing within
    // the maximum, a protected stage told protected.
    assert_accepted_without_the_heap(&[
        ("engine_limiter, shorter", || {
            engine_limiter(-3.0, 0.5, 50.0, 1.0)
        }),
        ("engine_limiter, longer", || {
            engine_limiter(-1.0, 2.0, 100.0, 0.5)
        }),
        ("engine_protect, protected as it is", || engine_protect(1)),
    ]);
    // The stage's own blocks, over its latency, live changes and crossfades, are rendered
    // without a heap operation in `audio-host`'s tests, natively at every rate and in every
    // format; here, the C interface adds none to a protected render. Two blocks: the calls
    // are the same at every block, and each costs seconds under Miri.
    let protected_operations = audio_testkit::heap_operations_during(|| {
        for _ in 0_u8..2_u8 {
            let _: *const u8 = core::hint::black_box(engine_process());
        }
    });
    assert_eq!(
        protected_operations,
        Ok(((), 0)),
        "a protected stage does not use the heap"
    );
    follows_the_switchable_protocol(sine);
}

/// A switchable stage stored for the next load of `sine`, set and switched live, then the
/// stored stage's settings.
fn follows_the_switchable_protocol(sine: &[u8]) {
    assert_eq!(
        engine_output(2, 2.0, 1, -1.0, 1.5, 100.0, 0.5),
        1,
        "switchable, 2 ms at most"
    );
    write(sine);
    assert_eq!(engine_load(48_000, 2), 1, "a switchable load");
    // A ceiling out of range: a `Display` of several pieces, its first "limiter " (8 bytes),
    // 54 bytes in all: grown to 8, then 16 at "ceiling", 32 at "1" and 64 at " is out of
    // range (", an allocation, 3 reallocations and a release, made in 5.
    assert_refused(
        &[("engine_limiter, a ceiling out of range", || {
            engine_limiter(1.0, 1.0, 100.0, 0.5)
        })],
        b"limiter ceiling 1 dBTP is out of range (-20 to 0 dBTP)",
        5,
    );
    assert_accepted_without_the_heap(&[
        ("engine_protect, protected by a crossfade", || {
            engine_protect(1)
        }),
        ("engine_protect, raw by a crossfade", || engine_protect(0)),
    ]);
    follows_the_stored_stage_protocol(sine);
}

/// The settings stored with a limited stage for the next load of `sine` reach its audio, then
/// a raw stage is stored again.
fn follows_the_stored_stage_protocol(sine: &[u8]) {
    // A ceiling of -6 dBTP stored with the stage before a load reaches its audio:
    // 10^(-6/20), computed apart, the nearest binary64; the sine at full scale is held under
    // it.
    let ceiling = 0.501_187_233_627_272_2_f64;
    assert_eq!(
        engine_output(1, 0.5, 1, -6.0, 0.5, 100.0, 0.5),
        1,
        "protected at -6 dBTP, 0.5 ms at most"
    );
    write(sine);
    assert_eq!(engine_load(8_000, 2), 1, "a protected load at 8 kHz");
    // Two blocks: past the latency of 20 frames (0.5 ms at 8 kHz, 4, and the precise
    // detector's 16), the sine, of 18 frames a period, reaches its peak.
    let peak = (0_u8..2_u8)
        .flat_map(|_| block())
        .map(|sample| f64::from(sample.abs()))
        .fold(0.0_f64, crate::tests::larger);
    assert!(peak <= ceiling, "{peak} above -6 dBTP");
    assert!(
        peak > ceiling / 2.0_f64,
        "{peak}: the sine limited, not silenced"
    );
    assert_eq!(
        engine_output(0, 9.0, 7, 9.0, 9.0, 9.0, 9.0),
        1,
        "raw again for later loads, nothing else read"
    );
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
