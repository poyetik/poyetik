//! Poyetik visual test bench.
//!
//! [`check_laws`] renders a visual box in isolation and checks the laws every visual box
//! must respect. Box crates call it on every box they define, with parameters across
//! their range: a convention, reviewed, until a registry lists the visual boxes as the
//! audio boxes' does (#35 step 2), for nothing yet checks that a new box is listed.
//!
//! Every law is checked at every image rate, image size and image index of the shared
//! visual [`matrix`] (`conformance/visual/test-matrix.json`).
//!
//! The laws:
//! - **determinism**: two fresh instances render bit-identical images;
//! - **random access**: an instance renders an image with the same bits as a fresh
//!   instance after rendering the other images of the matrix in reverse order, or
//!   jumping between both ends of the matrix, each image twice in a row (the second time
//!   over other values);
//! - **overwrite**: rendering overwrites every value, whatever the image held before
//!   (the image is filled with NaN first);
//! - **finite values**: never NaN or infinity, and alpha within `[0, 1]`;
//! - **no heap operation** while rendering, in every pass of every law: no allocation,
//!   reallocation or release (measured by [`CountingAllocator`], which the test binary
//!   must install as its global allocator);
//! - **pure trace**: tracing a point (a corner, the center, the last subpixel and the
//!   point past the image) twice gives the same answer, from a fresh instance or one that
//!   rendered other images.

#![forbid(unsafe_code)]
#![doc(test(attr(forbid(unsafe_code))))]

mod laws;
pub mod matrix;

pub use alloc_counter::{CountingAllocator, NotCounting, heap_operations_during};
pub use laws::{Factory, check_laws};

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
