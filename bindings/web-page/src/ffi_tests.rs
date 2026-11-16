//! Tests of the page's C interface, called as JavaScript calls it.

use super::{PAGE_ABI_VERSION, REFUSED, page_abi_version, page_image_at};

#[test]
fn names_its_interface_and_its_version() {
    assert_eq!(page_abi_version(), PAGE_ABI_VERSION);
    assert_eq!(PAGE_ABI_VERSION >> 16_u32, 0x5050, "PP");
    assert_eq!(PAGE_ABI_VERSION & 0xffff, 1_u32, "version 1");
}

#[test]
fn answers_the_image_of_a_frame_as_the_coordinating_layer_does() {
    // 48 000 Hz at 60 images a second: 800 frames an image.
    assert_eq!(page_image_at(799, 48_000, 60, 1), 0);
    assert_eq!(page_image_at(800, 48_000, 60, 1), 1);
    // The last frame, at the fastest images and the slowest sample rate.
    assert_eq!(page_image_at(u64::MAX, 8000, 1000, 1), u64::MAX / 8);
}

#[test]
fn refuses_a_sample_rate_or_an_image_rate_it_does_not_hold() {
    for (sample_rate, numerator, denominator) in [
        (7999, 60, 1),
        (384_001, 60, 1),
        (48_000, 0, 1),
        (48_000, 60, 0),
        (48_000, 2, 4),
        (48_000, 1001, 1),
        (0, 0, 0),
    ] {
        assert_eq!(
            page_image_at(0, sample_rate, numerator, denominator),
            REFUSED,
            "{sample_rate} Hz, {numerator}/{denominator}"
        );
    }
}
