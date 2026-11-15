//! The global allocator of this crate's test binary.

#![forbid(unsafe_code)]

/// The test binary counts heap operations, so that rendering can be checked not to use
/// the heap.
#[global_allocator]
static ALLOCATOR: audio_testkit::CountingAllocator = audio_testkit::CountingAllocator;

/// A test build names no engine block: the module's is the default one.
#[test]
fn builds_the_default_engine_block_when_none_is_named() {
    assert_eq!(
        super::ENGINE_BLOCK,
        audio_core::BLOCK_SIZE,
        "a test build names none"
    );
}
