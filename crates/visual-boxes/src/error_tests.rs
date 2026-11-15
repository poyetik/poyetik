//! Tests of `error.rs`.

extern crate alloc;

use alloc::string::ToString;

use super::BoxError;

#[test]
fn names_the_rejected_value() {
    assert_eq!(
        BoxError::InvalidSide(65_537).to_string(),
        "invalid square side 65537: at most 65536, a square as tall as the image (a side is in 65536ths of the image height)",
        "message"
    );
}

#[test]
fn is_an_error() {
    /// Compiles only for an error type.
    const fn error<E: core::error::Error>() {}
    error::<BoxError>();
}
