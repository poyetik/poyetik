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

/**
 * A thrown value in words, whatever it is, never throwing: a string as it is, `undefined`
 * as such, an error's own words, anything else as JSON; as `String` gives it where JSON
 * has no words (a symbol, a function) or cannot write it (a big integer, a cycle); as its
 * kind (`[object Object]`) where `String` throws (an object without a prototype); and
 * {@link UNWRITABLE} where even that throws.
 */
function text(value: unknown): string {
  return firstWords([
    () => (typeof value === "string" ? value : value === undefined ? "undefined" : undefined),
    () => (value instanceof Error ? value.toString() : undefined),
    // `JSON.stringify` answers undefined, though typed as a string, for a symbol or a function.
    (): string | undefined => JSON.stringify(value),
    () => String(value),
    () => Object.prototype.toString.call(value),
  ]);
}

/** The cause `value` carries, if it is an error, or undefined (also when reading it throws). */
function causeOf(value: unknown): unknown {
  try {
    return value instanceof Error ? value.cause : undefined;
  } catch {
    return undefined;
  }
}

/**
 * `error` in words, and each cause it carries after it (a trap's own error, a download's),
 * never the text of the error alone: in finite time and never throwing, whatever was
 * thrown, a cause met again (a cycle of causes) ending the words.
 * @param error What was thrown or rejected.
 * @returns Its words, then `caused by` each cause's.
 */
export function failure(error: unknown): string {
  const words = [text(error)];
  const seen = new Set<unknown>([error]);
  for (let cause = causeOf(error); cause !== undefined && !seen.has(cause); cause = causeOf(cause)) {
    seen.add(cause);
    words.push(`caused by ${text(cause)}`);
  }
  return words.join(", ");
}
