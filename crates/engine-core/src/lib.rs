//! Poyetik engine core: the contract every box implements, strong types, and the
//! deterministic execution plan.
//!
//! This crate contains no concrete box. It is `no_std` and forbids `unsafe`, so it
//! cannot reach the OS, the clock, threads or the network: the engine is
//! deterministic by construction.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod block;
mod error;
mod flush;
mod object;
mod patch;
mod plan;
mod rate;
mod render;

pub use block::{BLOCK_SIZE, Block, SILENCE};
pub use error::CoreError;
pub use flush::flush;
pub use object::{Capability, Factory, Object, ObjectSpec, ParamSpec, PortSpec};
pub use patch::{Connection, Node, NodeId, Patch};
pub use plan::Plan;
pub use rate::SampleRate;
pub use render::Renderer;
