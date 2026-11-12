//! The inputs every box is tried with, by the battery and by the box laws: the blocks of a
//! run, the parameter sets and the special inputs.

use alloc::vec::Vec;

use engine_core::host::{ParamBounds, Raw};
use engine_core::{Binary32, Binary64, ObjectSpec, ParamSpec, ParamValues};

use crate::matrix::{SPECIAL_INPUTS, SPECIAL_INPUTS_BINARY64};

/// A sample the battery feeds, in either precision.
pub trait BatterySample: Raw {
    /// The special inputs of the matrix in this precision.
    fn special_inputs() -> [Self; 8];
}

impl BatterySample for Binary64 {
    fn special_inputs() -> [Self; 8] {
        SPECIAL_INPUTS_BINARY64.map(Self::from_raw)
    }
}

impl BatterySample for Binary32 {
    fn special_inputs() -> [Self; 8] {
        SPECIAL_INPUTS.map(Self::from_raw)
    }
}

/// Blocks rendered per run.
pub const BLOCKS: usize = 8;

/// A value that differs from the default whenever the range allows it: the midpoint, or
/// the point halfway between the default and the maximum when the default is the midpoint.
/// So that a box hard-coding its default cannot pass. Halving bounds a few units of the
/// last place above 2^-126 rounds, so either form can fall outside the range: the set then
/// gives the default ([`ParamValues::within`]).
fn away_from_default(param: &ParamSpec) -> f32 {
    let midpoint = param.minimum() / 2.0 + param.maximum() / 2.0;
    if midpoint.to_bits() == param.default_value().to_bits() {
        param.default_value() / 2.0 + param.maximum() / 2.0
    } else {
        midpoint
    }
}

/// Parameter sets every box is tried with, as a box is given them.
///
/// All minimums, all defaults, all values away from their default, all maximums, then
/// special values: `-0.0`, and the smallest normal binary32 value, `2^-126`, the smallest
/// magnitude above zero a box is given (`Params` makes anything below a zero, so a
/// subnormal is no value a box can be tried with). Each through
/// [`ParamValues::within`]: a value its range refuses is the default, which another set
/// already tries, so every set is one of the box type's by construction.
#[must_use]
pub fn param_sets(spec: &'static ObjectSpec) -> [ParamValues; 6] {
    [
        ParamValues::within(spec, ParamBounds::minimum),
        ParamValues::within(spec, ParamBounds::default_value),
        ParamValues::within(spec, away_from_default),
        ParamValues::within(spec, ParamBounds::maximum),
        ParamValues::within(spec, |_| -0.0),
        ParamValues::within(spec, |_| f32::MIN_POSITIVE),
    ]
}

/// The special inputs of the matrix in the precision `S`, repeated to fill every inlet for
/// every block: `length` samples.
#[must_use]
pub fn special_input<S: BatterySample>(length: usize) -> Vec<S> {
    S::special_inputs()
        .iter()
        .copied()
        .cycle()
        .take(length)
        .collect()
}
