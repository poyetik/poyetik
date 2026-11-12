//! Poyetik test bench.
//!
//! [`check_laws`] runs a box in isolation and checks the laws every box must respect;
//! [`check_registry`] checks a whole registry. Box crates call them on every registered
//! box type, so a new box cannot escape them. [`measure`] gives the objective measurements
//! of a signal (`spec/measurements.md`), by which every box proves its audio quality.
//! The parameter sets and special inputs the laws try are the fingerprint battery's
//! (`engine-battery`), as is the shared test [`matrix`], re-exported here.
//!
//! Every law is checked in both precisions (binary64 and binary32, [`matrix::PRECISIONS`])
//! and at every sample rate of the shared [`matrix`], with the
//! parameters at their minimum, default and maximum, at a value away from the default
//! (see `engine_battery::param_sets`), and at `-0.0` and the smallest normal binary32 value (where
//! the range accepts them), given as `engine_core::Params`, as a plan gives them.
//!
//! The laws:
//! - **metadata**: names and descriptions are present, port and parameter names are
//!   unique (a parameter's range is finite and holds its default by construction:
//!   `engine_core::ParamSpec::new`);
//! - **determinism**: two fresh instances fed the same inputs produce bit-identical
//!   outputs;
//! - **finite output**: never NaN or infinity, for random inputs in `[-1, 1]`, the
//!   special inputs of the [`matrix`] and a long run (which reveals late overflow, not
//!   slow drift: it has no reference);
//! - **no subnormal output**: every output sample is zero or normal, in every one of those
//!   runs (`spec/engine-design.md`, decision 11: a box flushes its outputs with
//!   [`engine_core::Sample::flush`]). A box's recursive state is out of the laws' reach until box
//!   state can be read (roadmap #12 step 28): each box documents and tests how its own
//!   state stays free of subnormals;
//! - **no heap operation** while processing: no allocation, reallocation or release
//!   (measured by [`CountingAllocator`], which the test binary must install as its
//!   global allocator).

#![forbid(unsafe_code)]
#![doc(test(attr(forbid(unsafe_code))))]

extern crate alloc;

mod laws;
pub mod measure;
pub mod runner;

pub use engine_alloc_counter::{
    CountingAllocator, NotCounting, heap_bytes_during, heap_operations_during,
};
pub use engine_battery::matrix;
pub use laws::{check_laws, check_metadata, check_registry};

use engine_core::host::Raw;
use engine_core::{Binary64, Sample};

/// `value`, a literal of a test written in binary64, as a sample of the precision `S`
/// (rounded to it when `S` is binary32): the one conversion the unit tests of the laws and
/// the plan laws share.
#[must_use]
pub fn sample_of<S: Sample>(value: f64) -> S {
    S::from_binary64(Binary64::from_raw(value))
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
