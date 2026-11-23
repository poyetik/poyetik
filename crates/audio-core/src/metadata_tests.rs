//! Tests of the metadata of a box type and of their reader.

#![forbid(unsafe_code)]

use alloc::string::ToString;
use alloc::vec;
use alloc::vec::Vec;

use super::{
    DescribedParam, DescribedPort, Fault, Field, Metadata, MetadataRefusal, PortKind, Response,
};
use crate::{BoxMode, Capability, ParamSpec, ParamSpecError};

/// A text as the format writes it.
fn text(value: &[u8]) -> Vec<u8> {
    let length = u32::try_from(value.len()).expect("a text of a description");
    [length.to_le_bytes().as_slice(), value].concat()
}

/// A port: its name, its description, its kind.
fn port(name: &str, description: &str, kind: u8) -> Vec<u8> {
    [
        text(name.as_bytes()),
        text(description.as_bytes()),
        vec![kind],
    ]
    .concat()
}

/// A parameter: its name, its description, then its three values' bits.
fn param(name: &str, description: &str, values: [f64; 3]) -> Vec<u8> {
    let bits = values.map(|value| value.to_bits().to_le_bytes()).concat();
    [text(name.as_bytes()), text(description.as_bytes()), bits].concat()
}

/// A list of items, as the format writes it.
fn list(items: &[Vec<u8>]) -> Vec<u8> {
    let count = u32::try_from(items.len()).expect("a list of a description");
    [count.to_le_bytes().to_vec(), items.concat()].concat()
}

/// The fields of a description, each replaceable: what a test changes.
fn fields() -> [Vec<u8>; 9] {
    [
        vec![7, 0, 0, 0],
        vec![1],
        vec![2],
        text(b"fold~"),
        text("Folds, \u{e9}crit ici".as_bytes()),
        list(&[port("in", "A signal", 0), port("edge", "Jumps", 1)]),
        list(&[port("out", "The folded signal", 0)]),
        list(&[param("gain", "Before folding", [0.5, -1.0, 2.0])]),
        list(&[vec![0]]),
    ]
}

/// [`fields`], with field `index` replaced by `replaced`, as the bytes of a description.
fn with(index: usize, replaced: &[u8]) -> Vec<u8> {
    fields()
        .iter()
        .enumerate()
        .flat_map(|(at, field)| if at == index { replaced } else { field })
        .copied()
        .collect()
}

/// What reading [`with`] refuses.
fn refusal(index: usize, replaced: &[u8]) -> Option<MetadataRefusal> {
    Metadata::read(&with(index, replaced)).err()
}

/// A refusal of `field` for `fault`.
const fn refused(field: Field, fault: Fault) -> MetadataRefusal {
    MetadataRefusal { field, fault }
}

#[test]
fn reads_every_field() {
    let bytes = fields().concat();
    let read = Metadata::read(&bytes).expect("a valid description");
    assert_eq!(read.algorithm_version().get(), 7, "algorithm version");
    assert_eq!(read.mode(), BoxMode::OfflineOnly, "mode");
    assert_eq!(read.response(), Response::NonLinear, "response");
    assert_eq!(read.name(), "fold~", "name");
    assert_eq!(read.description(), "Folds, \u{e9}crit ici", "description");
    let ports: Vec<_> = read
        .inlets()
        .iter()
        .chain(read.outlets())
        .map(|port| (port.name(), port.description(), port.kind()))
        .collect();
    assert_eq!(
        ports,
        [
            ("in", "A signal", PortKind::Signal),
            ("edge", "Jumps", PortKind::Discontinuities),
            ("out", "The folded signal", PortKind::Signal),
        ],
        "ports"
    );
    assert_eq!((read.inlets().len(), read.outlets().len()), (2, 1), "ports");
    let params: Vec<_> = read
        .params()
        .iter()
        .map(|param| (param.name(), param.description(), param.bits()))
        .collect();
    assert_eq!(
        params,
        [(
            "gain",
            "Before folding",
            [
                0x3fe0_0000_0000_0000,
                0xbff0_0000_0000_0000,
                0x4000_0000_0000_0000
            ]
        )],
        "parameters"
    );
    assert_eq!(
        read.capabilities(),
        [Capability::AudioOutput],
        "capabilities"
    );
}

