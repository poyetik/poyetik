//! The shared visual test matrix: the values every dimension of a visual render is tested
//! at, defined once and never retyped (`CLAUDE.md`, "Never test only at the default").
//!
//! `conformance/visual/test-matrix.json` holds the same values for every other reader; a
//! test checks that both stay equal.

use visual_core::{ImageIndex, ImageRate, ImageSize};

/// Image rates, as reduced fractions.
///
/// The limits of [`ImageRate`] (1 and 1000 images per second), the broadcast rates that
/// are not whole numbers (23.976 and 29.97), the European 25, the usual 60 and a gaming
/// display's 144. Deliberately more than 60.
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

/// Where a hint runs: the repository's name, holding a space and a quote.
///
/// A path a hint names unquoted fails there. Each tool's followed-hint test runs in it, from
/// every directory of [`HINT_DIRECTORIES`], the damage in every state of [`HINT_STAGED`].
pub const HINT_REPOSITORY: &str = "restore it's";

/// The directories of [`HINT_REPOSITORY`] a hint runs from: its root and one below it.
pub const HINT_DIRECTORIES: [&str; 2] = ["", "sub dir"];

/// Whether the damage a hint repairs is staged: a restore from the index keeps it then.
pub const HINT_STAGED: [bool; 2] = [false, true];

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
#[path = "matrix_tests.rs"]
mod tests;
