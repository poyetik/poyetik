//! What reading the description of a module asks of the heap (`spec/box-contract.md`, section
//! 4): it follows what the module gives, never what it announces.

#![forbid(unsafe_code)]

/// Counts the heap operations of these tests.
#[global_allocator]
static ALLOCATOR: audio_testkit::CountingAllocator = audio_testkit::CountingAllocator;

#[cfg(test)]
mod tests {
    use audio_core::host::{Description, DescriptionError, Fault, Field};
    use audio_testkit::heap_operations_during;

    /// A description up to its inlets: version 1, live-capable, no instantaneous response, a
    /// name and a description of one byte.
    const START: [u8; 16] = [1, 0, 0, 0, 0, 0, 1, 0, 0, 0, b'n', 1, 0, 0, 0, b'd'];

    /// Reads a description announcing `announced` inlets and giving `given` of them, fewer,
    /// and returns the heap operations it took, the refusal being `fault`.
    fn operations(announced: u32, given: u32, fault: Fault) -> usize {
        let mut bytes = START.to_vec();
        bytes.extend(announced.to_le_bytes());
        for index in 0..given {
            bytes.extend([4, 0, 0, 0]);
            bytes.extend(format!("{index:04}").as_bytes());
            bytes.extend([1, 0, 0, 0, b'x', 0]);
        }
        let (refused, count) = heap_operations_during(|| Description::read(&bytes).err())
            .expect("the counting allocator is installed");
        assert_eq!(
            refused,
            Some(DescriptionError {
                field: Field::Inlets,
                fault
            }),
            "{given} inlets given of {announced}"
        );
        count
    }

    #[test]
    fn asks_the_heap_for_the_items_read_never_for_those_announced() {
        for announced in [1, 1_001, 65_536] {
            assert_eq!(
                operations(announced, 0, Fault::Truncated),
                0,
                "{announced} announced, none given"
            );
        }
        let read = operations(1_001, 1_000, Fault::Truncated);
        assert!(read > 0, "the control: reading inlets uses the heap");
        assert_eq!(
            operations(65_536, 1_000, Fault::Truncated),
            read,
            "65536 announced"
        );
        for announced in [65_537, u32::MAX] {
            assert_eq!(
                operations(announced, 1_000, Fault::TooMany),
                0,
                "{announced} announced, refused before any is read"
            );
        }
    }
}
