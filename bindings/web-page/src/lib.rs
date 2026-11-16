//! Poyetik's product module for the page of a browser (`spec/engine-design.md`, decision 33):
//! each thread of the product loads one module holding what it runs, and the page runs the
//! visual library and the coordinating layer.
//!
//! The module exports the visual library's C interface (`visual-wasm`, linked in whole) and
//! the coordinating layer's own ([`ffi`]): what the page asks of both, in one instance and
//! one memory, never two modules side by side. [`ffi`] only forwards to the safe
//! `coordinator` crate, using `unsafe` for `#[unsafe(no_mangle)]` alone, as one of the files
//! allowed to, listed in xtask's `UNSAFE_FILES` and run under Miri.

#![doc(test(attr(forbid(unsafe_code))))]

// Linked in whole: its exported functions are this module's too.
use visual_wasm as _;

pub mod ffi;
