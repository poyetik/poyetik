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

// The browser hashes this module's renders as strict (`renderHash` of the binding, given
// the strict mode by its callers), and this interface does not export the mode: a module
// built with any arithmetic option (`--cfg poyetik_math=...`, or a later one) would have
// its renders hashed under a mode they were not computed in. Derived from the math layer's
// own `MODE`, so that every option is refused, and refused until the mode is exported
// (`spec/engine-design.md`, decision 10, rule 4).
const _: () = assert!(
    engine_core::MODE.bits() == engine_core::ArithmeticMode::STRICT.bits(),
    "engine-wasm is built in the strict arithmetic mode only: the browser hashes its renders as strict; export the mode through the C interface first (decision 10, rule 4)"
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
/// It is [`engine_patch::MAX_PATCH_BYTES`], the limit of the patch format. The caller refuses a
/// longer patch before calling [`engine_input`], which would grow the module memory (never given
/// back) to hold it.
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
#[path = "ffi_tests.rs"]
mod tests;
