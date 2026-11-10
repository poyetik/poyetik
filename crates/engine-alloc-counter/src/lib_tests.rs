//! Tests of the counting allocator.

#![forbid(unsafe_code)]

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
