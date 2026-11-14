//! The fingerprint of a box type: the hash of its outputs over a fixed battery of inputs
//! (`spec/engine-design.md`, decision 29).
//!
//! The conformance cases see a bit change only where their patches reach; a fingerprint
//! sees one wherever the battery reaches: both precisions, every sample rate of the
//! [`crate::matrix`], the parameter sets every box is tried with, the matrix's special inputs and
//! a seeded random signal. `audio-cli bless` records it per box type and algorithm version
//! and refuses a change at a recorded version, as `audio-cli check` does.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::convert::Infallible;
use core::fmt::Write as _;

use audio_core::host::Raw;
use audio_core::{BLOCK_SIZE, Binary32, Binary64, Block, Dispatch, ObjectSpec, Precision};
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
/// Battery 2 gives the parameters as `audio_core::Params`, as a plan does, without the
/// subnormal parameter set no box can be given.
pub const FINGERPRINT_BATTERY: u32 = 2;

/// The fingerprint of a box type: 64 lowercase hexadecimal digits, the same at every run
/// and on every target for the same bits of its outputs.
///
/// The SHA-256 of [`FINGERPRINT_BATTERY`] (`u32`, little-endian), then of the bit pattern
/// (`u64`, little-endian, a binary32 sample widened from its bits) of every output sample
/// of each run, in this order: binary64 then binary32; every sample rate of the matrix;
/// every parameter set ([`param_sets`], given as `audio_core::Params`); the special
/// inputs of the matrix, then a seeded random signal in `[-1, 1)` (the same values in
/// both precisions, rounded to binary32 there). Each run is a fresh instance rendering
/// [`BLOCKS`] blocks; the signal is cut into blocks, which feed the inlets in turn (block
/// by block, inlet by inlet: inlet 0 the first block, inlet 1 the next), so that two
/// inlets are given different signals ([`run_blocks`], the outputs +0 on entry to every
/// block); the outputs are hashed block by block, outlet by outlet. A box type without
/// outlets has the battery's fingerprint alone, whatever it does: its bits are those the
/// plan reads from its inlets (the output boxes), which the conformance cases check.
///
/// Each run creates its instance as a plan holding its boxes as `I64` (binary64) or `I32`
/// (binary32) does ([`Dispatch::create`]): given the engine's instance type
/// (`audio_boxes::BuiltIn`, as `audio-cli` gives it), the fingerprint measures the
/// code the engine runs, not only the box type's factory. `spec` is a static: a plan
/// calls its box types by the specs of its registry, which `BuiltIn` tells apart by
/// address.
#[must_use]
pub fn fingerprint<I64: Dispatch<Binary64>, I32: Dispatch<Binary32>>(
    spec: &'static ObjectSpec,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(FINGERPRINT_BATTERY.to_le_bytes());
    let blocks = BLOCKS.saturating_mul(spec.inlets().len());
    let random = seeded_signal(blocks);
    for precision in PRECISIONS {
        match precision {
            Precision::Binary64 => {
                battery::<Binary64, I64>(spec, blocks, &random, &mut hasher);
            }
            Precision::Binary32 => {
                battery::<Binary32, I32>(spec, blocks, &random, &mut hasher);
            }
        }
    }
    hasher
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut text, byte| {
            // Writing to a `String` never fails.
            let _written = write!(text, "{byte:02x}");
            text
        })
}

/// Hashes every run of the battery computing in `S`, each instance created as `I`.
fn battery<S: BatterySample, I: Dispatch<S>>(
    spec: &'static ObjectSpec,
    blocks: usize,
    random: &[[f64; BLOCK_SIZE]],
    hasher: &mut Sha256,
) {
    let inputs = [
        special_input::<S>(blocks),
        random
            .iter()
            .map(|block| block.map(|value| S::from_binary64(Binary64::from_raw(value))))
            .collect(),
    ];
    for rate in sample_rates() {
        for params in param_sets(spec) {
            for input in &inputs {
                let mut object = I::create(rate, &params);
                let Ok(rendered) =
                    run_blocks::<S, Infallible>(spec, input, BLOCKS, S::ZERO, |given, outputs| {
                        object.process(given, outputs);
                        Ok(())
                    });
                for block in rendered {
                    for sample in block {
                        hasher.update(sample.bits().to_le_bytes());
                    }
                }
            }
        }
    }
}

/// The outputs of `blocks` blocks of a box of the box type `spec`, outlet by outlet at each
/// block, each block processed by `process`.
///
/// `process` is given the block's inputs and outputs: its inlets fed the successive blocks
/// of `input` in turn (silence past its end; whole blocks by type, so no part of a block
/// is left out), every sample of its outputs set to `entry` before every block. One run for
/// the battery and the box laws (`audio-testkit`), so that both feed a box alike.
///
/// The box contract leaves what the outputs hold on entry unspecified, since a box writes
/// every sample of every outlet (`audio_core::Object::process`): the battery gives +0,
/// the box laws also a NaN (their long run, a NaN only), and a box that follows the
/// contract gives the same bits.
///
/// # Errors
///
/// Returns the first error of `process`, which ends the run.
pub fn run_blocks<S: BatterySample, E>(
    spec: &ObjectSpec,
    input: &[Block<S>],
    blocks: usize,
    entry: S,
    mut process: impl FnMut(&[Block<S>], &mut [Block<S>]) -> Result<(), E>,
) -> Result<Vec<Block<S>>, E> {
    let mut chunks = input.iter();
    let mut inputs = vec![S::SILENCE; spec.inlets().len()];
    let mut outputs = vec![S::SILENCE; spec.outlets().len()];
    let mut rendered = Vec::with_capacity(blocks.saturating_mul(spec.outlets().len()));
    for _ in 0..blocks {
        for block in &mut inputs {
            *block = *chunks.next().unwrap_or(&S::SILENCE);
        }
        for output in &mut outputs {
            output.fill(entry);
        }
        process(&inputs, &mut outputs)?;
        rendered.extend_from_slice(&outputs);
    }
    Ok(rendered)
}

/// The next 64 bits of `SplitMix64` (Steele, Lea and Flood, 2014, as `spec/measurements.md`
/// defines it) from `state`, which it advances.
///
/// Written here so that no dependency's generator can change the battery. The crates that
/// test with this one draw from it too (the measurement tests of `audio-testkit`, with
/// their own mapping to floats); `audio-core`, which this crate depends on, keeps its own
/// copy, held to the same published values by its tests.
pub const fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut mixed = *state;
    mixed = (mixed ^ mixed.wrapping_shr(30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    mixed = (mixed ^ mixed.wrapping_shr(27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    mixed ^ mixed.wrapping_shr(31)
}

/// `blocks` blocks of values in `[-1, 1)` from a fixed seed: [`splitmix64`] from the state
/// 0, the top 32 bits of each draw scaled, drawn sample by sample in order, block after
/// block.
fn seeded_signal(blocks: usize) -> Vec<[f64; BLOCK_SIZE]> {
    let mut state = 0_u64;
    (0..blocks)
        .map(|_| {
            core::array::from_fn(|_| {
                let [first, second, third, fourth, ..] = splitmix64(&mut state).to_be_bytes();
                f64::from(u32::from_be_bytes([first, second, third, fourth])) / 2_147_483_648.0_f64
                    - 1.0_f64
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "fingerprint_tests.rs"]
mod tests;
