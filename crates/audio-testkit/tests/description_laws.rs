//! Laws of reading what a module gives (`spec/box-contract.md`, sections 4 and 5), over random
//! and mutated bytes and integers: the deterministic start of fuzzing (coverage-guided fuzzing
//! comes with #3 step 4).
//!
//! Whatever the bytes, `Description::read` and `Header::read` return `Ok` or `Err`, never
//! panic, and use the heap no more than the bytes given allow; a description accepted is
//! written back, from what is read of it, to the bytes it was read from; a header and two
//! runs of buffers are accepted or refused as a model in wider integers says.

#![forbid(unsafe_code)]

/// Counts the heap operations of these tests.
#[global_allocator]
static ALLOCATOR: audio_testkit::CountingAllocator = audio_testkit::CountingAllocator;

#[cfg(test)]
mod tests {
    use audio_core::host::{
        BufferRun, Description, Field, Header, HeaderError, PortKind, Response, RunError, Runs,
        RunsError,
    };
    use audio_core::{Binary32, Binary64, BoxMode, Capability, Sample};
    use audio_testkit::heap_operations_during;
    use audio_testkit::runner::{fork, runner};
    use proptest::collection::vec;
    use proptest::prelude::{Just, any, prop_oneof};
    use proptest::strategy::{Strategy, ValueTree as _};
    use proptest::test_runner::TestCaseError;

    /// Inputs tried per law.
    const CASES: u32 = 2_000;

    /// A text as the format writes it.
    fn text(value: &str) -> Vec<u8> {
        let length = u32::try_from(value.len()).expect("a text of a description");
        [length.to_le_bytes().as_slice(), value.as_bytes()].concat()
    }

    /// A list of items, as the format writes it.
    fn list(items: &[Vec<u8>]) -> Vec<u8> {
        let count = u32::try_from(items.len()).expect("a list of a description");
        [count.to_le_bytes().to_vec(), items.concat()].concat()
    }

    /// A text: ordinary, repeated often (names are few), or any.
    fn any_text() -> impl Strategy<Value = Vec<u8>> {
        prop_oneof![
            12 => "[a-z]{1,6}".prop_map(|value| text(&value)),
            2 => prop_oneof![Just("in"), Just("out"), Just("gain")].prop_map(text),
            1 => any::<String>().prop_map(|value| text(&value)),
        ]
    }

    /// A code below `known`, or rarely any.
    fn code(known: u8) -> impl Strategy<Value = Vec<u8>> {
        prop_oneof![
            19 => (0..known).prop_map(|code| vec![code]),
            1 => any::<u8>().prop_map(|code| vec![code]),
        ]
    }

    /// The bits of a parameter's value: ordinary, special, or any.
    fn value_bits() -> impl Strategy<Value = u64> {
        prop_oneof![
            8 => (-100.0_f64..100.0).prop_map(f64::to_bits),
            2 => prop_oneof![
                Just(0.0_f64),
                Just(-0.0),
                Just(1e-300),
                Just(f64::from(f32::MIN_POSITIVE)),
                Just(f64::MAX),
                Just(f64::MIN),
                Just(f64::INFINITY),
                Just(f64::NAN),
            ]
            .prop_map(f64::to_bits),
            1 => any::<u64>(),
        ]
    }

    /// A parameter's three values: mostly in order, the default between the bounds.
    fn values() -> impl Strategy<Value = Vec<u8>> {
        prop_oneof![
            6 => [-100.0_f64..100.0, -100.0_f64..100.0, -100.0_f64..100.0].prop_map(|mut given| {
                given.sort_by(f64::total_cmp);
                let [minimum, default, maximum] = given;
                [default, minimum, maximum].map(f64::to_bits)
            }),
            1 => [value_bits(), value_bits(), value_bits()],
        ]
        .prop_map(|bits| bits.map(u64::to_le_bytes).concat())
    }

    /// A list of ports.
    fn ports() -> impl Strategy<Value = Vec<u8>> {
        vec((any_text(), any_text(), code(1)), 0..4).prop_map(|ports| {
            list(
                &ports
                    .into_iter()
                    .map(|port| <[Vec<u8>; 3]>::from(port).concat())
                    .collect::<Vec<_>>(),
            )
        })
    }

