//! The shared test matrix: the values every dimension of the environment is tested at,
//! defined once and never retyped (`CLAUDE.md`, "Never test only at the default").
//!
//! `conformance/test-matrix.json` holds the same values for the TypeScript tests; a test
//! checks that both stay equal.

use engine_core::{Precision, SampleRate};

/// The precisions a render computes in (`spec/engine-design.md`, decision 10): every one,
/// the default first. Everything a box or a plan computes is tested in each.
pub const PRECISIONS: [Precision; 2] = [Precision::Binary64, Precision::Binary32];

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
mod tests {
    use engine_core::{Precision, SampleRate};

    use super::{
        HOST_BUFFER_SIZES, PRECISIONS, SAMPLE_RATES, SPECIAL_INPUTS, SPECIAL_INPUTS_BINARY64,
        sample_rates,
    };

    #[test]
    fn every_rate_is_supported_and_the_limits_are_included() {
        let rates: Vec<u32> = sample_rates().iter().map(|rate| rate.hz()).collect();
        assert_eq!(rates, SAMPLE_RATES.to_vec(), "every rate is valid");
        assert!(rates.contains(&SampleRate::MIN), "lowest rate");
        assert!(rates.contains(&SampleRate::MAX), "highest rate");
        assert!(rates.contains(&44_100), "the most common device rate");
    }

    #[test]
    fn special_inputs_stay_in_the_input_domain() {
        assert!(
            SPECIAL_INPUTS
                .iter()
                .all(|value| (-1.0..=1.0).contains(value)),
            "within [-1, 1]"
        );
        assert!(
            SPECIAL_INPUTS.iter().any(|value| value.is_subnormal()),
            "subnormals"
        );
        assert!(
            SPECIAL_INPUTS
                .iter()
                .any(|value| value.to_bits() == (-0.0_f32).to_bits()),
            "negative zero"
        );
        // Symmetric and without repetition: every value comes with its exact negation.
        let bits: alloc::collections::BTreeSet<u32> =
            SPECIAL_INPUTS.iter().map(|value| value.to_bits()).collect();
        assert_eq!(bits.len(), SPECIAL_INPUTS.len(), "no value twice");
        for value in SPECIAL_INPUTS {
            assert!(bits.contains(&(-value).to_bits()), "-{value} is there too");
        }
    }

    #[test]
    fn special_inputs_are_the_same_classes_in_binary64() {
        // The same list, class by class and sign by sign, at binary64's own extremes.
        for (narrow, wide) in SPECIAL_INPUTS.iter().zip(SPECIAL_INPUTS_BINARY64) {
            assert_eq!(
                narrow.classify(),
                wide.classify(),
                "{narrow:e} and {wide:e}"
            );
            assert_eq!(
                narrow.is_sign_negative(),
                wide.is_sign_negative(),
                "{narrow:e} and {wide:e}"
            );
            assert!(
                (-1.0_f64..=1.0_f64).contains(&wide),
                "{wide:e} within [-1, 1]"
            );
        }
        assert_eq!(
            SPECIAL_INPUTS_BINARY64[2].to_bits(),
            1,
            "the smallest binary64 subnormal"
        );
        assert_eq!(
            SPECIAL_INPUTS_BINARY64[4].to_bits(),
            1_u64 << 52_u32,
            "the smallest binary64 normal"
        );
    }

    #[test]
    fn every_precision_once_the_default_first() {
        assert_eq!(PRECISIONS[0], Precision::default(), "64-bit first");
        assert_eq!(
            PRECISIONS.map(Precision::bits),
            [64, 32],
            "binary64 and binary32"
        );
    }

    #[test]
    fn matches_the_shared_json_matrix() {
        let json: serde_json::Value =
            serde_json::from_str(include_str!("../../../conformance/test-matrix.json"))
                .expect("valid JSON");
        assert_eq!(
            json["sample_rates"],
            serde_json::json!(SAMPLE_RATES),
            "same sample rates"
        );
        assert_eq!(
            json["host_buffer_sizes"],
            serde_json::json!(HOST_BUFFER_SIZES),
            "same buffer sizes"
        );
        assert_eq!(
            json["precisions"],
            serde_json::json!(PRECISIONS.map(Precision::bits)),
            "same precisions, by their widths"
        );
    }
}
