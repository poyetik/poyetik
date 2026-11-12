//! A library that loads modules by `path` attributes: one outside its crate's `src/`, one
//! of another crate; its tests load a Rust file of their declared test data.

/// A module read from outside `src/`.
#[path = "../extra/outside.rs"]
pub mod outside;

/// A module read from another crate's file.
#[path = "../../other/src/lib.rs"]
pub mod other;

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
