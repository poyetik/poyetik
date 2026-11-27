//! Tests of the counting allocator.

#![forbid(unsafe_code)]

use core::hint::black_box;
use std::sync::{Barrier, BarrierWaitResult};

use super::{
    CountingAllocator, NotCounting, bytes_counted_by, heap_bytes_during, heap_operations_during,
    heap_reserved_during, operations_counted_by, require_counted,
};

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

/// The bytes `action` left reserved, which the installed counter always measures.
fn kept<R>(action: impl FnOnce() -> R) -> (R, usize) {
    heap_bytes_during(action).expect("the counter is installed")
}

#[test]
fn counts_the_bytes_an_action_leaves_reserved() {
    let (held, allocated) = kept(|| Box::new([7_u8; 24]));
    assert_eq!(allocated, 24, "an allocation the result holds");
    assert_eq!(*held, [7; 24], "the result is returned");
    let ((), zeroed) = kept(|| drop(black_box(vec![0_u8; 40])));
    assert_eq!(zeroed, 0, "alloc_zeroed, then its release");
    let mut grown = Vec::<u8>::with_capacity(3);
    grown.push(1);
    let ((), resized) = kept(|| {
        grown.reserve_exact(97);
        let _: &Vec<u8> = black_box(&grown);
    });
    assert_eq!(grown.capacity(), 98, "reallocated to the size asked");
    assert_eq!(
        resized, 95,
        "a reallocation keeps its new size less its old one"
    );
    let freed_first = Box::new([1_u8; 16]);
    let (smaller, released) = kept(|| {
        drop(black_box(freed_first));
        Box::new([2_u8; 4])
    });
    assert_eq!(released, 0, "released more than it reserved: nothing kept");
    assert_eq!(*smaller, [2; 4], "the smaller result");
    let freed_then = Box::new([1_u8; 16]);
    let (larger, net) = kept(|| {
        drop(black_box(freed_then));
        Box::new([3_u8; 56])
    });
    assert_eq!(net, 40, "56 reserved, 16 released");
    assert_eq!(*larger, [3; 56], "the larger result");
}

/// The bytes `action` asked for, which the installed counter always measures.
fn asked<R>(action: impl FnOnce() -> R) -> (R, usize) {
    heap_reserved_during(action).expect("the counter is installed")
}

#[test]
fn counts_the_bytes_an_action_asks_for_releases_aside() {
    let ((), reserved) = asked(|| drop(black_box(Vec::<u8>::with_capacity(1_000))));
    assert_eq!(reserved, 1_000, "an allocation released before the end");
    let (held, allocated) = asked(|| Box::new([7_u8; 24]));
    assert_eq!(
        (*held, allocated),
        ([7; 24], 24),
        "an allocation the result holds"
    );
    let ((), zeroed) = asked(|| drop(black_box(vec![0_u8; 40])));
    assert_eq!(zeroed, 40, "alloc_zeroed");
    let mut grown = Vec::<u8>::with_capacity(3);
    grown.push(1);
    let ((), resized) = asked(|| {
        grown.reserve_exact(97);
        let _: &Vec<u8> = black_box(&grown);
    });
    assert_eq!(
        (grown.capacity(), resized),
        (98, 98),
        "a reallocation asks for its new size"
    );
    let freed = Box::new([1_u8; 16]);
    let ((), released) = asked(|| drop(black_box(freed)));
    assert_eq!(released, 0, "a release asks for nothing");
    assert_eq!(
        heap_reserved_during(|| 6_i32 * 7_i32),
        Ok((42_i32, 0_usize)),
        "returns the result, and the probe is not counted"
    );
}

#[test]
fn counts_the_bytes_of_this_thread_only() {
    // As for the operations: the other thread keeps 4000 bytes inside this thread's
    // window, which a process-wide count would see every time.
    static START: Barrier = Barrier::new(2);
    let other = std::thread::spawn(|| {
        let _: BarrierWaitResult = START.wait();
        kept(|| black_box(vec![5_u8; 4_000])).1
    });
    let (joined, here) = kept(|| {
        let _: BarrierWaitResult = START.wait();
        other.join().ok()
    });
    assert_eq!(joined, Some(4_000), "the other thread counts its own");
    assert!(here < 4_000, "this thread's bytes: {here}");
}

#[test]
fn refuses_to_count_when_the_probe_is_not_counted() {
    /// A count the probe allocation does not move, as without the counting allocator.
    const fn still() -> usize {
        7
    }
    let mut ran = 0_u8;
    assert_eq!(
        operations_counted_by(still, || ran = ran.saturating_add(1)),
        Err(NotCounting),
        "operations"
    );
    assert_eq!(
        bytes_counted_by(still, || ran = ran.saturating_add(1)),
        Err(NotCounting),
        "bytes"
    );
    assert_eq!(ran, 2, "each action has run");
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
