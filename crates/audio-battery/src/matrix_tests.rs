//! Tests of the shared test matrix.

use audio_core::{Precision, SampleRate};

use super::{
    ENGINE_BLOCK_SIZES, HOST_BUFFER_SIZES, PRECISIONS, SAMPLE_RATES, SPECIAL_INPUTS,
    SPECIAL_INPUTS_BINARY64, sample_rates,
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
fn engine_blocks_span_the_range_with_every_offered_size() {
    assert_eq!(
        ENGINE_BLOCK_SIZES.first(),
        Some(&1),
        "the smallest, 1 frame"
    );
    assert_eq!(
        ENGINE_BLOCK_SIZES.last(),
        Some(&audio_core::MAX_ENGINE_BLOCK),
        "the largest"
    );
    for frames in audio_core::ENGINE_BLOCKS {
        assert!(
            ENGINE_BLOCK_SIZES.contains(&frames),
            "{frames}, a size a build offers"
        );
    }
    assert!(
        ENGINE_BLOCK_SIZES
            .iter()
            .any(|frames| !frames.is_power_of_two() && !audio_core::ENGINE_BLOCKS.contains(frames)),
        "an export size outside the list, no power of two"
    );
    assert!(
        ENGINE_BLOCK_SIZES.windows(2).all(|pair| pair[0] < pair[1]),
        "ascending, each once"
    );
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
    assert_eq!(PRECISIONS[0], Precision::default(), "binary64 first");
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
