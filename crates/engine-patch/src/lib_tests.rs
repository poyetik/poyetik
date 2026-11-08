//! Tests of patch reading.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use engine_core::{Connection, CoreError, Node, NodeId, Patch};

use super::{FORMAT, MAX_MESSAGE_CHARS, MAX_PATCH_BYTES, PatchError, parse_patch, safe_message};

const EXAMPLE: &str = r#"{
  "format": 1,
  "nodes": [
    { "id": 1, "box": "osc~", "params": { "frequency": 440 } },
    { "id": 2, "box": "dac~" }
  ],
  "connections": [
    { "from": [1, 0], "to": [2, 0] },
    { "from": [1, 0], "to": [2, 1] }
  ]
}"#;

fn invalid(json: &str) -> String {
    match parse_patch(json) {
        Err(PatchError::Invalid(reason)) => reason,
        other => panic!("expected an invalid patch, got {other:?}"),
    }
}

fn connection(from: u32, outlet: usize, to: u32, inlet: usize) -> Connection {
    Connection {
        from: NodeId(from),
        outlet,
        to: NodeId(to),
        inlet,
    }
}

#[test]
fn reads_the_specification_example() {
    let mut params = BTreeMap::new();
    let _: Option<f32> = params.insert(String::from("frequency"), 440.0);
    assert_eq!(
        parse_patch(EXAMPLE),
        Patch::new(
            vec![
                Node {
                    id: NodeId(1),
                    kind: String::from("osc~"),
                    params
                },
                Node {
                    id: NodeId(2),
                    kind: String::from("dac~"),
                    params: BTreeMap::new()
                },
            ],
            vec![connection(1, 0, 2, 0), connection(1, 0, 2, 1)],
        )
        .map_err(PatchError::Patch),
        "the example of spec/patch-format.md"
    );
}

