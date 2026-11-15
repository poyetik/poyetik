extern crate alloc;

use alloc::format;
use alloc::string::ToString;
use alloc::vec::Vec;

use serde_json::Value;
use sha2::{Digest, Sha256};
use visual_core::{ColorEncoding, Image, ImageIndex, ImageRange, ImageRate, ImageSize, VisualBox};

use super::{ImageHash, ImageHasher, Mismatch, SCHEME, hash_sequence};

/// The known-answer vectors of the specification, computed independently of every
/// implementation.
const VECTORS: &str = include_str!("../../../spec/image-hash-vectors.json");

/// The only encoding so far.
const ENCODING: ColorEncoding = ColorEncoding::LinearRec709Rgba32F;

/// A size.
fn size(width: u32, height: u32) -> ImageSize {
    ImageSize::new(width, height).expect("a valid size")
}

/// An image of `size` holding `values`, row by row.
fn image(size: ImageSize, values: &[f32]) -> Image {
    let mut image = Image::new(size);
    let mut rest = values;
    for row in image.rows_mut() {
        let (head, tail) = rest.split_at(row.len());
        row.copy_from_slice(head);
        rest = tail;
    }
    assert!(rest.is_empty(), "every value placed");
    image
}

/// The hash of `images` of `size` at 60 images per second.
fn hash(size: ImageSize, images: &[&[f32]]) -> ImageHash {
    let mut hasher = ImageHasher::new(ImageRate::PER_SECOND_60, size, ENCODING);
    for values in images {
        hasher.add(&image(size, values)).expect("same size");
    }
    hasher.finish()
}

/// Reads a field of a vector as an unsigned 32-bit integer.
fn number(value: &Value) -> u32 {
    u32::try_from(value.as_u64().expect("an unsigned integer")).expect("u32")
}

/// Reads a field of a vector as text.
fn text<'json>(vector: &'json Value, key: &str) -> &'json str {
    vector[key].as_str().expect("a string")
}

/// Decodes hexadecimal digits, two per byte.
fn hex_bytes(digits: &str) -> Vec<u8> {
    digits
        .as_bytes()
        .chunks(2)
        .map(|pair| {
            u8::from_str_radix(core::str::from_utf8(pair).expect("ASCII"), 16).expect("hexadecimal")
        })
        .collect()
}

#[test]
fn reproduces_the_known_answer_vectors_of_the_specification() {
    // If one changes, every reference hash changes: the scheme needs a new tag.
    let json: Value = serde_json::from_str(VECTORS).expect("valid JSON");
    assert_eq!(text(&json, "scheme"), SCHEME, "same scheme");
    assert_eq!(SCHEME, "image-hash/1", "its name and version");
    let vectors = json["vectors"].as_array().expect("a list of vectors");
    assert_eq!(
        vectors.len(),
        5,
        "every vector is read: the five of the file"
    );
    for vector in vectors {
        let name = text(vector, "name");
        let rate = ImageRate::new(number(&vector["rate"][0]), number(&vector["rate"][1]))
            .expect("a valid rate");
        let size = size(number(&vector["width"]), number(&vector["height"]));
        assert_eq!(number(&vector["encoding"]), 1, "{name}: encoding 1");
        let mut hasher = ImageHasher::new(rate, size, ENCODING);
        for values in vector["images"].as_array().expect("a list of images") {
            let values: Vec<f32> = values
                .as_array()
                .expect("a list of values")
                .iter()
                .map(|bits| {
                    f32::from_bits(
                        u32::from_str_radix(bits.as_str().expect("a string"), 16)
                            .expect("hexadecimal bits"),
                    )
                })
                .collect();
            hasher.add(&image(size, &values)).expect("same size");
        }
        let expected = text(vector, "sha256");
        assert_eq!(hasher.finish().to_string(), expected, "{name}");
        assert_eq!(
            Sha256::digest(hex_bytes(text(vector, "bytes"))).as_slice(),
            hex_bytes(expected),
            "{name}: the bytes of the vector give its hash"
        );
    }
}

#[test]
fn shows_64_lowercase_hex_digits() {
    let text = hash(size(1, 1), &[]).to_string();
    assert_eq!(text.len(), 64, "32 bytes");
    assert!(
        text.bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "{text}"
    );
}

