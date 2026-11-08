//! The Poyetik engine compiled to WebAssembly, for browsers (in an `AudioWorklet`) and
//! Node.
//!
//! The module exports a minimal C interface ([`ffi`]) with no imports and no generated
//! glue: JavaScript writes a patch into the module memory, loads it, then pulls one
//! block of planar samples at a time. All the logic lives in the safe [`Engine`] and
//! [`Instance`]; [`ffi`] uses `unsafe` (for `#[unsafe(no_mangle)]` and in its own tests),
//! as one of the files allowed to, listed in xtask's `UNSAFE_FILES` and run under Miri.

extern crate alloc;

mod engine;
pub mod ffi;
mod instance;

pub use engine::Engine;
pub use instance::Instance;

/// The test binary counts heap operations, so that rendering can be checked not to use
/// the heap.
#[cfg(test)]
#[global_allocator]
static ALLOCATOR: engine_testkit::CountingAllocator = engine_testkit::CountingAllocator;
