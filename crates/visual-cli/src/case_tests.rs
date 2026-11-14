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
        (&replace("[1001, 1000]", "[0, 1]"), "invalid duration 0/1 s: a reduced, nonzero fraction is required".to_owned()),
        (&replace("\"side\": 40000", "\"side\": 70000"), "invalid square side 70000: at most 65536 (the image height)".to_owned()),
    ] {
        assert_eq!(parse_case(text), Err(reason.clone()), "{reason}");
    }
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
