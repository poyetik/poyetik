//! Errors raised while creating a visual box.

use core::fmt;

/// Why a box could not be created from its parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxError {
    /// The side of a square is larger than the image height.
    InvalidSide(u32),
}

impl fmt::Display for BoxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::InvalidSide(side) => write!(
                f,
                "invalid square side {side}: at most {} (the image height)",
                crate::Square::FULL_SIDE
            ),
        }
    }
}

#[cfg(test)]
mod tests {
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
}
