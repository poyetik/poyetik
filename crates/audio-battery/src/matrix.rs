//! The shared test matrix: the values every dimension of the environment is tested at,
//! defined once and never retyped (`CLAUDE.md`, "Never test only at the default").
//!
//! `conformance/test-matrix.json` holds the values the TypeScript tests share: the precisions,
//! sample rates, host buffer sizes and the values of a hint's tests, which a test checks stay
//! equal, and the time zones and locales, which only they read.

use alloc::vec::Vec;

use audio_core::{Precision, SampleRate};

/// The precisions a render computes in (`spec/engine-design.md`, decision 10): every one,
/// the default first.
///
/// From the engine's own closed list, so that a precision added there is tested at once
/// (the matrix test still pins the two, so that it is reviewed). Everything a box or a plan
/// computes is tested in each.
pub const PRECISIONS: [Precision; 2] = Precision::ALL;

/// Sample rates, in hertz: the limits of [`SampleRate`], common device rates and the
/// low rates of telephony headsets. Deliberately more than the default 48 kHz.
pub const SAMPLE_RATES: [u32; 8] = [
    8_000, 11_025, 16_000, 22_050, 44_100, 48_000, 96_000, 384_000,
];

/// Host buffer sizes, in frames: how a host may split time (a single frame, odd sizes,
/// one engine block, a browser render quantum, a large buffer).
pub const HOST_BUFFER_SIZES: [usize; 6] = [1, 37, 64, 100, 128, 4096];

/// Engine blocks, in frames (`spec/engine-design.md`, decision 2), ascending.
///
/// The smallest (1), every size a patcher build offers (`audio_core::ENGINE_BLOCKS`), an
/// export size that is none of them and no power of two (441, 10 ms at 44.1 kHz) and the
/// largest (`audio_core::MAX_ENGINE_BLOCK`): the sizes the engine-block law renders at. A
/// box type created through its virtual table exists only at those of `ENGINE_BLOCKS`, the
/// built-in ones at all.
pub const ENGINE_BLOCK_SIZES: [usize; 10] = [1, 16, 32, 64, 128, 256, 441, 512, 1024, 1_048_576];

/// Special input values inside the input domain `[-1, 1]`.
///
/// Signed zeros, the smallest subnormals, the smallest normals and full scale. Values
/// outside the domain (NaN, infinities, huge values) are not in the matrix yet: decision 5
/// of `spec/engine-design.md` sets their policy, which roadmap #12 step 15 (where the boundary clamps,
/// holds or flushes a parameter's value) and step 9 (non-finite outputs) apply.
pub const SPECIAL_INPUTS: [f32; 8] = [
    0.0,
    -0.0,
    f32::from_bits(1),
    -f32::from_bits(1),
    f32::MIN_POSITIVE,
    -f32::MIN_POSITIVE,
    1.0,
    -1.0,
];

/// [`SPECIAL_INPUTS`] in binary64: signed zeros, the smallest subnormals and normals of
/// binary64, and full scale.
pub const SPECIAL_INPUTS_BINARY64: [f64; 8] = [
    0.0,
    -0.0,
    f64::from_bits(1),
    -f64::from_bits(1),
    f64::MIN_POSITIVE,
    -f64::MIN_POSITIVE,
    1.0,
    -1.0,
];

/// Where a hint runs: the repository's name, holding a space and a quote.
///
/// A path a hint names unquoted fails there. Each tool's followed-hint test runs in it, from
/// every directory of [`HINT_DIRECTORIES`], the damage in every state of [`HINT_STAGED`].
pub const HINT_REPOSITORY: &str = "restore it's";

/// The directories of [`HINT_REPOSITORY`] a hint runs from: its root and one below it.
pub const HINT_DIRECTORIES: [&str; 2] = ["", "sub dir"];

/// Whether the damage a hint repairs is staged: a restore from the index keeps it then.
pub const HINT_STAGED: [bool; 2] = [false, true];

/// [`SAMPLE_RATES`] as checked sample rates.
#[must_use]
pub fn sample_rates() -> Vec<SampleRate> {
    SAMPLE_RATES
        .iter()
        .filter_map(|&hz| SampleRate::new(hz).ok())
        .collect()
}

#[cfg(test)]
#[path = "matrix_tests.rs"]
mod tests;
