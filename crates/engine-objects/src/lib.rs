//! Poyetik boxes.
//!
//! Each box type lives in its own module and depends only on the `engine-core`
//! contract, written once for both precisions: generic over `engine_core::Sample`, its
//! arithmetic through the engine's math layer (this crate does not depend on `libm`). [`REGISTRY`]
//! lists every box type; being in the registry is enough for a box to be checked against the
//! generic laws of `engine-testkit`. [`BuiltIn`] is how the engine's plans hold them, so that a
//! built-in box is called with no indirect call.
//!
//! Like the core, this crate is `no_std` and forbids `unsafe`; outside its tests it also
//! forbids float arithmetic (Clippy's `float_arithmetic`): a box computes with the sample
//! types, whose operators are the math layer's, and the compiler refuses any lifting.

#![no_std]
#![forbid(unsafe_code)]
#![cfg_attr(not(test), forbid(clippy::float_arithmetic))]
#![doc(test(attr(forbid(unsafe_code))))]

extern crate alloc;

mod dac;
mod osc;

use alloc::boxed::Box;
use core::ptr;

use engine_core::{
    Block, Dispatch, Object, ObjectSpec, ParamValues, Plan, Renderer, Sample, SampleRate,
};

pub use dac::DAC;
pub use osc::OSC;

/// Every box type of the engine.
pub static REGISTRY: [&ObjectSpec; 2] = [&OSC, &DAC];

/// The engine's plan computing in `S`, its box instances held as [`BuiltIn`].
///
/// A built-in box type is then called directly (`spec/engine-design.md`, decision 33): the
/// plan of every host of the engine, where `engine_core::Plan` names the instance type
/// itself.
pub type EnginePlan<S> = Plan<S, BuiltIn<S>>;

/// The engine's offline renderer computing in `S`, of an [`EnginePlan`].
pub type EngineRenderer<S> = Renderer<S, BuiltIn<S>>;

/// An instance of a box type of [`REGISTRY`], called directly, or of any other box type
/// (one loaded at run time), called through its virtual table.
///
/// It is the instance type of the engine's plans ([`EnginePlan`], [`EngineRenderer`]),
/// which call a built-in box with no indirect call (`spec/engine-design.md`, decision 33).
///
/// A box whose state is larger than a pointer is kept on the heap, as the virtual table's
/// instances are: the form measured faster per block, while held inline it loads plans
/// faster but costs `osc~` more per block in binary32, the real-time path coming first
/// (`spec/benchmarks.md`, "Measured design choices", where the trade is dated; it is
/// measured again whenever this type or the plan's per-block code changes).
#[derive(Debug)]
pub struct BuiltIn<S: Sample>(Kind<S>);

/// Which box type an instance is of.
#[derive(Debug)]
enum Kind<S: Sample> {
    /// `osc~`.
    Osc(Box<osc::Osc>),
    /// `dac~`, which holds no state.
    Dac(dac::Dac),
    /// A box type outside the registry.
    Other(Box<dyn Object<S>>),
}

impl<S: Sample> Object<S> for BuiltIn<S> {
    fn process(&mut self, inputs: &[Block<S>], outputs: &mut [Block<S>]) {
        match &mut self.0 {
            Kind::Osc(instance) => instance.process(inputs, outputs),
            Kind::Dac(instance) => instance.process(inputs, outputs),
            Kind::Other(instance) => instance.process(inputs, outputs),
        }
    }
}

impl<S: Sample> Dispatch<S> for BuiltIn<S> {
    /// The box type is told by the address of the spec the values carry, the registry's
    /// statics being the specs of the built-in box types: a spec is built only from its box
    /// type's own parameters and instances (`engine_core::ObjectSpec::new`), and the types
    /// of the built-in boxes are private to this crate, so another spec at another address
    /// is another box type, created through its factory.
    #[inline]
    fn create(rate: SampleRate, values: &ParamValues) -> Self {
        let spec = values.spec();
        Self(if ptr::eq(spec, &raw const OSC) {
            Kind::Osc(Box::new(osc::Osc::of(rate, &values.params())))
        } else if ptr::eq(spec, &raw const DAC) {
            Kind::Dac(dac::Dac)
        } else {
            Kind::Other(<Box<dyn Object<S>> as Dispatch<S>>::create(rate, values))
        })
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