    /// A description built field by field, each mostly valid.
    fn built() -> impl Strategy<Value = Vec<u8>> {
        let params = vec((any_text(), any_text(), values()), 0..4).prop_map(|params| {
            list(
                &params
                    .into_iter()
                    .map(|param| <[Vec<u8>; 3]>::from(param).concat())
                    .collect::<Vec<_>>(),
            )
        });
        let version = prop_oneof![19 => 1_u32..9, 1 => Just(0)];
        let capabilities = vec(code(1), 0..3).prop_map(|codes| list(&codes));
        (
            (version, code(2), code(3), any_text(), any_text()),
            (ports(), ports(), params, capabilities),
        )
            .prop_map(
                |((version, mode, response, name, what), (inlets, outlets, params, asked))| {
                    [
                        version.to_le_bytes().to_vec(),
                        mode,
                        response,
                        name,
                        what,
                        inlets,
                        outlets,
                        params,
                        asked,
                    ]
                    .concat()
                },
            )
    }

    /// Bytes given as a description: built, built then changed (bytes replaced, cut, added
    /// to), or any.
    fn given() -> impl Strategy<Value = Vec<u8>> {
        let changed = (
            built(),
            vec((any::<proptest::sample::Index>(), any::<u8>()), 1..4),
            any::<proptest::sample::Index>(),
            prop_oneof![Just(0_u8), Just(1), Just(2)],
        )
            .prop_map(|(mut bytes, replaced, at, form)| {
                match form {
                    0 => {
                        for (place, byte) in replaced {
                            let place = place.index(bytes.len());
                            bytes[place] = byte;
                        }
                    }
                    1 => bytes.truncate(at.index(bytes.len())),
                    _ => bytes.extend(replaced.into_iter().map(|(_, byte)| byte)),
                }
                bytes
            });
        prop_oneof![
            4 => built(),
            3 => changed,
            1 => vec(any::<u8>(), 0..200),
        ]
    }

