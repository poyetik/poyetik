//! Poyetik boxes.
//!
//! Each box type lives in its own module and depends only on the `engine-core`
//! contract, written once for both precisions: generic over `engine_core::Sample`, its
//! arithmetic through the engine's math layer (this crate does not depend on `libm`). [`REGISTRY`] lists every box type; being in the registry is enough for a
//! box to be checked against the generic laws of `engine-testkit`. [`BuiltIn`] is how the
//! engine's plans hold them, so that a built-in box is called with no indirect call.
//!
//! Like the core, this crate is `no_std` and forbids `unsafe`; outside its tests it also
//! forbids float arithmetic (Clippy's `float_arithmetic`): a box computes with the sample
//! types, whose operators are the math layer's, and the compiler refuses any lifting.

#![no_std]
#![forbid(unsafe_code)]
#![cfg_attr(not(test), forbid(clippy::float_arithmetic))]

extern crate alloc;

mod dac;
mod osc;

use alloc::boxed::Box;
use core::ptr;

use engine_core::{Binary64, Block, Instance, Object, ObjectSpec, Sample, SampleRate};

pub use dac::DAC;
pub use osc::OSC;

/// Every box type of the engine.
pub static REGISTRY: [&ObjectSpec; 2] = [&OSC, &DAC];

/// An instance of a box type of [`REGISTRY`], called directly, or of any other box type
/// (one loaded at run time), called through its virtual table.
///
/// It is the instance type of the engine's plans (`engine_core::Plan<S, BuiltIn<S>>`),
/// which call a built-in box with no indirect call (`spec/engine-design.md`, decision 33).
///
/// A box whose state is larger than a pointer is kept on the heap, as the virtual table's
/// instances are: an instance held inline made every instance as large as the largest box
/// type, and loading a plan cost more (`spec/benchmarks.md`).
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

impl<S: Sample> Instance<S> for BuiltIn<S> {
    /// The box type is told by the address of its spec, the registry's statics being the
    /// only specs of the built-in box types: a copy of one is another box type, created
    /// through its factory.
    fn create(spec: &'static ObjectSpec, rate: SampleRate, params: &[Binary64]) -> Self {
        Self(if ptr::eq(spec, &raw const OSC) {
            Kind::Osc(Box::new(osc::Osc::of(rate, params)))
        } else if ptr::eq(spec, &raw const DAC) {
            Kind::Dac(dac::Dac)
        } else {
            Kind::Other(spec.create.create(rate, params))
        })
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
