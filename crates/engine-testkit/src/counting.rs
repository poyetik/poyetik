//! A global allocator that counts allocations per thread, to prove that boxes never
//! allocate while processing audio.

// The only `unsafe` code of the test bench: implementing `GlobalAlloc` is unsafe by
// definition. Every operation is delegated unchanged to the system allocator.
#![expect(
    unsafe_code,
    reason = "a counting GlobalAlloc must implement an unsafe trait"
)]

use core::alloc::{GlobalAlloc, Layout};
use core::cell::Cell;
use std::alloc::System;

thread_local! {
    /// Number of allocations made by the current thread.
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

/// Counts one allocation on the current thread.
fn record() {
    // `try_with` fails only while the thread is being torn down: nothing to count then.
    let _counted: Result<(), _> =
        ALLOCATIONS.try_with(|count| count.set(count.get().saturating_add(1)));
}

/// The current thread's allocation count.
fn current() -> usize {
    ALLOCATIONS.with(Cell::get)
}

/// Global allocator that counts allocations, then delegates to the system allocator.
///
/// Install it in a test binary:
/// `#[global_allocator] static ALLOCATOR: CountingAllocator = CountingAllocator;`
#[derive(Debug, Clone, Copy, Default)]
pub struct CountingAllocator;

// SAFETY: every method forwards its arguments unchanged to `System`, which upholds the
// `GlobalAlloc` contract; counting only touches a thread-local `Cell` and never allocates.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record();
        // SAFETY: same contract as the caller's: forwarded unchanged to `System`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record();
        // SAFETY: same contract as the caller's: forwarded unchanged to `System`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record();
        // SAFETY: same contract as the caller's: forwarded unchanged to `System`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: same contract as the caller's: forwarded unchanged to `System`.
        unsafe { System.dealloc(ptr, layout) }
    }
}

/// Runs `action` and returns its result with the number of allocations it made on the
/// current thread. Always zero when [`CountingAllocator`] is not the global allocator.
pub fn allocations_during<R>(action: impl FnOnce() -> R) -> (R, usize) {
    let before = current();
    let result = action();
    (result, current().saturating_sub(before))
}
