//! The C interface exported by the WebAssembly module.
//!
//! Protocol, for one module instance:
//!
//! 1. [`visual_abi_version`] first: a module with another version is a different
//!    interface.
//! 2. [`visual_setup`] sets the square up: 1 on success, 0 when a value is invalid.
//! 3. [`visual_render`] renders an image and returns the address of its values;
//!    [`visual_values_len`] gives their count (four per pixel). The address is valid only
//!    until the next call: the caller copies what it needs.
//!
//! Every function returns an unsigned 32-bit value (addresses and lengths included), which
//! JavaScript receives as a signed `i32`: values from 2^31 on must be read back as
//! unsigned. A trap (a bug, or memory exhausted) aborts the call in progress and leaves
//! this instance in an unknown state: the caller must not use it again.
//!
//! The only `unsafe` here is `#[unsafe(no_mangle)]`, which gives each function a fixed
//! export name, and the tests that use the pointers exactly as JavaScript does.

#![expect(
    unsafe_code,
    reason = "the exported C interface: `#[unsafe(no_mangle)]` and pointer-level tests"
)]

use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::{View, image_index};

/// Identity and version of this interface: `0x5056` ("PV") in the high 16 bits, the
/// version in the low 16 bits.
pub const ABI_VERSION: u32 = 0x5056_0002;

// The lock below never waits: without WebAssembly threads (no `atomics` target feature),
// std's `Mutex` cannot block. Built with them, it would become a real lock that a page's
// render could wait on: such a build is refused. Threads need a nightly toolchain (a std
// rebuilt with atomics), which is also the only one exposing this `cfg`: the pinned
// nightly sets it from `-C target-feature=+atomics` alone, std not rebuilt, and this guard
// then stops the build; the stable toolchain ignores the feature with a warning and builds
// a module whose memory is not shared, where the lock still cannot block.
#[cfg(all(target_arch = "wasm32", target_feature = "atomics"))]
compile_error!(
    "visual-wasm with WebAssembly atomics: the view lock could block a render; reach the view without a lock first"
);

/// The state of this module instance.
static VIEW: Mutex<View> = Mutex::new(View::new());

/// Locks the state of this module instance; a panic aborts the module, so the lock is
/// never actually poisoned.
fn view() -> MutexGuard<'static, View> {
    VIEW.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The version of this interface, [`ABI_VERSION`].
#[unsafe(no_mangle)]
pub const extern "C" fn visual_abi_version() -> u32 {
    ABI_VERSION
}

/// Sets the square up (see [`View::setup`]): 1 on success, 0 when a value is invalid.
#[unsafe(no_mangle)]
pub extern "C" fn visual_setup(
    width: u32,
    height: u32,
    rate_numerator: u32,
    rate_denominator: u32,
    side: u32,
    cycle_numerator: u32,
    cycle_denominator: u32,
) -> u32 {
    u32::from(view().setup(
        width,
        height,
        (rate_numerator, rate_denominator),
        side,
        (cycle_numerator, cycle_denominator),
    ))
}

/// Renders image `high × 2³² + low` and returns the address of its values.
#[unsafe(no_mangle)]
pub extern "C" fn visual_render(low: u32, high: u32) -> *const f32 {
    view().render(image_index(low, high)).as_ptr()
}

/// Renders `count` images from image `first_high × 2³² + first_low` and returns the address
/// of their image hash.
///
/// The hash is 32 bytes, all zero when no square is set up. The address is 0 when the last
/// of those images would be past image 2⁶⁴ − 1.
#[unsafe(no_mangle)]
pub extern "C" fn visual_hash(first_low: u32, first_high: u32, count: u32) -> *const u8 {
    view()
        .hash(image_index(first_low, first_high), u64::from(count))
        .map_or(core::ptr::null(), |hash| hash.as_ptr())
}

/// The number of values of the last image rendered: width × height × 4.
#[unsafe(no_mangle)]
pub extern "C" fn visual_values_len() -> usize {
    view().render_len()
}

#[cfg(test)]
#[path = "ffi_tests.rs"]
mod tests;
