//! Poyetik visual library core: images, exact time, the contract every visual box
//! implements, and the exact CPU reference renderer. The boxes are in `visual-boxes`.
//!
//! A standalone library: it depends on no other part of the project (the audio library
//! included), so it can be used on its own. It is `no_std` and forbids `unsafe`, so it
//! cannot reach the OS, the clock, threads or the GPU: what it renders is deterministic
//! by construction, and it is the exact reference every other renderer (the GPU one
//! included) is measured against.
//!
//! The design decisions of this first version are provisional and listed in
//! `spec/visual-design.md`.

#![no_std]
#![forbid(unsafe_code)]
#![doc(test(attr(forbid(unsafe_code))))]

extern crate alloc;

mod contract;
pub mod coverage;
mod display;
mod error;
mod image;
mod rate;
mod render;

pub use contract::{SubpixelPoint, Trace, VisualBox};
pub use coverage::{SUBPIXELS, coverage_to_intensity};
pub use display::{
    ALPHA8_THRESHOLDS, DISPLAY_THRESHOLD_BITS, DISPLAY_WGSL, DisplayImage, SRGB8_THRESHOLDS,
    alpha8, pixel8, srgb8, straight8,
};
pub use error::VisualError;
pub use image::{ColorEncoding, Image, ImageSize};
pub use rate::{ImageIndex, ImageRange, ImageRate, Seconds};
pub use render::render_sequence;
