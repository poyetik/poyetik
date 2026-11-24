//! Untrusted text made safe to show on one line (the rule of `spec/safe-message-vectors.json`,
//! written again here, tied to the other copies by those vectors, so that the visual
//! library's tool depends on no audio crate: xtask's library gate holds it to that,
//! `CrateRole::VisualTool`).

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

/// `character` as a safe text holds it: escaped as `\u{..}` when it [`needs_escape`].
fn piece(character: char) -> String {
    if needs_escape(character) {
        character.escape_unicode().to_string()
    } else {
        String::from(character)
    }
}

/// `remedy` made safe to show on one line, whole: what follows a reason bounded by
/// [`safe_message`], the tool's own words and the paths the user gave, never cut.
pub(crate) fn safe_remedy(remedy: &str) -> String {
    remedy.chars().map(piece).collect()
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
        let piece = piece(character);
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
#[path = "safe_tests.rs"]
mod tests;
