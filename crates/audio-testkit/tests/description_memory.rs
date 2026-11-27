//! What reading the description of a module asks of the heap (`spec/box-contract.md`, section
//! 4): it follows what the module gives, never what it announces, in every list it reads.

#![forbid(unsafe_code)]

/// Counts the heap operations and bytes of these tests.
#[global_allocator]
static ALLOCATOR: audio_testkit::CountingAllocator = audio_testkit::CountingAllocator;

#[cfg(test)]
mod tests {
    use audio_core::host::{Description, Fault, Field, Part};
    use audio_testkit::heap_reserved_during;

    /// A description up to its inlets: version 1, live-capable, no instantaneous response, a
    /// name and a description of one byte.
    const START: [u8; 16] = [1, 0, 0, 0, 0, 0, 1, 0, 0, 0, b'n', 1, 0, 0, 0, b'd'];

    /// A list of a description, each read by the same reader of a counted list.
    #[derive(Debug, Clone, Copy)]
    enum List {
        /// The inlets.
        Inlets,
        /// The outlets.
        Outlets,
        /// The parameters.
        Params,
        /// The capabilities, of which only one may be given: one code is known, and a list
        /// holds each once.
        Capabilities,
    }

    impl List {
        /// The lists before this one, each announcing none.
        const fn before(self) -> usize {
            match self {
                Self::Inlets => 0,
                Self::Outlets => 1,
                Self::Params => 2,
                Self::Capabilities => 3,
            }
        }

        /// The bytes of item `index`, whose name is unique.
        fn item(self, index: u32) -> Vec<u8> {
            let mut item = vec![];
            if matches!(self, Self::Capabilities) {
                item.push(0);
                return item;
            }
            item.extend([4, 0, 0, 0]);
            item.extend(format!("{index:04}").as_bytes());
            item.extend([1, 0, 0, 0, b'x']);
            if matches!(self, Self::Params) {
                for value in [0.0_f64, 0.0_f64, 1.0_f64] {
                    item.extend(value.to_bits().to_le_bytes());
                }
            } else {
                item.push(0);
            }
            item
        }

        /// The field a description cut before item `index` refuses.
        const fn cut(self, index: u32) -> Field {
            match self {
                Self::Inlets => Field::Inlet {
                    index,
                    part: Part::Name,
                },
                Self::Outlets => Field::Outlet {
                    index,
                    part: Part::Name,
                },
                Self::Params => Field::Param {
                    index,
                    part: Part::Name,
                },
                Self::Capabilities => Field::Capability { index },
            }
        }
    }

    /// Reads a description whose list `list` announces `announced` items and gives `given`
    /// of them, fewer, and returns the bytes it asked the heap for, what it released
    /// included, the refusal being the cut before item `given`.
    fn reserved(list: List, announced: u32, given: u32) -> usize {
        let mut bytes = START.to_vec();
        for _ in 0..list.before() {
            bytes.extend(0_u32.to_le_bytes());
        }
        bytes.extend(announced.to_le_bytes());
        for index in 0..given {
            bytes.extend(list.item(index));
        }
        let (refused, count) = heap_reserved_during(|| Description::read(&bytes).err())
            .expect("the counting allocator is installed");
        assert_eq!(
            refused.map(|error| (error.field, error.fault)),
            Some((list.cut(given), Fault::Truncated)),
            "{list:?}: {given} given of {announced}"
        );
        count
    }

    #[test]
    fn asks_the_heap_for_the_items_read_never_for_those_announced() {
        for (list, given) in [
            (List::Inlets, 1_000),
            (List::Outlets, 1_000),
            (List::Params, 1_000),
            (List::Capabilities, 1),
        ] {
            let largest = if matches!(list, List::Inlets | List::Outlets) {
                65_536
            } else {
                u32::MAX
            };
            for announced in [1, 1_001, largest] {
                assert_eq!(
                    reserved(list, announced, 0),
                    0,
                    "{list:?}: {announced} announced, none given"
                );
            }
            let read = reserved(list, given + 1, given);
            assert!(read > 0, "the control: reading {list:?} uses the heap");
            for announced in [given + 64, 65_536, largest] {
                assert_eq!(
                    reserved(list, announced, given),
                    read,
                    "{list:?}: {announced} announced"
                );
            }
        }
    }

    #[test]
    fn refuses_more_ports_than_the_limit_before_any_is_read() {
        for announced in [65_537, u32::MAX] {
            let mut bytes = START.to_vec();
            bytes.extend(announced.to_le_bytes());
            for index in 0..1_000 {
                bytes.extend(List::Inlets.item(index));
            }
            let (refused, count) = heap_reserved_during(|| Description::read(&bytes).err())
                .expect("the counting allocator is installed");
            assert_eq!(
                (refused.map(|error| (error.field, error.fault)), count),
                (
                    Some((Field::Inlets, Fault::TooMany { count: announced })),
                    0
                ),
                "{announced} announced"
            );
        }
    }
}
