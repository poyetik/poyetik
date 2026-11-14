//! The global allocator of this crate's test binary.

use super::CountingAllocator;

/// The test binary of this crate counts heap operations, like every box test binary must.
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;