#[test]
fn reads_each_code_of_the_mode_and_the_response() {
    for (code, mode) in [(0, BoxMode::LiveCapable), (1, BoxMode::OfflineOnly)] {
        let mut changed = fields();
        changed[1] = vec![code];
        let bytes = changed.concat();
        assert_eq!(
            Metadata::read(&bytes).map(|read| read.mode()),
            Ok(mode),
            "mode {code}"
        );
    }
    for (code, response) in [
        (0, Response::None),
        (1, Response::Linear),
        (2, Response::NonLinear),
    ] {
        let mut changed = fields();
        changed[2] = vec![code];
        let bytes = changed.concat();
        let read = Metadata::read(&bytes).map(|read| read.response());
        assert_eq!(read, Ok(response), "response {code}");
    }
}

#[test]
fn refuses_every_truncation_and_any_byte_after_the_last() {
    let bytes = fields().concat();
    for length in 0..bytes.len() {
        let cut = bytes.get(..length).expect("shorter");
        let fault = Metadata::read(cut).expect_err("truncated").fault;
        assert_eq!(fault, Fault::Truncated, "the first {length} bytes");
    }
    let longer = [bytes, vec![0]].concat();
    assert_eq!(
        Metadata::read(&longer).err(),
        Some(refused(Field::End, Fault::Trailing)),
        "one byte more"
    );
}

#[test]
fn refuses_each_code_it_does_not_know() {
    for code in [2, 255] {
        assert_eq!(
            refusal(1, &[code]),
            Some(refused(Field::Mode, Fault::Code(code))),
            "mode"
        );
        let ports = list(&[port("in", "x", code)]);
        assert_eq!(
            refusal(5, &ports),
            Some(refused(Field::Inlets, Fault::Code(code))),
            "inlet"
        );
        assert_eq!(
            refusal(6, &ports),
            Some(refused(Field::Outlets, Fault::Code(code))),
            "outlet"
        );
    }
    for code in [3, 255] {
        assert_eq!(
            refusal(2, &[code]),
            Some(refused(Field::Response, Fault::Code(code))),
            "response"
        );
    }
    for code in [1, 255] {
        let asked = list(&[vec![code]]);
        assert_eq!(
            refusal(8, &asked),
            Some(refused(Field::Capabilities, Fault::Code(code))),
            "capability"
        );
    }
}

#[test]
fn refuses_algorithm_version_0() {
    assert_eq!(
        refusal(0, &[0; 4]),
        Some(refused(Field::AlgorithmVersion, Fault::Zero)),
        "versions count from 1"
    );
    assert_eq!(refusal(0, &[0xff; 4]), None, "the largest version");
}

/// The bytes of a description whose description text has `length` bytes.
fn described(length: usize) -> Vec<u8> {
    with(4, &text("d".repeat(length).as_bytes()))
}

#[test]
fn bounds_the_whole_description_and_nothing_inside_it() {
    let rest = described(0).len();
    let longest = 16_777_216_usize.checked_sub(rest).expect("a short rest");
    let bytes = described(longest);
    assert_eq!(bytes.len(), 16_777_216, "the limit, as a literal");
    let read = Metadata::read(&bytes).expect("a text as long as the limit allows");
    assert_eq!(read.description().len(), longest, "read whole");
    assert_eq!(
        Metadata::read(&described(longest.saturating_add(1))).err(),
        Some(refused(Field::Whole, Fault::TooLong)),
        "one byte more"
    );
}

#[test]
fn reads_names_and_lists_past_16_bits() {
    let name = "n".repeat(65_536);
    assert_eq!(refusal(3, &text(name.as_bytes())), None, "name");
    let ports = list(&[port(&name, &name, 0)]);
    assert_eq!(refusal(5, &ports), None, "port");
    let params = list(&[param(&name, &name, [0.0_f64; 3])]);
    assert_eq!(refusal(7, &params), None, "parameter");
    let many: Vec<_> = (0..65_537_u32)
        .map(|index| port(&index.to_string(), "x", 0))
        .collect();
    let bytes = with(6, &list(&many));
    let read = Metadata::read(&bytes).expect("65,537 outlets");
    assert_eq!(read.outlets().len(), 65_537, "outlets");
    let last = read.outlets().last().map(DescribedPort::name);
    assert_eq!(last, Some("65536"), "in order");
}