#[test]
fn gives_the_bytes_it_shows_first_byte_first() {
    let shown = hash(
        size(2, 1),
        &[&[0.25, 0.5, 0.75, 1.0, 0.0, -0.5, 2.0, 0.125]],
    );
    assert_eq!(
        hex_bytes(&shown.to_string()),
        shown.bytes(),
        "the digits, two per byte, in order"
    );
}

#[test]
fn covers_every_bit_of_every_value() {
    let one = size(1, 1);
    let base = hash(one, &[&[0.25, 0.5, 0.75, 1.0]]);
    let next = f32::from_bits(0.5_f32.to_bits() + 1);
    assert_ne!(base, hash(one, &[&[0.25, next, 0.75, 1.0]]), "1 ulp");
    assert_ne!(
        hash(one, &[&[0.0, 0.0, 0.0, 0.0]]),
        hash(one, &[&[-0.0, 0.0, 0.0, 0.0]]),
        "sign of zero"
    );
    assert_ne!(
        base,
        hash(one, &[&[0.5, 0.25, 0.75, 1.0]]),
        "order of values"
    );
    assert_eq!(base, hash(one, &[&[0.25, 0.5, 0.75, 1.0]]), "deterministic");
}

#[test]
fn covers_the_sequence() {
    let one = size(1, 1);
    let first: &[f32] = &[0.0, 0.0, 0.0, 1.0];
    let second: &[f32] = &[1.0, 1.0, 1.0, 1.0];
    let base = hash(one, &[first, second]);
    assert_ne!(base, hash(one, &[second, first]), "order of images");
    assert_ne!(base, hash(one, &[first]), "number of images");
    assert_ne!(hash(one, &[]), hash(one, &[first]), "no image");
}

#[test]
fn covers_the_format() {
    let values: &[f32] = &[0.0, 0.5, 1.0, 1.0, 1.0, 0.5, 0.0, 1.0];
    let wide = hash(size(2, 1), &[values]);
    assert_ne!(wide, hash(size(1, 2), &[values]), "width and height");
    let rate = ImageRate::new(24_000, 1001).expect("a valid rate");
    let mut other = ImageHasher::new(rate, size(2, 1), ENCODING);
    other.add(&image(size(2, 1), values)).expect("same size");
    assert_ne!(wide, other.finish(), "image rate");
    let numerator_only = ImageRate::new(25, 1).expect("a valid rate");
    let mut another = ImageHasher::new(numerator_only, size(2, 1), ENCODING);
    another.add(&image(size(2, 1), values)).expect("same size");
    assert_ne!(wide, another.finish(), "another rate");
}

#[test]
fn refuses_an_image_of_another_size_and_adds_nothing() {
    let sequence = size(2, 1);
    let mut hasher = ImageHasher::new(ImageRate::PER_SECOND_60, sequence, ENCODING);
    hasher
        .add(&image(sequence, &[0.0; 8]))
        .expect("the sequence's size");
    for other in [size(1, 2), size(1, 1), size(3, 1)] {
        assert_eq!(
            hasher.add(&Image::new(other)),
            Err(Mismatch { image: 1 }),
            "{other:?}"
        );
    }
    assert_eq!(
        hasher.finish(),
        hash(sequence, &[&[0.0; 8]]),
        "the refused images are not in the hash"
    );
}

#[test]
fn names_the_mismatched_image() {
    assert_eq!(
        Mismatch { image: 7 }.to_string(),
        "image 7 differs from the sequence in size or color encoding",
        "message"
    );
}

/// Writes `index × 0.25 + offset` in every value of image `index`, a different image for
/// every index, so that a hash of the wrong images or in the wrong order differs.
#[derive(Debug)]
struct Ramp(f32);

impl VisualBox for Ramp {
    fn render(&mut self, index: ImageIndex, output: &mut Image) {
        let level = f32::from(u16::try_from(index.0).expect("a small index")) * 0.25 + self.0;
        for row in output.rows_mut() {
            row.fill(level);
        }
    }
}

