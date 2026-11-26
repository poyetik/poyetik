//! The inputs every box is tried with, by the battery and by the box laws: the blocks of a
//! run, the parameter sets and the special inputs.

use alloc::vec::Vec;

use audio_core::host::{ParamBounds, Raw};
use audio_core::{Binary32, Binary64, Block, ObjectSpec, ParamSpec, ParamValues};

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

/// A value a box that hard-codes the default of `param` does not give: the midpoint of the
/// range, or the point halfway between the default and the maximum when the default, as a box
/// is given it, is the midpoint (a zero of either sign alike). A range of one value has no
/// other.
///
/// Within the range, rounded: a bound is zero or at least 2^-126 in magnitude, so halving it
/// is exact in binary64, and the sum of two halves of the range rounds to a value between
/// them.
fn away_from_default(param: &ParamSpec) -> f64 {
    let midpoint = param.minimum() / 2.0_f64 + param.maximum() / 2.0_f64;
    // As a box is given it, a zero of either sign alike.
    let given = |value: f64| {
        if value.abs() < f64::from(f32::MIN_POSITIVE) {
            0.0_f64
        } else {
            value
        }
    };
    if given(midpoint).to_bits() == given(param.default_value()).to_bits() {
        param.default_value() / 2.0_f64 + param.maximum() / 2.0_f64
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
        ParamValues::within(spec, |_| -0.0_f64),
        ParamValues::within(spec, |_| f64::from(f32::MIN_POSITIVE)),
    ]
}

/// The special inputs of the matrix in the precision `S`, repeated to fill `blocks` blocks
/// (every inlet for every block of a run), the cycle running on from one block to the next.
#[must_use]
pub fn special_input<S: BatterySample>(blocks: usize) -> Vec<Block<S>> {
    let mut cycle = S::special_inputs().into_iter().cycle();
    (0..blocks)
        .map(|_| {
            let mut block = S::SILENCE;
            for (sample, value) in block.iter_mut().zip(&mut cycle) {
                *sample = value;
            }
            block
        })
        .collect()
}
