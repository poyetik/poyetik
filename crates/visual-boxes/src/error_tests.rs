//! Tests of `error.rs`.

extern crate alloc;

use alloc::string::ToString;

use super::BoxError;

#[test]
fn names_the_rejected_value() {
    assert_eq!(
        BoxError::InvalidSide(65_537).to_string(),
        "invalid square side 65537: at most 65536 (the image height)",
        "message"
    );
}