#[test]
fn hashes_a_rendered_sequence_as_adding_each_image() {
    let size = size(3, 2);
    for (first, count) in [(0_u64, 0_u64), (0, 1), (5, 3), (2, 4)] {
        let mut separately = ImageHasher::new(ImageRate::PER_SECOND_60, size, ENCODING);
        for offset in 0..count {
            let mut one = Image::new(size);
            Ramp(0.5).render(ImageIndex(first + offset), &mut one);
            separately.add(&one).expect("same size");
        }
        let mut image = Image::new(size);
        assert_eq!(
            hash_sequence(
                &mut Ramp(0.5),
                ImageRate::PER_SECOND_60,
                ImageRange::new(ImageIndex(first), count).expect("a valid range"),
                &mut image
            ),
            separately.finish(),
            "{count} images from {first}"
        );
    }
    let mut image = Image::new(size);
    let at = |visual: &mut Ramp, into: &mut Image, first: u64| {
        hash_sequence(
            visual,
            ImageRate::PER_SECOND_60,
            ImageRange::new(ImageIndex(first), 2).expect("a valid range"),
            into,
        )
    };
    let base = at(&mut Ramp(0.5), &mut image, 1);
    assert_ne!(
        base,
        at(&mut Ramp(0.5), &mut image, 2),
        "from another image"
    );
    assert_ne!(base, at(&mut Ramp(0.75), &mut image, 1), "other values");
    assert_ne!(
        base,
        hash_sequence(
            &mut Ramp(0.5),
            ImageRate::new(30, 1).expect("a valid rate"),
            ImageRange::new(ImageIndex(1), 2).expect("a valid range"),
            &mut image
        ),
        "another rate"
    );
}

#[test]
fn reports_a_mismatch_as_an_error() {
    /// Compiles only for an error type.
    const fn error<E: core::error::Error>() {}
    error::<Mismatch>();
}

#[test]
fn hashes_the_bytes_the_specification_lays_out_for_any_values() {
    // At random rates, sizes and counts of images whose values are any bit patterns (NaN
    // payloads, -0, subnormals, infinities included): the hash is the SHA-256 of the bytes
    // laid out here from the specification's table, apart from the hasher.
    use visual_testkit::generate::{rate_terms, u32_edges};
    use visual_testkit::proptest::collection::vec;
    use visual_testkit::proptest::prelude::{Just, Strategy, prop_oneof};
    use visual_testkit::proptest::test_runner::TestCaseError;
    use visual_testkit::runner::runner;
    let bits = prop_oneof![
        u32_edges(),
        Just(0x8000_0000_u32),
        Just(0x0000_0001_u32),
        Just(0x7fc0_0001_u32),
        Just(0xff80_0000_u32),
    ];
    let cases = (rate_terms(), 1_u32..=3, 1_u32..=3).prop_flat_map(move |(rate, width, height)| {
        let values = usize::try_from(width * height * 4).unwrap_or(0);
        (
            Just((rate, width, height)),
            vec(vec(bits.clone(), values), 0..=3),
        )
    });
    let verdict = runner(256).run(&cases, |((rate, width, height), images)| {
        let image_rate =
            ImageRate::new(rate.0, rate.1).map_err(|_| TestCaseError::fail("a rate"))?;
        let mut hasher = ImageHasher::new(image_rate, size(width, height), ENCODING);
        let mut bytes: Vec<u8> = Vec::new();
        bytes.extend_from_slice(b"image-hash/1");
        for field in [rate.0, rate.1, width, height, 1] {
            bytes.extend_from_slice(&field.to_le_bytes());
        }
        for values in &images {
            let mut image = Image::new(size(width, height));
            for (slot, &pattern) in image.rows_mut().flatten().zip(values) {
                *slot = f32::from_bits(pattern);
            }
            hasher
                .add(&image)
                .map_err(|_| TestCaseError::fail("same size"))?;
            for &pattern in values {
                bytes.extend_from_slice(&pattern.to_le_bytes());
            }
        }
        bytes.extend_from_slice(&u64::try_from(images.len()).unwrap_or(0).to_le_bytes());
        let expected: [u8; 32] = Sha256::digest(&bytes).into();
        if hasher.finish().bytes() == expected {
            Ok(())
        } else {
            Err(TestCaseError::fail(format!(
                "{rate:?} {width}x{height} {images:?}"
            )))
        }
    });
    assert_eq!(
        verdict.map_err(|error| error.to_string()),
        Ok(()),
        "every layout"
    );
}
