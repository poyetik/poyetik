//! Exact time: the image rate, durations, the index of an image and runs of images.

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
    /// The seconds `numerator` images take.
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
    /// The divisor: the duration is `numerator / denominator` seconds.
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

    /// The divisor: the duration is [`Seconds::numerator`] / this, in seconds.
    #[must_use]
    pub const fn denominator(self) -> u32 {
        self.denominator
    }
}

/// The index of an image in a sequence, from 0. Image `n` is shown at the exact instant
/// `n / rate` seconds: `n × denominator / numerator`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ImageIndex(pub u64);

/// A run of consecutive images: `count` images from image `first`, the last one at most
/// image 2⁶⁴ − 1, so that every index of a range exists (none is repeated or wrapped).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImageRange {
    /// The first image.
    first: ImageIndex,
    /// How many images, possibly none.
    count: u64,
}

impl ImageRange {
    /// Creates the range of `count` images from image `first`.
    ///
    /// # Errors
    ///
    /// Returns [`VisualError::InvalidImageRange`] when its last image, `first + count − 1`,
    /// would be past image 2⁶⁴ − 1.
    pub fn new(first: ImageIndex, count: u64) -> Result<Self, VisualError> {
        // An empty range has no last image; any other ends at `first + count - 1`.
        let ends = count
            .checked_sub(1)
            .is_none_or(|rest| first.0.checked_add(rest).is_some());
        if ends {
            Ok(Self { first, count })
        } else {
            Err(VisualError::InvalidImageRange {
                first: first.0,
                count,
            })
        }
    }

    /// The first image.
    #[must_use]
    pub const fn first(self) -> ImageIndex {
        self.first
    }

    /// How many images.
    #[must_use]
    pub const fn count(self) -> u64 {
        self.count
    }

    /// Its images, in order, from the first.
    pub fn indices(self) -> impl Iterator<Item = ImageIndex> {
        // The last index exists by construction; an inclusive range ending at it never
        // computes one past it.
        let last = self
            .count
            .checked_sub(1)
            .and_then(|rest| self.first.0.checked_add(rest));
        last.into_iter()
            .flat_map(move |last| self.first.0..=last)
            .map(ImageIndex)
    }
}

#[cfg(test)]
#[path = "rate_tests.rs"]
mod tests;
