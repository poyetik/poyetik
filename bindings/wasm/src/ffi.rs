//! The C interface exported by the WebAssembly module.
//!
//! Protocol, for one module instance (one engine):
//!
//! 1. `engine_input(len)` ([`engine_input`]) returns where to write the `len` bytes of a patch;
//!    the caller first refuses a patch longer than [`engine_max_patch_bytes`], so that the
//!    module memory never grows for a patch the engine would refuse anyway.
//! 2. `engine_load(rate, format)` ([`engine_load`]) loads it, to render in the render format
//!    of code `format` (0 binary64, 1 binary64 given as binary32, 2 binary32); on failure,
//!    [`engine_message_ptr`] and [`engine_message_len`] locate a UTF-8 explanation.
//! 3. [`engine_process`] renders the next block and returns where it is:
//!    [`engine_channels`] × [`engine_block_size`] samples, channel after channel: `f64`
//!    samples (8 bytes) in binary64, `f32` samples (4 bytes) in the two binary32 formats.
//!
//! Every pointer is valid only until the next call: the caller copies what it needs
//! and never keeps a view of the memory across calls, which may grow it.
//!
//! Before anything else, the caller checks [`engine_abi_version`]: a module with another
//! version is a different interface. Every function returns an unsigned 32-bit value
//! (addresses and lengths included), which JavaScript receives as a signed `i32`: values
//! from 2^31 on must be read back as unsigned.
//!
//! A trap (a bug, or memory exhausted) aborts the call in progress and leaves this
//! instance in an unknown state: the caller must not use it again, and loads the patch
//! into a new instance instead.
//!
//! The only `unsafe` here is `#[unsafe(no_mangle)]`, which gives each function a fixed
//! export name, and the tests that use the pointers exactly as JavaScript does.

#![expect(
    unsafe_code,
    reason = "the exported C interface: `#[unsafe(no_mangle)]` and pointer-level tests"
)]

use std::sync::{Mutex, MutexGuard, PoisonError};

use engine_core::BLOCK_SIZE;

use crate::Instance;

/// Identity and version of this interface.
///
/// `0x5059` ("PY") in the high 16 bits, so that a module exporting a function of the
/// same name by chance is not mistaken for an engine; the version in the low 16 bits,
/// changed whenever a function is added, removed or changes meaning, so that a caller
/// never drives a module it does not understand.
pub const ABI_VERSION: u32 = 0x5059_0003;

/// The version of this interface, [`ABI_VERSION`].
#[unsafe(no_mangle)]
pub const extern "C" fn engine_abi_version() -> u32 {
    ABI_VERSION
}

// The lock below never waits: without WebAssembly threads (no `atomics` target feature),
// std's `Mutex` cannot block. Built with them, it would become a real lock taken by
// `engine_process` on the audio thread: such a build is refused until the instance is
// reached without a lock (lock-free handles, roadmap #32). Threads need a nightly toolchain
// (a std rebuilt with atomics), which is also the only one exposing this `cfg`: the pinned
// nightly sets it from `-C target-feature=+atomics` alone, std not rebuilt, and this guard
// then stops the build; the stable toolchain ignores the feature with a warning and builds
// a module whose memory is not shared, where the lock still cannot block.
#[cfg(all(target_arch = "wasm32", target_feature = "atomics"))]
compile_error!(
    "engine-wasm with WebAssembly atomics: the instance lock could block the audio thread; reach the instance without a lock first (roadmap #32)"
);

/// The state of this module instance.
static INSTANCE: Mutex<Instance> = Mutex::new(Instance::new());

/// Locks the state of this module instance.
///
/// A panic aborts the WebAssembly module, so the lock is never actually poisoned.
fn instance() -> MutexGuard<'static, Instance> {
    INSTANCE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Largest patch the engine reads, in bytes.
///
/// It is [`engine_patch::MAX_PATCH_BYTES`], the limit of the patch format. The caller refuses a longer patch before calling [`engine_input`],
/// which would grow the module memory (never given back) to hold it.
#[unsafe(no_mangle)]
pub const extern "C" fn engine_max_patch_bytes() -> usize {
    engine_patch::MAX_PATCH_BYTES
}

/// Makes the input buffer `len` bytes long and returns its address, for the caller to
/// write a patch into.
#[unsafe(no_mangle)]
pub extern "C" fn engine_input(len: usize) -> *mut u8 {
    instance().input(len).as_mut_ptr()
}

/// Loads the patch in the input buffer at `rate` hertz, rendering in the render format of
/// code `format` (its index in `engine_core::RenderFormat::ALL`): 1 on success, 0 on failure.
#[unsafe(no_mangle)]
pub extern "C" fn engine_load(rate: u32, format: u32) -> u32 {
    u32::from(instance().load(rate, format))
}

