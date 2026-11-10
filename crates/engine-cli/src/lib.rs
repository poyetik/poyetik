//! Offline renderer and conformance checker.
//!
//! A **conformance case** is a JSON file holding a sample rate and a duration in frames,
//! with its patch in a file of its own. `engine-cli` renders cases offline with the native engine, writes WAV
//! files, and compares render hashes with the reference hashes of
//! `conformance/references.json`, which only `bless` may change.
//!
//! It runs on `x86_64` and aarch64 only: it renders in the processor's default
//! floating-point mode (`engine-fpenv`, `spec/engine-design.md` decision 11), and another
//! architecture refuses to build until its mode is set there.

#![forbid(unsafe_code)]

extern crate alloc;

mod case;
mod cli;
mod files;
mod wav;

pub use case::{
    CaseError, MAX_FRAMES, Prepared, Rendered, RenderedSamples, prepare_case, render_case,
};
pub use cli::{BestEffort, CliError, MAX_CASE_BYTES, MAX_REFERENCES_BYTES, USAGE, arguments, run};
pub use files::{Disk, Files};
pub use wav::{HEADER_LEN, wav_bytes, wav_file, wav_header};

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