    /// `description` as the format writes it, from what is read of it.
    fn written(description: &Description<'_>) -> Vec<u8> {
        let port = |port: &audio_core::host::DescribedPort<'_>| {
            let kind = match port.kind() {
                PortKind::Signal => 0_u8,
                other => panic!("{other:?}: a kind this law does not write"),
            };
            [text(port.name()), text(port.description()), vec![kind]].concat()
        };
        let mode = match description.mode() {
            BoxMode::LiveCapable => 0_u8,
            BoxMode::OfflineOnly => 1,
        };
        let response = match description.response() {
            Response::None => 0_u8,
            Response::Linear => 1,
            Response::NonLinear => 2,
            other => panic!("{other:?}: a response this law does not write"),
        };
        let params: Vec<_> = description
            .params()
            .iter()
            .map(|param| {
                let bits = [
                    param.default_bits(),
                    param.minimum_bits(),
                    param.maximum_bits(),
                ];
                [
                    text(param.name()),
                    text(param.description()),
                    bits.map(u64::to_le_bytes).concat(),
                ]
                .concat()
            })
            .collect();
        let asked: Vec<_> = description
            .capabilities()
            .iter()
            .map(|capability| match capability {
                Capability::AudioOutput => vec![0_u8],
                other => panic!("{other:?}: a capability this law does not write"),
            })
            .collect();
        [
            description.algorithm_version().get().to_le_bytes().to_vec(),
            vec![mode, response],
            text(description.name()),
            text(description.description()),
            list(&description.inlets().iter().map(port).collect::<Vec<_>>()),
            list(&description.outlets().iter().map(port).collect::<Vec<_>>()),
            list(&params),
            list(&asked),
        ]
        .concat()
    }

    /// Checks one description: no more heap operations than its bytes allow, and its bytes
    /// written back when accepted.
    ///
    /// Every item takes a byte at least. A list of n items grows its vector n times at most,
    /// releases it once, and makes and releases n parts of its set of keys at most: 1 growth, 1
    /// release, 1 part made and 1 released for a list of one, fewer an item past it.
    fn reads(bytes: &[u8]) -> Result<bool, TestCaseError> {
        let (read, operations) = heap_operations_during(|| Description::read(bytes))
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        if operations > bytes.len().saturating_mul(4) {
            return Err(TestCaseError::fail(format!(
                "{operations} heap operations for {} bytes",
                bytes.len()
            )));
        }
        match read {
            Err(_) => Ok(false),
            Ok(description) if written(&description) == bytes => Ok(true),
            Ok(description) => Err(TestCaseError::fail(format!(
                "{description:?} is not what {bytes:?} holds"
            ))),
        }
    }

    #[test]
    fn a_description_is_refused_or_holds_its_bytes_never_panics() {
        assert_eq!(
            runner(CASES).run(&given(), |bytes| reads(&bytes).map(|_| ())),
            Ok(()),
            "descriptions"
        );
    }

    /// The control of the law: its bytes reach acceptance and refusals of every field.
    #[test]
    fn the_descriptions_reach_acceptance_and_every_field() {
        let mut runner = runner(CASES);
        let strategy = given();
        let mut accepted = 0_u32;
        let mut refused = Vec::new();
        for _ in 0..CASES {
            let bytes = strategy
                .new_tree(&mut runner)
                .expect("a description")
                .current();
            match Description::read(&bytes) {
                Ok(description) => {
                    accepted = accepted.saturating_add(u32::from(
                        !description.params().is_empty() && !description.inlets().is_empty(),
                    ));
                }
                Err(error) => {
                    refused.push(match error.field {
                        Field::Whole => "whole",
                        Field::AlgorithmVersion => "algorithm version",
                        Field::Mode => "mode",
                        Field::Response => "response",
                        Field::Name => "name",
                        Field::DescriptionText => "description text",
                        Field::Inlets => "inlets",
                        Field::Inlet { .. } => "inlet",
                        Field::Outlets => "outlets",
                        Field::Outlet { .. } => "outlet",
                        Field::Params => "params",
                        Field::Param { .. } => "param",
                        Field::Capabilities => "capabilities",
                        Field::Capability { .. } => "capability",
                        Field::End => "end",
                        _ => "another",
                    });
                }
            }
        }
        refused.sort_unstable();
        refused.dedup();
        assert!(
            accepted >= 100,
            "{accepted} accepted with ports and parameters"
        );
        let every = [
            "algorithm version",
            "capabilities",
            "capability",
            "description text",
            "end",
            "inlet",
            "inlets",
            "mode",
            "name",
            "outlet",
            "outlets",
            "param",
            "params",
            "response",
        ];
        assert_eq!(
            refused, every,
            "every field but the whole, refused above 2^24 bytes"
        );
    }

    /// An integer a module gives: small, or any.
    fn integer() -> impl Strategy<Value = u32> {
        prop_oneof![3 => 0_u32..40, 1 => any::<u32>()]
    }

    #[test]
    fn a_header_is_read_within_its_memory_or_refused() {
        let memory = (integer(), integer(), vec(any::<u8>(), 0..40), 0_usize..4).prop_map(
            |(version, length, rest, before)| {
                [
                    vec![0xee; before],
                    version.to_le_bytes().to_vec(),
                    length.to_le_bytes().to_vec(),
                    rest,
                ]
                .concat()
            },
        );
        let result = runner(CASES).run(&(memory, integer()), |(memory, offset)| {
            let refused_as = HeaderError::Header {
                offset,
                memory: wide(memory.len()),
            };
            // The model: its two integers, then its description, each within the memory.
            let expected = within(&memory, offset.into(), 8)
                .ok_or(refused_as)
                .and_then(|integers| {
                    let mut parts = integers.into_iter();
                    let version = little(&mut parts);
                    let length = little(&mut parts);
                    let first = u64::from(offset).saturating_add(8);
                    within(&memory, first, length.into())
                        .map(|description| (version, description))
                        .ok_or_else(|| HeaderError::Description {
                            offset: first,
                            length,
                            memory: wide(memory.len()),
                        })
                });
            let read = Header::read(offset, &memory).map(|header| {
                (
                    header.version().packed(),
                    header.description_bytes().to_vec(),
                )
            });
            if read == expected {
                Ok(())
            } else {
                Err(TestCaseError::fail(format!("{read:?}, not {expected:?}")))
            }
        });
        assert_eq!(result, Ok(()), "headers");
    }

    /// `length` as 64 bits.
    fn wide(length: usize) -> u64 {
        u64::try_from(length).expect("a memory of this test")
    }

    /// The next four bytes, as a little-endian integer, one byte at a time.
    fn little(bytes: &mut impl Iterator<Item = u8>) -> u32 {
        (0..4).fold(0, |value, place| {
            let byte = u32::from(bytes.next().expect("a byte"));
            value.saturating_add(byte.saturating_mul(256_u32.saturating_pow(place)))
        })
    }

    /// The `length` bytes of `memory` from byte `first`, if it holds them.
    fn within(memory: &[u8], first: u64, length: u64) -> Option<Vec<u8>> {
        let after = first.saturating_add(length);
        (after <= wide(memory.len())).then(|| {
            let held = (0_u64..).zip(memory);
            held.filter(|(at, _)| (first..after).contains(at))
                .map(|(_, byte)| *byte)
                .collect()
        })
    }

    /// The description of a box type of `inlets` inlets and `outlets` outlets.
    fn of_ports(inlets: u32, outlets: u32) -> Vec<u8> {
        let ports = |count: u32| {
            let items: Vec<_> = (0..count)
                .map(|index| [text(&format!("p{index}")), text("x"), vec![0]].concat())
                .collect();
            list(&items)
        };
        [
            vec![1, 0, 0, 0, 0, 0],
            text("n~"),
            text("x"),
            ports(inlets),
            ports(outlets),
            list(&[]),
            list(&[]),
        ]
        .concat()
    }

    /// The run of `buffers` buffers of `frames` samples of `bytes` bytes at `offset` of a memory
    /// of `memory` bytes: where it ends, or why it is refused, in 128 bits.
    fn modeled(
        offset: u32,
        frames: u32,
        buffers: u32,
        bytes: u128,
        memory: u64,
    ) -> Result<u128, RunError> {
        if !offset.is_multiple_of(64) {
            return Err(RunError::Misaligned { offset });
        }
        if frames == 0 || frames > 1_048_576 {
            return Err(RunError::Span { frames });
        }
        let lines = u128::from(frames).saturating_mul(bytes).div_ceil(64);
        let run = lines.saturating_mul(64).saturating_mul(u128::from(buffers));
        let end = u128::from(offset).saturating_add(run);
        if end > u128::from(memory) {
            // Below 2^32 + 2^32 x 2^23.
            let end = u64::try_from(end).unwrap_or(u64::MAX);
            return Err(RunError::OutsideMemory { end, memory });
        }
        Ok(end)
    }

    /// Checks the runs at `offsets` of a module of `ports` ports, for the sample type `S` of
    /// `bytes` bytes.
    fn runs<S: Sample>(
        offsets: [u32; 2],
        frames: u32,
        ports: [u32; 2],
        bytes: u128,
        memory: u64,
    ) -> Result<(), TestCaseError> {
        let given = of_ports(ports[0], ports[1]);
        let description =
            Description::read(&given).map_err(|error| TestCaseError::fail(error.to_string()))?;
        let taken = modeled(offsets[0], frames, ports[0], bytes, memory);
        let told = modeled(offsets[1], frames, ports[1], bytes, memory);
        // Each within a memory of 64 bits.
        let narrow = |end: u128| u64::try_from(end).unwrap_or(u64::MAX);
        let expected = match (taken, told) {
            (Err(refused), _) => Err(RunsError::Inputs(refused)),
            (_, Err(refused)) => Err(RunsError::Outputs(refused)),
            (Ok(first), Ok(second)) => {
                let [start, after] = [u128::from(offsets[0]), u128::from(offsets[1])];
                // A byte of each: the last of the two starts, if both hold it.
                let last = if start > after { start } else { after };
                if last < first && last < second {
                    Err(RunsError::Overlap {
                        inputs: [offsets[0].into(), narrow(first)],
                        outputs: [offsets[1].into(), narrow(second)],
                    })
                } else {
                    Ok([first, second].map(narrow))
                }
            }
        };
        let read = Runs::<S>::new(&description, offsets, frames, memory)
            .map(|runs| [runs.inputs(), runs.outputs()].map(BufferRun::end));
        if read == expected {
            Ok(())
        } else {
            Err(TestCaseError::fail(format!("{read:?}, not {expected:?}")))
        }
    }

    #[test]
    fn runs_of_buffers_are_accepted_or_refused_as_wider_integers_say() {
        let offset = || {
            prop_oneof![
                6 => (0_u32..64).prop_map(|line| line * 64),
                1 => (0_u32..64).prop_map(|line| u32::MAX - 63 - line * 64),
                1 => any::<u32>(),
            ]
        };
        let frames = prop_oneof![
            6 => 1_u32..65,
            1 => prop_oneof![Just(0_u32), Just(1_048_576), Just(1_048_577), Just(u32::MAX)],
            1 => any::<u32>(),
        ];
        let memory =
            prop_oneof![3 => 0_u64..16_384, 2 => Just(0x1_0000_0000_u64), 1 => any::<u64>()];
        let given = ([offset(), offset()], frames, [0_u32..4, 0_u32..4], memory);
        let mut first = runner(CASES);
        let mut second = fork(&mut first, CASES);
        assert_eq!(
            first.run(&given, |(offsets, span, ports, bytes)| {
                runs::<Binary64>(offsets, span, ports, 8, bytes)
            }),
            Ok(()),
            "binary64"
        );
        assert_eq!(
            second.run(&given, |(offsets, span, ports, bytes)| {
                runs::<Binary32>(offsets, span, ports, 4, bytes)
            }),
            Ok(()),
            "binary32"
        );
    }
}
