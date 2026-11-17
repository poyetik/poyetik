//! The global allocator of this crate's test binary, and the tests of its library's own
//! functions.

use super::CountingAllocator;

/// The test binary of this crate counts heap operations, like every box test binary must.
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[test]
fn counts_the_frames_of_a_duration_to_the_nearest_and_at_least_one() {
    let at = |hz| audio_core::SampleRate::new(hz).expect("a rate");
    for (hz, milliseconds, frames) in [
        (48_000, 1.5_f64, 72),
        (48_000, 5.0_f64, 240),
        (48_000, 0.01_f64, 1),
        // Half a frame: the next one.
        (48_000, 1.0_f64 / 96.0_f64, 1),
        (48_000, 1.5_f64 / 48.0_f64, 2),
        (44_100, 5.0_f64, 221),
        (44_100, 1.5_f64, 66),
        (8_000, 0.5_f64, 4),
        (384_000, 5.0_f64, 1_920),
        // None: at least one.
        (48_000, 0.0_f64, 1),
    ] {
        assert_eq!(
            super::frames_of_duration(milliseconds, at(hz)),
            frames,
            "{milliseconds} ms at {hz} Hz"
        );
    }
}

#[test]
fn counts_each_detectors_delay_apart() {
    use audio_core::Detector;
    assert_eq!(
        Detector::ALL.map(super::detector_delay),
        [6, 16, 64],
        "standard, precise, mastering"
    );
}
