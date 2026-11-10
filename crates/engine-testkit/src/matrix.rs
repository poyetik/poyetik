//! The shared test matrix: the values every dimension of the environment is tested at,
//! defined once and never retyped (`CLAUDE.md`, "Never test only at the default").
//!
//! `conformance/test-matrix.json` holds the same values for the TypeScript tests; a test
//! checks that both stay equal.

use engine_core::{Precision, SampleRate};

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

/// Special input values inside the input domain `[-1, 1]`.
///
/// Signed zeros, the smallest subnormals, the smallest normals and full scale. Values
/// outside the domain (NaN, infinities, huge values) have no policy yet (roadmap #12,
/// step 1).
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
