//! Tests of the C interface, at the pointer level: they use `unsafe` under the lifting of
//! the FFI module that declares them, and run under Miri.
//!
//! The interface keeps one view per module instance, a static every test of this binary
//! would share: one test drives the whole protocol, in order.

use visual_boxes::Square;
use visual_core::{
    DisplayImage, Image, ImageIndex, ImageRange, ImageRate, ImageSize, Seconds, VisualBox,
};
use visual_hash::hash_sequence;

use super::{
    ABI_VERSION, visual_abi_version, visual_display, visual_display_len, visual_encode,
    visual_encode_values, visual_hash, visual_message_len, visual_message_ptr, visual_placement,
    visual_render, visual_setup, visual_shader_len, visual_shader_ptr, visual_thresholds,
    visual_values_len,
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

/// Image `index` encoded for display through the exported interface, its bytes copied as
/// JavaScript reads them.
fn displayed(index: u64) -> Vec<u8> {
    let (low, high) = split(index);
    let pointer = visual_display(low, high);
    let len = visual_display_len();
    // SAFETY: the image is `len` initialized bytes at `pointer`, unchanged until the next
    // call that sets up, renders or hashes (`visual_display_len` only reads); an empty
    // image is an empty slice at a dangling, aligned pointer.
    unsafe { core::slice::from_raw_parts(pointer, len) }.to_vec()
}

/// Where the square is in image `index`, through the exported interface: none when the
/// address is 0.
fn placed(index: u64) -> Option<[u32; 4]> {
    let (low, high) = split(index);
    let pointer = visual_placement(low, high);
    (!pointer.is_null()).then(|| {
        // SAFETY: a placement is four initialized edges at `pointer`, unchanged until the next
        // call that sets up, renders, hashes or places.
        unsafe { core::slice::from_raw_parts(pointer, 4) }
            .try_into()
            .expect("four edges")
    })
}

/// The WGSL source of shader `kind`, through the exported interface.
fn shader_source(kind: u32) -> String {
    let (pointer, length) = (visual_shader_ptr(kind), visual_shader_len(kind));
    // SAFETY: the source is `length` initialized bytes at `pointer`, static.
    String::from_utf8(unsafe { core::slice::from_raw_parts(pointer, length) }.to_vec())
        .expect("UTF-8")
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
    assert_eq!(ABI_VERSION & 0xFFFF, 4, "the fourth version");
    assert_eq!(message(), "", "no message before a setup");
    assert_eq!(visual_values_len(), 0, "nothing before a setup");
    assert_eq!(visual_display_len(), 0, "no bytes before a setup");
    assert!(rendered(3).is_empty(), "no image before a setup");
    assert!(
        displayed(3).is_empty(),
        "no image to display before a setup"
    );
    assert!(visual_hash(0, 0, 2).is_null(), "no hash before a setup");
    assert_eq!(placed(3), None, "no placement before a setup");
    assert_eq!(
        shader_source(0),
        visual_boxes::SQUARE_WGSL,
        "the square's shader"
    );
    assert_eq!(
        shader_source(1),
        visual_core::DISPLAY_WGSL,
        "the display shader"
    );
    assert_eq!(shader_source(2), "", "no other shader");
    // SAFETY: 510 initialized thresholds at a static address.
    let thresholds = unsafe { core::slice::from_raw_parts(visual_thresholds(), 510) };
    assert_eq!(
        thresholds,
        visual_core::DISPLAY_THRESHOLD_BITS,
        "the display shader's thresholds"
    );
    assert_eq!(visual_setup(0, 5, 60, 1, 1000, 1, 1), 0, "an invalid setup");
    assert_eq!(
        message(),
        "width and height: invalid image size 0x5: each side must be from 1 to 16384 pixels",
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
    let mut image =
        Image::try_new(ImageSize::new(9, 4).expect("a valid size")).expect("room for the image");
    assert_eq!(visual_values_len(), 9 * 4 * 4, "four values per pixel");
    assert_eq!(visual_display_len(), 9 * 4 * 4, "four bytes per pixel");
    let mut display = DisplayImage::try_new(image.size()).expect("room for the image");
    for index in [0, 17, u64::from(u32::MAX) + 1, (7_u64 << 32_u32) + 11] {
        square.render(ImageIndex(index), &mut image);
        let expected: Vec<u32> = image.values().iter().map(|value| value.to_bits()).collect();
        assert_eq!(
            rendered(index),
            expected,
            "image {index}, both halves of the index"
        );
        display.encode(&image).expect("the same size");
        let edges = square.placement(ImageIndex(index), image.size());
        assert_eq!(
            placed(index),
            Some([edges.left(), edges.right(), edges.top(), edges.bottom()]),
            "image {index} placed, both halves of the index"
        );
        assert_eq!(
            displayed(index),
            display.bytes(),
            "image {index} for display, both halves of the index"
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

#[test]
fn encodes_any_values_for_display_as_the_native_encoding() {
    // Apart from the view: the encoding's own static, which only this test drives.
    let pixels: [[f32; 4]; 4] = [
        [1.0, 0.5, 0.25, 1.0],
        [0.2, 0.1, 0.05, 0.4],
        [-0.0, f32::NAN, f32::INFINITY, 0.5],
        [2.0, -1.0, f32::MIN_POSITIVE / 2.0, f32::NEG_INFINITY],
    ];
    let count = u32::try_from(pixels.len()).expect("four pixels");
    let room = visual_encode_values(count);
    assert!(!room.is_null(), "room for four pixels");
    let values: Vec<f32> = pixels.iter().flatten().copied().collect();
    // SAFETY: the room is four values a pixel at `room`, writable until the next
    // `visual_encode_values`; JavaScript writes them the same way, through the memory.
    unsafe { core::ptr::copy_nonoverlapping(values.as_ptr(), room, values.len()) };
    let encoded = visual_encode();
    // SAFETY: the bytes are four a pixel at `encoded`, unchanged until the next
    // `visual_encode_values`.
    let bytes = unsafe { core::slice::from_raw_parts(encoded, values.len()) }.to_vec();
    let expected: Vec<u8> = pixels
        .iter()
        .flat_map(|&[red, green, blue, alpha]| visual_core::pixel8(red, green, blue, alpha))
        .collect();
    assert_eq!(bytes, expected, "each pixel as the native encoding");
    assert!(visual_encode_values(0).is_null(), "no pixel: no room");
    assert!(
        !visual_encode().is_null(),
        "with no room, an address of no byte, never 0"
    );
    assert!(
        visual_encode_values(u32::MAX).is_null(),
        "more than an image holds"
    );
}
