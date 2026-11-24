//! Tests of the safe message rule against the shared vectors.

use serde_json::Value;

use super::{MAX_MESSAGE_CHARS, remedied, safe_message};

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
    assert_eq!(
        vectors.len(),
        9,
        "every vector is read: the nine of the file"
    );
    for vector in vectors {
        let name = vector["name"].as_str().expect("a name");
        assert_eq!(
            safe_message(&text(&vector["input"])),
            text(&vector["output"]),
            "{name}"
        );
    }
}

#[test]
fn leaves_a_remedy_whole_and_its_reason_the_room_left() {
    // Finding K13-2 of the sixth deep review of visual block 2 (#34), one character either side of
    // the room a remedy of 300 characters leaves, an escape (6 characters) counted whole on
    // either side.
    let x = |count: usize| "x".repeat(count);
    let remedy = x(300);
    let escaped = format!("\u{1b}{}", x(294));
    for (reason, helps, line) in [
        (x(699), &remedy, x(999)),
        (x(700), &remedy, x(1_000)),
        (x(701), &remedy, format!("{}…{remedy}", x(699))),
        (x(700), &escaped, format!("{}\\u{{1b}}{}", x(700), x(294))),
        (x(701), &escaped, format!("{}…\\u{{1b}}{}", x(699), x(294))),
        (
            format!("{}\u{1b}x", x(693)),
            &remedy,
            format!("{}\\u{{1b}}x{remedy}", x(693)),
        ),
        (
            format!("{}\u{1b}xx", x(693)),
            &remedy,
            format!("{}\\u{{1b}}…{remedy}", x(693)),
        ),
        // The least room, then none: the remedy alone.
        (x(2), &x(999), format!("…{}", x(999))),
        (x(2), &x(1_000), x(1_000)),
        // KNOWN GAP, #24 step 109: a remedy past the limit alone is cut, after its reason.
        (x(2), &x(1_001), format!("{}…", x(999))),
        ("ab".to_owned(), &x(1_001), format!("ab{}…", x(997))),
    ] {
        assert_eq!(safe_message(&remedied(&reason, helps)), line, "{reason}");
    }
}
