//! Tests of patch reading.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;

use engine_core::{Connection, Node, NodeId, Patch};

use super::{FORMAT, PatchError, parse_patch};

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

#[test]
fn reads_the_specification_example() {
    let mut params = BTreeMap::new();
    let _: Option<f32> = params.insert(String::from("frequency"), 440.0);
    assert_eq!(
        parse_patch(EXAMPLE),
        Ok(Patch {
            nodes: vec![
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
            connections: vec![
                Connection {
                    from: NodeId(1),
                    outlet: 0,
                    to: NodeId(2),
                    inlet: 0
                },
                Connection {
                    from: NodeId(1),
                    outlet: 0,
                    to: NodeId(2),
                    inlet: 1
                },
            ],
        }),
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
        patch.map(|p| p.nodes[0].id),
        Ok(NodeId(u32::MAX)),
        "u32::MAX"
    );
}

#[test]
fn parses_numbers_with_correct_rounding() {
    let patch = parse_patch(
        r#"{"format":1,"nodes":[{"id":1,"box":"x","params":{"a":0.1,"b":3.4028235e38,"c":-0.0}}],"connections":[]}"#,
    )
    .expect("valid patch");
    let params = &patch.nodes[0].params;
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
    for json in [
        "",
        "not json",
        "[]",
        r#"{"nodes":[],"connections":[]}"#,
        r#"{"format":1,"connections":[]}"#,
        r#"{"format":1,"nodes":[]}"#,
        r#"{"format":1,"nodes":[{"box":"x"}],"connections":[]}"#,
        r#"{"format":1,"nodes":[{"id":1}],"connections":[]}"#,
        r#"{"format":1,"nodes":[{"id":-1,"box":"x"}],"connections":[]}"#,
        r#"{"format":1,"nodes":[{"id":4294967296,"box":"x"}],"connections":[]}"#,
        r#"{"format":1,"nodes":[{"id":1,"box":"x","params":{"a":"loud"}}],"connections":[]}"#,
        r#"{"format":1,"nodes":[],"connections":[{"from":[1],"to":[2,0]}]}"#,
        r#"{"format":1,"nodes":[],"connections":[{"from":[1,0,0],"to":[2,0]}]}"#,
        r#"{"format":"1","nodes":[],"connections":[]}"#,
    ] {
        assert!(
            !invalid(json).is_empty(),
            "{json:?} is rejected with a reason"
        );
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
    let error: &dyn core::error::Error = &PatchError::UnsupportedFormat(2);
    assert!(error.source().is_none(), "no underlying cause");
}
