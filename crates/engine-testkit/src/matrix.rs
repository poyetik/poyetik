//! The shared test matrix: the values every dimension of the environment is tested at,
//! defined once and never retyped (`CLAUDE.md`, "Never test only at the default").
//!
//! `conformance/test-matrix.json` holds the same values for the TypeScript tests; a test
//! checks that both stay equal.

use engine_core::SampleRate;

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
    use engine_core::SampleRate;

    use super::{HOST_BUFFER_SIZES, SAMPLE_RATES, SPECIAL_INPUTS, sample_rates};

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
            SPECIAL_INPUTS.iter().all(|v| (-1.0..=1.0).contains(v)),
            "within [-1, 1]"
        );
        assert!(
            SPECIAL_INPUTS.iter().any(|v| v.is_subnormal()),
            "subnormals"
        );
        assert!(
            SPECIAL_INPUTS
                .iter()
                .any(|v| v.to_bits() == (-0.0_f32).to_bits()),
            "negative zero"
        );
        // Symmetric and without repetition: every value comes with its exact negation.
        let bits: alloc::collections::BTreeSet<u32> =
            SPECIAL_INPUTS.iter().map(|v| v.to_bits()).collect();
        assert_eq!(bits.len(), SPECIAL_INPUTS.len(), "no value twice");
        for value in SPECIAL_INPUTS {
            assert!(bits.contains(&(-value).to_bits()), "-{value} is there too");
        }
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
    }
}
