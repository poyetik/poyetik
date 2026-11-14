//! Tests of the view: setting a square up, rendering it and hashing its images.

#![forbid(unsafe_code)]

use visual_boxes::Square;
use visual_core::{Image, ImageIndex, ImageRate, ImageSize, Seconds, VisualBox};
use visual_hash::{ImageHasher, hash_sequence};
use visual_testkit::heap_operations_during;

use super::{View, image_index};

/// The square every test sets up: 7 × 5 images at 24000/1001 images per second, a side
/// of 0.6 of the height, a sweep every 1.001 s.
fn set_up(view: &mut View) -> bool {
    view.setup(7, 5, (24_000, 1001), 39_322, (1001, 1000))
}

/// The same square built directly, and an image of its size.
fn direct() -> (Square, Image, ImageRate) {
    let rate = ImageRate::new(24_000, 1001).expect("a valid rate");
    let cycle = Seconds::new(1001, 1000).expect("a valid cycle");
    let square = Square::new(rate, 39_322, cycle).expect("a valid square");
    (
        square,
        Image::new(ImageSize::new(7, 5).expect("a valid size")),
        rate,
    )
}

#[test]
fn joins_the_two_halves_of_an_image_index() {
    for (low, high, index) in [
        (0, 0, 0),
        (1, 0, 1),
        (0, 1, 1 << 32_u32),
        (0x89AB_CDEF, 0x0123_4567, 0x0123_4567_89AB_CDEF),
        (u32::MAX, 0, u64::from(u32::MAX)),
        (0, u32::MAX, u64::MAX - u64::from(u32::MAX)),
        (u32::MAX, u32::MAX, u64::MAX),
    ] {
        assert_eq!(image_index(low, high), index, "{low:#x} and {high:#x}");
    }
}

#[test]
fn shows_nothing_until_a_square_is_set_up() {
    let mut view = View::default();
    assert_eq!(view.render_len(), 0, "no values");
    assert!(view.render(3).is_empty(), "no image");
    assert_eq!(view.hash(0, 2), &[0; 32], "no hash");
}

#[test]
fn renders_the_square_it_was_given() {
    let mut view = View::new();
    assert!(set_up(&mut view), "valid values");
    assert_eq!(view.render_len(), 7 * 5 * 4, "four values per pixel");
    let (mut square, mut image, _) = direct();
    for index in [0, 1, 23, u64::from(u32::MAX) + 5, u64::MAX] {
        square.render(ImageIndex(index), &mut image);
        let expected: Vec<u32> = image.values().iter().map(|value| value.to_bits()).collect();
        let shown: Vec<u32> = view
            .render(index)
            .iter()
            .map(|value| value.to_bits())
            .collect();
        assert_eq!(shown, expected, "image {index}, bit for bit");
    }
}

#[test]
fn hashes_the_images_it_renders() {
    let mut view = View::new();
    assert!(set_up(&mut view), "valid values");
    let (mut square, mut image, rate) = direct();
    for (first, count) in [(0, 0), (0, 1), (24, 30), (u64::from(u32::MAX), 3)] {
        let expected = hash_sequence(&mut square, rate, ImageIndex(first), count, &mut image);
        assert_eq!(
            view.hash(first, count),
            &expected.bytes(),
            "{count} images from {first}"
        );
    }
    let mut one = ImageHasher::new(rate, image.size(), image.encoding());
    square.render(ImageIndex(9), &mut image);
    one.add(&image).expect("same size");
    assert_eq!(
        view.hash(9, 1),
        &one.finish().bytes(),
        "as adding the image"
    );
}

#[test]
fn refuses_each_invalid_value_and_keeps_its_square() {
    let mut view = View::new();
    assert!(set_up(&mut view), "valid values");
    let before: Vec<u32> = view.render(4).iter().map(|value| value.to_bits()).collect();
    for (width, height, rate, side, cycle, what) in [
        (0, 5, (24_000, 1001), 39_322, (1001, 1000), "a zero width"),
        (7, 0, (24_000, 1001), 39_322, (1001, 1000), "a zero height"),
        (7, 5, (0, 1), 39_322, (1001, 1000), "a zero rate"),
        (
            7,
            5,
            (24_000, 0),
            39_322,
            (1001, 1000),
            "a zero rate denominator",
        ),
        (
            7,
            5,
            (24_000, 1001),
            65_537,
            (1001, 1000),
            "a side past the height",
        ),
        (7, 5, (24_000, 1001), 39_322, (0, 1), "a zero cycle"),
        (
            7,
            5,
            (24_000, 1001),
            39_322,
            (1, 0),
            "a zero cycle denominator",
        ),
    ] {
        assert!(!view.setup(width, height, rate, side, cycle), "{what}");
        let after: Vec<u32> = view.render(4).iter().map(|value| value.to_bits()).collect();
        assert_eq!(after, before, "{what}: the square kept");
    }
    assert!(view.setup(3, 2, (60, 1), 65_536, (1, 2)), "another square");
    assert_eq!(view.render_len(), 3 * 2 * 4, "replaced");
}

#[test]
fn renders_and_hashes_without_the_heap() {
    let mut view = View::new();
    assert!(set_up(&mut view), "valid values");
    let ((), rendering) = heap_operations_during(|| {
        let _: usize = view.render(77).len();
    })
    .expect("the counting allocator is installed");
    assert_eq!(rendering, 0, "rendering an image");
    let ((), hashing) = heap_operations_during(|| {
        let _: u8 = view.hash(5, 4)[0];
    })
    .expect("the counting allocator is installed");
    assert_eq!(hashing, 0, "hashing a sequence");
}
