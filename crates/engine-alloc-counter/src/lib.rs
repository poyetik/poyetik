//! A global allocator that counts allocations per thread, to prove that boxes, plans
//! and the engine never allocate while processing audio.
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

#[cfg(test)]
mod tests {
    use core::hint::black_box;
    use std::sync::{Barrier, BarrierWaitResult};

    use super::{CountingAllocator, allocations_during};

    /// The test binary counts allocations with the allocator under test.
    #[global_allocator]
    static ALLOCATOR: CountingAllocator = CountingAllocator;

    #[test]
    fn counts_allocations_but_not_deallocations() {
        let ((), boxed) = allocations_during(|| drop(black_box(Box::new(1_u64))));
        assert_eq!(boxed, 1, "alloc");
        let ((), zeroed) = allocations_during(|| drop(black_box(vec![0_u8; 32])));
        assert_eq!(zeroed, 1, "alloc_zeroed");
        let mut grown = Vec::<u8>::with_capacity(1);
        grown.push(1);
        let ((), resized) = allocations_during(|| {
            grown.extend_from_slice(&[0; 64]);
            let _: &Vec<u8> = black_box(&grown);
        });
        assert_eq!(resized, 1, "realloc");
        assert_eq!(grown.len(), 65, "the reallocated memory holds the data");
        let prebuilt = Box::new(2_u64);
        let ((), freed) = allocations_during(|| drop(black_box(prebuilt)));
        assert_eq!(freed, 0, "dealloc is not an allocation");
        let (value, none) = allocations_during(|| 6_i32 * 7_i32);
        assert_eq!((value, none), (42_i32, 0_usize), "returns the result");
    }

    #[test]
    fn counts_per_thread() {
        // The other thread waits for this thread's window to open, allocates 1000 times,
        // and is joined before the window closes: a process-wide counter would count its
        // 1000 allocations here too, every time, not only when threads happen to overlap.
        static START: Barrier = Barrier::new(2);
        let other = std::thread::spawn(|| {
            let _: BarrierWaitResult = START.wait();
            allocations_during(|| {
                for value in 0_u16..1_000_u16 {
                    drop(black_box(Box::new(value)));
                }
            })
            .1
        });
        let (counted, here) = allocations_during(|| {
            let _: BarrierWaitResult = START.wait();
            other.join().ok()
        });
        assert_eq!(counted, Some(1_000), "the other thread counts its own");
        assert!(here < 1_000, "this thread's allocations: {here}");
    }
}
