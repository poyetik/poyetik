//! The C interface exported by the WebAssembly module.
//!
//! Protocol, for one module instance:
//!
//! 1. [`visual_abi_version`] first: a module with another version is a different
//!    interface.
//! 2. [`visual_setup`] sets the square up: 1 on success, 0 when a value is invalid; then
//!    [`visual_message_ptr`] and [`visual_message_len`] locate a UTF-8 explanation.
//! 3. [`visual_render`] renders an image and returns the address of its values;
//!    [`visual_values_len`] gives their count (four per pixel), that of the square set up.
//!    [`visual_display`] renders an image the same way and returns the address of its
//!    bytes encoded for display (8-bit sRGB, straight alpha, as a canvas holds them);
//!    [`visual_display_len`] gives their count (four per pixel).
//! 4. For a GPU drawing the square itself: [`visual_placement`] answers where the square is
//!    in an image (four 32-bit edges); [`visual_shader_ptr`] and [`visual_shader_len`] locate
//!    the library's WGSL shaders, [`visual_thresholds`] the display shader's 510 thresholds:
//!    static, valid for the life of the instance.
//! 5. [`visual_hash`](fn@visual_hash) renders a sequence of images and returns the address
//!    of their image hash, 32 bytes; it renders into the image [`visual_render`] uses.
//!
//! Every address is valid until the next call that sets up, renders or hashes: the caller
//! copies what it needs. [`visual_values_len`], [`visual_display_len`],
//! [`visual_message_ptr`] and [`visual_message_len`] only read, and move nothing.
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

use visual_core::DISPLAY_THRESHOLD_BITS;
use visual_host::{View, image_index, shader};

/// Identity and version of this interface: `0x5056` ("PV") in the high 16 bits, the
/// version in the low 16 bits.
///
/// Version 1 was a prototype's, never released, version 2 added the image hash, version 3
/// the message of a refused setup, version 4 the image encoded for display and what a GPU
/// needs to draw it (the placement, the shaders, the thresholds).
pub const ABI_VERSION: u32 = 0x5056_0004;

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

/// Sets the square up (see [`View::setup`]): 1 on success, 0 when a value is invalid, the
/// square set up before then kept.
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
    u32::from(
        view()
            .setup(
                width,
                height,
                (rate_numerator, rate_denominator),
                side,
                (cycle_numerator, cycle_denominator),
            )
            .is_ok(),
    )
}

/// Address of the UTF-8 message explaining the last refused setup.
#[unsafe(no_mangle)]
pub extern "C" fn visual_message_ptr() -> *const u8 {
    view().message().as_ptr()
}

/// Length in bytes of the message explaining the last refused setup; 0 after a success.
#[unsafe(no_mangle)]
pub extern "C" fn visual_message_len() -> usize {
    view().message().len()
}

/// Renders image `high × 2³² + low` and returns the address of its values.
#[unsafe(no_mangle)]
pub extern "C" fn visual_render(low: u32, high: u32) -> *const f32 {
    view().render(image_index(low, high)).as_ptr()
}

/// Renders `count` images from image `first_high × 2³² + first_low` and returns the address
/// of their image hash.
///
/// The hash is 32 bytes. The address is 0 when no square is set up, or when the last
/// of those images would be past image 2⁶⁴ − 1.
#[unsafe(no_mangle)]
pub extern "C" fn visual_hash(first_low: u32, first_high: u32, count: u32) -> *const u8 {
    view()
        .hash(image_index(first_low, first_high), u64::from(count))
        .map_or(core::ptr::null(), |hash| hash.as_ptr())
}

/// The number of values of an image of the square set up: width × height × 4; 0 before
/// any.
#[unsafe(no_mangle)]
pub extern "C" fn visual_values_len() -> usize {
    view().render_len()
}

/// Renders image `high × 2³² + low` and returns the address of its bytes encoded for
/// display (`spec/visual-design.md`, "Display encoding").
#[unsafe(no_mangle)]
pub extern "C" fn visual_display(low: u32, high: u32) -> *const u8 {
    view().display(image_index(low, high)).as_ptr()
}

/// The number of bytes of an image of the square set up encoded for display: width ×
/// height × 4; 0 before any.
#[unsafe(no_mangle)]
pub extern "C" fn visual_display_len() -> usize {
    view().display_len()
}

/// Where the square is in image `high × 2³² + low`: the address of its four edges, left,
/// right, top and bottom, in subpixel units; 0 when no square is set up.
#[unsafe(no_mangle)]
pub extern "C" fn visual_placement(low: u32, high: u32) -> *const u32 {
    view()
        .placement(image_index(low, high))
        .map_or(core::ptr::null(), |placed| placed.as_ptr())
}

/// Address of the UTF-8 WGSL source of shader `kind` (0 the square's, 1 the display
/// encoding's); static.
#[unsafe(no_mangle)]
pub const extern "C" fn visual_shader_ptr(kind: u32) -> *const u8 {
    shader(kind).as_ptr()
}

/// Length in bytes of the WGSL source of shader `kind`; 0 for a kind that is none.
#[unsafe(no_mangle)]
pub const extern "C" fn visual_shader_len(kind: u32) -> usize {
    shader(kind).len()
}

/// Address of the display shader's 510 thresholds, as bits: the sRGB ones then the alpha
/// ones; static.
#[unsafe(no_mangle)]
pub extern "C" fn visual_thresholds() -> *const u32 {
    THRESHOLDS.as_ptr()
}

/// The display shader's thresholds at one address for the life of the instance (a constant
/// would be a temporary at each use).
static THRESHOLDS: [u32; 510] = DISPLAY_THRESHOLD_BITS;

#[cfg(test)]
#[path = "ffi_tests.rs"]
mod tests;
