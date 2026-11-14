//! Tests of `matrix.rs`.

use visual_core::{ImageRate, ImageSize};

use super::{IMAGE_INDICES, IMAGE_RATES, IMAGE_SIZES, image_indices, image_rates, image_sizes};

#[test]
fn every_rate_is_valid_and_the_limits_are_included() {
    let rates: Vec<(u32, u32)> = image_rates()
        .iter()
        .map(|rate| (rate.numerator(), rate.denominator()))
        .collect();
    assert_eq!(rates, IMAGE_RATES.to_vec(), "every rate is valid");
    assert!(rates.contains(&(1, 1)), "lowest rate");
    assert!(
        rates.contains(&(ImageRate::MAX_PER_SECOND, 1)),
        "highest rate"
    );
    assert!(
        rates.contains(&(24_000, 1001)),
        "a rate that is not a whole number"
    );
}

#[test]
fn every_size_is_valid_and_the_limits_are_included() {
    let sizes: Vec<(u32, u32)> = image_sizes()
        .iter()
        .map(|size| (size.width(), size.height()))
        .collect();
    assert_eq!(sizes, IMAGE_SIZES.to_vec(), "every size is valid");
    assert!(sizes.contains(&(1, 1)), "smallest");
    assert!(sizes.contains(&(ImageSize::MAX_SIDE, 1)), "widest");
    assert!(sizes.contains(&(1, ImageSize::MAX_SIDE)), "tallest");
}

#[test]
fn the_indices_include_the_first_and_the_last() {
    let indices: Vec<u64> = image_indices().iter().map(|index| index.0).collect();
    assert_eq!(indices, IMAGE_INDICES.to_vec(), "every index");
    assert!(indices.contains(&0), "first");
    assert!(indices.contains(&u64::MAX), "last");
}

#[test]
fn matches_the_shared_json_matrix() {
    let json: serde_json::Value =
        serde_json::from_str(include_str!("../../../conformance/visual/test-matrix.json"))
            .expect("valid JSON");
    let keys: Vec<&String> = json.as_object().expect("an object").keys().collect();
    assert_eq!(
        keys,
        ["image_indices", "image_rates", "image_sizes"],
        "every key is read here"
    );
    assert_eq!(
        json["image_rates"],
        serde_json::json!(IMAGE_RATES),
        "same rates"
    );
    assert_eq!(
        json["image_sizes"],
        serde_json::json!(IMAGE_SIZES),
        "same sizes"
    );
    assert_eq!(
        json["image_indices"],
        serde_json::json!(IMAGE_INDICES),
        "same indices"
    );
}
