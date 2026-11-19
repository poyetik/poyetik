//! Poyetik visual boxes: what renders images, each checked against the visual box laws
//! of `visual-testkit`.
//!
//! `no_std` and no `unsafe`, like the visual library's core: every box renders the same
//! bits on every target. Outside its tests it also forbids float arithmetic (Clippy's
//! `float_arithmetic`): a box computes its coverage in integers and turns it into values
//! through the visual core, and the compiler refuses any lifting.

#![no_std]
#![forbid(unsafe_code)]
#![cfg_attr(not(test), forbid(clippy::float_arithmetic))]
#![doc(test(attr(forbid(unsafe_code))))]
// A public enum that may gain a variant is `#[non_exhaustive]`, so that adding one breaks no
// caller; one that is closed by its decision says why where it is declared.
#![deny(clippy::exhaustive_enums)]
#![deny(clippy::exhaustive_structs)]

mod error;
mod square;

pub use error::BoxError;
pub use square::{Placement, SQUARE_WGSL, Square};

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
