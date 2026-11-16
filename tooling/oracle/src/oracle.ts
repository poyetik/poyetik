/**
 * The verdict of the oracle on a conformance corpus: for every case and every render
 * format, the WAV file the engine wrote must declare the case's rate and duration and the
 * format's sample width, hash to the reference of that format, and contain the expected
 * signal within the bound of that format.
 */

import { RENDER_FORMATS, sampleBits, type RenderFormat } from "./format.ts";
import { renderHash, STRICT } from "./hash.ts";
import { InputError, parseCase, parseExpectations, parseReferences, type Case, type Expectation } from "./inputs.ts";
import { describe, judgeQuality, UNMEASURED, worst, type Levels } from "./quality.ts";
import { judgeLimited, limitedPlan, needMargin, type LimitedPlan } from "./limited.ts";
import { channel, sineError, sineErrorBound, type Sine } from "./signal.ts";
import { WavError, parseWav, type Wav } from "./wav.ts";

/** Everything the oracle reads, as found on disk. */
export interface Corpus {
  /** Case name → the text of its case file, parsed by `parseCase` (which needs the text of its numbers). */
  readonly cases: ReadonlyMap<string, string>;
  /** The parsed JSON of `references.json`. */
  readonly references: unknown;
  /** The parsed JSON of `expectations.json`. */
  readonly expectations: unknown;
  /** `<case>.<format>` (the file name without `.wav`) → the bytes of that WAV file. */
  readonly wavs: ReadonlyMap<string, Uint8Array>;
}

/** What the oracle found: one line per case and format or problem, and the number of problems. */
export interface Verdict {
  /** What to print on stdout, in order: one line per case and format, or per problem. */
  readonly lines: readonly string[];
  /** Number of problems found: 0 when every case passes, the exit code then being 0. */
  readonly failures: number;
}

/** The result of judging one render: its summary when it passes, else its problems. */
type Judgement = { readonly ok: string } | { readonly problems: readonly string[] };

/** One render of a case to judge: its format, its WAV file and its reference hash. */
interface Render {
  /** The render format. */
  readonly format: RenderFormat;
  /** The bytes of its WAV file. */
  readonly bytes: Uint8Array;
  /** Its reference hash. */
  readonly reference: string;
}

/** What a case's channels must hold, its expectation joined with its output stage: limited channels and their plans, or sines. */
export type Joined = { readonly plans: readonly LimitedPlan[] } | { readonly sines: readonly Sine[] };

/** A sine's frequency, amplitude and phase, as one key: two sines are the same exactly when their keys are. */
function sineKey(sine: Sine): string {
  return JSON.stringify([sine.frequency, sine.amplitude, sine.phase]);
}

/**
 * A case joined with its expectation, or why the corpus cannot be judged, a fault of the
 * corpus rather than of a render. A protected case limits every channel by one gain
 * (decision 12: the channels are linked), so each of its channels must be the same limited
 * sine, judged by the plan the oracle derives for it (`limitedPlan`, which may refuse it); a
 * raw case has no limiter, so each of its channels must be a sine.
 */
export function join(declared: Case, expectation: Expectation): Joined | string {
  const sines = expectation.channels.flatMap((expected) => ("sine" in expected ? [expected.sine] : []));
  const limited = expectation.channels.flatMap((expected) => ("limited" in expected ? [expected.limited] : []));
  if (declared.output.mode === "raw") {
    return limited.length === 0 ? { sines } : "a limited channel needs a protected output";
  }
  if (sines.length > 0 || new Set(limited.map(sineKey)).size > 1) {
    return "a protected output links its channels: every channel must be the same limited sine";
  }
  const plans: LimitedPlan[] = [];
  for (const sine of limited) {
    const plan = limitedPlan(declared.output, declared.rate, declared.frames, sine, needMargin(declared.output, declared.rate));
    if (typeof plan === "string") {
      return plan;
    }
    plans.push(plan);
  }
  return { plans };
}

