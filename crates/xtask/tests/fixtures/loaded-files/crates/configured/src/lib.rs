//! A library that reads, built for WebAssembly only, a file named as test code and a file
//! that is not Rust: what only a build of that target shows the loaded-files gate.

/// A module named as test code, compiled into the library.
#[cfg(target_arch = "wasm32")]
pub mod wasm_tests;

/// Bytes of a file that is not Rust, compiled into the library.
#[cfg(target_arch = "wasm32")]
pub const BLOB: &[u8] = include_bytes!("blob.inc");
