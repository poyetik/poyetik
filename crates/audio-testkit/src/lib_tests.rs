//! The global allocator of this crate's test binary, and the tests of its library's own
//! functions.

use audio_core::PortSpec;

use super::CountingAllocator;

/// The inlet of the boxes of the laws' tests, the box laws' and the driven laws'.
pub(crate) const IN: PortSpec = PortSpec {
    name: "in",
    description: "a signal",
};

/// The outlet of the boxes of the laws' tests.
pub(crate) const OUT: PortSpec = PortSpec {
    name: "out",
    description: "a signal",
};

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
#[should_panic(expected = "a duration of NaN ms at 48000 Hz")]
fn refuses_a_duration_that_is_not_a_number() {
    let _: usize = super::frames_of_duration(
        f64::NAN,
        audio_core::SampleRate::new(48_000).expect("a rate"),
    );
}

#[test]
#[should_panic(expected = "a duration of -1 ms at 48000 Hz")]
fn refuses_a_negative_duration() {
    let _: usize = super::frames_of_duration(
        -1.0_f64,
        audio_core::SampleRate::new(48_000).expect("a rate"),
    );
}

#[test]
#[should_panic(expected = "a duration of 100000000000 ms at 48000 Hz")]
fn refuses_a_duration_past_the_count() {
    // 4.8·10^12 frames, past the 2^32 - 1 a `u32` counts.
    let _: usize = super::frames_of_duration(
        1e11_f64,
        audio_core::SampleRate::new(48_000).expect("a rate"),
    );
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