#[test]
fn refuses_a_length_or_a_count_beyond_the_bytes_left() {
    for announced in [8, 65_536, 2_147_483_648, u32::MAX] {
        let short = [&u32::to_le_bytes(announced)[..], b"few"].concat();
        for (index, field) in [
            (3, Field::Name),
            (4, Field::Description),
            (5, Field::Inlets),
            (6, Field::Outlets),
            (7, Field::Params),
        ] {
            let mut bytes = fields()
                .into_iter()
                .take(index)
                .collect::<Vec<_>>()
                .concat();
            bytes.extend(&short);
            assert_eq!(
                Metadata::read(&bytes).err(),
                Some(refused(field, Fault::Truncated)),
                "{announced} announced in {field}"
            );
        }
        let mut bytes = fields().into_iter().take(8).collect::<Vec<_>>().concat();
        bytes.extend(u32::to_le_bytes(announced));
        bytes.push(0);
        assert_eq!(
            Metadata::read(&bytes).err(),
            Some(refused(Field::Capabilities, Fault::Truncated)),
            "{announced} capabilities announced, one given"
        );
    }
}

#[test]
fn refuses_blank_texts_and_whitespace_in_the_name() {
    for blank in ["", " ", "\t\n", "\u{2003}"] {
        assert_eq!(
            refusal(3, &text(blank.as_bytes())),
            Some(refused(Field::Name, Fault::Blank)),
            "name"
        );
        assert_eq!(
            refusal(4, &text(blank.as_bytes())),
            Some(refused(Field::Description, Fault::Blank)),
            "description"
        );
        for (name, said) in [(blank, "x"), ("p", blank)] {
            assert_eq!(
                refusal(6, &list(&[port(name, said, 0)])),
                Some(refused(Field::Outlets, Fault::Blank)),
                "port"
            );
            assert_eq!(
                refusal(7, &list(&[param(name, said, [0.0_f64; 3])])),
                Some(refused(Field::Params, Fault::Blank)),
                "parameter"
            );
        }
    }
    for spaced in ["a b", " a", "a\u{a0}"] {
        assert_eq!(
            refusal(3, &text(spaced.as_bytes())),
            Some(refused(Field::Name, Fault::Whitespace)),
            "{spaced:?}"
        );
    }
    assert_eq!(
        refusal(5, &list(&[port("a b", "x", 0)])),
        None,
        "a port's name may hold one"
    );
}

#[test]
fn refuses_text_that_is_not_utf_8() {
    for bytes in [&[0xff][..], &[0xc3], &[0xed, 0xa0, 0x80], &[0xc0, 0xaf]] {
        assert_eq!(
            refusal(4, &text(bytes)),
            Some(refused(Field::Description, Fault::NotUtf8)),
            "{bytes:?}"
        );
    }
}

#[test]
fn refuses_what_repeats() {
    let twice = list(&[port("a", "x", 0), port("b", "x", 0), port("a", "y", 1)]);
    assert_eq!(
        refusal(5, &twice),
        Some(refused(Field::Inlets, Fault::Repeated)),
        "inlets"
    );
    assert_eq!(
        refusal(6, &twice),
        Some(refused(Field::Outlets, Fault::Repeated)),
        "outlets"
    );
    let params = list(&[param("a", "x", [0.0_f64; 3]), param("a", "y", [1.0_f64; 3])]);
    assert_eq!(
        refusal(7, &params),
        Some(refused(Field::Params, Fault::Repeated)),
        "parameters"
    );
    let asked = list(&[vec![0], vec![0]]);
    assert_eq!(
        refusal(8, &asked),
        Some(refused(Field::Capabilities, Fault::Repeated)),
        "capabilities"
    );
    assert_eq!(
        refusal(6, &list(&[port("in", "x", 0)])),
        None,
        "an outlet may have an inlet's name"
    );
}

#[test]
fn refuses_a_parameter_whose_range_or_default_no_value_meets() {
    for (values, fault) in [
        ([0.0_f64, 1.0_f64, 0.0_f64], Fault::Range),
        ([0.0_f64, f64::NEG_INFINITY, 1.0_f64], Fault::Range),
        ([0.0_f64, 0.0_f64, f64::INFINITY], Fault::Range),
        ([0.0_f64, f64::NAN, 1.0_f64], Fault::Range),
        ([0.0_f64, 0.0_f64, f64::NAN], Fault::Range),
        ([0.5_f64, 1e-40_f64, 1.0_f64], Fault::Range),
        ([0.0_f64, -1.0_f64, 1e-310_f64], Fault::Range),
        ([1.5_f64, 0.0_f64, 1.0_f64], Fault::Default),
        ([-0.5_f64, 0.0_f64, 1.0_f64], Fault::Default),
        ([f64::NAN, 0.0_f64, 1.0_f64], Fault::Default),
        ([1e-40_f64, SMALLEST_NORMAL, 1.0_f64], Fault::Default),
    ] {
        assert_eq!(
            refusal(7, &list(&[param("p", "x", values)])),
            Some(refused(Field::Params, fault)),
            "{values:?}"
        );
    }
}

