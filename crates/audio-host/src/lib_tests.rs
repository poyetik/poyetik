//! The global allocator of this crate's test binary.

#![forbid(unsafe_code)]

/// The test binary counts heap operations, so that rendering can be checked not to use
/// the heap.
#[global_allocator]
static ALLOCATOR: audio_testkit::CountingAllocator = audio_testkit::CountingAllocator;

/// A test build names no engine block: the build's is the default one.
#[test]
fn builds_the_default_engine_block_when_none_is_named() {
    assert_eq!(
        super::ENGINE_BLOCK,
        audio_core::BLOCK_SIZE,
        "a test build names none"
    );
}

/// The larger of two numbers, the first when they compare equal: `f64::max` is banned
/// (`clippy.toml`), since it may return either zero when they are `+0` and `-0`, a sign that
/// would differ between targets. A copy of audio-core's test helper (this test target cannot
/// reach that one), with its own test.
pub(crate) fn larger(first: f64, second: f64) -> f64 {
    if second > first { second } else { first }
}

#[test]
fn takes_the_larger_number_and_the_first_of_equal_ones() {
    for (first, second, larger_one) in [
        (1.0_f64, 2.0_f64, 2.0_f64),
        (2.0_f64, 1.0_f64, 2.0_f64),
        (0.0_f64, -0.0_f64, 0.0_f64),
        (-0.0_f64, 0.0_f64, -0.0_f64),
    ] {
        assert_eq!(
            larger(first, second).to_bits(),
            larger_one.to_bits(),
            "{first} and {second}"
        );
    }
}
