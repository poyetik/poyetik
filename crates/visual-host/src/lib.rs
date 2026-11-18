//! The square a host shows, safe and `no_std`: set up from the raw values a host passes
//! (width, height, rate, side, cycle), refusing each invalid one with its reason, then
//! rendered one image at a time or hashed as a sequence, without the heap.
//!
//! A C interface only forwards to it (`visual-wasm`'s, the one place `unsafe` code drives
//! it), so that all the logic is tested natively and only the boundary runs under Miri.

#![no_std]
#![forbid(unsafe_code)]
#![doc(test(attr(forbid(unsafe_code))))]

mod view;

pub use view::{SetupError, View, image_index, shader};

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
