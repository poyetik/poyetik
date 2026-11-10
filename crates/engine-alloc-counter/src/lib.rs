//! A global allocator that counts heap operations per thread, to prove that boxes,
//! plans and the engine never touch the heap while processing audio.
//!
//! Every heap operation counts, a release included: `alloc`, `alloc_zeroed`, `realloc`
//! and `dealloc` all go through the system allocator, which may take a lock, so freeing
//! memory on the audio thread is as forbidden as allocating it.
//!
//! A crate of its own, and the only `unsafe` code of the test bench: implementing
//! `GlobalAlloc` is unsafe by definition. Every operation is delegated unchanged to the
//! system allocator. Its tests run under Miri in `cargo xtask check`.

#![expect(
    unsafe_code,
    reason = "a counting GlobalAlloc must implement an unsafe trait"
)]

use core::alloc::{GlobalAlloc, Layout};
use core::cell::Cell;
use core::hint::black_box;
use std::alloc::System;

thread_local! {
    /// Number of heap operations made by the current thread.
    static HEAP_OPERATIONS: Cell<usize> = const { Cell::new(0) };
}

/// Counts one heap operation on the current thread.
fn record() {
    // `try_with` fails only while the thread is being torn down: nothing to count then.
    let _counted: Result<(), _> =
        HEAP_OPERATIONS.try_with(|count| count.set(count.get().saturating_add(1)));
}

/// The current thread's heap operation count.
fn current() -> usize {
    HEAP_OPERATIONS.with(Cell::get)
}

/// Global allocator that counts heap operations (allocations, reallocations and
/// releases), then delegates to the system allocator.
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
        record();
        // SAFETY: same contract as the caller's: forwarded unchanged to `System`.
        unsafe { System.dealloc(ptr, layout) }
    }
}

/// [`CountingAllocator`] is not the global allocator: nothing is counted, so a count of
/// zero would prove nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotCounting;

impl core::fmt::Display for NotCounting {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("CountingAllocator is not installed as the global allocator")
    }
}

impl From<NotCounting> for String {
    fn from(error: NotCounting) -> Self {
        error.to_string()
    }
}

/// Refuses a probe that counted nothing: the counter is not installed.
const fn require_counted(probe: usize) -> Result<(), NotCounting> {
    if probe == 0 { Err(NotCounting) } else { Ok(()) }
}

/// Runs `action` and returns its result with the number of heap operations
/// (allocations, reallocations and releases) it made on the current thread.
///
/// Before the action, a probe allocation checks that the operations are counted at all.
///
/// # Errors
///
/// Returns [`NotCounting`] when the probe was not counted: [`CountingAllocator`] is not
/// the global allocator, and a count of zero would pass vacuously. The action has run.
pub fn heap_operations_during<R>(action: impl FnOnce() -> R) -> Result<(R, usize), NotCounting> {
    let before = current();
    drop(black_box(Box::new(0_u8)));
    let probed = current();
    let result = action();
    let operations = current().saturating_sub(probed);
    require_counted(probed.saturating_sub(before))?;
    Ok((result, operations))
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
