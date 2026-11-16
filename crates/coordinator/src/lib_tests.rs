//! Tests of the alignment of images to the audio clock.

extern crate alloc;

use alloc::format;
use alloc::string::ToString as _;

use audio_battery::matrix::SAMPLE_RATES;
use audio_core::SampleRate;
use visual_core::{ImageIndex, ImageRate};
use visual_testkit::generate::{rate_terms, u64_edges};
use visual_testkit::matrix::image_rates;
use visual_testkit::proptest::prelude::Strategy as _;
use visual_testkit::proptest::sample::select;
use visual_testkit::proptest::test_runner::TestCaseError;
use visual_testkit::runner::runner;

use super::image_at;

/// A sample rate of the matrix.
fn hz(rate: u32) -> SampleRate {
    SampleRate::new(rate).expect("a supported sample rate")
}

/// Whether `image` is the last image of `rate` not after `frame` at `sample_rate`, by the
/// inequality that defines it rather than by a division: its instant is not after the
/// frame's, and the next image's is.
fn shown(image: ImageIndex, frame: u64, sample_rate: u32, rate: ImageRate) -> bool {
    // Each product below 2^64 × 2^32 × 2^19: exact in 128 bits.
    let frame_time = u128::from(frame).saturating_mul(u128::from(rate.numerator()));
    let per_image = u128::from(rate.denominator()).saturating_mul(u128::from(sample_rate));
    let start = u128::from(image.0).saturating_mul(per_image);
    start <= frame_time && frame_time < start.saturating_add(per_image)
}

#[test]
fn shows_each_image_from_its_first_frame_to_the_frame_before_the_next() {
    let sixty = ImageRate::new(60, 1).expect("a valid rate");
    // 48 000 Hz at 60 images a second: 800 frames an image.
    for (frame, image) in [
        (0, 0),
        (799, 0),
        (800, 1),
        (1599, 1),
        (1600, 2),
        (48_000, 60),
    ] {
        assert_eq!(
            image_at(frame, hz(48_000), sixty),
            ImageIndex(image),
            "{frame}"
        );
    }
    // 44 100 Hz at 30 000/1001 images a second: image 1 at 1001/30 000 s, frame 1471.47.
    let ntsc = ImageRate::new(30_000, 1001).expect("a valid rate");
    assert_eq!(image_at(1471, hz(44_100), ntsc), ImageIndex(0));
    assert_eq!(image_at(1472, hz(44_100), ntsc), ImageIndex(1));
    // The last frame of the clock, at the fastest images and the slowest sample rate.
    let fastest = ImageRate::new(1000, 1).expect("a valid rate");
    assert_eq!(
        image_at(u64::MAX, hz(SampleRate::MIN), fastest),
        ImageIndex(u64::MAX / 8),
        "no overflow at the extremes"
    );
}

#[test]
fn shows_the_image_of_the_instant_at_every_rate_of_the_matrices() {
    // Every sample rate of the audio matrix, every image rate of the visual one, frames
    // from the edges of 64 bits: the image the definition gives.
    let cases = (
        select(SAMPLE_RATES.to_vec()),
        select(image_rates()),
        u64_edges(),
    );
    let verdict = runner(512).run(&cases, |(sample_rate, rate, frame)| {
        let image = image_at(frame, hz(sample_rate), rate);
        if shown(image, frame, sample_rate, rate) {
            Ok(())
        } else {
            Err(TestCaseError::fail(format!(
                "{frame} at {sample_rate} Hz, {rate:?}: image {}",
                image.0
            )))
        }
    });
    assert_eq!(verdict.map_err(|error| error.to_string()), Ok(()));
}

#[test]
fn shows_the_image_of_the_instant_at_any_rate() {
    let cases = (
        (SampleRate::MIN..=SampleRate::MAX),
        rate_terms().prop_filter_map("a valid rate", |(numerator, denominator)| {
            ImageRate::new(numerator, denominator).ok()
        }),
        u64_edges(),
    );
    let verdict = runner(1024).run(&cases, |(sample_rate, rate, frame)| {
        let image = image_at(frame, hz(sample_rate), rate);
        if shown(image, frame, sample_rate, rate) {
            Ok(())
        } else {
            Err(TestCaseError::fail(format!(
                "{frame} at {sample_rate} Hz, {rate:?}: image {}",
                image.0
            )))
        }
    });
    assert_eq!(verdict.map_err(|error| error.to_string()), Ok(()));
}
