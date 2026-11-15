//! The global allocator of this crate's test binary.

#![forbid(unsafe_code)]

/// The test binary counts heap operations, so that rendering can be checked not to use
/// the heap.
#[global_allocator]
static ALLOCATOR: audio_testkit::CountingAllocator = audio_testkit::CountingAllocator;

/// The names of the engine blocks are those a build offers, each its own decimal name.
#[test]
fn names_every_engine_block_offered() {
    assert_eq!(
        super::NAMED.map(|(_, frames)| frames),
        audio_core::ENGINE_BLOCKS,
        "the sizes named"
    );
    for (name, frames) in super::NAMED {
        assert_eq!(
            name,
            alloc::format!("{frames}"),
            "{frames} named in decimal"
        );
        assert_eq!(
            super::named(name.as_bytes(), &super::NAMED),
            frames,
            "{name} found"
        );
    }
    for unknown in ["", "6", "640", "6x", "064", "1025", "32 "] {
        assert_eq!(
            super::named(unknown.as_bytes(), &super::NAMED),
            0,
            "{unknown:?} refused"
        );
    }
    assert!(super::same(b"", b""), "empty names");
    assert_eq!(
        super::ENGINE_BLOCK,
        audio_core::BLOCK_SIZE,
        "a test build names none"
    );
}
