//! Poyetik boxes.
//!
//! Each box type lives in its own module and depends only on the `engine-core`
//! contract, written once for both precisions: generic over `engine_core::Sample`, its
//! arithmetic through the engine's math layer (this crate does not depend on `libm`). [`REGISTRY`] lists every box type; being in the registry is enough for a
//! box to be checked against the generic laws of `engine-testkit`.
//!
//! Like the core, this crate is `no_std` and forbids `unsafe`.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod dac;
mod osc;

use engine_core::ObjectSpec;

pub use dac::DAC;
pub use osc::OSC;

/// Every box type of the engine.
pub static REGISTRY: [&ObjectSpec; 2] = [&OSC, &DAC];

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
