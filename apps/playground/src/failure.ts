/**
 * What failed, in words, for those who debug: any value a promise rejected with or a call
 * threw, which the page only ever holds as `unknown`.
 */

/** The words of a value nothing can write: a revoked proxy, which every conversion throws on. */
export const UNWRITABLE = "a value that cannot be written";

/**
 * The first answer of `ways` that neither throws nor answers undefined, tried in order, or
 * {@link UNWRITABLE}.
 */
function firstWords(ways: readonly (() => string | undefined)[]): string {
  for (const way of ways) {
    try {
      const words = way();
      if (words !== undefined) {
        return words;
      }
    } catch {
      // This way cannot write the value: the next one tries.
    }
  }
  return UNWRITABLE;
}

/** Whether `value` is an error, a `DOMException` included (what the browser throws is no `Error` of the page's own everywhere). */
function isError(value: unknown): value is Error {
  return value instanceof Error || value instanceof DOMException;
}

/**
 * A thrown value in words, whatever it is, never throwing: a string as it is, an error's own words, anything else as JSON; as `String` gives it where JSON
 * has no words (a symbol, a function) or cannot write it (a big integer, a cycle); as its
 * kind (`[object Object]`) where `String` throws (an object without a prototype); and
 * {@link UNWRITABLE} where even that throws.
 */
function text(value: unknown): string {
  return firstWords([
    // `undefined` falls through to `String`'s words, "undefined", as JSON has none for it.
    () => (typeof value === "string" ? value : undefined),
    () => (isError(value) ? value.toString() : undefined),
    // `JSON.stringify` answers undefined, though typed as a string, for a symbol or a function.
    (): string | undefined => JSON.stringify(value),
    () => String(value),
    () => Object.prototype.toString.call(value),
  ]);
}

/** The cause `value` carries, if it is an error, or undefined (also when reading it throws). */
function causeOf(value: unknown): unknown {
  try {
    return isError(value) ? value.cause : undefined;
  } catch {
    // Reading it threw: no cause to word.
  }
  return undefined;
}

/** The most causes {@link failure} words: enough for any chain the page builds (two or three), and a bound on a chain without end. */
export const MOST_CAUSES = 16;

/** What {@link failure} says past {@link MOST_CAUSES} causes. */
export const MORE_CAUSES = "and more causes";

/**
 * `error` in words, never its text alone, and after it each cause it carries (a trap's own
 * error, a download's) but one whose words the words before it end with, as whole words (an
 * error that writes its cause in its message), said once: never throwing, whatever was thrown, and bounded, a cause
 * met again (a cycle of causes) ending the words, and at most {@link MOST_CAUSES} causes
 * worded, then {@link MORE_CAUSES} (a cause that is new at every read, a getter making one,
 * would never end).
 * @param error What was thrown or rejected.
 * @returns Its words, then `caused by` each cause's.
 */
export function failure(error: unknown): string {
  let last = text(error);
  const words = [last];
  const seen = new Set<unknown>([error]);
  for (let cause = causeOf(error); cause !== undefined && !seen.has(cause); cause = causeOf(cause)) {
    if (seen.size > MOST_CAUSES) {
      words.push(MORE_CAUSES);
      break;
    }
    seen.add(cause);
    const said = text(cause);
    if (!` ${last}`.endsWith(` ${said}`)) {
      words.push(`caused by ${said}`);
    }
    last = said;
  }
  return words.join(", ");
}
