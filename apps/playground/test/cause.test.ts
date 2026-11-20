// The law of the remedies: each remedy fits every failure of its cause, whatever explains it
// (the square, the sound), so that a remedy is never chosen by a stage that holds several
// causes (deep review 3 found a defect of the page's own told to try another browser).
import { expect, test } from "vitest";
import type { Cause } from "../src/cause.ts";
import { SOUND_REMEDY } from "../src/player.ts";
import { REMEDY } from "../src/Square.tsx";

/** What a remedy of each cause must say, and must never say: the words of the remedies of the other causes. */
const WORDS: Readonly<Record<Cause, { readonly says: readonly string[]; readonly never: readonly string[] }>> = {
  // Try again, then the connection; never the browser, which cannot fix a download.
  network: { says: ["again", "connection"], never: ["browser", "details"] },
  // This browser or machine; never a reload, which cannot help, nor the connection.
  environment: { says: [], never: ["reload", "connection"] },
  // Try again, then the details; never the browser nor the connection, which are not at fault.
  defect: { says: ["again"], never: ["browser", "connection", "output"] },
};

/** Whether `key` is a cause: a key of {@link WORDS}. */
function isCause(key: string): key is Cause {
  return key in WORDS;
}

/** What `remedy`, given for a failure of `cause`, says that it must not, or lacks, in any case. */
function faults(cause: Cause, remedy: string): string[] {
  const { says, never } = WORDS[cause];
  const words = remedy.toLowerCase();
  return [...says.filter((word) => !words.includes(word)).map((word) => `${cause}: lacks "${word}"`), ...never.filter((word) => words.includes(word)).map((word) => `${cause}: says "${word}"`)];
}

test("gives each failure the remedy of its cause, the square's and the sound's alike", () => {
  for (const remedies of [REMEDY, SOUND_REMEDY]) {
    expect(Object.entries(remedies).flatMap(([cause, remedy]) => (isCause(cause) ? faults(cause, remedy) : [`not a cause: ${cause}`]))).toStrictEqual([]);
  }
});

test("finds the remedies review 2 left: by stage, a defect told to try another browser", () => {
  // The module stage held the page's own defects, with this browser's remedy; and the
  // download's remedy sent to the connection first, true of no server's error.
  expect(faults("defect", "This browser cannot show it; its details say what failed.")).toStrictEqual(['defect: lacks "again"', 'defect: says "browser"']);
  expect(faults("network", "Check your connection, then reload the page.")).toStrictEqual(['network: lacks "again"']);
  expect(faults("environment", "Reload the page; if it fails again, check your connection.")).toStrictEqual(['environment: says "reload"', 'environment: says "connection"']);
});
