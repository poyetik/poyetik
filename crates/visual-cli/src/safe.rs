//! Untrusted text made safe to show on one line (the rule of `spec/safe-message-vectors.json`,
//! written again here so that the visual library depends on no audio crate).

/// The longest message shown, in characters; longer ones are cut and end with `…`.
pub const MAX_MESSAGE_CHARS: usize = 1000;

/// Whether `character` could break the line or drive a terminal: control characters, and
/// the bidirectional formatting and line separator characters.
const fn needs_escape(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{61c}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
                | '\u{2028}'
                | '\u{2029}'
        )
}

/// `message` made safe to show, on one line and bounded.
///
/// Every character that could break the line or drive a terminal is escaped as `\u{..}`;
/// past [`MAX_MESSAGE_CHARS`]
/// characters (escapes counted whole, never split), the text is cut and marked with `…`.
#[must_use]
pub fn safe_message(message: &str) -> String {
    let mut safe = String::new();
    let mut cut = 0_usize;
    let mut used = 0_usize;
    for character in message.chars() {
        let piece = if needs_escape(character) {
            character.escape_unicode().to_string()
        } else {
            String::from(character)
        };
        used = used.saturating_add(piece.chars().count());
        if used > MAX_MESSAGE_CHARS {
            safe.truncate(cut);
            safe.push('…');
            return safe;
        }
        safe.push_str(&piece);
        if used < MAX_MESSAGE_CHARS {
            cut = safe.len();
        }
    }
    safe
}

#[cfg(test)]
mod tests {
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
}