/// Address of the UTF-8 message explaining the last failed load.
#[unsafe(no_mangle)]
pub extern "C" fn engine_message_ptr() -> *const u8 {
    instance().message().as_ptr()
}

/// Length in bytes of the message explaining the last failed load; 0 after a success.
#[unsafe(no_mangle)]
pub extern "C" fn engine_message_len() -> usize {
    instance().message().len()
}

/// Number of output channels of the loaded engine; 0 when none is loaded.
#[unsafe(no_mangle)]
pub extern "C" fn engine_channels() -> usize {
    instance().channels()
}

/// Number of frames per block.
#[unsafe(no_mangle)]
pub const extern "C" fn engine_block_size() -> usize {
    BLOCK_SIZE
}

/// Renders the next block and returns the address of its planar samples, in bytes.
#[unsafe(no_mangle)]
pub extern "C" fn engine_process() -> *const u8 {
    instance().process().as_ptr()
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use engine_core::{BLOCK_SIZE, Plan, Renderer, SampleRate};
    use engine_objects::REGISTRY;

    use super::{
        ABI_VERSION, engine_abi_version, engine_block_size, engine_channels, engine_input,
        engine_load, engine_max_patch_bytes, engine_message_len, engine_message_ptr,
        engine_process,
    };

    /// Writes `bytes` through the exported input buffer, as JavaScript does.
    fn write(bytes: &[u8]) {
        let pointer = engine_input(bytes.len());
        // SAFETY: `engine_input` returned a buffer of exactly `bytes.len()` writable
        // bytes, which cannot overlap `bytes`, and nothing else runs until `load`.
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer, bytes.len()) };
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

    /// The first channel of `patch`'s first block at `rate`, in the render format of code
    /// `format`, through the exported interface, as bits; and the same from the renderer.
    fn first_channels(sine: &[u8], rate: SampleRate, format: u32) -> (Vec<u64>, Vec<u64>) {
        let patch = engine_patch::parse_patch(core::str::from_utf8(sine).expect("UTF-8"))
            .expect("valid patch");
        write(sine);
        assert_eq!(
            engine_load(rate.hz(), format),
            1,
            "loaded at {} Hz",
            rate.hz()
        );
        let wide = || Renderer::new(Plan::<f64>::build(&patch, &REGISTRY, rate).expect("plan"));
        let (exported, expected): (Vec<u64>, Vec<u64>) = match format {
            0 => (
                wide_block(),
                wide()
                    .render::<f64>(BLOCK_SIZE)
                    .expect("small render")
                    .iter()
                    .map(|sample| sample.to_bits())
                    .collect(),
            ),
            1 => (
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
            _ => (
                block()
                    .iter()
                    .map(|sample| u64::from(sample.to_bits()))
                    .collect(),
                Renderer::new(Plan::<f32>::build(&patch, &REGISTRY, rate).expect("plan"))
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
        assert_eq!(ABI_VERSION & 0xFFFF_u32, 3_u32, "third version");
        assert_eq!(engine_block_size(), BLOCK_SIZE, "block size");
        assert_eq!(
            engine_max_patch_bytes(),
            engine_patch::MAX_PATCH_BYTES,
            "the limit of the patch reader"
        );

        write(b"not a patch");
        assert_eq!(engine_load(48_000, 1), 0, "rejected");
        assert_eq!(engine_channels(), 0, "nothing loaded");
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
            b"unknown render format: 3 (supported: 0 binary64, 1 binary64-as-binary32, 2 binary32)",
            "explained"
        );
        write(sine);
        assert_eq!(engine_load(48_000, 1), 1, "loaded");
        assert_eq!(engine_message_len(), 0, "no error");
        assert_eq!(engine_channels(), 2, "stereo");

        let patch = engine_patch::parse_patch(core::str::from_utf8(sine).expect("UTF-8"))
            .expect("valid patch");
        let mut renderer = Renderer::new(
            Plan::<f64>::build(&patch, &REGISTRY, SampleRate::HZ_48000).expect("valid plan"),
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

        // What the audio thread calls, block after block: no heap operation.
        let operations = engine_testkit::heap_operations_during(|| {
            for _ in 0_u16..100_u16 {
                let _: *const u8 = core::hint::black_box(engine_process());
                let _: usize = core::hint::black_box(engine_channels());
            }
        });
        assert_eq!(
            operations,
            Ok(((), 0)),
            "engine_process does not use the heap"
        );

        // `engine_load` hands its rate and format to the engine, at every rate of the matrix.
        for rate in engine_testkit::matrix::sample_rates() {
            for format in 0_u32..3 {
                let (exported, reference) = first_channels(sine, rate, format);
                assert_eq!(exported, reference, "format {format} at {} Hz", rate.hz());
            }
        }
    }
}
