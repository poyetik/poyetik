//! Poyetik visual test bench.
//!
//! [`check_laws`] renders a visual box in isolation and checks the laws every visual box
//! must respect. Box crates call it on every box they define, with parameters across
//! their range, so a new box cannot escape them.
//!
//! Every law is checked at every image rate, image size and image index of the shared
//! visual [`matrix`] (`conformance/visual/test-matrix.json`).
//!
//! The laws:
//! - **determinism**: two fresh instances render bit-identical images;
//! - **random access**: an instance that rendered other images first, in any order,
//!   renders an image with the same bits as a fresh instance;
//! - **overwrite**: rendering overwrites every value, whatever the image held before
//!   (the image is filled with NaN first);
//! - **finite values**: never NaN or infinity, and alpha within `[0, 1]`;
//! - **no heap operation** while rendering: no allocation, reallocation or release
//!   (measured by [`CountingAllocator`], which the test binary must install as its
//!   global allocator);
//! - **pure trace**: tracing a point twice gives the same answer, from a fresh instance
//!   or one that rendered other images.

#![forbid(unsafe_code)]

mod laws;
pub mod matrix;

pub use alloc_counter::{CountingAllocator, NotCounting, heap_operations_during};
pub use laws::{Factory, check_laws};

/// The test binary of this crate counts heap operations, like every box test binary must.
#[cfg(test)]
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;
