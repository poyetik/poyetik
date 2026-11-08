//! Poyetik test bench.
//!
//! [`check_laws`] runs a box in isolation and checks the laws every box must respect;
//! [`check_registry`] checks a whole registry. Box crates call them on every registered
//! box type, so a new box cannot escape them.
//!
//! Every law is checked at every sample rate of the shared [`matrix`], with the
//! parameters at their minimum, default and maximum, at a value away from the default
//! (see `away_from_default`), and at `-0.0` and the smallest subnormal (where the range
//! accepts them).
//!
//! The laws:
//! - **metadata**: names and descriptions are present, port and parameter names are
//!   unique, parameter ranges are finite and contain their default;
//! - **determinism**: two fresh instances fed the same inputs produce bit-identical
//!   outputs;
//! - **finite output**: never NaN or infinity, for random inputs in `[-1, 1]`, the
//!   special inputs of the [`matrix`] and a long run (which reveals late overflow, not
//!   slow drift: it has no reference);
//! - **no heap operation** while processing: no allocation, reallocation or release
//!   (measured by [`CountingAllocator`], which the test binary must install as its
//!   global allocator).

#![forbid(unsafe_code)]

extern crate alloc;

mod laws;
pub mod matrix;
pub mod runner;

pub use engine_alloc_counter::{CountingAllocator, NotCounting, heap_operations_during};
pub use laws::{check_laws, check_metadata, check_registry};

/// The test binary of this crate counts heap operations, like every box test binary must.
#[cfg(test)]
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;
