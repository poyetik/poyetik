//! Tests of the supported sample rates.

use super::SampleRate;
use crate::CoreError;

#[test]
fn accepts_the_supported_range() {
    for hz in [8_000, 44_100, 48_000, 384_000] {
        assert_eq!(
            SampleRate::new(hz).map(SampleRate::hz),
            Ok(hz),
            "{hz} Hz is supported"
        );
    }
}

#[test]
fn provides_48_khz() {
    assert_eq!(SampleRate::HZ_48000.hz(), 48_000, "48 kHz");
    assert_eq!(
        SampleRate::new(48_000),
        Ok(SampleRate::HZ_48000),
        "same value"
    );
}

#[test]
fn rejects_rates_outside_the_range() {
    for hz in [0, 7_999, 384_001, u32::MAX] {
        assert_eq!(
            SampleRate::new(hz),
            Err(CoreError::InvalidSampleRate(hz)),
            "{hz} Hz is rejected"
        );
    }
}
