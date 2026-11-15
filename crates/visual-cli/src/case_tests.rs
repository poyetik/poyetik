use visual_boxes::Square;
use visual_core::{ImageIndex, ImageRate, ImageSize, Seconds, render_sequence};
use visual_hash::ImageHasher;

use super::{Case, MAX_CASE_BYTES, MAX_IMAGES, SquareCase, parse_case, render_case};

/// A small valid case.
const SMALL: &str = r#"{"rate": [24000, 1001], "width": 17, "height": 9, "first": 3, "images": 5,
  "square": {"side": 40000, "cycle": [1001, 1000]}}"#;

#[test]
fn reads_a_valid_case() {
    assert_eq!(
        parse_case(SMALL),
        Ok(Case {
            rate: [24_000, 1001],
            width: 17,
            height: 9,
            first: 3,
            images: 5,
            square: SquareCase {
                side: 40_000,
                cycle: [1001, 1000]
            }
        }),
        "every field"
    );
}

#[test]
fn renders_the_hash_of_its_images() {
    // The same images hashed directly with the library give the same hash.
    let case = parse_case(SMALL).expect("a valid case");
    let rate = ImageRate::new(24_000, 1001).expect("a valid rate");
    let size = ImageSize::new(17, 9).expect("a valid size");
    let cycle = Seconds::new(1001, 1000).expect("a valid cycle");
    let mut square = Square::new(rate, 40_000, cycle).expect("a valid square");
    let mut image = visual_core::Image::new(size);
    let mut hasher = ImageHasher::new(rate, size, image.encoding());
    render_sequence(&mut square, ImageIndex(3), 5, &mut image, |_, rendered| {
        hasher.add(rendered).expect("same size");
    });
    assert_eq!(render_case(&case), Ok(hasher.finish()), "same hash");
}

#[test]
fn refuses_invalid_cases_with_a_reason() {
    let replace = |from: &str, to: &str| SMALL.replace(from, to);
    for (text, reason) in [
        ("{", "EOF while parsing an object at line 1 column 1".to_owned()),
        (&replace("\"images\": 5", "\"images\": 0"), format!("images must be from 1 to {MAX_IMAGES}, not 0")),
        (&replace("\"images\": 5", "\"images\": 10001"), format!("images must be from 1 to {MAX_IMAGES}, not 10001")),
        (&replace("\"first\": 3", "\"first\": 18446744073709551615"), "the last image is past the largest index: first 18446744073709551615 and 5 images".to_owned()),
        (&replace("[24000, 1001]", "[120, 2]"), "invalid image rate 120/2: a reduced fraction from 1 to 1000 images per second is required".to_owned()),
        (&replace("\"width\": 17", "\"width\": 0"), "invalid image size 0x9: each side must be from 1 to 16384 pixels".to_owned()),
        (&replace("\"width\": 17, \"height\": 9", "\"width\": 16384, \"height\": 2049"), "invalid image size 16384x2049: an image holds at most 33554432 pixels".to_owned()),
        (&replace("[1001, 1000]", "[0, 1]"), "invalid duration 0/1 s: a reduced, nonzero fraction is required".to_owned()),
        (&replace("\"side\": 40000", "\"side\": 70000"), "invalid square side 70000: at most 65536 (the image height)".to_owned()),
    ] {
        assert_eq!(parse_case(text), Err(reason.clone()), "{reason}");
    }
}

#[test]
fn reads_a_case_of_sixty_four_kibibytes_and_no_more() {
    // The limit as documented, written out: a wrong constant fails here, not only past
    // itself.
    let padded = format!("{SMALL}{}", " ".repeat(65_536_usize - SMALL.len()));
    assert_eq!(padded.len(), 65_536_usize, "exactly 64 KiB");
    assert_eq!(parse_case(&padded), parse_case(SMALL), "read whole");
    let over = format!("{padded} ");
    assert_eq!(
        parse_case(&over),
        Err(format!(
            "a case file holds at most {} bytes, this one {}",
            65_536_usize, 65_537_usize
        )),
        "one byte more"
    );
}

#[test]
fn refuses_unknown_fields_and_large_files() {
    let unknown = SMALL.replace("\"first\"", "\"extra\": 1, \"first\"");
    assert!(
        parse_case(&unknown).is_err_and(|reason| reason.starts_with("unknown field `extra`")),
        "unknown field"
    );
    let large = format!("{SMALL}{}", " ".repeat(MAX_CASE_BYTES));
    assert_eq!(
        parse_case(&large),
        Err(format!(
            "a case file holds at most {MAX_CASE_BYTES} bytes, this one {}",
            large.len()
        )),
        "too large"
    );
    let invalid = Case {
        images: 0,
        ..parse_case(SMALL).expect("a valid case")
    };
    assert!(
        render_case(&invalid).is_err(),
        "an invalid case does not render"
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
        assert!(
            parse_case(&at(first, images)).is_ok(),
            "{images} images from {first}: the last is {}",
            first + (images - 1)
        );
    }
    for (first, images) in [(u64::MAX, 2), (u64::MAX - 3, 5)] {
        assert_eq!(
            parse_case(&at(first, images)),
            Err(format!(
                "the last image is past the largest index: first {first} and {images} images"
            )),
            "{images} images from {first}"
        );
    }
}
