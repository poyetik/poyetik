//! The fingerprint of a box type: the hash of its outputs over a fixed battery of inputs
//! (`spec/engine-design.md`, decision 29).
//!
//! The conformance cases see a bit change only where their patches reach; a fingerprint
//! sees one wherever the battery reaches: both precisions, every sample rate of the
//! [`crate::matrix`], the parameter sets every box is tried with, the matrix's special inputs and
//! a seeded random signal. `engine-cli bless` records it per box type and algorithm version
//! and refuses a change at a recorded version, as `engine-cli check` does.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt::Write as _;

use engine_core::host::Raw;
use engine_core::{
    BLOCK_SIZE, Binary32, Binary64, Block, Object, ObjectSpec, ParamError, ParamValues, Precision,
};
use sha2::{Digest, Sha256};

use crate::inputs::{BLOCKS, BatterySample, param_sets, special_input};
use crate::matrix::{PRECISIONS, sample_rates};

/// The version of the battery.
///
/// Raised with any change of what [`fingerprint`] renders or hashes, the inputs it reads
/// included (the matrix, the parameter sets, the special inputs, the blocks of a run): a
/// test pins a digest of those inputs beside this number, so that changing them without
/// raising it fails. It is hashed first, so that a new battery gives every box type a new
/// fingerprint, which no recorded one is compared with; `bless --battery-changed` records
/// them (`spec/engine-design.md`, decision 29).
///
/// Battery 2 gives the parameters as `engine_core::Params`, as a plan does, without the
/// subnormal parameter set no box can be given.
pub const FINGERPRINT_BATTERY: u32 = 2;

/// The fingerprint of a box type: 64 lowercase hexadecimal digits, the same at every run
/// and on every target for the same bits of its outputs.
///
/// The SHA-256 of [`FINGERPRINT_BATTERY`] (`u32`, little-endian), then of the bit pattern
/// (`u64`, little-endian, a binary32 sample widened from its bits) of every output sample
/// of each run, in this order: binary64 then binary32; every sample rate of the matrix;
/// every parameter set ([`param_sets`], given as `engine_core::Params`); the special
/// inputs of the matrix, then a seeded random signal in `[-1, 1)` (the same values in
/// both precisions, rounded to binary32 there). Each run is a fresh instance rendering
/// [`BLOCKS`] blocks; the signal is cut into blocks, which feed the inlets in turn (block
/// by block, inlet by inlet: inlet 0 the first block, inlet 1 the next), so that two
/// inlets are given different signals; the outputs are hashed block by block, outlet by
/// outlet. A box type without outlets has the battery's fingerprint alone, whatever it
/// does: its bits are those the plan reads from its inlets (the output boxes), which the
/// conformance cases check.
///
/// # Errors
///
/// Returns why a parameter set is not one of the box type's ([`ParamValues::of`]): the
/// sets keep every value within its range, so only a default outside its range (which the
/// box laws refuse) gives one.
pub fn fingerprint(spec: &ObjectSpec) -> Result<String, ParamError> {
    let mut hasher = Sha256::new();
    hasher.update(FINGERPRINT_BATTERY.to_le_bytes());
    let length = BLOCKS
        .saturating_mul(BLOCK_SIZE)
        .saturating_mul(spec.inlets.len());
    let random = seeded_signal(length);
    for precision in PRECISIONS {
        match precision {
            Precision::Binary64 => battery::<Binary64>(spec, length, &random, &mut hasher)?,
            Precision::Binary32 => battery::<Binary32>(spec, length, &random, &mut hasher)?,
        }
    }
    Ok(hasher
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut text, byte| {
            // Writing to a `String` never fails.
            let _written = write!(text, "{byte:02x}");
            text
        }))
}

/// Hashes every run of the battery computing in `S`.
///
/// # Errors
///
/// Returns why a parameter set is not one of the box type's.
fn battery<S: BatterySample>(
    spec: &ObjectSpec,
    length: usize,
    random: &[f64],
    hasher: &mut Sha256,
) -> Result<(), ParamError> {
    let inputs = [
        special_input::<S>(length),
        random
            .iter()
            .map(|&value| S::from_binary64(Binary64::from_raw(value)))
            .collect(),
    ];
    for rate in sample_rates() {
        for params in param_sets(spec) {
            let values: Vec<Binary64> = params
                .iter()
                .map(|&param| Binary64::from_raw(f64::from(param)))
                .collect();
            let wide = ParamValues::of(spec, &values)?;
            for input in &inputs {
                let mut object: Box<dyn Object<S>> = spec.create.create(rate, wide.params());
                for block in outputs(object.as_mut(), spec, input) {
                    for sample in block {
                        hasher.update(sample.bits().to_le_bytes());
                    }
                }
            }
        }
    }
    Ok(())
}

/// The outputs of [`BLOCKS`] blocks of `object`, outlet by outlet at each block, its inlets
/// fed the successive blocks of `input` in turn (silence past its end).
fn outputs<S: BatterySample>(
    object: &mut dyn Object<S>,
    spec: &ObjectSpec,
    input: &[S],
) -> Vec<Block<S>> {
    let (chunked, _remainder) = input.as_chunks::<BLOCK_SIZE>();
    let mut chunks = chunked.iter();
    let mut inputs = vec![S::SILENCE; spec.inlets.len()];
    let mut outputs = vec![S::SILENCE; spec.outlets.len()];
    let mut rendered = Vec::with_capacity(BLOCKS.saturating_mul(spec.outlets.len()));
    for _ in 0..BLOCKS {
        for block in &mut inputs {
            for (sample, value) in block.iter_mut().zip(chunks.next().unwrap_or(&S::SILENCE)) {
                *sample = *value;
            }
        }
        object.process(&inputs, &mut outputs);
        rendered.extend_from_slice(&outputs);
    }
    rendered
}

/// The next 64 bits of `SplitMix64` (Steele, Lea and Flood, 2014, as `spec/measurements.md`
/// defines it) from `state`, which it advances.
///
/// Written here so that no dependency's generator can change the battery, and the one copy
/// of the crates that depend on this one (the measurement tests of `engine-testkit` draw
/// their noise from it, each with its own mapping to floats).
pub const fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut mixed = *state;
    mixed = (mixed ^ mixed.wrapping_shr(30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    mixed = (mixed ^ mixed.wrapping_shr(27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    mixed ^ mixed.wrapping_shr(31)
}

/// `length` values in `[-1, 1)` from a fixed seed: [`splitmix64`] from the state 0, the top
/// 32 bits of each draw scaled.
fn seeded_signal(length: usize) -> Vec<f64> {
    let mut state = 0_u64;
    (0..length)
        .map(|_| {
            let [first, second, third, fourth, ..] = splitmix64(&mut state).to_be_bytes();
            f64::from(u32::from_be_bytes([first, second, third, fourth])) / 2_147_483_648.0_f64
                - 1.0_f64
        })
        .collect()
}

#[cfg(test)]
#[path = "fingerprint_tests.rs"]
mod tests;
