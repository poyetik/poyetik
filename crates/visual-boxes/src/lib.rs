//! Poyetik visual boxes: what renders images, each checked against the visual box laws
//! of `visual-testkit`.
//!
//! `no_std` and no `unsafe`, like the visual library's core: every box renders the same
//! bits on every target.

#![no_std]
#![forbid(unsafe_code)]

mod error;
mod square;

pub use error::BoxError;
pub use square::Square;

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
