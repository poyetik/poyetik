//! The Poyetik engine as a host drives it: one patch loaded from its bytes, rendered one
//! block at a time in its render format, through its output stage ([`Engine`]), and the
//! state behind a C interface ([`Instance`]): the input buffer its caller writes a patch
//! into, the codes of formats, modes and detectors, and the last error message.
//!
//! Safe and target-neutral. The WebAssembly module (`audio-wasm`) is a thin C interface
//! over [`Instance`]: its `unsafe` code stays in that crate, whose tests Miri runs, while
//! the rendering this crate does is proven by its own tests, natively.

#![no_std]
#![forbid(unsafe_code)]
#![doc(test(attr(forbid(unsafe_code))))]

extern crate alloc;

mod engine;
mod instance;

pub use engine::{Blocks, Engine, Output, OutputMode};
pub use instance::Instance;

/// The engine block of the build (`spec/engine-design.md`, decision 2), in frames.
///
/// The one its build names ([`audio_core::NAMED_ENGINE_BLOCK`], set by each build of
/// `pnpm build:wasm`), [`audio_core::BLOCK_SIZE`] when it names none.
pub const ENGINE_BLOCK: usize = match audio_core::NAMED_ENGINE_BLOCK {
    Some(frames) => frames,
    None => audio_core::BLOCK_SIZE,
};

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