#[test]
fn reads_an_empty_patch_and_the_largest_id() {
    assert_eq!(
        parse_patch(r#"{"format": 1, "nodes": [], "connections": []}"#),
        Ok(Patch::default()),
        "empty"
    );
    let patch =
        parse_patch(r#"{"format":1,"nodes":[{"id":4294967295,"box":"x"}],"connections":[]}"#);
    assert_eq!(
        patch.map(|parsed| parsed.nodes().map(|node| node.id).collect::<Vec<_>>()),
        Ok(vec![NodeId(u32::MAX)]),
        "u32::MAX"
    );
}

#[test]
fn reads_port_indices_from_0_to_65535_only() {
    let with_ports = |outlet: &str, inlet: &str| {
        format!(
            r#"{{"format":1,"nodes":[],"connections":[{{"from":[1,{outlet}],"to":[2,{inlet}]}}]}}"#
        )
    };
    assert_eq!(
        parse_patch(&with_ports("65535", "0"))
            .map(|patch| patch.connections().copied().collect::<Vec<_>>()),
        Ok(vec![connection(1, 65_535, 2, 0)]),
        "the largest outlet"
    );
    assert_eq!(
        parse_patch(&with_ports("0", "65535"))
            .map(|patch| patch.connections().copied().collect::<Vec<_>>()),
        Ok(vec![connection(1, 0, 2, 65_535)]),
        "the largest inlet"
    );
    // The same refusal on every target, whatever the width of its pointers.
    assert_eq!(
        invalid(&with_ports("65536", "0")),
        "invalid value: integer `65536`, expected u16 at line 1 column 54",
        "outlet 65536"
    );
    assert_eq!(
        invalid(&with_ports("0", "65536")),
        "invalid value: integer `65536`, expected u16 at line 1 column 65",
        "inlet 65536"
    );
    assert_eq!(
        invalid(&with_ports("4294967296", "0")),
        "invalid value: integer `4294967296`, expected u16 at line 1 column 59",
        "beyond 32 bits"
    );
}

#[test]
fn refuses_repeated_nodes_cables_and_parameters() {
    assert_eq!(
        parse_patch(
            r#"{"format":1,"nodes":[{"id":1,"box":"x"},{"id":1,"box":"y"}],"connections":[]}"#
        ),
        Err(PatchError::Patch(CoreError::DuplicateNode(NodeId(1)))),
        "node"
    );
    assert_eq!(
        parse_patch(
            r#"{"format":1,"nodes":[],"connections":[{"from":[1,0],"to":[2,0]},{"from":[1,0],"to":[2,0]}]}"#
        ),
        Err(PatchError::Patch(CoreError::DuplicateConnection(
            connection(1, 0, 2, 0)
        ))),
        "cable"
    );
    let reason = invalid(
        r#"{"format":1,"nodes":[{"id":1,"box":"x","params":{"a":1,"a":2}}],"connections":[]}"#,
    );
    assert!(reason.contains("parameter `a` is given twice"), "{reason}");
    let mistyped =
        invalid(r#"{"format":1,"nodes":[{"id":1,"box":"x","params":[1]}],"connections":[]}"#);
    assert!(
        mistyped.contains("an object of parameter values"),
        "{mistyped}"
    );
}

#[test]
fn refuses_documents_beyond_the_size_limit() {
    let padding = |bytes: usize| {
        let base = r#"{"format":1,"nodes":[],"connections":[]}"#;
        format!("{base}{}", " ".repeat(bytes - base.len()))
    };
    assert_eq!(
        parse_patch(&padding(MAX_PATCH_BYTES)),
        Ok(Patch::default()),
        "exactly at the limit"
    );
    assert_eq!(
        parse_patch(&padding(MAX_PATCH_BYTES + 1)),
        Err(PatchError::TooLarge(MAX_PATCH_BYTES + 1)),
        "one byte more"
    );
}

#[test]
fn refuses_more_nodes_than_a_patch_holds() {
    let nodes: Vec<String> = (0..=Patch::MAX_NODES)
        .map(|id| format!(r#"{{"id":{id},"box":"x"}}"#))
        .collect();
    let json = format!(
        r#"{{"format":1,"nodes":[{}],"connections":[]}}"#,
        nodes.join(",")
    );
    assert_eq!(
        parse_patch(&json),
        Err(PatchError::Patch(CoreError::TooManyNodes(
            Patch::MAX_NODES + 1
        ))),
        "one node more than the limit"
    );
}

#[test]
fn parses_numbers_with_correct_rounding() {
    // Each number is rounded once, from its exact decimal value to the nearest binary32
    // (spec/patch-format.md). Reading it as binary64 first, then rounding again, gives
    // another value for the first two below, one ulp off: they sit just above a binary32
    // midpoint, closer than binary64 can tell. These checks fail without serde_json's
    // `float_roundtrip` feature, which nothing else pins.
    let values = [
        // 1 + 2^-24 + 10^-29: just above the midpoint between 1 and the next binary32.
        ("1.00000005960464477539062500001", 0x3F80_0001_u32),
        // 440 + 2^-16 + 2^-70, exactly: just above the midpoint after 440.
        (
            "440.0000152587890625000008470329472543003390683225006796419620513916015625",
            0x43DC_0001,
        ),
        // 1 + 2^-24, exactly the midpoint: ties to even, down to 1.
        ("1.000000059604644775390625", 0x3F80_0000),
        ("0.1", 0.1_f32.to_bits()),
        ("3.4028235e38", f32::MAX.to_bits()),
        ("-0.0", (-0.0_f32).to_bits()),
        ("-0", (-0.0_f32).to_bits()),
        // The smallest subnormal, and a value that rounds to it.
        ("1e-45", 0x0000_0001),
        ("1.401298464324817e-45", 0x0000_0001),
    ];
    for (text, bits) in values {
        let patch = parse_patch(&format!(
            r#"{{"format":1,"nodes":[{{"id":1,"box":"x","params":{{"a":{text}}}}}],"connections":[]}}"#
        ))
        .expect("valid patch");
        let params = &patch.nodes().next().expect("one node").params;
        assert_eq!(params["a"].to_bits(), bits, "{text} rounds to {bits:#010x}");
    }
}

#[test]
fn rejects_other_format_versions() {
    for version in [0_u32, 2, 99] {
        let json = format!(r#"{{"format": {version}, "nodes": [], "connections": []}}"#);
        assert_eq!(
            parse_patch(&json),
            Err(PatchError::UnsupportedFormat(version)),
            "{version}"
        );
    }
    assert_eq!(FORMAT, 1, "version 1");
}

#[test]
fn rejects_unknown_fields_everywhere() {
    for (json, field) in [
        (
            r#"{"format":1,"nodes":[],"connections":[],"extra":0}"#,
            "extra",
        ),
        (
            r#"{"format":1,"nodes":[{"id":1,"box":"x","name":"n"}],"connections":[]}"#,
            "name",
        ),
        (
            r#"{"format":1,"nodes":[],"connections":[{"from":[1,0],"to":[2,0],"gain":1}]}"#,
            "gain",
        ),
    ] {
        let reason = invalid(json);
        assert!(
            reason.contains(&format!("unknown field `{field}`")),
            "{reason}"
        );
    }
}

#[test]
fn rejects_missing_and_mistyped_fields() {
    // The exact reasons: each names what is wrong and where.
    for (json, reason) in [
        ("", "EOF while parsing a value at line 1 column 0"),
        ("not json", "expected ident at line 1 column 2"),
        (
            "[]",
            "invalid length 0, expected struct Document with 3 elements at line 1 column 2",
        ),
        (
            r#"{"nodes":[],"connections":[]}"#,
            "missing field `format` at line 1 column 29",
        ),
        (
            r#"{"format":1,"connections":[]}"#,
            "missing field `nodes` at line 1 column 29",
        ),
        (
            r#"{"format":1,"nodes":[]}"#,
            "missing field `connections` at line 1 column 23",
        ),
        (
            r#"{"format":1,"nodes":[{"box":"x"}],"connections":[]}"#,
            "missing field `id` at line 1 column 32",
        ),
        (
            r#"{"format":1,"nodes":[{"id":1}],"connections":[]}"#,
            "missing field `box` at line 1 column 29",
        ),
        (
            r#"{"format":1,"nodes":[{"id":-1,"box":"x"}],"connections":[]}"#,
            "invalid value: integer `-1`, expected u32 at line 1 column 29",
        ),
        (
            r#"{"format":1,"nodes":[{"id":4294967296,"box":"x"}],"connections":[]}"#,
            "invalid value: integer `4294967296`, expected u32 at line 1 column 37",
        ),
        (
            r#"{"format":1,"nodes":[{"id":1,"box":"x","params":{"a":"loud"}}],"connections":[]}"#,
            "invalid type: string \"loud\", expected f32 at line 1 column 59",
        ),
        (
            r#"{"format":1,"nodes":[],"connections":[{"from":[1],"to":[2,0]}]}"#,
            "invalid length 1, expected a tuple of size 2 at line 1 column 49",
        ),
        (
            r#"{"format":1,"nodes":[],"connections":[{"from":[1,0,0],"to":[2,0]}]}"#,
            "trailing characters at line 1 column 52",
        ),
        (
            r#"{"format":"1","nodes":[],"connections":[]}"#,
            "invalid type: string \"1\", expected u32 at line 1 column 13",
        ),
    ] {
        assert_eq!(invalid(json), reason, "{json:?}");
    }
}

#[test]
fn describes_errors() {
    assert_eq!(
        PatchError::Invalid(String::from("bad")).to_string(),
        "invalid patch: bad",
        "invalid"
    );
    assert_eq!(
        PatchError::UnsupportedFormat(2).to_string(),
        "unsupported patch format 2 (expected 1)",
        "version"
    );
    assert_eq!(
        PatchError::TooLarge(20_000_000).to_string(),
        "patch too large: 20000000 bytes (at most 16777216)",
        "size"
    );
    assert_eq!(
        PatchError::Patch(CoreError::DuplicateNode(NodeId(3))).to_string(),
        "invalid patch: box 3 is declared twice",
        "structure"
    );
    for error in [
        PatchError::TooLarge(1),
        PatchError::Invalid(String::new()),
        PatchError::UnsupportedFormat(2),
    ] {
        let error: &dyn core::error::Error = &error;
        assert!(error.source().is_none(), "no underlying cause");
    }
    let error: &dyn core::error::Error = &PatchError::Patch(CoreError::DuplicateNode(NodeId(3)));
    assert_eq!(
        error.source().map(ToString::to_string),
        Some(String::from("box 3 is declared twice")),
        "the patch error is the cause"
    );
}

#[test]
fn makes_a_message_safe_to_show() {
    assert_eq!(
        safe_message("box 1: unknown box type \"x~\""),
        "box 1: unknown box type \"x~\"",
        "plain text is unchanged, quotes included"
    );
    let hostile = "a\nb\rc\td\u{7f}e\u{85}f\u{202e}g\u{2066}h\u{200f}i\u{200e}i\u{61c}j\u{2028}k\u{2029}l\u{202a}m\u{2069}n\u{1b}";
    assert_eq!(
        safe_message(hostile),
        "a\\u{a}b\\u{d}c\\u{9}d\\u{7f}e\\u{85}f\\u{202e}g\\u{2066}h\\u{200f}i\\u{200e}i\\u{61c}j\\u{2028}k\\u{2029}l\\u{202a}m\\u{2069}n\\u{1b}",
        "control, bidi and line-separator characters are escaped"
    );
    assert_eq!(
        safe_message("é ü 音 \u{202f}\u{206a}\u{2065}"),
        "é ü 音 \u{202f}\u{206a}\u{2065}",
        "other text is kept, the neighbours of the escaped ranges included"
    );
    assert_eq!(safe_message(""), "", "nothing to show");
}

#[test]
fn bounds_the_length_of_a_message() {
    let at_limit = "x".repeat(MAX_MESSAGE_CHARS);
    assert_eq!(safe_message(&at_limit), at_limit, "exactly at the limit");
    assert_eq!(
        safe_message(&format!("{at_limit}y")),
        format!("{}…", "x".repeat(MAX_MESSAGE_CHARS - 1)),
        "one character more: cut, and marked"
    );
    // An escape is never cut in the middle.
    assert_eq!(
        safe_message(&"\n".repeat(MAX_MESSAGE_CHARS)),
        format!("{}…", "\\u{a}".repeat((MAX_MESSAGE_CHARS - 1) / 5)),
        "whole escapes only"
    );
    assert_eq!(MAX_MESSAGE_CHARS, 1_000, "the documented limit");
}
