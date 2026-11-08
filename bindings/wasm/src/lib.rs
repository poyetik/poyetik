//! The poyetik engine compiled to WebAssembly, for browsers (in an `AudioWorklet`) and
//! Node.
//!
//! The module exports a minimal C interface ([`ffi`]) with no imports and no generated
//! glue: JavaScript writes a patch into the module memory, loads it, then pulls one
//! block of planar samples at a time. All the logic lives in the safe [`Engine`] and
//! [`Instance`]; [`ffi`] is the only module of the repository allowed to use `unsafe`
//! (for `#[unsafe(no_mangle)]` and in its own tests).

extern crate alloc;

mod engine;
pub mod ffi;
mod instance;

pub use engine::Engine;
pub use instance::Instance;

/// The test binary counts allocations, so that rendering can be checked not to allocate.
#[cfg(test)]
#[global_allocator]
static ALLOCATOR: engine_testkit::CountingAllocator = engine_testkit::CountingAllocator;
