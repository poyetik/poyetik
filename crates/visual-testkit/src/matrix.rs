//! The shared visual test matrix: the values every dimension of a visual render is tested
//! at, defined once and never retyped (`CLAUDE.md`, "Never test only at the default").
//!
//! `conformance/visual/test-matrix.json` holds the same values for every other reader; a
//! test checks that both stay equal.

use visual_core::{ImageIndex, ImageRate, ImageSize};

/// Image rates, as reduced fractions.
///
/// The limits of [`ImageRate`] (1 and 1000 images per second), the broadcast rates that are not whole numbers (23.976 and 29.97), the
/// European 25, the usual 60 and a gaming display's 144. Deliberately more than 60.
pub const IMAGE_RATES: [(u32, u32); 7] = [
    (1, 1),
    (24_000, 1001),
    (25, 1),
    (30_000, 1001),
    (60, 1),
    (144, 1),
    (1000, 1),
];

/// Image sizes, width by height: one pixel, odd and tiny, sizes whose aspect ratio is
/// not a common one, a small 16:9, and the longest supported side in each direction.
pub const IMAGE_SIZES: [(u32, u32); 6] = [
    (1, 1),
    (2, 3),
    (17, 9),
    (64, 36),
    (1, ImageSize::MAX_SIDE),
    (ImageSize::MAX_SIDE, 1),
];

/// Image indices: the first images, the end of a second at 60 images per second, a long
/// run, and the last index.
pub const IMAGE_INDICES: [u64; 6] = [0, 1, 2, 59, 1_000_000, u64::MAX];

/// [`IMAGE_RATES`] as checked image rates.
#[must_use]
pub fn image_rates() -> Vec<ImageRate> {
    IMAGE_RATES
        .iter()
        .filter_map(|&(numerator, denominator)| ImageRate::new(numerator, denominator).ok())
        .collect()
}

/// [`IMAGE_SIZES`] as checked image sizes.
#[must_use]
pub fn image_sizes() -> Vec<ImageSize> {
    IMAGE_SIZES
        .iter()
        .filter_map(|&(width, height)| ImageSize::new(width, height).ok())
        .collect()
}

/// [`IMAGE_INDICES`] as image indices.
#[must_use]
pub fn image_indices() -> Vec<ImageIndex> {
    IMAGE_INDICES
        .iter()
        .map(|&index| ImageIndex(index))
        .collect()
}

#[cfg(test)]
mod tests {
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
}
