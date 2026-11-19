//! Tests of the encoding of values a host gives: the bytes of the display encoding, the room
//! refused, and no heap operation while encoding.

#![forbid(unsafe_code)]

extern crate alloc;

use alloc::collections::TryReserveError;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

use visual_core::{ImageSize, pixel8};
use visual_testkit::heap_operations_during;

use super::Encoding;

/// A reservation that fails as the allocator does when the machine has no room left.
fn refused<T>(room: &mut Vec<T>, _: usize) -> Result<(), TryReserveError> {
    room.try_reserve_exact(usize::MAX)
}

#[test]
fn encodes_the_values_written_as_the_display_encoding_does() {
    let mut encoding = Encoding::new();
    assert!(encoding.encode().is_empty(), "no room, no byte");
    let pixels = [
        [0.0, 0.0, 0.0, 0.0],
        [1.0, 0.5, 0.25, 1.0],
        [0.2, 0.1, 0.05, 0.4],
        [-0.0, f32::NAN, f32::INFINITY, 0.5],
        [2.0, -1.0, f32::MIN_POSITIVE / 2.0, f32::NEG_INFINITY],
    ];
    let values = encoding
        .values(u32::try_from(pixels.len()).expect("a few pixels"))
        .expect("room for five pixels");
    assert!(values.iter().all(|value| value.to_bits() == 0), "all 0");
    for (slot, value) in values.iter_mut().zip(pixels.iter().flatten()) {
        *slot = *value;
    }
    let expected: Vec<u8> = pixels
        .iter()
        .flat_map(|&[red, green, blue, alpha]| pixel8(red, green, blue, alpha))
        .collect();
    let (bytes, operations) = heap_operations_during(|| encoding.encode().len())
        .expect("the counting allocator is installed");
    assert_eq!(
        (bytes, operations),
        (20, 0),
        "four bytes a pixel, without the heap"
    );
    assert_eq!(
        encoding.encode(),
        expected.as_slice(),
        "each pixel by pixel8"
    );
}

/// The count of values the last [`asked`] reservation was asked for.
static ASKED: AtomicUsize = AtomicUsize::new(0);

/// A reservation that records the count it is asked for, then fails as the allocator does
/// when the machine has no room left: the bounds of a count are tested without its memory.
fn asked(room: &mut Vec<f32>, count: usize) -> Result<(), TryReserveError> {
    ASKED.store(count, Ordering::SeqCst);
    room.try_reserve_exact(usize::MAX)
}

#[test]
fn makes_room_for_one_pixel_up_to_the_pixels_of_the_largest_image() {
    let mut encoding = Encoding::new();
    assert_eq!(encoding.values(1).map(|values| values.len()), Some(4));
    assert_eq!(encoding.encode().len(), 4);
    let largest = u32::try_from(ImageSize::MAX_PIXELS).expect("2^25 pixels");
    // The largest image's pixels are asked of the memory (640 MiB, not taken by the test).
    assert_eq!(
        encoding.values_with(largest, asked, Vec::try_reserve_exact),
        None
    );
    assert_eq!(
        ASKED.load(Ordering::SeqCst),
        usize::try_from(ImageSize::MAX_PIXELS * 4).expect("the values' count"),
        "the largest image's values asked"
    );
    for refused_count in [0, largest + 1, u32::MAX] {
        ASKED.store(0, Ordering::SeqCst);
        assert_eq!(
            encoding.values_with(refused_count, asked, Vec::try_reserve_exact),
            None,
            "{refused_count} pixels"
        );
        assert_eq!(
            ASKED.load(Ordering::SeqCst),
            0,
            "{refused_count}: nothing asked"
        );
        assert!(encoding.encode().is_empty(), "no room left");
    }
}

#[test]
fn leaves_no_room_when_the_memory_cannot_hold_the_values_or_their_bytes() {
    let mut encoding = Encoding::new();
    assert!(encoding.values(3).is_some());
    assert_eq!(
        encoding.values_with(2, refused, Vec::try_reserve_exact),
        None,
        "the values refused"
    );
    assert_eq!(encoding, Encoding::new(), "the old room released");
    assert_eq!(
        encoding.values_with(2, Vec::try_reserve_exact, refused),
        None,
        "the bytes refused"
    );
    assert!(encoding.encode().is_empty(), "no room left");
    assert_eq!(Encoding::default(), Encoding::new());
}