/** Judges one render of a case whose inputs are all present, read and joined (`joined`). */
function judgeRender(declared: Case, expectation: Expectation, joined: Joined, render: Render): Judgement {
  let wav;
  try {
    wav = parseWav(render.bytes);
  } catch (error) {
    if (error instanceof WavError) {
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
  shape("bits per sample (format)", wav.bits, sampleBits(render.format));
  if (wav.bits !== sampleBits(render.format)) {
    // Neither its hash nor its signal can be read in this format.
    return { problems };
  }
  const hash = renderHash(wav, render.format, STRICT);
  if (hash !== render.reference) {
    problems.push(`hash: the WAV file hashes to ${hash}, the reference is ${render.reference}`);
  }
  const said = "plans" in joined ? judgeLimitedChannels(wav, joined.plans.slice(0, wav.channels), render.format, problems) : judgeSines(wav, joined.sines.slice(0, wav.channels), render.format, problems);
  return problems.length > 0 ? { problems } : { ok: `${hash} (${String(wav.channels)} channel(s), ${said})` };
}

/** Judges each channel of `wav` limited by its plan of `plans`, adding their problems to `problems`; says each passing channel's fundamental and THD+N. */
function judgeLimitedChannels(wav: Wav, plans: readonly LimitedPlan[], format: RenderFormat, problems: string[]): string {
  const notes: string[] = [];
  plans.forEach((plan, index) => {
    const limited = judgeLimited(channel(wav, index), plan, format);
    if ("problems" in limited) {
      problems.push(...limited.problems.map((problem) => `channel ${String(index)}: ${problem}`));
    } else {
      notes.push(`channel ${String(index)} limited, fundamental ${limited.fundamental.toFixed(6)}, THD+N ${limited.thdPlusN.toFixed(1)} dB`);
    }
  });
  return notes.join("; ");
}

/** Judges each channel of `wav` against its sine of `sines`, adding their problems to `problems`; says the largest sine error and the worst levels. */
function judgeSines(wav: Wav, sines: readonly Sine[], format: RenderFormat, problems: string[]): string {
  let largest = 0;
  const measured: Levels[] = [];
  sines.forEach((sine, index) => {
    const samples = channel(wav, index);
    const error = sineError(samples, wav.rate, sine);
    const bound = sineErrorBound(sine.amplitude, wav.frames, format);
    largest = Math.max(largest, error);
    if (!(error <= bound)) {
      problems.push(`channel ${String(index)}: off the expected sine by ${error.toExponential(2)} (at most ${bound.toExponential(2)})`);
    }
    const quality = judgeQuality(samples, wav.rate, sine, format);
    if (quality.kind === "failed") {
      problems.push(...quality.problems.map((problem) => `channel ${String(index)}: ${problem}`));
    } else if (quality.kind === "measured") {
      measured.push(quality.levels);
    }
  });
  const combined = worst(measured);
  return `sine error ${largest.toExponential(2)}; ${combined === null ? `levels not measured: ${UNMEASURED}` : describe(combined)}`;
}

/**
 * The case and the render format a WAV file is named after (`<case>.<format>`, without
 * `.wav`), or null when its name ends with no render format.
 */
function wavName(key: string): { readonly name: string; readonly format: RenderFormat } | null {
  const format = RENDER_FORMATS.find((candidate) => key.endsWith(`.${candidate}`));
  return format === undefined ? null : { name: key.slice(0, -format.length - 1), format };
}

/** Judges every case of the corpus, sorted by name, each in every render format. */
export function judge(corpus: Corpus): Verdict {
  const references = parseReferences(corpus.references);
  const expectations = parseExpectations(corpus.expectations);
  const lines: string[] = [];
  let failures = 0;
  const fail = (line: string): void => {
    lines.push(`FAIL     ${line}`);
    failures += 1;
  };
  const named = [...corpus.wavs.keys()].map((key) => ({ key, parsed: wavName(key) }));
  const names = [
    ...new Set([...corpus.cases.keys(), ...references.keys(), ...expectations.keys(), ...named.flatMap(({ parsed }) => (parsed === null ? [] : [parsed.name]))]),
  ].sort();
  if (names.length === 0 && named.length === 0) {
    // A corpus judged empty proves nothing: it is a failure, not a pass.
    return { lines: ["FAIL     the corpus has no case"], failures: 1 };
  }
  for (const { key, parsed } of named) {
    if (parsed === null) {
      fail(`${key}.wav: named after no render format (${RENDER_FORMATS.join(", ")})`);
    }
  }
  for (const name of names) {
    const raw = corpus.cases.get(name);
    const hashes = references.get(name);
    const expectation = expectations.get(name);
    const missing = [raw === undefined ? "case file" : undefined, hashes === undefined ? "reference hashes" : undefined, expectation === undefined ? "expectations" : undefined].filter(
      (piece) => piece !== undefined,
    );
    if (raw === undefined || hashes === undefined || expectation === undefined) {
      fail(`${name}: missing ${missing.join(", ")}`);
      continue;
    }
    let joined;
    let declared;
    try {
      declared = parseCase(raw, name);
      joined = join(declared, expectation);
    } catch (error) {
      if (error instanceof InputError) {
        fail(`${name}: ${error.message}`);
        continue;
      }
      throw error;
    }
    if (typeof joined === "string") {
      fail(`${name}: invalid conformance input: ${joined}`);
      continue;
    }
    for (const [format, reference] of hashes) {
      const bytes = corpus.wavs.get(`${name}.${format}`);
      const judgement: Judgement = bytes === undefined ? { problems: ["missing WAV file"] } : judgeRender(declared, expectation, joined, { format, bytes, reference });
      if ("ok" in judgement) {
        lines.push(`ok       ${name} ${format} ${judgement.ok}`);
      } else {
        judgement.problems.forEach((problem) => {
          fail(`${name} ${format}: ${problem}`);
        });
      }
    }
  }
  return { lines, failures };
}
