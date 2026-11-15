//! Tests of `error.rs`.

extern crate alloc;

use alloc::string::ToString;

use super::VisualError;

#[test]
fn names_the_rejected_values() {
    let cases = [
        (
            VisualError::InvalidImageRate {
                numerator: 2,
                denominator: 4,
            },
            "invalid image rate 2/4: a reduced fraction from 1 to 1000 images per second is required",
        ),
        (
            VisualError::InvalidImageSize {
                width: 0,
                height: 7,
            },
            "invalid image size 0x7: each side must be from 1 to 16384 pixels",
        ),
        (
            VisualError::ImageTooLarge {
                width: 16_384,
                height: 2049,
            },
            "invalid image size 16384x2049: an image holds at most 33554432 pixels",
        ),
        (
            VisualError::InvalidImageRange {
                first: u64::MAX,
                count: 2,
            },
            "invalid image range: 2 images from image 18446744073709551615 pass the largest index, 18446744073709551615",
        ),
        (
            VisualError::InvalidSeconds {
                numerator: 0,
                denominator: 3,
            },
            "invalid duration 0/3 s: a reduced, nonzero fraction is required",
        ),
    ];
    for (error, message) in cases {
        assert_eq!(error.to_string(), message, "{error:?}");
    }
}

#[test]
fn is_an_error() {
    /// Compiles only for an error type.
    const fn error<E: core::error::Error>() {}
    error::<VisualError>();
}

#[test]
fn states_the_supported_rate_from_its_constant() {
    assert_eq!(
        crate::ImageRate::MAX_PER_SECOND,
        1000,
        "the limit the message names"
    );
}
