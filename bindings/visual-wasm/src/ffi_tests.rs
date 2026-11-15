//! Tests of the C interface, at the pointer level: they use `unsafe` under the lifting of
//! the FFI module that declares them, and run under Miri.
//!
//! The interface keeps one view per module instance, a static every test of this binary
//! would share: one test drives the whole protocol, in order.

use visual_boxes::Square;
use visual_core::{Image, ImageIndex, ImageRange, ImageRate, ImageSize, Seconds, VisualBox};
use visual_hash::hash_sequence;

use super::{
    ABI_VERSION, visual_abi_version, visual_hash, visual_message_len, visual_message_ptr,
    visual_render, visual_setup, visual_values_len,
};

/// Renders image `index` through the exported interface and copies its values, as
/// JavaScript reads them, as bits.
fn rendered(index: u64) -> Vec<u32> {
    let (low, high) = split(index);
    let pointer = visual_render(low, high);
    let len = visual_values_len();
    // SAFETY: the image is `len` initialized values at `pointer`, unchanged until the next
    // call that sets up, renders or hashes (`visual_values_len` only reads); an empty image
    // is an empty slice at a dangling, aligned pointer.
    unsafe { core::slice::from_raw_parts(pointer, len) }
        .iter()
        .map(|value| value.to_bits())
        .collect()
}

/// The image hash of `count` images from `first`, through the exported interface.
fn hashed(first: u64, count: u32) -> [u8; 32] {
    let (low, high) = split(first);
    let pointer = visual_hash(low, high, count);
    // SAFETY: the hash is 32 initialized bytes at `pointer`, a square being set up,
    // unchanged until the next call that sets up, renders or hashes.
    unsafe { core::slice::from_raw_parts(pointer, 32) }
        .try_into()
        .expect("32 bytes")
}

/// The message explaining the last refused setup, through the exported interface.
fn message() -> String {
    let (pointer, length) = (visual_message_ptr(), visual_message_len());
    // SAFETY: the message is `length` initialized bytes at `pointer`, unchanged until the
    // next call that sets up, renders or hashes (both exports only read).
    String::from_utf8(unsafe { core::slice::from_raw_parts(pointer, length) }.to_vec())
        .expect("UTF-8")
}

/// `index` as the two 32-bit halves the interface takes: low, then high.
fn split(index: u64) -> (u32, u32) {
    (
        u32::try_from(index & u64::from(u32::MAX)).expect("the low half"),
        u32::try_from(index >> 32_u32).expect("the high half"),
    )
}

#[test]
fn follows_the_protocol() {
    assert_eq!(visual_abi_version(), ABI_VERSION, "the version");
    assert_eq!(ABI_VERSION >> 16_u32, 0x5056, "\"PV\" in the high 16 bits");
    assert_eq!(ABI_VERSION & 0xFFFF, 3, "the third version");
    assert_eq!(message(), "", "no message before a setup");
    assert_eq!(visual_values_len(), 0, "nothing before a setup");
    assert!(rendered(3).is_empty(), "no image before a setup");
    assert!(visual_hash(0, 0, 2).is_null(), "no hash before a setup");
    assert_eq!(visual_setup(0, 5, 60, 1, 1000, 1, 1), 0, "an invalid setup");
    assert_eq!(
        message(),
        "invalid image size 0x5: each side must be from 1 to 16384 pixels",
        "why"
    );
    assert_eq!(visual_values_len(), 0, "still nothing");
    assert_eq!(
        visual_setup(9, 4, 30_000, 1001, 50_000, 3, 2),
        1,
        "a valid setup, every value in its place"
    );
    assert_eq!(message(), "", "no message after a valid setup");
    let rate = ImageRate::new(30_000, 1001).expect("a valid rate");
    let cycle = Seconds::new(3, 2).expect("a valid cycle");
    let mut square = Square::new(rate, 50_000, cycle).expect("a valid square");
    let mut image = Image::new(ImageSize::new(9, 4).expect("a valid size"));
    assert_eq!(visual_values_len(), 9 * 4 * 4, "four values per pixel");
    for index in [0, 17, u64::from(u32::MAX) + 1, (7_u64 << 32_u32) + 11] {
        square.render(ImageIndex(index), &mut image);
        let expected: Vec<u32> = image.values().iter().map(|value| value.to_bits()).collect();
        assert_eq!(
            rendered(index),
            expected,
            "image {index}, both halves of the index"
        );
    }
    for (first, count) in [(5_u64, 3_u32), ((2_u64 << 32_u32) + 1, 2)] {
        let range = ImageRange::new(ImageIndex(first), u64::from(count)).expect("a valid range");
        let expected = hash_sequence(&mut square, rate, range, &mut image);
        assert_eq!(
            hashed(first, count),
            expected.bytes(),
            "{count} images from {first}"
        );
    }
    let (low, high) = split(u64::MAX);
    assert!(
        visual_hash(low, high, 2).is_null(),
        "no address for images past the largest index"
    );
}
