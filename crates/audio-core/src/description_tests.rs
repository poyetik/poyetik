//! Tests of the description of a box type and of its reader.

#![forbid(unsafe_code)]

use alloc::format;
use alloc::string::ToString;
use alloc::vec;
use alloc::vec::Vec;

use super::{
    DescribedParam, DescribedPort, Description, DescriptionError, Fault, Field, PORTS_MAX, Part,
    PortKind, RESERVED_PORT_KINDS, Response,
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
        list(&[port("in", "A signal", 0), port("edge", "Jumps", 0)]),
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
fn refusal(index: usize, replaced: &[u8]) -> Option<DescriptionError> {
    Description::read(&with(index, replaced)).err()
}

/// The field of part `part` of inlet `index`.
const fn inlet(index: u32, part: Part) -> Field {
    Field::Inlet { index, part }
}

/// The field of part `part` of outlet `index`.
const fn outlet(index: u32, part: Part) -> Field {
    Field::Outlet { index, part }
}

/// The field of part `part` of parameter `index`.
const fn param_at(index: u32, part: Part) -> Field {
    Field::Param { index, part }
}

/// A refusal of `field` for `fault`.
const fn refused(field: Field, fault: Fault) -> DescriptionError {
    DescriptionError { field, fault }
}

#[test]
fn reads_every_field() {
    let bytes = fields().concat();
    let read = Description::read(&bytes).expect("a valid description");
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
            ("edge", "Jumps", PortKind::Signal),
            ("out", "The folded signal", PortKind::Signal),
        ],
        "ports"
    );
    assert_eq!((read.inlets().len(), read.outlets().len()), (2, 1), "ports");
    let params: Vec<_> = read
        .params()
        .iter()
        .map(|param| {
            (
                param.name(),
                param.description(),
                [
                    param.default_bits(),
                    param.minimum_bits(),
                    param.maximum_bits(),
                ],
            )
        })
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
            Description::read(&bytes).map(|read| read.mode()),
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
        let read = Description::read(&bytes).map(|read| read.response());
        assert_eq!(read, Ok(response), "response {code}");
    }
}

/// The bytes of [`fields`], each piece with the field a cut inside it names: a list's
/// count, then each part of each item.
fn pieces() -> Vec<(Vec<u8>, Field)> {
    let count = |items: u32| items.to_le_bytes().to_vec();
    vec![
        (vec![7, 0, 0, 0], Field::AlgorithmVersion),
        (vec![1], Field::Mode),
        (vec![2], Field::Response),
        (text(b"fold~"), Field::Name),
        (
            text("Folds, \u{e9}crit ici".as_bytes()),
            Field::DescriptionText,
        ),
        (count(2), Field::Inlets),
        (text(b"in"), inlet(0, Part::Name)),
        (text(b"A signal"), inlet(0, Part::DescriptionText)),
        (vec![0], inlet(0, Part::Kind)),
        (text(b"edge"), inlet(1, Part::Name)),
        (text(b"Jumps"), inlet(1, Part::DescriptionText)),
        (vec![0], inlet(1, Part::Kind)),
        (count(1), Field::Outlets),
        (text(b"out"), outlet(0, Part::Name)),
        (text(b"The folded signal"), outlet(0, Part::DescriptionText)),
        (vec![0], outlet(0, Part::Kind)),
        (count(1), Field::Params),
        (text(b"gain"), param_at(0, Part::Name)),
        (text(b"Before folding"), param_at(0, Part::DescriptionText)),
        (
            [0.5_f64, -1.0_f64, 2.0_f64]
                .map(|value| value.to_bits().to_le_bytes())
                .concat(),
            param_at(0, Part::Values),
        ),
        (count(1), Field::Capabilities),
        (vec![0], Field::Capability { index: 0 }),
    ]
}

#[test]
fn refuses_every_truncation_naming_the_field_it_cuts() {
    let bytes = fields().concat();
    let pieces = pieces();
    let joined: Vec<u8> = pieces.iter().flat_map(|(piece, _)| piece.clone()).collect();
    assert_eq!(joined, bytes, "the pieces are the fields");
    // The field each byte belongs to: a cut before a byte leaves that byte's field short.
    let owners: Vec<_> = pieces
        .iter()
        .flat_map(|(piece, field)| vec![*field; piece.len()])
        .collect();
    for (length, &field) in owners.iter().enumerate() {
        assert_eq!(
            Description::read(bytes.get(..length).expect("shorter")),
            Err(refused(field, Fault::Truncated)),
            "the first {length} bytes"
        );
    }
}

