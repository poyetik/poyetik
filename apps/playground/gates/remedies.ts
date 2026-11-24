// The law of the remedies: each remedy fits every failure of its cause, whatever explains it
// (the square, the sound), so that a remedy is never chosen by a stage that holds several
// causes (the third deep review of visual block 2 found a defect of the page's own told to
// try another browser); and each failure's whole explanation, the line the page shows, fits
// it, so that a clause of a shared remedy, or of the words before it, that fits one member of
// its cause alone is never told to another (the fifth found a module this browser cannot
// compile told to check the sound output). The square's lines are shared by cause: its module
// and its canvas, both this browser's, are told one text, held to one row.
import type { Cause } from "../src/cause.ts";
import type { Failure } from "../src/player.ts";
import type { SquareFailure } from "../src/Square.tsx";

/** What a text must say, and must never say, in any case. */
export interface Words {
  /** The words it must hold. */
  readonly says: readonly string[];
  /** The words it must not hold. */
  readonly never: readonly string[];
}

/** What a remedy of each cause must say, and must never say: the words of the remedies of the other causes. */
export const WORDS: Readonly<Record<Cause, Words>> = {
  // Try again, then the connection; never the browser, which cannot fix a download.
  network: { says: ["again", "connection"], never: ["browser", "details"] },
  // Something else to try (another browser, another device); never a reload, which cannot
  // help, nor the connection; nor a retry alone, which changes nothing.
  environment: { says: ["try", "another"], never: ["reload", "connection"] },
  // Try again, then the details; never the browser nor the connection, which are not at fault.
  defect: { says: ["again", "details"], never: ["browser", "connection", "output"] },
};

/** What the explanation of each failure of the sound must say, and must never say: the words of the other failures of its cause. */
export const SOUND_WORDS: Readonly<Record<Failure, Words>> = {
  load: { says: ["could not load"], never: ["output", "compile"] },
  module: { says: ["compile"], never: ["output", "connection"] },
  engine: { says: ["stopped"], never: ["output", "compile"] },
  device: { says: ["output"], never: ["compile", "connection"] },
};

/** What the line of each failure of the square must say, and must never say: nothing of the sound's, nor the words of another cause. */
export const SQUARE_WORDS: Readonly<Record<SquareFailure, Words>> = {
  download: { says: ["connection"], never: ["output", "start", "browser"] },
  module: { says: ["browser"], never: ["output", "start", "connection"] },
  defect: { says: ["details"], never: ["output", "start", "browser", "connection"] },
  canvas: { says: ["browser"], never: ["output", "start", "connection"] },
};

/** What `text`, told of `member`, lacks of `words`, or says that it must not, in any case. */
export function misfits(member: string, { says, never }: Words, text: string): readonly string[] {
  const words = text.toLowerCase();
  return [...says.filter((word) => !words.includes(word)).map((word) => `${member}: lacks "${word}"`), ...never.filter((word) => words.includes(word)).map((word) => `${member}: says "${word}"`)];
}

/** What `remedy`, given for a failure of `cause`, says that it must not, or lacks, in any case. */
export function faults(cause: Cause, remedy: string): readonly string[] {
  return misfits(cause, WORDS[cause], remedy);
}
