//! The Poyetik visual library compiled to WebAssembly, for browsers and Node.
//!
//! The module exports a minimal C interface ([`ffi`]) with no imports and no generated
//! glue: JavaScript sets the square up, then asks for one image at a time and reads its
//! linear-light values from the module memory, or for the image hash of a sequence. All
//! the logic lives in the safe [`View`]; [`ffi`] uses `unsafe` (for
//! `#[unsafe(no_mangle)]` and in its own tests), as one of the files allowed to, listed
//! in xtask's `UNSAFE_FILES` and run under Miri.

#![doc(test(attr(forbid(unsafe_code))))]

pub mod ffi;
mod view;

pub use view::{SetupError, View, image_index};

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
