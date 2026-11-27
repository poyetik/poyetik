//! Poyetik engine core: the contract every box implements, strong types, the math layer,
//! the deterministic execution plan and its execution modes, the output stage with its
//! true-peak limiter, the settings a host states, and the types of the box contract across a
//! module boundary (`spec/box-contract.md`), in its `host` module.
//!
//! The engine computes in binary64 by default and in binary32 as a host setting
//! ([`Precision`]): the plan and every box are generic over the [`Sample`] type, the
//! engine's math layer, through which all box arithmetic goes.
//!
//! This crate contains no concrete box. It is `no_std` and forbids `unsafe`, so it
//! cannot reach the OS, the clock, threads or the network: the engine is
//! deterministic by construction. It builds for every target; its unit tests build on
//! `x86_64` and aarch64 hosts only, since their host differential forces a floating-point mode
//! through `fpenv`, which supports those two (#24 step 159 runs them on wasm32 and a 32-bit
//! target).
//!
//! Natively, every public function that does float work, building, processing, rendering,
//! limiting or setting the limiter (among them `Plan::build`, `Plan::process`, `Renderer`,
//! `OutputStage::apply`, `OutputStage::apply_narrowing`, `OutputStage::limited`,
//! `OutputStage::set_limiter`, `OutputStage::set_protected`, `TruePeakLimiter::new`,
//! `TruePeakLimiter::set` and `Detector::coefficients`),
//! runs in the CPU's default floating-point mode, which the host may have changed
//! (flush-to-zero, another rounding): a native caller makes it inside
//! `fpenv::with_default_mode` (`spec/engine-design.md`, decision 11). These entry points take no
//! proof of it, since this crate also builds for WebAssembly, whose floats have no mode: the
//! precondition is prose here until the native host API makes it a type (#32 step 17).

#![no_std]
#![forbid(unsafe_code)]
#![doc(test(attr(forbid(unsafe_code))))]
// A public enum that may gain a variant is `#[non_exhaustive]`, so that adding one breaks no
// caller; one that is closed by its decision says why where it is declared.
#![deny(clippy::exhaustive_enums)]
#![deny(clippy::exhaustive_structs)]

extern crate alloc;

mod abi;
mod block;
mod description;
mod error;
mod flush;
mod limiter;
#[expect(
    clippy::exhaustive_enums,
    reason = "Precision, RenderFormat and ArithmeticOption are the closed lists of decision 10 \
              (two precisions, three formats, the options the render hash names): a new one \
              changes the format and the hash, so every match must be revisited"
)]
mod math;
mod mode;
mod object;
mod output;
mod patch;
mod plan;
mod rate;
mod render;
mod settings;
mod subnormals;

pub use block::{BLOCK_SIZE, Block, MAX_ENGINE_BLOCK, NAMED_ENGINE_BLOCK};
pub use error::CoreError;
pub use limiter::{Detector, LimiterError, LimiterSettings, Setting, TruePeakLimiter};
pub use math::{
    ArithmeticMode, ArithmeticOption, Binary32, Binary64, MODE, Precision, RenderFormat, Sample,
    TRUE_PEAK_FILTER, TRUE_PEAK_TAPS, UnknownArithmeticOptions, host, ordered_sum,
};
pub use mode::{BoxMode, ExecutionMode};
pub use object::{
    AlgorithmVersion, BoxInstance, Capability, Create, Dispatch, Driven, DrivenValues, Driving,
    ENGINE_BLOCKS, Factory, Object, ObjectSpec, ParamError, ParamSpec, ParamSpecError, ParamValues,
    Params, PortSpec, RENDER_RULES_VERSION, Typed, Values,
};
pub use output::{AsComputed, FirstReplaced, LimiterMode, OutputStage, Replaced, StageError};
pub use patch::{Connection, Node, NodeId, Patch};
pub use plan::Plan;
pub use rate::SampleRate;
pub use render::{Endless, Lasting, Length, Renderer};
pub use settings::{
    AppliedSettings, Direction, HostSettings, InputChannels, MAX_CHANNELS, OutputChannels,
    Unhonored,
};
pub use subnormals::{SubnormalArithmetic, SubnormalsKept};

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