#[test]
fn refuses_any_byte_after_the_last() {
    let longer = [fields().concat(), vec![0]].concat();
    assert_eq!(
        Description::read(&longer),
        Err(refused(Field::End, Fault::Trailing { bytes: 1 })),
        "one byte more"
    );
    let much_longer = [fields().concat(), vec![0; 300]].concat();
    assert_eq!(
        Description::read(&much_longer),
        Err(refused(Field::End, Fault::Trailing { bytes: 300 })),
        "300 bytes more"
    );
}

/// The port kinds reserved, as the box contract names them (section 4), each refused on an
/// inlet and on an outlet as reserved, not as unknown (decided 2026-11-25: code 1 waits for
/// #12 step 19, code 2 for #12 step 42).
#[test]
fn refuses_the_port_kinds_a_later_version_defines_as_reserved() {
    assert_eq!(
        RESERVED_PORT_KINDS,
        [1, 2],
        "discontinuities, then spectral frames"
    );
    for code in [1, 2] {
        let ports = list(&[port("in", "x", 0), port("later", "x", code)]);
        assert_eq!(
            refusal(5, &ports),
            Some(refused(inlet(1, Part::Kind), Fault::Reserved(code))),
            "inlet"
        );
        assert_eq!(
            refusal(6, &ports),
            Some(refused(outlet(1, Part::Kind), Fault::Reserved(code))),
            "outlet"
        );
    }
}

