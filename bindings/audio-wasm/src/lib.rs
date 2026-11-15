//! The Poyetik engine compiled to WebAssembly, for browsers (in an `AudioWorklet`) and
//! Node.
//!
//! The module exports a minimal C interface ([`ffi`]) with no imports and no generated
//! glue: JavaScript writes a patch into the module memory, loads it, then pulls one
//! block of planar samples at a time. All the logic lives in the safe [`Engine`] and
//! [`Instance`]; [`ffi`] uses `unsafe` (for `#[unsafe(no_mangle)]` and in its own tests),
//! as one of the files allowed to, listed in xtask's `UNSAFE_FILES` and run under Miri.

#![doc(test(attr(forbid(unsafe_code))))]

extern crate alloc;

mod engine;
pub mod ffi;
mod instance;

pub use engine::{Blocks, Engine, Output, OutputMode};
pub use instance::Instance;

/// The engine block this module is built for (`spec/engine-design.md`, decision 2), in
/// frames.
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
