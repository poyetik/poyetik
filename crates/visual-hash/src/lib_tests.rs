extern crate alloc;

use alloc::string::ToString;
use alloc::vec::Vec;

use serde_json::Value;
use sha2::{Digest, Sha256};
use visual_core::{ColorEncoding, Image, ImageRate, ImageSize};

use super::{ImageHash, ImageHasher, Mismatch, TAG};

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
    assert_eq!(text(&json, "scheme").as_bytes(), TAG, "same scheme");
    let vectors = json["vectors"].as_array().expect("a list of vectors");
    assert!(vectors.len() >= 4, "every vector is read");
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
    let denominator_too = ImageRate::new(50, 2);
    assert!(
        denominator_too.is_err(),
        "rates are reduced, so 25/1 has one form"
    );
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
