//! Tests of the safe message rule against the shared vectors.

use alloc::string::String;
use alloc::vec::Vec;

use super::{MAX_MESSAGE_CHARS, safe_message};

/// The shared known-answer vectors.
const VECTORS: &str = include_str!("../../../spec/safe-message-vectors.json");

/// The text of `line`, a JSON string, its comma apart: none when it is written otherwise
/// (an escape this reader does not know included).
fn json_string(line: &str) -> Option<String> {
    let body = line
        .strip_suffix(',')
        .unwrap_or(line)
        .strip_prefix('"')?
        .strip_suffix('"')?;
    let mut units = Vec::new();
    let mut rest = body.chars();
    while let Some(character) = rest.next() {
        let character = if character == '\\' {
            match rest.next()? {
                'u' => {
                    let digits: String = rest.by_ref().take(4).collect();
                    units.push(u16::from_str_radix(&digits, 16).ok()?);
                    continue;
                }
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                kept @ ('"' | '\\') => kept,
                _ => return None,
            }
        } else {
            character
        };
        units.extend_from_slice(character.encode_utf16(&mut [0; 2]));
    }
    String::from_utf16(&units).ok()
}

/// The text of the pieces `lines` starts with, the one `[`, then a string and its times, each
/// on a line of its own, then `]`: none when they are written otherwise.
fn text<'line>(lines: &mut impl Iterator<Item = &'line str>) -> Option<String> {
    let mut text = String::new();
    loop {
        match lines.next()?.trim_start() {
            "[" => {
                let piece = json_string(lines.next()?.trim_start())?;
                let times = lines.next()?.trim_start().parse().ok()?;
                text.push_str(&piece.repeat(times));
                let closed = lines.next()?.trim_start();
                if closed != "]" && closed != "]," {
                    return None;
                }
            }
            "]" | "]," => return Some(text),
            _ => return None,
        }
    }
}

/// The vectors of `json`, each an input and its output, as the file is written (one value a
/// line, an empty list on its own).
fn vectors(json: &str) -> Option<Vec<(String, String)>> {
    let mut lines = json.lines();
    let mut read = Vec::new();
    let mut input = None;
    while let Some(line) = lines.next() {
        match line.trim_start() {
            "\"input\": []," => input = Some(String::new()),
            "\"input\": [" => input = Some(text(&mut lines)?),
            "\"output\": []" => read.push((input.take()?, String::new())),
            "\"output\": [" => read.push((input.take()?, text(&mut lines)?)),
            _ => {}
        }
    }
    Some(read)
}

#[test]
fn reproduces_the_shared_vectors() {
    assert!(
        VECTORS.contains(&alloc::format!("\n  \"max_chars\": {MAX_MESSAGE_CHARS},\n")),
        "same limit"
    );
    let vectors = vectors(VECTORS).expect("the vectors as the file writes them");
    assert_eq!(
        (vectors.len(), VECTORS.matches("\"name\": ").count()),
        (9, 9),
        "every vector is read: the nine of the file"
    );
    for (input, output) in vectors {
        assert_eq!(safe_message(&input), output, "{input:?}");
    }
}

#[test]
fn reads_the_shared_vectors_only_as_they_are_written() {
    assert_eq!(
        json_string(r#""a\"\\\n\r\t\u00e9\ud834\udd1e","#),
        Some("a\"\\\n\r\t\u{e9}\u{1d11e}".to_owned())
    );
    for other in [
        r#""\q""#,
        r#""\u00g0""#,
        r#""\ud834""#,
        r#""\"#,
        "\"a",
        "a\"",
        "1",
    ] {
        assert_eq!(json_string(other), None, "{other}");
    }
    let vector = |pieces: &str| vectors(&alloc::format!("\"input\": [],\n\"output\": [\n{pieces}"));
    assert_eq!(
        vector("[\n\"a\",\n2\n]\n]\n"),
        Some(alloc::vec![(String::new(), "aa".to_owned())])
    );
    for other in [
        "",
        "[",
        "[\n\"a\",",
        "[\n\"a\",\nx\n]\n]",
        "[\n\"a\",\n2\n}\n]",
        "\"a\"",
        "[\na\n2\n]\n]",
    ] {
        assert_eq!(vector(other), None, "{other:?}");
    }
    assert_eq!(vectors("\"output\": []"), None, "an output of no input");
    assert_eq!(vectors("\"output\": ["), None, "an output of no input");
}
