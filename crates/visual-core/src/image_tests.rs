//! Tests of `image.rs`.

extern crate alloc;

use alloc::format;
use alloc::string::ToString as _;

use super::{ColorEncoding, Image, ImageSize};
use crate::VisualError;

#[test]
fn accepts_sizes_within_range() {
    for (width, height) in [
        (1, 1),
        (1920, 1080),
        (16_384, 1),
        (1, 16_384),
        (16_384, 2048),
        (2048, 16_384),
        (7680, 4320),
        (5792, 5792),
    ] {
        let size = ImageSize::new(width, height).expect("a valid size");
        assert_eq!(
            (size.width(), size.height()),
            (width, height),
            "{width}x{height}"
        );
        assert_eq!(
            size.pixels(),
            u64::from(width) * u64::from(height),
            "{width}x{height}"
        );
    }
}

#[test]
fn rejects_sizes_outside_range() {
    for (width, height) in [
        (0, 1),
        (1, 0),
        (16_385, 1),
        (1, 16_385),
        (u32::MAX, u32::MAX),
    ] {
        assert_eq!(
            ImageSize::new(width, height),
            Err(VisualError::InvalidImageSize { width, height }),
            "{width}x{height}"
        );
    }
}

#[test]
fn rejects_images_of_more_pixels_than_every_target_holds() {
    assert_eq!(ImageSize::MAX_PIXELS, 33_554_432, "2^25 pixels");
    for (width, height) in [
        (16_384, 2049),
        (2049, 16_384),
        (5793, 5793),
        (8192, 8192),
        (16_384, 16_384),
    ] {
        assert_eq!(
            ImageSize::new(width, height),
            Err(VisualError::ImageTooLarge { width, height }),
            "{width}x{height}"
        );
    }
}

#[test]
fn creates_the_largest_image() {
    for (width, height) in [(16_384, 2048), (2048, 16_384)] {
        let size = ImageSize::new(width, height).expect("the largest area");
        assert_eq!(size.pixels(), ImageSize::MAX_PIXELS, "{width}x{height}");
        let image = Image::new(size);
        assert_eq!(
            image.values().len(),
            usize::try_from(ImageSize::MAX_PIXELS).expect("fits") * Image::CHANNELS,
            "{width}x{height}: every value allocated"
        );
    }
}

#[test]
fn starts_transparent_black() {
    let size = ImageSize::new(3, 2).expect("a valid size");
    let image = Image::new(size);
    assert_eq!(image.size(), size, "its size");
    assert_eq!(
        image.encoding(),
        ColorEncoding::LinearRec709Rgba32F,
        "encoding"
    );
    assert_eq!(image.values().len(), 24, "4 values per pixel");
    assert!(
        image.values().iter().all(|value| value.to_bits() == 0),
        "every value is +0"
    );
}

#[test]
fn gives_its_rows_from_the_top() {
    let mut image = Image::new(ImageSize::new(3, 2).expect("a valid size"));
    let mut rows = 0_u8;
    for (index, row) in image.rows_mut().enumerate() {
        assert_eq!(row.len(), 12, "3 pixels of 4 values");
        row.fill(f32::from(u8::try_from(index).expect("small")) + 1.0);
        rows += 1;
    }
    assert_eq!(rows, 2, "two rows");
    let (first, second) = image.values().split_at(12);
    assert!(
        first
            .iter()
            .all(|value| value.to_bits() == 1.0_f32.to_bits()),
        "top row first"
    );
    assert!(
        second
            .iter()
            .all(|value| value.to_bits() == 2.0_f32.to_bits()),
        "then the next"
    );
}

#[test]
fn accepts_exactly_the_sizes_its_rules_allow() {
    // Against the rule written apart, at random sides, edges and the limits often.
    use visual_testkit::generate::u32_edges;
    use visual_testkit::proptest::prelude::{Just, prop_oneof};
    use visual_testkit::proptest::test_runner::TestCaseError;
    use visual_testkit::runner::runner;
    let sides = || {
        prop_oneof![
            u32_edges(),
            Just(16_384_u32),
            Just(16_385_u32),
            Just(2048_u32),
            Just(2049_u32),
            1_u32..=16_385
        ]
    };
    let verdict = runner(4096).run(&(sides(), sides()), |(width, height)| {
        let expected = if !(1..=16_384).contains(&width) || !(1..=16_384).contains(&height) {
            Err(VisualError::InvalidImageSize { width, height })
        } else if u64::from(width) * u64::from(height) > 1 << 25_u32 {
            Err(VisualError::ImageTooLarge { width, height })
        } else {
            Ok((width, height))
        };
        let made = ImageSize::new(width, height).map(|size| (size.width(), size.height()));
        if made == expected {
            Ok(())
        } else {
            Err(TestCaseError::fail(format!("{width}x{height}: {made:?}")))
        }
    });
    assert_eq!(verdict.map_err(|error| error.to_string()), Ok(()), "sizes");
}
