//! Exact time: the image rate and the index of an image.

use crate::VisualError;

/// Greatest common divisor, by Euclid's algorithm.
const fn gcd(mut first: u64, mut second: u64) -> u64 {
    // `checked_rem` is `None` exactly when `second` is 0, which ends the algorithm.
    while let Some(rest) = first.checked_rem(second) {
        first = second;
        second = rest;
    }
    first
}

/// An image rate: `numerator / denominator` images per second, an exact rational number
/// (23.976 images per second is `24000 / 1001`), always a reduced fraction so that one
/// rate has one representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImageRate {
    /// Images per `denominator` seconds.
    numerator: u32,
    /// Seconds.
    denominator: u32,
}

impl ImageRate {
    /// Highest supported rate, in images per second.
    pub const MAX_PER_SECOND: u32 = 1000;

    /// 60 images per second.
    pub const PER_SECOND_60: Self = Self {
        numerator: 60,
        denominator: 1,
    };

    /// Creates an image rate of `numerator / denominator` images per second.
    ///
    /// # Errors
    ///
    /// Returns [`VisualError::InvalidImageRate`] unless the fraction is reduced, its
    /// denominator is nonzero, and the rate is from 1 to [`ImageRate::MAX_PER_SECOND`]
    /// images per second.
    pub fn new(numerator: u32, denominator: u32) -> Result<Self, VisualError> {
        let rejected = VisualError::InvalidImageRate {
            numerator,
            denominator,
        };
        let top = u64::from(numerator);
        let bottom = u64::from(denominator);
        let per_second_max = bottom.saturating_mul(u64::from(Self::MAX_PER_SECOND));
        if denominator == 0 || top < bottom || top > per_second_max || gcd(top, bottom) != 1 {
            Err(rejected)
        } else {
            Ok(Self {
                numerator,
                denominator,
            })
        }
    }

    /// Images per [`ImageRate::denominator`] seconds.
    #[must_use]
    pub const fn numerator(self) -> u32 {
        self.numerator
    }

    /// The seconds [`ImageRate::numerator`] images last.
    #[must_use]
    pub const fn denominator(self) -> u32 {
        self.denominator
    }
}

/// A duration: `numerator / denominator` seconds, an exact, reduced, nonzero fraction, so
/// that one duration has one representation (a cycle of 1001/1000 s is exact).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Seconds {
    /// Seconds, times `denominator`.
    numerator: u32,
    /// Divides `numerator`.
    denominator: u32,
}

impl Seconds {
    /// Creates a duration of `numerator / denominator` seconds.
    ///
    /// # Errors
    ///
    /// Returns [`VisualError::InvalidSeconds`] unless both terms are nonzero and the
    /// fraction is reduced.
    pub fn new(numerator: u32, denominator: u32) -> Result<Self, VisualError> {
        if numerator == 0
            || denominator == 0
            || gcd(u64::from(numerator), u64::from(denominator)) != 1
        {
            Err(VisualError::InvalidSeconds {
                numerator,
                denominator,
            })
        } else {
            Ok(Self {
                numerator,
                denominator,
            })
        }
    }

    /// Seconds, times [`Seconds::denominator`].
    #[must_use]
    pub const fn numerator(self) -> u32 {
        self.numerator
    }

    /// Divides [`Seconds::numerator`].
    #[must_use]
    pub const fn denominator(self) -> u32 {
        self.denominator
    }
}

/// The index of an image in a sequence, from 0. Image `n` is shown at the exact instant
/// `n / rate` seconds: `n × denominator / numerator`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ImageIndex(pub u64);

#[cfg(test)]
mod tests {
    use super::{ImageRate, Seconds, gcd};
    use crate::VisualError;

    #[test]
    fn computes_the_greatest_common_divisor() {
        for (first, second, divisor) in [
            (12, 18, 6),
            (18, 12, 6),
            (7, 0, 7),
            (0, 7, 7),
            (24_000, 1001, 1),
        ] {
            assert_eq!(gcd(first, second), divisor, "gcd({first}, {second})");
        }
    }

    #[test]
    fn accepts_reduced_rates_within_range() {
        for (numerator, denominator) in [(1, 1), (24_000, 1001), (30_000, 1001), (1000, 1), (25, 1)]
        {
            let rate = ImageRate::new(numerator, denominator);
            assert_eq!(
                rate.map(|accepted| (accepted.numerator(), accepted.denominator())),
                Ok((numerator, denominator)),
                "{numerator}/{denominator}"
            );
        }
        assert_eq!(ImageRate::new(60, 1), Ok(ImageRate::PER_SECOND_60), "60");
    }

    #[test]
    fn rejects_unreduced_out_of_range_or_undefined_rates() {
        for (numerator, denominator) in [
            (120, 2),
            (0, 1),
            (1, 2),
            (999, 1000),
            (1001, 1),
            (2001, 2),
            (5, 0),
            (0, 0),
            (u32::MAX, 1),
        ] {
            assert_eq!(
                ImageRate::new(numerator, denominator),
                Err(VisualError::InvalidImageRate {
                    numerator,
                    denominator
                }),
                "{numerator}/{denominator}"
            );
        }
    }

    #[test]
    fn accepts_the_edges_of_the_range() {
        assert!(ImageRate::new(1000, 1).is_ok(), "1000 per second");
        assert!(ImageRate::new(1999, 2).is_ok(), "just under 1000");
        assert!(
            ImageRate::new(1001, 1001).is_err(),
            "1 per second, unreduced"
        );
        assert!(ImageRate::new(1002, 1001).is_ok(), "just over 1 per second");
        assert!(
            ImageRate::new(u32::MAX, u32::MAX - 1).is_ok(),
            "largest terms, just over 1 per second"
        );
    }

    #[test]
    fn accepts_only_reduced_nonzero_durations() {
        for (numerator, denominator) in [(1, 1), (1001, 1000), (u32::MAX, 1), (1, u32::MAX)] {
            let seconds = Seconds::new(numerator, denominator);
            assert_eq!(
                seconds.map(|accepted| (accepted.numerator(), accepted.denominator())),
                Ok((numerator, denominator)),
                "{numerator}/{denominator}"
            );
        }
        for (numerator, denominator) in [(0, 1), (1, 0), (0, 0), (2, 4), (6, 3)] {
            assert_eq!(
                Seconds::new(numerator, denominator),
                Err(VisualError::InvalidSeconds {
                    numerator,
                    denominator
                }),
                "{numerator}/{denominator}"
            );
        }
    }
}
