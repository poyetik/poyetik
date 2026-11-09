//! Poyetik engine core: the contract every box implements, strong types, the math layer
//! and the deterministic execution plan.
//!
//! The engine computes in binary64 by default and in binary32 as a host setting
//! ([`Precision`]): the plan and every box are generic over the [`Sample`] type, the
//! engine's math layer, through which all box arithmetic goes.
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
mod math;
mod object;
mod patch;
mod plan;
mod rate;
mod render;

pub use block::{BLOCK_SIZE, Block};
pub use error::CoreError;
pub use math::{
    ArithmeticMode, ArithmeticOption, Binary32, Binary64, Constructor, MODE, Precision,
    RenderFormat, Sample, UnknownArithmeticOptions, host, ordered_sum,
};
pub use object::{
    AlgorithmVersion, Capability, Create, Factory, Object, ObjectSpec, ParamSpec, PortSpec,
};
pub use patch::{Connection, Node, NodeId, Patch};
pub use plan::Plan;
pub use rate::SampleRate;
pub use render::Renderer;
