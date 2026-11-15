use core::error::Error as _;

use visual_boxes::{BoxError, Square};
use visual_core::{
    ImageIndex, ImageRange, ImageRate, ImageSize, Seconds, VisualError, render_sequence,
};
use visual_hash::ImageHasher;

use super::{CaseError, MAX_CASE_BYTES, MAX_IMAGES, MAX_RENDERED_PIXELS, hash_case, parse_case};

/// A small valid case.
const SMALL: &str = r#"{"rate": [24000, 1001], "width": 17, "height": 9, "first": 3, "images": 5,
  "square": {"side": 40000, "cycle": [1001, 1000]}}"#;

/// `SMALL` with `from` replaced by `to`, which must occur once.
fn replaced(from: &str, to: &str) -> String {
    assert_eq!(SMALL.matches(from).count(), 1, "{from} once");
    SMALL.replace(from, to)
}

#[test]
fn reads_every_value_of_a_valid_case() {
    let case = parse_case(SMALL).expect("a valid case");
    let rate = ImageRate::new(24_000, 1001).expect("a valid rate");
    assert_eq!(case.rate(), rate, "rate");
    assert_eq!(
        case.size(),
        ImageSize::new(17, 9).expect("a valid size"),
        "size"
    );
    assert_eq!(
        case.range(),
        ImageRange::new(ImageIndex(3), 5).expect("a valid range"),
        "images"
    );
    let cycle = Seconds::new(1001, 1000).expect("a valid cycle");
    assert_eq!(
        case.square(),
        Square::new(rate, 40_000, cycle).expect("a valid square"),
        "square"
    );
}

#[test]
fn renders_the_hash_of_its_images() {
    // The same images hashed directly with the library give the same hash.
    let rate = ImageRate::new(24_000, 1001).expect("a valid rate");
    let size = ImageSize::new(17, 9).expect("a valid size");
    let cycle = Seconds::new(1001, 1000).expect("a valid cycle");
    let mut square = Square::new(rate, 40_000, cycle).expect("a valid square");
    let mut image = visual_core::Image::new(size);
    let mut hasher = ImageHasher::new(rate, size, image.encoding());
    render_sequence(
        &mut square,
        ImageRange::new(ImageIndex(3), 5).expect("a valid range"),
        &mut image,
        |_, rendered| {
            hasher.add(rendered).expect("same size");
        },
    );
    let expected = hasher.finish();
    assert_eq!(
        parse_case(SMALL).map(|case| case.hash()),
        Ok(expected),
        "same hash"
    );
    assert_eq!(hash_case(SMALL), Ok(expected), "the same through hash_case");
}

#[test]
fn refuses_each_invalid_value_naming_its_field() {
    for (text, error, message) in [
        (replaced("\"images\": 5", "\"images\": 0"), CaseError::Images(0), "images: from 1 to 10000, not 0".to_owned()),
        (replaced("\"images\": 5", "\"images\": 10001"), CaseError::Images(10_001), "images: from 1 to 10000, not 10001".to_owned()),
        (
            replaced("\"first\": 3", "\"first\": 18446744073709551615"),
            CaseError::Range(VisualError::InvalidImageRange { first: u64::MAX, count: 5 }),
            "first and images: invalid image range: 5 images from image 18446744073709551615 pass the largest index, 18446744073709551615".to_owned(),
        ),
        (
            replaced("[24000, 1001]", "[120, 2]"),
            CaseError::Rate(VisualError::InvalidImageRate { numerator: 120, denominator: 2 }),
            "rate: invalid image rate 120/2: a reduced fraction from 1 to 1000 images per second is required".to_owned(),
        ),
        (
            replaced("\"width\": 17", "\"width\": 0"),
            CaseError::Size(VisualError::InvalidImageSize { width: 0, height: 9 }),
            "width and height: invalid image size 0x9: each side must be from 1 to 16384 pixels".to_owned(),
        ),
        (
            replaced("\"width\": 17, \"height\": 9", "\"width\": 16384, \"height\": 2049"),
            CaseError::Size(VisualError::ImageTooLarge { width: 16_384, height: 2049 }),
            "width and height: invalid image size 16384x2049: an image holds at most 33554432 pixels".to_owned(),
        ),
        (
            replaced("[1001, 1000]", "[0, 1]"),
            CaseError::Cycle(VisualError::InvalidSeconds { numerator: 0, denominator: 1 }),
            "square.cycle: invalid duration 0/1 s: a reduced, nonzero fraction is required".to_owned(),
        ),
        (
            replaced("\"side\": 40000", "\"side\": 70000"),
            CaseError::Side(BoxError::InvalidSide(70_000)),
            "square.side: invalid square side 70000: at most 65536, a square as tall as the image (a side is in 65536ths of the image height)".to_owned(),
        ),
        (
            replaced("\"width\": 17, \"height\": 9, \"first\": 3, \"images\": 5", "\"width\": 16384, \"height\": 2048, \"first\": 3, \"images\": 9"),
            CaseError::TooMuchWork(9_u64 << 25_u32),
            format!("images, width and height: a case renders at most {MAX_RENDERED_PIXELS} pixels in all, this one {}", 9_u64 << 25_u32),
        ),
    ] {
        assert_eq!(parse_case(&text).map(|_| ()), Err(error.clone()), "{message}");
        assert_eq!(error.to_string(), message, "{error:?}");
        assert!(error.source().is_none(), "{message}: the message says it all");
    }
}

