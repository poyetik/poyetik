//! Sample rate.

use crate::CoreError;

/// A sample rate in hertz, guaranteed to be within the supported range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SampleRate(u32);

impl SampleRate {
    /// Lowest supported sample rate, in hertz.
    pub const MIN: u32 = 8_000;
    /// Highest supported sample rate, in hertz.
    pub const MAX: u32 = 384_000;

    /// 48 kHz, the usual rate of audio interfaces and browsers.
    pub const HZ_48000: Self = Self(48_000);

    /// Creates a sample rate.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::InvalidSampleRate`] when `hz` is outside
    /// [`SampleRate::MIN`]..=[`SampleRate::MAX`].
    pub fn new(hz: u32) -> Result<Self, CoreError> {
        if (Self::MIN..=Self::MAX).contains(&hz) {
            Ok(Self(hz))
        } else {
            Err(CoreError::InvalidSampleRate(hz))
        }
    }

    /// The sample rate in hertz.
    #[must_use]
    pub const fn hz(self) -> u32 {
        self.0
    }
}

#[cfg(test)]
#[path = "rate_tests.rs"]
mod tests;
