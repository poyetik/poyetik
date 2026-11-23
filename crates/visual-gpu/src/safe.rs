//! Untrusted text (a driver's name, wgpu's errors, an argument) made safe to show on one line:
//! the rule of `spec/safe-message-vectors.json`, written again here, tied to the other copies
//! by those vectors, since this crate is a workspace of its own and depends on no tool of the
//! other (one shared crate: roadmap #24 step 109).

use alloc::string::{String, ToString as _};

/// The longest message shown, in characters; longer ones are cut and end with `…`.
pub(crate) const MAX_MESSAGE_CHARS: usize = 1000;

/// Whether `character` could break the line or drive a terminal: control characters, and
/// the bidirectional formatting and line separator characters.
pub(crate) const fn needs_escape(character: char) -> bool {
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
pub(crate) fn safe_message(message: &str) -> String {
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
#[path = "safe_tests.rs"]
mod tests;
