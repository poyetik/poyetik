//! The C interface exported by the WebAssembly module.
//!
//! Protocol, for one module instance (one engine):
//!
//! 1. `engine_input(len)` ([`engine_input`]) returns where to write the `len` bytes of a patch.
//! 2. `engine_load(rate)` ([`engine_load`]) loads it; on failure, [`engine_message_ptr`] and
//!    [`engine_message_len`] locate a UTF-8 explanation.
//! 3. [`engine_process`] renders the next block and returns where it is:
//!    [`engine_channels`] × [`engine_block_size`] `f32` samples, channel after channel.
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

/// Version of this interface: changed whenever a function is added, removed or changes
/// meaning, so that a caller never drives a module it does not understand.
pub const ABI_VERSION: u32 = 1;

/// The version of this interface, [`ABI_VERSION`].
#[unsafe(no_mangle)]
pub const extern "C" fn engine_abi_version() -> u32 {
    ABI_VERSION
}

/// The state of this module instance.
static INSTANCE: Mutex<Instance> = Mutex::new(Instance::new());

/// Locks the state of this module instance.
///
/// A panic aborts the WebAssembly module, so the lock is never actually poisoned.
fn instance() -> MutexGuard<'static, Instance> {
    INSTANCE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Makes the input buffer `len` bytes long and returns its address, for the caller to
/// write a patch into.
#[unsafe(no_mangle)]
pub extern "C" fn engine_input(len: usize) -> *mut u8 {
    instance().input(len).as_mut_ptr()
}

/// Loads the patch in the input buffer at `rate` hertz: 1 on success, 0 on failure.
#[unsafe(no_mangle)]
pub extern "C" fn engine_load(rate: u32) -> u32 {
    u32::from(instance().load(rate))
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

/// Renders the next block and returns the address of its planar samples.
#[unsafe(no_mangle)]
pub extern "C" fn engine_process() -> *const f32 {
    instance().process().as_ptr().cast()
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use engine_core::{BLOCK_SIZE, Plan, SampleRate};
    use engine_objects::REGISTRY;

    use super::{
        ABI_VERSION, engine_abi_version, engine_block_size, engine_channels, engine_input,
        engine_load, engine_message_len, engine_message_ptr, engine_process,
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

    /// Renders one block through the exported interface and copies it, planar.
    fn block() -> Vec<f32> {
        let pointer = engine_process();
        let len = engine_channels()
            .checked_mul(engine_block_size())
            .expect("small block");
        // SAFETY: the block is `channels × block size` initialized samples at
        // `pointer`, unchanged until the next call.
        unsafe { core::slice::from_raw_parts(pointer, len) }.to_vec()
    }

    /// The whole protocol, in one test: the state is global to the module.
    #[test]
    fn follows_the_protocol() {
        assert_eq!(engine_abi_version(), ABI_VERSION, "version");
        assert_eq!(ABI_VERSION, 1, "first version");
        assert_eq!(engine_block_size(), BLOCK_SIZE, "block size");

        write(b"not a patch");
        assert_eq!(engine_load(48_000), 0, "rejected");
        assert_eq!(engine_channels(), 0, "nothing loaded");
        let explanation = message();
        assert!(!explanation.is_empty(), "explained");
        assert!(core::str::from_utf8(&explanation).is_ok(), "UTF-8");

        let sine = br#"{"format":1,"nodes":[{"id":1,"box":"osc~"},{"id":2,"box":"dac~"}],
            "connections":[{"from":[1,0],"to":[2,0]},{"from":[1,0],"to":[2,1]}]}"#;
        write(sine);
        assert_eq!(engine_load(48_000), 1, "loaded");
        assert_eq!(engine_message_len(), 0, "no error");
        assert_eq!(engine_channels(), 2, "stereo");

        let patch = engine_patch::parse_patch(core::str::from_utf8(sine).expect("UTF-8"))
            .expect("valid patch");
        let mut plan = Plan::build(&patch, &REGISTRY, SampleRate::HZ_48000).expect("valid plan");
        let expected = plan.render(2 * BLOCK_SIZE).expect("small render");
        let mut actual = Vec::new();
        for _ in 0..2_u8 {
            let planar = block();
            let (left, right) = planar.split_at(BLOCK_SIZE);
            for (l, r) in left.iter().zip(right) {
                actual.extend([*l, *r]);
            }
        }
        let bits = |samples: &[f32]| samples.iter().map(|s| s.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&actual), bits(&expected), "bit-identical");

        // What the audio thread calls, block after block: no allocation.
        let ((), allocations) = engine_testkit::allocations_during(|| {
            for _ in 0_u16..100_u16 {
                let _: *const f32 = core::hint::black_box(engine_process());
                let _: usize = core::hint::black_box(engine_channels());
            }
        });
        assert_eq!(allocations, 0, "engine_process does not allocate");
    }
}
