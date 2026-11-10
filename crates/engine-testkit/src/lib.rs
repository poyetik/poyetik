//! Poyetik test bench.
//!
//! [`check_laws`] runs a box in isolation and checks the laws every box must respect;
//! [`check_registry`] checks a whole registry. Box crates call them on every registered
//! box type, so a new box cannot escape them. [`measure`] gives the objective measurements
//! of a signal (`spec/measurements.md`), by which every box proves its audio quality.
//! [`fingerprint()`] hashes a box type's outputs over a fixed battery, so that a change of
//! its bits is seen beyond the conformance cases (`spec/engine-design.md`, decision 29).
//!
//! Every law is checked in both precisions (binary64 and binary32, [`matrix::PRECISIONS`])
//! and at every sample rate of the shared [`matrix`], with the
//! parameters at their minimum, default and maximum, at a value away from the default
//! (see `away_from_default`), and at `-0.0`, the smallest subnormal and the smallest
//! normal (where the range accepts them).
//!
//! The laws:
//! - **metadata**: names and descriptions are present, port and parameter names are
//!   unique, parameter ranges are finite and contain their default;
//! - **determinism**: two fresh instances fed the same inputs produce bit-identical
//!   outputs;
//! - **finite output**: never NaN or infinity, for random inputs in `[-1, 1]`, the
//!   special inputs of the [`matrix`] and a long run (which reveals late overflow, not
//!   slow drift: it has no reference);
//! - **no subnormal output**: every output sample is zero or normal, in every one of those
//!   runs (`spec/engine-design.md`, decision 11: a box flushes its outputs with
//!   `engine_core::flush`). A box's recursive state is out of the laws' reach until box
//!   state can be read (roadmap #12, step 28): each box documents and tests how its own
//!   state stays free of subnormals;
//! - **no heap operation** while processing: no allocation, reallocation or release
//!   (measured by [`CountingAllocator`], which the test binary must install as its
//!   global allocator).

#![forbid(unsafe_code)]

extern crate alloc;

mod fingerprint;
mod laws;
pub mod matrix;
pub mod measure;
pub mod runner;

pub use engine_alloc_counter::{CountingAllocator, NotCounting, heap_operations_during};
pub use fingerprint::{FINGERPRINT_BATTERY, fingerprint};
pub use laws::{check_laws, check_metadata, check_registry};

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
