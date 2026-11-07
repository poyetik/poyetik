//! poyetik test bench.
//!
//! [`check_laws`] runs a box in isolation and checks the laws every box must respect;
//! [`check_registry`] checks a whole registry. Box crates call them on every registered
//! box type, so a new box cannot escape them.
//!
//! Every law is checked at every sample rate of the shared [`matrix`], with the
//! parameters at their minimum, default, midpoint and maximum.
//!
//! The laws:
//! - **metadata**: names and descriptions are present, port and parameter names are
//!   unique, parameter ranges are finite and contain their default;
//! - **determinism**: two fresh instances fed the same inputs produce bit-identical
//!   outputs;
//! - **finite output**: never NaN or infinity, for random inputs in `[-1, 1]`, the
//!   special inputs of the [`matrix`] and a long run;
//! - **no allocation** while processing (measured by [`CountingAllocator`], which the
//!   test binary must install as its global allocator).

extern crate alloc;

mod counting;
mod laws;
pub mod matrix;

pub use counting::{CountingAllocator, allocations_during};
pub use laws::{check_laws, check_metadata, check_registry};

/// The test binary of this crate counts allocations, like every box test binary must.
#[cfg(test)]
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;
