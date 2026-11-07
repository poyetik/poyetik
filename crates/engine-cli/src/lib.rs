//! Offline renderer and conformance checker.
//!
//! A **conformance case** is a JSON file holding a patch, a sample rate and a duration
//! in frames. `engine-cli` renders cases offline with the native engine, writes WAV
//! files, and compares render hashes with the reference hashes of
//! `conformance/references.json`, which only `bless` may change.

#![forbid(unsafe_code)]

extern crate alloc;

mod case;
mod cli;
mod files;
mod wav;

pub use case::{Prepared, Rendered, prepare_case, render_case};
pub use cli::{USAGE, run};
pub use files::{Disk, Files};
pub use wav::{HEADER_LEN, wav_bytes, wav_file, wav_header};

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
