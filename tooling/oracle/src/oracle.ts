/**
 * The verdict of the oracle on a conformance corpus: for every case, the WAV file the
 * engine wrote must declare the case's rate and duration, hash to the reference, and
 * contain the expected signal.
 */

import { renderHash } from "./hash.ts";
import { InputError, parseCase, parseExpectations, parseReferences, type Expectation } from "./inputs.ts";
import { channel, sineError, sineErrorBound } from "./signal.ts";
import { WavError, parseWav } from "./wav.ts";

/** Everything the oracle reads, as found on disk. */
export interface Corpus {
  /** Case name → the parsed JSON of its case file. */
  readonly cases: ReadonlyMap<string, unknown>;
  /** The parsed JSON of `references.json`. */
  readonly references: unknown;
  /** The parsed JSON of `expectations.json`. */
  readonly expectations: unknown;
  /** Case name → the bytes of its WAV file. */
  readonly wavs: ReadonlyMap<string, Uint8Array>;
}

/** What the oracle found: one line per case or problem, and the number of problems. */
export interface Verdict {
  readonly lines: readonly string[];
  readonly failures: number;
}

/** The result of judging one case: its summary when it passes, else its problems. */
type Judgement = { readonly ok: string } | { readonly problems: readonly string[] };

/** Judges one case whose four inputs are all present. */
function judgeCase(name: string, raw: unknown, reference: string, expectation: Expectation, bytes: Uint8Array): Judgement {
  let wav;
  let declared;
  try {
    declared = parseCase(raw, name);
    wav = parseWav(bytes);
  } catch (error) {
    if (error instanceof InputError || error instanceof WavError) {
      return { problems: [error.message] };
    }
    throw error;
  }
  const problems: string[] = [];
  const shape = (what: string, actual: number, expected: number): void => {
    if (actual !== expected) {
      problems.push(`${what}: the WAV file has ${String(actual)}, expected ${String(expected)}`);
    }
  };
  shape("sample rate (case)", wav.rate, declared.rate);
  shape("sample rate (expectations)", wav.rate, expectation.rate);
  shape("frames (case)", wav.frames, declared.frames);
  shape("frames (expectations)", wav.frames, expectation.frames);
  shape("channels (expectations)", wav.channels, expectation.channels.length);
  const hash = renderHash(wav);
  if (hash !== reference) {
    problems.push(`hash: the WAV file hashes to ${hash}, the reference is ${reference}`);
  }
  let worst = 0;
  expectation.channels.forEach((expected, index) => {
    if (index < wav.channels) {
      const error = sineError(channel(wav, index), wav.rate, expected.sine);
      const bound = sineErrorBound(expected.sine.amplitude, wav.frames);
      worst = Math.max(worst, error);
      if (!(error <= bound)) {
        problems.push(`channel ${String(index)}: off the expected sine by ${error.toExponential(2)} (at most ${bound.toExponential(2)})`);
      }
    }
  });
  return problems.length > 0 ? { problems } : { ok: `${hash} (${String(wav.channels)} channel(s), sine error ${worst.toExponential(2)})` };
}

/** Judges every case of the corpus, sorted by name. */
export function judge(corpus: Corpus): Verdict {
  const references = parseReferences(corpus.references);
  const expectations = parseExpectations(corpus.expectations);
  const names = [...new Set([...corpus.cases.keys(), ...references.keys(), ...expectations.keys(), ...corpus.wavs.keys()])].sort();
  if (names.length === 0) {
    // A corpus judged empty proves nothing: it is a failure, not a pass.
    return { lines: ["FAIL     the corpus has no case"], failures: 1 };
  }
  const lines: string[] = [];
  let failures = 0;
  for (const name of names) {
    const raw = corpus.cases.get(name);
    const reference = references.get(name);
    const expectation = expectations.get(name);
    const bytes = corpus.wavs.get(name);
    const missing = [
      raw === undefined ? "case file" : undefined,
      reference === undefined ? "reference hash" : undefined,
      expectation === undefined ? "expectations" : undefined,
      bytes === undefined ? "WAV file" : undefined,
    ].filter((piece) => piece !== undefined);
    const judgement: Judgement =
      raw === undefined || reference === undefined || expectation === undefined || bytes === undefined
        ? { problems: [`missing ${missing.join(", ")}`] }
        : judgeCase(name, raw, reference, expectation, bytes);
    if ("ok" in judgement) {
      lines.push(`ok       ${name} ${judgement.ok}`);
    } else {
      failures += judgement.problems.length;
      lines.push(...judgement.problems.map((problem) => `FAIL     ${name}: ${problem}`));
    }
  }
  return { lines, failures };
}
