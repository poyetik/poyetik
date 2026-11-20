//! The Poyetik visual library compiled to WebAssembly, for those who use the visual library
//! alone, tested in Node; in browsers the product links both libraries into its own modules instead (the page's is `web-page`'s)
//! (`spec/engine-design.md`, decision 33).
//!
//! The module exports a minimal C interface ([`ffi`]) with no imports and no generated
//! glue: JavaScript sets the square up, then asks for one image at a time and reads its
//! linear-light values from the module memory, its display bytes, where the square is for a
//! GPU drawing it, or the image hash of a sequence, and encodes any values for display (the
//! protocol, every export in order, is [`ffi`]'s documentation). All
//! the logic lives in the safe `visual-host` crate, tested natively; [`ffi`] only forwards
//! to it, using `unsafe` (for `#[unsafe(no_mangle)]` and in its own tests), as one of the
//! files allowed to, listed in xtask's `UNSAFE_FILES` and run under Miri.

#![doc(test(attr(forbid(unsafe_code))))]
// A public enum that may gain a variant is `#[non_exhaustive]`, so that adding one breaks no
// caller; one that is closed by its decision says why where it is declared.
#![deny(clippy::exhaustive_enums)]
#![deny(clippy::exhaustive_structs)]

pub mod ffi;
