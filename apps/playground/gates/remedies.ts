// The law of the remedies: each remedy fits every failure of its cause, whatever explains it
// (the square, the sound), so that a remedy is never chosen by a stage that holds several
// causes (deep review 3 found a defect of the page's own told to try another browser).
import type { Cause } from "../src/cause.ts";

/** What a remedy of each cause must say, and must never say: the words of the remedies of the other causes. */
export const WORDS: Readonly<Record<Cause, { readonly says: readonly string[]; readonly never: readonly string[] }>> = {
  // Try again, then the connection; never the browser, which cannot fix a download.
  network: { says: ["again", "connection"], never: ["browser", "details"] },
  // Something to try (another browser, another device or output); never a reload, which
  // cannot help, nor the connection.
  environment: { says: ["try"], never: ["reload", "connection"] },
  // Try again, then the details; never the browser nor the connection, which are not at fault.
  defect: { says: ["again", "details"], never: ["browser", "connection", "output"] },
};

/** Whether `key` is a cause: a key of {@link WORDS}. */
export function isCause(key: string): key is Cause {
  return key in WORDS;
}

/** What `remedy`, given for a failure of `cause`, says that it must not, or lacks, in any case. */
export function faults(cause: Cause, remedy: string): string[] {
  const { says, never } = WORDS[cause];
  const words = remedy.toLowerCase();
  return [...says.filter((word) => !words.includes(word)).map((word) => `${cause}: lacks "${word}"`), ...never.filter((word) => words.includes(word)).map((word) => `${cause}: says "${word}"`)];
}
