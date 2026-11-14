//! The global allocator of this crate's test binary.

/// Counts heap operations in the tests, to prove rendering never allocates.
#[global_allocator]
static ALLOCATOR: visual_testkit::CountingAllocator = visual_testkit::CountingAllocator;
