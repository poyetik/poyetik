//! Poyetik fingerprint battery.
//!
//! [`fingerprint()`] hashes a box type's outputs over a fixed battery of inputs, so that a
//! change of its bits is seen beyond the conformance cases (`spec/engine-design.md`,
//! decision 29): `engine-cli` records and compares it, so this crate is part of the
//! conformance tool, not of the tests only. Its inputs are the shared test [`matrix`] (the
//! values every dimension of the environment is tested at) and the parameter sets and
//! special inputs every box is tried with ([`param_sets`], [`special_input`]), which the
//! box laws of `engine-testkit` read too: one source for both.

#![forbid(unsafe_code)]
#![doc(test(attr(forbid(unsafe_code))))]

extern crate alloc;

mod fingerprint;
mod inputs;
pub mod matrix;

pub use fingerprint::{FINGERPRINT_BATTERY, fingerprint, splitmix64};
pub use inputs::{BLOCKS, BatterySample, param_sets, special_input};
