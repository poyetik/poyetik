//! The Poyetik engine compiled to WebAssembly, for browsers (in an `AudioWorklet`) and
//! Node.
//!
//! The module exports a minimal C interface ([`ffi`]) with no imports and no generated
//! glue: JavaScript writes a patch into the module memory, loads it, then pulls one
//! block of planar samples at a time. All the logic lives in the safe `audio-host` crate
//! ([`audio_host::Instance`]); [`ffi`] only forwards to it, and uses `unsafe` (for
//! `#[unsafe(no_mangle)]` and in its own tests), as one of the files allowed to, listed in
//! xtask's `UNSAFE_FILES` and run under Miri.

#![doc(test(attr(forbid(unsafe_code))))]

pub mod ffi;

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
