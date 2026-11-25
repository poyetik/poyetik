//! A global allocator that counts heap operations per thread, to prove that boxes,
//! plans and the engine never touch the heap while processing audio, and the bytes they
//! reserve, to bound what building a plan keeps.
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
#![doc(test(attr(forbid(unsafe_code))))]

use core::alloc::{GlobalAlloc, Layout};
use core::cell::Cell;
use core::hint::black_box;
use std::alloc::System;

thread_local! {
    /// Number of heap operations made by the current thread.
    static HEAP_OPERATIONS: Cell<usize> = const { Cell::new(0) };
    /// Bytes reserved by the current thread: the size of every allocation, and the new
    /// size of every reallocation.
    static BYTES_RESERVED: Cell<usize> = const { Cell::new(0) };
    /// Bytes released by the current thread: the size of every release, and the old size
    /// of every reallocation.
    static BYTES_RELEASED: Cell<usize> = const { Cell::new(0) };
}

/// Counts one heap operation on the current thread, which reserves `reserved` bytes and
/// releases `released`.
fn record(reserved: usize, released: usize) {
    /// Adds `bytes` to the thread's counter `counter`.
    fn add(counter: &'static std::thread::LocalKey<Cell<usize>>, bytes: usize) {
        // `try_with` fails only while the thread is being torn down: nothing to count then.
        let _counted: Result<(), _> =
            counter.try_with(|count| count.set(count.get().saturating_add(bytes)));
    }
    add(&HEAP_OPERATIONS, 1);
    add(&BYTES_RESERVED, reserved);
    add(&BYTES_RELEASED, released);
}

/// The current thread's heap operation count.
fn current() -> usize {
    HEAP_OPERATIONS.with(Cell::get)
}

/// The bytes the current thread has reserved and released.
fn bytes() -> (usize, usize) {
    (
        BYTES_RESERVED.with(Cell::get),
        BYTES_RELEASED.with(Cell::get),
    )
}

/// Global allocator that counts heap operations (allocations, reallocations and
/// releases) and the bytes they reserve and release, then delegates to the system
/// allocator.
///
/// Install it in a test binary:
/// `#[global_allocator] static ALLOCATOR: CountingAllocator = CountingAllocator;`
#[derive(Debug, Clone, Copy, Default)]
pub struct CountingAllocator;

// SAFETY: every method forwards its arguments unchanged to `System`, which upholds the
// `GlobalAlloc` contract; counting only touches a thread-local `Cell` and never allocates.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(layout.size(), 0);
        // SAFETY: same contract as the caller's: forwarded unchanged to `System`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record(layout.size(), 0);
        // SAFETY: same contract as the caller's: forwarded unchanged to `System`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record(new_size, layout.size());
        // SAFETY: same contract as the caller's: forwarded unchanged to `System`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        record(0, layout.size());
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
    operations_counted_by(current, action)
}

/// [`heap_operations_during`], its operations read by `count` ([`current`]): the seam
/// through which a test reads a count that the probe does not move, as when
/// [`CountingAllocator`] is not installed.
fn operations_counted_by<R>(
    count: fn() -> usize,
    action: impl FnOnce() -> R,
) -> Result<(R, usize), NotCounting> {
    let before = count();
    drop(black_box(Box::new(0_u8)));
    let probed = count();
    let result = action();
    let operations = count().saturating_sub(probed);
    require_counted(probed.saturating_sub(before))?;
    Ok((result, operations))
}

/// Runs `action` and returns its result with the bytes it left reserved on the current
/// thread.
///
/// Those are the bytes it reserved (allocations, and the new size of reallocations) minus
/// those it released (releases, and the old size of reallocations), or zero when it
/// released more. What the result holds on the heap is counted, since it is not released
/// before the count ends.
///
/// Before the action, a probe allocation checks that the operations are counted at all.
///
/// # Errors
///
/// Returns [`NotCounting`] when the probe was not counted: [`CountingAllocator`] is not
/// the global allocator, and a count of zero would pass vacuously. The action has run.
pub fn heap_bytes_during<R>(action: impl FnOnce() -> R) -> Result<(R, usize), NotCounting> {
    bytes_counted_by(current, action)
}

/// [`heap_bytes_during`], its probe read by `count` ([`current`]): the seam through which
/// a test reads a count that the probe does not move, as when [`CountingAllocator`] is not
/// installed.
fn bytes_counted_by<R>(
    count: fn() -> usize,
    action: impl FnOnce() -> R,
) -> Result<(R, usize), NotCounting> {
    let before = count();
    drop(black_box(Box::new(0_u8)));
    let probed = count();
    let (reserved, released) = bytes();
    let result = action();
    let (now_reserved, now_released) = bytes();
    require_counted(probed.saturating_sub(before))?;
    let kept = now_reserved
        .saturating_sub(reserved)
        .saturating_sub(now_released.saturating_sub(released));
    Ok((result, kept))
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
