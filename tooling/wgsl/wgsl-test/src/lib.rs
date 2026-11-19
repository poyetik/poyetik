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
//! accepts. A test may also prove its kernels on every adapter of the machine
//! ([`Gpu::every`]), and every report of a device names its adapter (roadmap #44 step 5):
//! the harness needs no display, so it runs headless, on a CI runner's software GPU
//! (lavapipe, which xtask's workflow installs) as on a real one.
//!
//! Kernels chain on buffers that stay on the GPU ([`Buffers`], roadmap #44 step 6): buffers
//! made once, bound to each kernel that uses them, then passes and copies ([`Step`]) in one
//! submission, as a renderer chains them, with any buffer rewritten or read back between
//! submissions; every binding checked as a one-dispatch run checks it, which is the
//! one-pass case of the same path.
//!
//! What a kernel wrote is compared with a reference ([`Outputs::check`], [`compare`], roadmap
//! #44 step 4): bit for bit by default, or within a [`Tolerance`] carried with its
//! derivation, `f32` values only; a mismatch ([`Mismatch`]) reports the first difference, its
//! index, the adapter that computed it, both values with their bits, how far apart they are
//! against the bound, and the inputs the test describes at that index.
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
mod compare;
mod layout;
mod run;
mod safe;
mod validation;

pub use compare::{Inputs, Mismatch, Tolerance, Value, Values, compare};
pub use layout::{Access, Binding, Scalar, Size};
pub use run::{
    Bound, BufferId, Buffers, Data, Given, Gpu, Kernel, Limits, Outputs, RunError, Step, Workgroups,
};
pub use safe::{MAX_MESSAGE_CHARS, safe_message};
pub use validation::{Invalid, Location, Stage, validate};
