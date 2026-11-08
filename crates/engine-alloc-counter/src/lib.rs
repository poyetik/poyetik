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
mod tests {
    use core::hint::black_box;
    use std::sync::{Barrier, BarrierWaitResult};

    use super::{CountingAllocator, NotCounting, heap_operations_during, require_counted};

    /// The test binary counts heap operations with the allocator under test.
    #[global_allocator]
    static ALLOCATOR: CountingAllocator = CountingAllocator;

    /// The heap operations of `action`, which the installed counter always measures.
    fn counted(action: impl FnOnce()) -> usize {
        heap_operations_during(action).map_or(usize::MAX, |((), count)| count)
    }

    #[test]
    fn counts_every_heap_operation_releases_included() {
        let built = counted(|| drop(black_box(Box::new(1_u64))));
        assert_eq!(built, 2, "alloc, then dealloc");
        let zeroed = counted(|| drop(black_box(vec![0_u8; 32])));
        assert_eq!(zeroed, 2, "alloc_zeroed, then dealloc");
        let mut grown = Vec::<u8>::with_capacity(1);
        grown.push(1);
        let resized = counted(|| {
            grown.extend_from_slice(&[0; 64]);
            let _: &Vec<u8> = black_box(&grown);
        });
        assert_eq!(resized, 1, "realloc");
        assert_eq!(grown.len(), 65, "the reallocated memory holds the data");
        let prebuilt = Box::new(2_u64);
        let freed = counted(|| drop(black_box(prebuilt)));
        assert_eq!(freed, 1, "dealloc is a heap operation");
        assert_eq!(
            heap_operations_during(|| 6_i32 * 7_i32),
            Ok((42_i32, 0_usize)),
            "returns the result, and the probe is not counted"
        );
    }

    #[test]
    fn refuses_a_count_when_nothing_is_counted() {
        assert_eq!(require_counted(0), Err(NotCounting), "no vacuous zero");
        assert_eq!(require_counted(1), Ok(()), "counted");
        assert_eq!(
            String::from(NotCounting),
            "CountingAllocator is not installed as the global allocator",
            "the reason"
        );
    }

    #[test]
    fn counts_per_thread() {
        // The other thread waits for this thread's window to open, allocates and frees
        // 1000 times, and is joined before the window closes: a process-wide counter
        // would count its 2000 operations here too, every time, not only when threads
        // happen to overlap.
        static START: Barrier = Barrier::new(2);
        let other = std::thread::spawn(|| {
            let _: BarrierWaitResult = START.wait();
            counted(|| {
                for value in 0_u16..1_000_u16 {
                    drop(black_box(Box::new(value)));
                }
            })
        });
        let here = counted(|| {
            let _: BarrierWaitResult = START.wait();
            assert_eq!(
                other.join().ok(),
                Some(2_000),
                "the other thread counts its own"
            );
        });
        assert!(here < 2_000, "this thread's heap operations: {here}");
    }
}
