//! The global allocator of this crate's test binary.

#![forbid(unsafe_code)]

/// The test binary counts heap operations, so that the C interface can be checked not to
/// use the heap while rendering.
#[global_allocator]
static ALLOCATOR: audio_testkit::CountingAllocator = audio_testkit::CountingAllocator;
