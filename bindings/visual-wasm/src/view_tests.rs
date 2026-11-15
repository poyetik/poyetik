//! Tests of the view: setting a square up, rendering it and hashing its images.

#![forbid(unsafe_code)]

use visual_boxes::{BoxError, Square};
use visual_core::{
    Image, ImageIndex, ImageRange, ImageRate, ImageSize, Seconds, VisualBox, VisualError,
};
use visual_hash::{ImageHasher, hash_sequence};
use visual_testkit::heap_operations_during;

use super::{SetupError, View, image_index};

/// The square every test sets up: 7 × 5 images at 24000/1001 images per second, a side
/// of 0.6 of the height, a sweep every 1.001 s.
fn set_up(view: &mut View) -> bool {
    view.setup(7, 5, (24_000, 1001), 39_322, (1001, 1000))
        .is_ok()
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
    assert_eq!(view.hash(0, 2), None, "no hash, not a placeholder");
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
        let range = ImageRange::new(ImageIndex(first), count).expect("a valid range");
        let expected = hash_sequence(&mut square, rate, range, &mut image);
        assert_eq!(
            view.hash(first, count),
            Some(&expected.bytes()),
            "{count} images from {first}"
        );
    }
    let mut one = ImageHasher::new(rate, image.size(), image.encoding());
    square.render(ImageIndex(9), &mut image);
    one.add(&image).expect("same size");
    assert_eq!(
        view.hash(9, 1),
        Some(&one.finish().bytes()),
        "as adding the image"
    );
}

/// The largest image of the matrix the view is checked at: every image natively (none holds
/// more than `ImageSize::MAX_PIXELS`, 2²⁵), those of at most 64 pixels under Miri (the others
/// cost minutes there, and their arithmetic is the library's, run natively).
const MATRIX_PIXELS: u64 = if cfg!(miri) { 64 } else { 1 << 25_u32 };

#[test]
fn renders_and_hashes_across_the_visual_matrix() {
    // Every rate, size and index of the shared matrix, through the view as through the
    // library (the sizes of at most `MATRIX_PIXELS`).
    let cycle = Seconds::new(1001, 1000).expect("a valid cycle");
    let mut view = View::new();
    for rate in visual_testkit::matrix::image_rates() {
        let sizes = visual_testkit::matrix::image_sizes();
        for size in sizes
            .into_iter()
            .filter(|size| size.pixels() <= MATRIX_PIXELS)
        {
            assert!(
                view.setup(
                    size.width(),
                    size.height(),
                    (rate.numerator(), rate.denominator()),
                    39_322,
                    (1001, 1000)
                )
                .is_ok(),
                "{rate:?} {size:?}"
            );
            let mut square = Square::new(rate, 39_322, cycle).expect("a valid square");
            let mut image = Image::new(size);
            for index in visual_testkit::matrix::image_indices() {
                square.render(index, &mut image);
                let expected: Vec<u32> =
                    image.values().iter().map(|value| value.to_bits()).collect();
                let shown: Vec<u32> = view
                    .render(index.0)
                    .iter()
                    .map(|value| value.to_bits())
                    .collect();
                assert_eq!(shown, expected, "{rate:?} {size:?} image {index:?}");
                let range = ImageRange::new(index, 1).expect("one image");
                assert_eq!(
                    view.hash(index.0, 1),
                    Some(&hash_sequence(&mut square, rate, range, &mut image).bytes()),
                    "{rate:?} {size:?} image {index:?}"
                );
            }
        }
    }
}

#[test]
fn refuses_images_past_the_largest_index() {
    let mut view = View::new();
    assert!(set_up(&mut view), "valid values");
    for (first, count) in [(u64::MAX, 2), (u64::MAX - 3, 5)] {
        assert_eq!(view.hash(first, count), None, "{count} images from {first}");
    }
    let (mut square, mut image, rate) = direct();
    for (first, count) in [(u64::MAX, 1), (u64::MAX - 4, 5)] {
        let range = ImageRange::new(ImageIndex(first), count).expect("a valid range");
        let expected = hash_sequence(&mut square, rate, range, &mut image);
        assert_eq!(
            view.hash(first, count),
            Some(&expected.bytes()),
            "{count} images from {first}: up to the last"
        );
    }
}

#[test]
fn refuses_each_invalid_value_with_its_reason_and_keeps_its_square() {
    let mut view = View::new();
    assert_eq!(view.message(), "", "no message before a setup");
    assert!(set_up(&mut view), "valid values");
    let before: Vec<u32> = view.render(4).iter().map(|value| value.to_bits()).collect();
    let size = |width, height| SetupError::Visual(VisualError::InvalidImageSize { width, height });
    let rate = |numerator, denominator| {
        SetupError::Visual(VisualError::InvalidImageRate {
            numerator,
            denominator,
        })
    };
    let cycle = |numerator, denominator| {
        SetupError::Visual(VisualError::InvalidSeconds {
            numerator,
            denominator,
        })
    };
    for (width, height, images, side, sweep, refused) in [
        (0, 5, (24_000, 1001), 39_322, (1001, 1000), size(0, 5)),
        (7, 0, (24_000, 1001), 39_322, (1001, 1000), size(7, 0)),
        (
            16_384,
            2049,
            (24_000, 1001),
            39_322,
            (1001, 1000),
            SetupError::Visual(VisualError::ImageTooLarge {
                width: 16_384,
                height: 2049,
            }),
        ),
        (7, 5, (0, 1), 39_322, (1001, 1000), rate(0, 1)),
        (7, 5, (24_000, 0), 39_322, (1001, 1000), rate(24_000, 0)),
        (
            7,
            5,
            (24_000, 1001),
            65_537,
            (1001, 1000),
            SetupError::Square(BoxError::InvalidSide(65_537)),
        ),
        (7, 5, (24_000, 1001), 39_322, (0, 1), cycle(0, 1)),
        (7, 5, (24_000, 1001), 39_322, (1, 0), cycle(1, 0)),
    ] {
        assert_eq!(
            view.setup(width, height, images, side, sweep),
            Err(refused),
            "{refused}"
        );
        assert_eq!(
            view.message(),
            refused.to_string(),
            "{refused}: its message"
        );
        let after: Vec<u32> = view.render(4).iter().map(|value| value.to_bits()).collect();
        assert_eq!(after, before, "{refused}: the square kept");
    }
    assert_eq!(
        view.setup(3, 2, (60, 1), 65_536, (1, 2)),
        Ok(()),
        "another square"
    );
    assert_eq!(
        view.message(),
        "",
        "no message after a setup that succeeded"
    );
    assert_eq!(view.render_len(), 3 * 2 * 4, "replaced");
}

#[test]
fn says_its_errors_are_those_of_the_value_refused() {
    let visual = VisualError::InvalidImageSize {
        width: 0,
        height: 1,
    };
    let square = BoxError::InvalidSide(65_537);
    for (error, source) in [
        (SetupError::from(visual), visual.to_string()),
        (SetupError::from(square), square.to_string()),
    ] {
        assert_eq!(
            error.to_string(),
            source,
            "the message of the value refused"
        );
        assert_eq!(
            core::error::Error::source(&error).map(ToString::to_string),
            Some(source),
            "its source"
        );
    }
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
        let _: bool = view.hash(5, 4).is_some();
    })
    .expect("the counting allocator is installed");
    assert_eq!(hashing, 0, "hashing a sequence");
}