/// 2^-126, the smallest normal binary32 value, below which a parameter is zero (decision 5).
const SMALLEST_NORMAL: f64 = 1.175_494_350_822_287_5e-38;

#[test]
fn reads_parameters_over_the_whole_of_binary64_bit_for_bit() {
    for values in [
        [1.0_f64, 0.0_f64, 1.0_f64],
        [0.0_f64, -0.0_f64, 0.0_f64],
        [0.1_f64, 0.1_f64, 0.1_f64],
        [1e300_f64, -f64::MAX, f64::MAX],
        [SMALLEST_NORMAL, SMALLEST_NORMAL, 1.0_f64],
        [-1.0_f64, -1.0_f64, -SMALLEST_NORMAL],
        [1e-310_f64, -1.0_f64, 1.0_f64],
    ] {
        let bytes = with(7, &list(&[param("p", "x", values)]));
        let read = Metadata::read(&bytes).expect("a range its default is in");
        let bits: Vec<_> = read.params().iter().map(DescribedParam::bits).collect();
        assert_eq!(bits, [values.map(f64::to_bits)], "{values:?}");
    }
}

#[test]
fn accepts_of_binary32_values_exactly_what_a_built_in_box_may_declare() {
    let edges = [
        0.0_f32,
        -0.0,
        1e-40,
        -1e-40,
        f32::MIN_POSITIVE,
        -f32::MIN_POSITIVE,
        0.1,
        1.0,
        -1.0,
        f32::MAX,
        f32::MIN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NAN,
    ];
    let mut accepted = 0_u32;
    for default in edges {
        for minimum in edges {
            for maximum in edges {
                let values = [default, minimum, maximum].map(f64::from);
                let expected = match ParamSpec::new("p", "x", default, minimum, maximum) {
                    Ok(_) => None,
                    Err(ParamSpecError::InvalidRange { .. }) => Some(Fault::Range),
                    Err(ParamSpecError::DefaultOutsideRange { .. }) => Some(Fault::Default),
                };
                accepted = accepted.saturating_add(u32::from(expected.is_none()));
                assert_eq!(
                    refusal(7, &list(&[param("p", "x", values)])),
                    expected.map(|fault| refused(Field::Params, fault)),
                    "default {default:?}, {minimum:?} to {maximum:?}"
                );
            }
        }
    }
    assert!(
        accepted > 100,
        "the control: {accepted} combinations accepted"
    );
}

#[test]
fn says_the_field_and_the_fault() {
    for (field, fault, said) in [
        (
            Field::AlgorithmVersion,
            Fault::Zero,
            "algorithm version: is 0, versions count from 1",
        ),
        (
            Field::Mode,
            Fault::Code(9),
            "execution mode: 9 is not a code of the box contract",
        ),
        (
            Field::Response,
            Fault::Truncated,
            "instantaneous response: the description ends before it does",
        ),
        (Field::Name, Fault::Whitespace, "name: holds whitespace"),
        (Field::Description, Fault::Blank, "description: is blank"),
        (
            Field::Whole,
            Fault::TooLong,
            "description of the box type: longer than 16777216 bytes",
        ),
        (Field::Inlets, Fault::Blank, "inlets: is blank"),
        (Field::Outlets, Fault::Repeated, "outlets: an item repeats"),
        (
            Field::Params,
            Fault::Range,
            "parameters: a range is not one a parameter may have",
        ),
        (
            Field::Params,
            Fault::Default,
            "parameters: a default is outside its range",
        ),
        (
            Field::Capabilities,
            Fault::Truncated,
            "capabilities: the description ends before it does",
        ),
        (
            Field::Description,
            Fault::NotUtf8,
            "description: a text is not UTF-8",
        ),
        (
            Field::End,
            Fault::Trailing,
            "end: bytes follow the last field",
        ),
    ] {
        assert_eq!(
            MetadataRefusal { field, fault }.to_string(),
            said,
            "{field:?}"
        );
    }
}