#[test]
fn refuses_each_code_it_does_not_know() {
    for code in [2, 255] {
        assert_eq!(
            refusal(1, &[code]),
            Some(refused(Field::Mode, Fault::Code(code))),
            "mode"
        );
    }
    for code in [3, 255] {
        let ports = list(&[port("in", "x", code)]);
        assert_eq!(
            refusal(5, &ports),
            Some(refused(inlet(0, Part::Kind), Fault::Code(code))),
            "inlet"
        );
        assert_eq!(
            refusal(6, &ports),
            Some(refused(outlet(0, Part::Kind), Fault::Code(code))),
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
            Some(refused(Field::Capability { index: 0 }, Fault::Code(code))),
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
    let read = Description::read(&bytes).expect("a text as long as the limit allows");
    assert_eq!(read.description().len(), longest, "read whole");
    assert_eq!(
        Description::read(&described(longest.saturating_add(1))).err(),
        Some(refused(Field::Whole, Fault::TooLong { bytes: 16_777_217 })),
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
        .map(|index| param(&index.to_string(), "x", [0.0_f64; 3]))
        .collect();
    let bytes = with(7, &list(&many));
    let read = Description::read(&bytes).expect("65,537 parameters");
    assert_eq!(read.params().len(), 65_537, "parameters");
    let last = read.params().last().map(DescribedParam::name);
    assert_eq!(last, Some("65536"), "in order");
}

#[test]
fn reads_as_many_ports_as_a_patch_can_name_and_refuses_one_more() {
    assert_eq!(PORTS_MAX, 65_536, "16-bit port indices");
    let ports = |count: u32| {
        let items: Vec<_> = (0..count)
            .map(|index| port(&index.to_string(), "x", 0))
            .collect();
        list(&items)
    };
    let most = ports(65_536);
    for (index, which) in [(5, "inlets"), (6, "outlets")] {
        let bytes = with(index, &most);
        let read = Description::read(&bytes).expect("65,536 ports");
        let listed = if index == 5 {
            read.inlets()
        } else {
            read.outlets()
        };
        assert_eq!(
            (listed.len(), listed.last().map(DescribedPort::name)),
            (65_536, Some("65535")),
            "{which}, in order"
        );
    }
    let one_more = ports(65_537);
    assert_eq!(
        refusal(5, &one_more),
        Some(refused(Field::Inlets, Fault::TooMany { count: 65_537 })),
        "inlets"
    );
    assert_eq!(
        refusal(6, &one_more),
        Some(refused(Field::Outlets, Fault::TooMany { count: 65_537 })),
        "outlets"
    );
}

#[test]
fn refuses_a_length_or_a_count_beyond_the_bytes_left() {
    for announced in [8, 65_536, 2_147_483_648, u32::MAX] {
        let short = [&u32::to_le_bytes(announced)[..], b"few"].concat();
        for (index, list_field, field) in [
            (3, Field::Name, Field::Name),
            (4, Field::DescriptionText, Field::DescriptionText),
            (5, Field::Inlets, inlet(0, Part::Name)),
            (6, Field::Outlets, outlet(0, Part::Name)),
            (7, Field::Params, param_at(0, Part::Name)),
        ] {
            let mut bytes = fields()
                .into_iter()
                .take(index)
                .collect::<Vec<_>>()
                .concat();
            bytes.extend(&short);
            let ports = matches!(list_field, Field::Inlets | Field::Outlets);
            let expected = if ports && announced > 65_536 {
                refused(list_field, Fault::TooMany { count: announced })
            } else {
                refused(field, Fault::Truncated)
            };
            assert_eq!(
                Description::read(&bytes).err(),
                Some(expected),
                "{announced} announced in {list_field}"
            );
        }
        let mut bytes = fields().into_iter().take(8).collect::<Vec<_>>().concat();
        bytes.extend(u32::to_le_bytes(announced));
        bytes.push(0);
        assert_eq!(
            Description::read(&bytes).err(),
            Some(refused(Field::Capability { index: 1 }, Fault::Truncated)),
            "{announced} capabilities announced, one given"
        );
    }
}

#[test]
fn refuses_blank_texts_and_whitespace_in_the_name() {
    for blank in ["", " ", "\u{2003}", " \u{a0} "] {
        assert_eq!(
            refusal(3, &text(blank.as_bytes())),
            Some(refused(Field::Name, Fault::Blank)),
            "name"
        );
        assert_eq!(
            refusal(4, &text(blank.as_bytes())),
            Some(refused(Field::DescriptionText, Fault::Blank)),
            "description"
        );
        for (name, said, part) in [
            (blank, "x", Part::Name),
            ("p", blank, Part::DescriptionText),
        ] {
            assert_eq!(
                refusal(6, &list(&[port(name, said, 0)])),
                Some(refused(outlet(0, part), Fault::Blank)),
                "port"
            );
            assert_eq!(
                refusal(7, &list(&[param(name, said, [0.0_f64; 3])])),
                Some(refused(param_at(0, part), Fault::Blank)),
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

/// Every text of a description, each with the field that refuses it and whether it is a
/// name: the box type's name and description text, then a port's and a parameter's.
fn texts_in(value: &str) -> [(Option<DescriptionError>, Field, bool); 6] {
    [
        (refusal(3, &text(value.as_bytes())), Field::Name, true),
        (
            refusal(4, &text(value.as_bytes())),
            Field::DescriptionText,
            false,
        ),
        (
            refusal(5, &list(&[port(value, "x", 0)])),
            inlet(0, Part::Name),
            true,
        ),
        (
            refusal(6, &list(&[port("p", value, 0)])),
            outlet(0, Part::DescriptionText),
            false,
        ),
        (
            refusal(7, &list(&[param(value, "x", [0.0_f64; 3])])),
            param_at(0, Part::Name),
            true,
        ),
        (
            refusal(7, &list(&[param("p", value, [0.0_f64; 3])])),
            param_at(0, Part::DescriptionText),
            false,
        ),
    ]
}

/// Each end of every range a message escapes (`spec/safe-message-vectors.json`, its
/// `unsafe` vector), a newline, a tab and an escape among them, refused in every text,
/// named by its code point, before the text is found blank.
#[test]
fn refuses_in_every_text_the_characters_a_message_escapes() {
    for character in [
        '\u{0}', '\t', '\n', '\r', '\u{1b}', '\u{1f}', '\u{7f}', '\u{80}', '\u{9f}', '\u{61c}',
        '\u{200e}', '\u{200f}', '\u{202a}', '\u{202e}', '\u{2066}', '\u{2069}', '\u{2028}',
        '\u{2029}',
    ] {
        for (refused_as, field, _) in texts_in(&format!("a{character}b")) {
            assert_eq!(
                refused_as,
                Some(refused(field, Fault::Character(character))),
                "{character:?} in {field}"
            );
        }
        for (refused_as, field, _) in texts_in(&character.to_string()) {
            assert_eq!(
                refused_as,
                Some(refused(field, Fault::Character(character))),
                "{character:?} alone in {field}"
            );
        }
    }
}

/// Each end of every range of Unicode's `Default_Ignorable_Code_Point`
/// (`DerivedCoreProperties.txt`), refused in a name, which it would hide, and kept in a
/// description text (a joiner shapes emoji and scripts); a description text of those and
/// whitespace alone shows nothing, and is blank.
#[test]
fn refuses_in_names_the_code_points_that_show_nothing() {
    for character in [
        '\u{ad}',
        '\u{34f}',
        '\u{115f}',
        '\u{1160}',
        '\u{17b4}',
        '\u{17b5}',
        '\u{180b}',
        '\u{180f}',
        '\u{200b}',
        '\u{200d}',
        '\u{2060}',
        '\u{206f}',
        '\u{3164}',
        '\u{fe00}',
        '\u{fe0f}',
        '\u{feff}',
        '\u{ffa0}',
        '\u{fff0}',
        '\u{fff8}',
        '\u{1bca0}',
        '\u{1bca3}',
        '\u{1d173}',
        '\u{1d17a}',
        '\u{e0000}',
        '\u{e0fff}',
    ] {
        for (refused_as, field, name) in texts_in(&format!("gain{character}")) {
            let expected = name.then_some(refused(field, Fault::Character(character)));
            assert_eq!(refused_as, expected, "{character:?} in {field}");
        }
        assert_eq!(
            refusal(4, &text(format!(" {character}\u{a0}").as_bytes())),
            Some(refused(Field::DescriptionText, Fault::Blank)),
            "{character:?} with whitespace alone"
        );
    }
}

/// The neighbors of every range either rule refuses, kept in every text but the box type's
/// name, which refuses whitespace (`refuses_blank_texts_and_whitespace_in_the_name`).
#[test]
fn keeps_the_neighbors_of_every_refused_range() {
    for character in [
        '\u{20}',
        '\u{7e}',
        '\u{a0}',
        '\u{ac}',
        '\u{ae}',
        '\u{34e}',
        '\u{350}',
        '\u{61b}',
        '\u{61d}',
        '\u{115e}',
        '\u{1161}',
        '\u{17b3}',
        '\u{17b6}',
        '\u{180a}',
        '\u{1810}',
        '\u{200a}',
        '\u{2010}',
        '\u{2027}',
        '\u{202f}',
        '\u{205f}',
        '\u{2070}',
        '\u{3163}',
        '\u{3165}',
        '\u{fdff}',
        '\u{fe10}',
        '\u{fefe}',
        '\u{ff00}',
        '\u{ff9f}',
        '\u{ffa1}',
        '\u{ffef}',
        '\u{fff9}',
        '\u{1bc9f}',
        '\u{1bca4}',
        '\u{1d172}',
        '\u{1d17b}',
        '\u{dffff}',
        '\u{e1000}',
        '\u{e9}',
        '\u{97f3}',
        '\u{1d11e}',
    ] {
        let shown = format!("gain{character}");
        for (refused_as, field, _) in texts_in(&shown).into_iter().skip(1) {
            assert_eq!(refused_as, None, "{character:?} in {field}");
        }
    }
}

#[test]
fn refuses_text_that_is_not_utf_8() {
    for bytes in [&[0xff][..], &[0xc3], &[0xed, 0xa0, 0x80], &[0xc0, 0xaf]] {
        assert_eq!(
            refusal(4, &text(bytes)),
            Some(refused(Field::DescriptionText, Fault::NotUtf8)),
            "{bytes:?}"
        );
    }
}

#[test]
fn refuses_what_repeats() {
    let twice = list(&[port("a", "x", 0), port("b", "x", 0), port("a", "y", 0)]);
    assert_eq!(
        refusal(5, &twice),
        Some(refused(inlet(2, Part::Name), Fault::Repeated { first: 0 })),
        "inlets"
    );
    assert_eq!(
        refusal(6, &twice),
        Some(refused(outlet(2, Part::Name), Fault::Repeated { first: 0 })),
        "outlets"
    );
    let params = list(&[param("a", "x", [0.0_f64; 3]), param("a", "y", [1.0_f64; 3])]);
    assert_eq!(
        refusal(7, &params),
        Some(refused(
            param_at(1, Part::Name),
            Fault::Repeated { first: 0 }
        )),
        "parameters"
    );
    let asked = list(&[vec![0], vec![0]]);
    assert_eq!(
        refusal(8, &asked),
        Some(refused(
            Field::Capability { index: 1 },
            Fault::Repeated { first: 0 }
        )),
        "capabilities"
    );
    let later = list(&[
        port("a", "x", 0),
        port("b", "x", 0),
        port("c", "x", 0),
        port("b", "y", 0),
    ]);
    assert_eq!(
        refusal(5, &later),
        Some(refused(inlet(3, Part::Name), Fault::Repeated { first: 1 })),
        "the earlier item named"
    );
    assert_eq!(
        refusal(6, &list(&[port("in", "x", 0)])),
        None,
        "an outlet may have an inlet's name"
    );
}

#[test]
fn refuses_a_parameter_whose_range_or_default_no_value_meets() {
    let bits = f64::to_bits;
    let not_finite = |bound: f64| Fault::NotFinite { bound: bits(bound) };
    let reversed = |minimum: f64, maximum: f64| Fault::Reversed {
        minimum: bits(minimum),
        maximum: bits(maximum),
    };
    let unreachable = |bound: f64| Fault::Unreachable { bound: bits(bound) };
    let width = |minimum: f64, maximum: f64| Fault::Width {
        minimum: bits(minimum),
        maximum: bits(maximum),
    };
    let default = |[value, minimum, maximum]: [f64; 3]| Fault::Default {
        value: bits(value),
        minimum: bits(minimum),
        maximum: bits(maximum),
    };
    let max = f64::MAX;
    for (values, fault) in [
        ([0.0_f64, 1.0_f64, 0.0_f64], reversed(1.0_f64, 0.0_f64)),
        (
            [0.0_f64, f64::NEG_INFINITY, 1.0_f64],
            not_finite(f64::NEG_INFINITY),
        ),
        ([0.0_f64, 0.0_f64, f64::INFINITY], not_finite(f64::INFINITY)),
        ([0.0_f64, f64::NAN, 1.0_f64], not_finite(f64::NAN)),
        ([0.0_f64, 0.0_f64, f64::NAN], not_finite(f64::NAN)),
        ([0.5_f64, 1e-40_f64, 1.0_f64], unreachable(1e-40_f64)),
        ([0.0_f64, -1.0_f64, 1e-310_f64], unreachable(1e-310_f64)),
        // The minimum is told before the maximum, and the order before the bounds no value
        // reaches.
        (
            [0.0_f64, f64::NEG_INFINITY, f64::NAN],
            not_finite(f64::NEG_INFINITY),
        ),
        (
            [0.0_f64, f64::INFINITY, 1e-310_f64],
            not_finite(f64::INFINITY),
        ),
        ([0.0_f64, -1e-310_f64, 1e-310_f64], unreachable(-1e-310_f64)),
        (
            [0.0_f64, 1e-310_f64, -1.0_f64],
            reversed(1e-310_f64, -1.0_f64),
        ),
        // Finite bounds, but no finite width: from the largest number, the width overflows from
        // half its last place, 2^970, on (a tie, to the even infinity).
        ([0.0_f64, -max, max], width(-max, max)),
        (
            [0.0_f64, -max, HALF_LAST_PLACE],
            width(-max, HALF_LAST_PLACE),
        ),
        (
            [0.0_f64, -HALF_LAST_PLACE, max],
            width(-HALF_LAST_PLACE, max),
        ),
        // The order is told before the width: this range is reversed too.
        ([0.0_f64, max, -max], reversed(max, -max)),
        (
            [1.5_f64, 0.0_f64, 1.0_f64],
            default([1.5_f64, 0.0_f64, 1.0_f64]),
        ),
        (
            [-0.5_f64, 0.0_f64, 1.0_f64],
            default([-0.5_f64, 0.0_f64, 1.0_f64]),
        ),
        (
            [f64::NAN, 0.0_f64, 1.0_f64],
            default([f64::NAN, 0.0_f64, 1.0_f64]),
        ),
        (
            [1e-40_f64, SMALLEST_NORMAL, 1.0_f64],
            default([1e-40_f64, SMALLEST_NORMAL, 1.0_f64]),
        ),
    ] {
        assert_eq!(
            refusal(7, &list(&[param("p", "x", values)])),
            Some(refused(param_at(0, Part::Values), fault)),
            "{values:?}"
        );
    }
    let second = list(&[
        param("p", "x", [0.0_f64; 3]),
        param("q", "x", [2.0_f64, 0.0_f64, 1.0_f64]),
    ]);
    assert_eq!(
        refusal(7, &second),
        Some(refused(
            param_at(1, Part::Values),
            default([2.0_f64, 0.0_f64, 1.0_f64])
        )),
        "the second parameter named"
    );
}

/// 2^970, half the last place of the largest binary64 number.
const HALF_LAST_PLACE: f64 = f64::from_bits(0x7c90_0000_0000_0000);

/// The binary64 number before [`HALF_LAST_PLACE`].
const BEFORE_HALF_LAST_PLACE: f64 = f64::from_bits(0x7c8f_ffff_ffff_ffff);

/// 2^-126, the smallest normal binary32 value, below which a parameter is zero (decision 5).
const SMALLEST_NORMAL: f64 = 1.175_494_350_822_287_5e-38;

#[test]
fn reads_parameters_over_the_whole_of_binary64_bit_for_bit() {
    for values in [
        [1.0_f64, 0.0_f64, 1.0_f64],
        [0.0_f64, -0.0_f64, 0.0_f64],
        [0.1_f64, 0.1_f64, 0.1_f64],
        [1e300_f64, -1.0_f64, f64::MAX],
        [0.0_f64, -f64::MAX / 2.0_f64, f64::MAX / 2.0_f64],
        [0.0_f64, -f64::MAX, BEFORE_HALF_LAST_PLACE],
        [0.0_f64, -BEFORE_HALF_LAST_PLACE, f64::MAX],
        [SMALLEST_NORMAL, SMALLEST_NORMAL, 1.0_f64],
        [-1.0_f64, -1.0_f64, -SMALLEST_NORMAL],
        [1e-310_f64, -1.0_f64, 1.0_f64],
    ] {
        let bytes = with(7, &list(&[param("p", "x", values)]));
        let read = Description::read(&bytes).expect("a range its default is in");
        let bits: Vec<_> = read
            .params()
            .iter()
            .map(|param| {
                [
                    param.default_bits(),
                    param.minimum_bits(),
                    param.maximum_bits(),
                ]
            })
            .collect();
        assert_eq!(bits, [values.map(f64::to_bits)], "{values:?}");
    }
}

#[test]
fn accepts_exactly_what_a_built_in_box_type_may_declare() {
    let edges = [
        0.0_f64,
        -0.0_f64,
        1e-300_f64,
        -1e-300_f64,
        f64::from(f32::from_bits(1)),
        f64::from(f32::MIN_POSITIVE).next_down(),
        f64::from(f32::MIN_POSITIVE),
        -f64::from(f32::MIN_POSITIVE),
        0.1_f64,
        1.0_f64,
        -1.0_f64,
        f64::MAX,
        f64::MIN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
    ];
    let mut verdicts = [0_u32; 6];
    for default in edges {
        for minimum in edges {
            for maximum in edges {
                let expected = match ParamSpec::new("p", "x", default, minimum, maximum) {
                    Ok(_) => 0,
                    Err(ParamSpecError::NotFinite { .. }) => 1,
                    Err(ParamSpecError::InfiniteWidth { .. }) => 2,
                    Err(ParamSpecError::DefaultOutsideRange { .. }) => 3,
                    Err(ParamSpecError::Reversed { .. }) => 4,
                    Err(ParamSpecError::Unreachable { .. }) => 5,
                };
                let read = match refusal(7, &list(&[param("p", "x", [default, minimum, maximum])]))
                {
                    None => 0,
                    Some(DescriptionError {
                        field:
                            Field::Param {
                                index: 0,
                                part: Part::Values,
                            },
                        fault,
                    }) => match fault {
                        Fault::NotFinite { .. } => 1,
                        Fault::Reversed { .. } => 4,
                        Fault::Unreachable { .. } => 5,
                        Fault::Width { .. } => 2,
                        Fault::Default { .. } => 3,
                        Fault::Truncated
                        | Fault::Trailing { .. }
                        | Fault::Code(_)
                        | Fault::Reserved(_)
                        | Fault::Zero
                        | Fault::TooLong { .. }
                        | Fault::TooMany { .. }
                        | Fault::NotUtf8
                        | Fault::Blank
                        | Fault::Character(_)
                        | Fault::Whitespace
                        | Fault::Repeated { .. } => 6,
                    },
                    Some(_) => 7,
                };
                let counted = &mut verdicts[expected];
                *counted = counted.saturating_add(1);
                assert_eq!(
                    read, expected,
                    "default {default:?}, {minimum:?} to {maximum:?}"
                );
            }
        }
    }
    // The control: how many of the 16^3 combinations each verdict takes, as an independent
    // model of the rule counts them (decided from the bits, in Python's binary64).
    assert_eq!(
        verdicts,
        [283, 1_392, 16, 437, 1_232, 736],
        "accepted, not finite, width, default, reversed, unreachable"
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
            "execution mode: 9 is none of its codes in this version of the box contract",
        ),
        (
            Field::Response,
            Fault::Truncated,
            "instantaneous response: the bytes end inside it",
        ),
        (Field::Name, Fault::Whitespace, "name: holds whitespace"),
        (
            Field::Name,
            Fault::Character('\u{200b}'),
            "name: holds U+200B",
        ),
        (
            param_at(4, Part::DescriptionText),
            Fault::Character('\u{1b}'),
            "parameter 4, its description text: holds U+001B",
        ),
        (
            inlet(0, Part::Name),
            Fault::Character('\u{e0fff}'),
            "inlet 0, its name: holds U+E0FFF",
        ),
        (
            Field::DescriptionText,
            Fault::Blank,
            "description text: is blank",
        ),
        (
            Field::Whole,
            Fault::TooLong { bytes: 16_777_217 },
            "description: 16777217 bytes, more than 16777216",
        ),
        (
            inlet(2, Part::Name),
            Fault::Blank,
            "inlet 2, its name: is blank",
        ),
        (
            outlet(3, Part::Name),
            Fault::Repeated { first: 1 },
            "outlet 3, its name: repeats item 1 of its list",
        ),
        (
            Field::Inlets,
            Fault::TooMany {
                count: 4_294_967_295,
            },
            "inlets: 4294967295 ports, more than 65536",
        ),
        (
            outlet(0, Part::Kind),
            Fault::Reserved(1),
            "outlet 0, its kind: 1 is reserved for a later version of the box contract",
        ),
        (
            Field::Capability { index: 2 },
            Fault::Truncated,
            "capability 2: the bytes end inside it",
        ),
        (
            Field::Capabilities,
            Fault::Truncated,
            "capabilities: the bytes end inside it",
        ),
        (
            Field::Outlets,
            Fault::Truncated,
            "outlets: the bytes end inside it",
        ),
        (
            outlet(0, Part::DescriptionText),
            Fault::NotUtf8,
            "outlet 0, its description text: is not UTF-8",
        ),
        (
            Field::End,
            Fault::Trailing { bytes: 3 },
            "end: 3 bytes follow the last field",
        ),
    ] {
        assert_eq!(
            DescriptionError { field, fault }.to_string(),
            said,
            "{field:?}"
        );
    }
}

#[test]
fn says_the_values_of_a_parameter_it_refuses() {
    let bits = f64::to_bits;
    for (field, fault, said) in [
        (
            Field::Params,
            Fault::Truncated,
            "parameters: the bytes end inside it",
        ),
        (
            param_at(0, Part::Values),
            Fault::NotFinite {
                bound: bits(f64::NEG_INFINITY),
            },
            "parameter 0, its values: the bound -inf is not finite",
        ),
        (
            param_at(0, Part::Values),
            Fault::NotFinite {
                bound: bits(f64::NAN),
            },
            "parameter 0, its values: the bound NaN is not finite",
        ),
        (
            param_at(1, Part::Values),
            Fault::Reversed {
                minimum: bits(1.0_f64),
                maximum: bits(-0.5_f64),
            },
            "parameter 1, its values: the minimum 1.0 is above the maximum -0.5",
        ),
        (
            param_at(0, Part::Values),
            Fault::Unreachable {
                bound: bits(1e-310_f64),
            },
            "parameter 0, its values: the bound 1e-310 is below 2^-126 in magnitude but not \
             zero: no value reaches it",
        ),
        (
            param_at(0, Part::Values),
            Fault::Width {
                minimum: bits(-f64::MAX),
                maximum: bits(f64::MAX),
            },
            "parameter 0, its values: the width, 1.7976931348623157e308 less \
             -1.7976931348623157e308, is not finite in binary64",
        ),
        (
            param_at(0, Part::Values),
            Fault::Default {
                value: bits(f64::NAN),
                minimum: bits(0.0_f64),
                maximum: bits(1.0_f64),
            },
            "parameter 0, its values: the default NaN, flushed, is not within 0.0 to 1.0",
        ),
    ] {
        assert_eq!(
            DescriptionError { field, fault }.to_string(),
            said,
            "{fault:?}"
        );
    }
}
