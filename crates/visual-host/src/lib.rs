//! The square a host shows, safe and `no_std`: set up from the raw values a host passes
//! (width, height, rate, side, cycle), refusing each invalid one with its reason, then
//! rendered one image at a time, encoded for display, placed for a GPU drawing it or hashed
//! as a sequence, without the heap once set up; apart from the square, [`Encoding`] encodes
//! any values for display, into buffers it reserves on the heap, fallibly, at each
//! [`Encoding::values`], never while it encodes; and [`shader`] gives the library's WGSL
//! shaders by their kind ([`ShaderKind`]), for a GPU drawing the square or encoding for display.
//!
//! A C interface only forwards to it (`visual-wasm`'s, the one place `unsafe` code drives
//! it), so that all the logic is tested natively and only the boundary runs under Miri.

#![no_std]
#![forbid(unsafe_code)]
#![doc(test(attr(forbid(unsafe_code))))]
// A public enum that may gain a variant is `#[non_exhaustive]`, so that adding one breaks no
// caller; one that is closed by its decision says why where it is declared.
#![deny(clippy::exhaustive_enums)]
#![deny(clippy::exhaustive_structs)]

extern crate alloc;

mod encoding;
mod view;

pub use encoding::Encoding;
pub use view::{SetupError, ShaderKind, View, image_index, shader};

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
