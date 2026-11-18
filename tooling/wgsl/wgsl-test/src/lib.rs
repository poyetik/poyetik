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
//! command-line contract. It is meant as a building block of the suite: its harness (below)
//! and its other tools (#45 step 1, #46 step 1, #47 step 1 and #48 step 1) validate a shader
//! through it before they run it.
//!
//! Its harness runs a compute kernel from a test ([`Kernel`], roadmap #44 step 3): the
//! buffers the kernel binds are read from its shader ([`Binding`]), each buffer a test gives
//! ([`Data`]) is checked against its binding before anything reaches the GPU, one dispatch
//! runs, and every buffer the kernel may write comes back bit for bit ([`Outputs`]). Every
//! failure is a [`RunError`], never a panic. The tests of a program share one device
//! ([`Gpu::shared`]), on the APIs wgpu's own `WGPU_BACKEND` variable lists, under WebGPU's
//! default limits ([`Limits::WebGpu`]), so that a kernel its tests prove is one the web
//! accepts.
//!
//! The crate depends on no crate of the project that holds it, so that it can be published
//! and used alone.

#![forbid(unsafe_code)]
#![doc(test(attr(forbid(unsafe_code))))]
#![expect(
    clippy::multiple_crate_versions,
    reason = "wgpu's tree holds two versions of syn (zerocopy-derive's beside bytemuck_derive's) \
              and of hashbrown (gpu-allocator's beside naga's), which this crate cannot \
              choose; each pair is a reviewed exception of deny.toml"
)]

pub mod cli;
mod layout;
mod run;
mod safe;
mod validation;

pub use layout::{Access, Binding, Scalar, Size};
pub use run::{Data, Given, Gpu, Kernel, Limits, Outputs, RunError, Workgroups};
pub use safe::{MAX_MESSAGE_CHARS, safe_message};
pub use validation::{Invalid, Location, Stage, validate};
