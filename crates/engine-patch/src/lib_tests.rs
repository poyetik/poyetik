//! Tests of patch reading.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use engine_core::{Connection, CoreError, Node, NodeId, Patch};

use super::{FORMAT, MAX_PATCH_BYTES, PatchError, parse_patch};

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
    let patch = parse_patch(
        r#"{"format":1,"nodes":[{"id":1,"box":"x","params":{"a":0.1,"b":3.4028235e38,"c":-0.0}}],"connections":[]}"#,
    )
    .expect("valid patch");
    let params = &patch.nodes().next().expect("one node").params;
    assert_eq!(
        params["a"].to_bits(),
        0.1_f32.to_bits(),
        "0.1 rounds like the f32 literal"
    );
    assert_eq!(params["b"].to_bits(), f32::MAX.to_bits(), "largest f32");
    assert_eq!(
        params["c"].to_bits(),
        (-0.0_f32).to_bits(),
        "negative zero is kept"
    );
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
        "invalid patch: node 3 is declared twice",
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
        Some(String::from("node 3 is declared twice")),
        "the patch error is the cause"
    );
}
