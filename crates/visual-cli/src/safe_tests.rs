//! Tests of the safe message rule against the shared vectors.

use serde_json::Value;

use super::{MAX_MESSAGE_CHARS, safe_message};

/// The shared known-answer vectors.
const VECTORS: &str = include_str!("../../../spec/safe-message-vectors.json");

/// The text of a list of `[piece, times]`.
fn text(pieces: &Value) -> String {
    pieces
        .as_array()
        .expect("a list of pieces")
        .iter()
        .map(|piece| {
            let part = piece[0].as_str().expect("a piece of text");
            let times = usize::try_from(piece[1].as_u64().expect("a count")).expect("usize");
            part.repeat(times)
        })
        .collect()
}

#[test]
fn reproduces_the_shared_vectors() {
    let json: Value = serde_json::from_str(VECTORS).expect("valid JSON");
    assert_eq!(
        json["max_chars"].as_u64(),
        Some(u64::try_from(MAX_MESSAGE_CHARS).expect("u64")),
        "same limit"
    );
    let vectors = json["vectors"].as_array().expect("a list of vectors");
    assert!(vectors.len() >= 5, "every vector is read");
    for vector in vectors {
        let name = vector["name"].as_str().expect("a name");
        assert_eq!(
            safe_message(&text(&vector["input"])),
            text(&vector["output"]),
            "{name}"
        );
    }
}
