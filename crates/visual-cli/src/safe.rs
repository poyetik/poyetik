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

/// `message` made safe to show, on one line and bounded.
///
/// Every character that could break the line or drive a terminal is escaped as `\u{..}`;
/// past [`MAX_MESSAGE_CHARS`]
/// characters (escapes counted whole, never split), the text is cut and marked with `…`.
#[must_use]
pub fn safe_message(message: &str) -> String {
    bounded(message, MAX_MESSAGE_CHARS)
}

/// `reason` then `remedy`: the reason made safe and cut to the room the remedy leaves under
/// [`MAX_MESSAGE_CHARS`], so that [`safe_message`] leaves the remedy whole.
///
/// Known gap (#24 step 109): a remedy whose paths alone pass that limit is cut by
/// [`safe_message`], after its whole reason.
pub(crate) fn remedied(reason: &str, remedy: &str) -> String {
    let taken = bounded(remedy, usize::MAX).chars().count();
    let room = MAX_MESSAGE_CHARS.checked_sub(taken);
    room.map_or_else(|| reason.to_owned(), |room| bounded(reason, room)) + remedy
}

/// [`safe_message`], bounded by `limit` characters; nothing at a limit of 0, not even the mark.
fn bounded(message: &str, limit: usize) -> String {
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
        if used > limit {
            safe.truncate(cut);
            safe.extend((limit > 0).then_some('…'));
            return safe;
        }
        safe.push_str(&piece);
        if used < limit {
            cut = safe.len();
        }
    }
    safe
}

#[cfg(test)]
#[path = "safe_tests.rs"]
mod tests;
