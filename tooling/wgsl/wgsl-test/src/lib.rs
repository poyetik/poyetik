//! Test WGSL shaders from Rust.
//!
//! The first tool of the WGSL proof suite: shaders held to the same bar as the rest of the
//! code.
//!
//! Its validation ([`validate`], and the command line `wgsl-test validate <file>...`,
//! [`cli`]) is a thin layer over naga, the WGSL compiler and validator of wgpu: naga does
//! all the work of parsing and validating, as its own command line (`naga-cli`) does. The
//! layer only adds what the suite needs around it: the very naga wgpu uses (its version
//! pinned to wgpu's and checked), every rule of naga's validator under naga's default
//! capabilities, naga's narrowest position of the fault rather than the function holding
//! it, and one line per problem, made safe to show ([`safe_message`]), under the project's
//! command-line contract. It is meant as a building block of the suite: its harness
//! (roadmap #44 step 3) and its other tools (#45 step 1, #46 step 1, #47 step 1 and
//! #48 step 1) are to validate a shader through it before they run it.
//!
//! The crate depends on no crate of the project that holds it, so that it can be published
//! and used alone.

#![forbid(unsafe_code)]
#![doc(test(attr(forbid(unsafe_code))))]
#![expect(
    clippy::multiple_crate_versions,
    reason = "naga's own tree holds two versions of syn (thiserror's derive and half's zerocopy \
              derive), which this crate cannot choose; the pair is a reviewed exception of \
              deny.toml"
)]

pub mod cli;
mod safe;
mod validation;

pub use safe::{MAX_MESSAGE_CHARS, safe_message};
pub use validation::{Invalid, Location, Stage, validate};
