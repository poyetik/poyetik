// What every command-line tool of the repository does to untrusted text before printing
// it: the same characters and the same bound as `audio_patch::safe_message` in Rust,
// checked on the shared vectors of spec/safe-message-vectors.json.

/** Longest line {@link oneLine} returns, in characters (code points) after escaping. */
export const MAX_LINE_CHARS = 1000;

/**
 * The ranges of code points that must not reach a terminal as they are, both ends
 * included: the control characters (C0, DEL and C1: a newline forges a second line, ESC
 * starts terminal sequences), the bidirectional formatting characters (they reorder what
 * is shown) and the Unicode line and paragraph separators.
 */
const UNSAFE: readonly (readonly [number, number])[] = [
  [0x00, 0x1f],
  [0x7f, 0x9f],
  [0x61c, 0x61c],
  [0x200e, 0x200f],
  [0x2028, 0x2029],
  [0x202a, 0x202e],
  [0x2066, 0x2069],
];

/** The code point of `character`, a string of one code point. */
function codeOf(character: string): number {
  // `codePointAt(0)` is undefined only for an empty string, which a character never is.
  return Number(character.codePointAt(0));
}

/** Whether `character` (one code point) is one {@link oneLine} escapes. */
function isUnsafe(character: string): boolean {
  const code = codeOf(character);
  return UNSAFE.some(([first, last]) => code >= first && code <= last);
}

/**
 * The first character of `text` that could break a line or drive a terminal (see
 * {@link oneLine}), or `undefined` when there is none: for the readers that refuse such
 * text instead of escaping it.
 */
export function firstUnsafe(text: string): string | undefined {
  for (const character of text) {
    if (isUnsafe(character)) {
      return character;
    }
  }
  return undefined;
}

/**
 * `text` made safe to print as one line, and bounded: every character that could break
 * the line or drive a terminal is escaped as `\u{..}` (lowercase hexadecimal, as Rust's
 * `char::escape_unicode` writes it); past {@link MAX_LINE_CHARS} characters (an escape
 * counted whole, never split) the text is cut and marked with `…`.
 */
export function oneLine(text: string): string {
  let safe = "";
  // Where to cut if the text is too long: after the last whole character (or escape)
  // that leaves room for the `…`.
  let cut = 0;
  let used = 0;
  for (const character of text) {
    // An escape is ASCII: its length is its count of characters; any other piece is one
    // character, of one or two UTF-16 units.
    const escaped = isUnsafe(character);
    const piece = escaped ? `\\u{${codeOf(character).toString(16)}}` : character;
    used += escaped ? piece.length : 1;
    if (used > MAX_LINE_CHARS) {
      return `${safe.slice(0, cut)}…`;
    }
    safe += piece;
    if (used < MAX_LINE_CHARS) {
      cut = safe.length;
    }
  }
  return safe;
}
