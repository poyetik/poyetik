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
