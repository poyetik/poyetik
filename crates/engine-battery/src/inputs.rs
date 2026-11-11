//! The inputs every box is tried with, by the battery and by the box laws: the blocks of a
//! run, the parameter sets and the special inputs.

use alloc::vec::Vec;

use engine_core::host::{ParamBounds, Raw};
use engine_core::{Binary32, Binary64, ObjectSpec, ParamSpec};

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

/// A value of the range that differs from the default whenever the range allows it: the
/// midpoint, or the point halfway between the default and the maximum when the default
/// is the midpoint. So that a box hard-coding its default cannot pass.
fn away_from_default(param: &ParamSpec) -> f32 {
    let midpoint = param.minimum() / 2.0 + param.maximum() / 2.0;
    if midpoint.to_bits() == param.default_value().to_bits() {
        param.default_value() / 2.0 + param.maximum() / 2.0
    } else {
        midpoint
    }
}

/// `value` when the range of `param` accepts it (bit for bit, `-0.0` included), else
/// the default, which another set already tries.
fn within(param: &ParamSpec, value: f32) -> f32 {
    if param.accepts(value) {
        value
    } else {
        param.default_value()
    }
}

/// Parameter sets every box is tried with.
///
/// All minimums, all defaults, all values away from their default, all maximums, then
/// special values where the range accepts them: `-0.0`, and the smallest normal binary32
/// value, `2^-126`, the smallest magnitude above zero a box is given (`Params` makes
/// anything below a zero, so a subnormal is no value a box can be tried with).
#[must_use]
pub fn param_sets(spec: &ObjectSpec) -> [Vec<f32>; 6] {
    let special = |value: f32| {
        spec.params
            .iter()
            .map(|param| within(param, value))
            .collect()
    };
    [
        spec.params.iter().map(ParamBounds::minimum).collect(),
        spec.params.iter().map(ParamBounds::default_value).collect(),
        spec.params.iter().map(away_from_default).collect(),
        spec.params.iter().map(ParamBounds::maximum).collect(),
        special(-0.0),
        special(f32::MIN_POSITIVE),
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
