//! Tests of the renderer's own arithmetic, at both pointer widths: the host running them
//! has one width only, so the width is an argument here, given as literals.

use super::fits;

/// The largest size a value may have (`isize::MAX` bytes) on a 32-bit target, such as
/// WebAssembly's, and on a 64-bit one.
const LARGEST: [u64; 2] = [(1_u64 << 31_u32) - 1, (1_u64 << 63_u32) - 1];

#[test]
fn fits_samples_in_memory_at_both_pointer_widths() {
    // The largest number of frames whose samples fit, for a size of memory, a channel count
    // and a sample's bytes (8 for binary64, 4 for binary32): `(largest + 1) / (channels ·
    // size) - 1`, written out.
    for (largest, channels, size, last) in [
        (LARGEST[0], 2, 8, (1_u64 << 27_u32) - 1),
        (LARGEST[0], 2, 4, (1_u64 << 28_u32) - 1),
        (LARGEST[0], 1, 4, (1_u64 << 29_u32) - 1),
        (LARGEST[1], 2, 8, (1_u64 << 59_u32) - 1),
        (LARGEST[1], 2, 4, (1_u64 << 60_u32) - 1),
        (LARGEST[1], 1, 4, (1_u64 << 61_u32) - 1),
    ] {
        assert!(
            fits(last, channels, size, largest),
            "{last} frames of {channels} channels of {size} bytes in {largest}"
        );
        assert!(
            !fits(last + 1, channels, size, largest),
            "{} frames of {channels} channels of {size} bytes in {largest}",
            last + 1
        );
    }
}

#[test]
fn fits_a_plan_without_output_at_every_length() {
    for largest in LARGEST {
        for size in [8, 4] {
            assert!(
                fits(u64::MAX, 0, size, largest),
                "no sample, at {largest} bytes of {size}"
            );
        }
    }
}

#[test]
fn fits_no_count_beyond_its_arithmetic() {
    assert!(
        !fits(u64::MAX, 2, 8, u64::MAX),
        "frames times channels overflows"
    );
    assert!(
        !fits(1_u64 << 62_u32, 1, 8, u64::MAX),
        "samples times bytes overflows"
    );
    assert!(
        fits(1_u64 << 60_u32, 1, 8, u64::MAX),
        "the largest product, kept"
    );
}

#[test]
fn fits_exactly_its_largest_size() {
    // A size of memory is never a multiple of a sample's bytes at a real width: its limit
    // is tested here, at 64 bytes, then one byte short.
    assert!(fits(4, 2, 8, 64), "64 bytes in 64");
    assert!(!fits(4, 2, 8, 63), "64 bytes in 63");
}
