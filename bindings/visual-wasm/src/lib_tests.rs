//! The global allocator of this crate's test binary.

#![forbid(unsafe_code)]

/// The test binary counts heap operations, so that rendering can be checked not to use
/// the heap.
#[global_allocator]
static ALLOCATOR: visual_testkit::CountingAllocator = visual_testkit::CountingAllocator;