#[test]
fn renders_as_many_pixels_as_its_limit_and_no_more() {
    assert_eq!(MAX_RENDERED_PIXELS, 268_435_456, "2^28");
    let images = |count: u64| {
        replaced(
            "\"width\": 17, \"height\": 9, \"first\": 3, \"images\": 5",
            &format!("\"width\": 16384, \"height\": 2048, \"first\": 3, \"images\": {count}"),
        )
    };
    assert!(
        parse_case(&images(8)).is_ok(),
        "8 of the largest images: 2^28 pixels"
    );
    assert_eq!(
        parse_case(&images(9)).map(|_| ()),
        Err(CaseError::TooMuchWork(9_u64 << 25_u32)),
        "one more"
    );
}

#[test]
fn refuses_what_is_not_a_case_object_with_the_reader_s_reason() {
    for (text, reason) in [
        (
            "{".to_owned(),
            "EOF while parsing an object at line 1 column 1",
        ),
        (
            "[]".to_owned(),
            "invalid type: sequence, expected a JSON object at line 1 column 0",
        ),
        (
            "[[24000, 1001], 17, 9, 3, 5, {\"side\": 40000, \"cycle\": [1001, 1000]}]".to_owned(),
            "invalid type: sequence, expected a JSON object at line 1 column 0",
        ),
        (
            replaced(
                "{\"side\": 40000, \"cycle\": [1001, 1000]}",
                "[40000, [1001, 1000]]",
            ),
            "invalid type: sequence, expected a JSON object at line 2 column 12",
        ),
        (
            replaced("\"first\": 3", "\"extra\": 1, \"first\": 3"),
            "unknown field `extra`, expected one of `rate`, `width`, `height`, `first`, `images`, `square` at line 1 column 57",
        ),
        (
            replaced("\"first\": 3", "\"first\": 3, \"first\": 4"),
            "duplicate field `first` at line 1 column 69",
        ),
        (
            replaced("\"side\": 40000", "\"side\": 40000, \"side\": 1"),
            "duplicate field `side` at line 2 column 34",
        ),
        (
            replaced("\"first\": 3", "\"first\": -0"),
            "invalid type: floating point `-0.0`, expected u64 at line 1 column 61",
        ),
        (
            replaced("\"height\": 9, ", ""),
            "missing field `height` at line 2 column 51",
        ),
        (
            format!("{SMALL} {{}}"),
            "trailing characters at line 2 column 53",
        ),
    ] {
        assert_eq!(
            parse_case(&text).map(|_| ()),
            Err(CaseError::Json(reason.to_owned())),
            "{text}"
        );
    }
}

#[test]
fn reads_a_case_of_sixty_four_kibibytes_and_no_more() {
    // The limit as documented, written out: a wrong constant fails here, not only past
    // itself.
    assert_eq!(MAX_CASE_BYTES, 65_536_usize, "64 KiB");
    let padded = format!("{SMALL}{}", " ".repeat(65_536_usize - SMALL.len()));
    assert_eq!(padded.len(), 65_536_usize, "exactly 64 KiB");
    assert_eq!(
        parse_case(&padded).map(|case| case.hash()),
        hash_case(SMALL),
        "read whole"
    );
    let over = format!("{padded} ");
    assert_eq!(
        parse_case(&over).map(|_| ()),
        Err(CaseError::TooLong(65_537)),
        "one byte more"
    );
    assert_eq!(
        CaseError::TooLong(65_537).to_string(),
        "a case file holds at most 65536 bytes, this one 65537",
        "its message"
    );
}

#[test]
fn accepts_a_sequence_ending_at_the_largest_index_only() {
    let at = |first: u64, images: u64| {
        SMALL
            .replace("\"first\": 3", &format!("\"first\": {first}"))
            .replace("\"images\": 5", &format!("\"images\": {images}"))
    };
    for (first, images) in [(u64::MAX, 1), (u64::MAX - 4, 5), (0, MAX_IMAGES)] {
        assert_eq!(
            parse_case(&at(first, images)).map(|case| case.range()),
            Ok(ImageRange::new(ImageIndex(first), images).expect("a valid range")),
            "{images} images from {first}"
        );
    }
    for (first, images) in [(u64::MAX, 2), (u64::MAX - 3, 5)] {
        assert_eq!(
            parse_case(&at(first, images)).map(|_| ()),
            Err(CaseError::Range(VisualError::InvalidImageRange {
                first,
                count: images
            })),
            "{images} images from {first}"
        );
    }
}
