/**
 * The conformance files the oracle reads, parsed at the boundary: cases, reference
 * hashes and expectations arrive as `unknown` and leave typed, or are rejected.
 */

import { RENDER_FORMATS, type RenderFormat } from "./format.ts";
import { unmeasurable } from "./quality.ts";
import { MAX_EXACT_FRAMES, type Sine } from "./signal.ts";

/** Thrown when a conformance file does not have the expected shape. */
export class InputError extends Error {
  /**
   * Names the file and what is wrong.
   * @param detail What is wrong, and where.
   */
  constructor(detail: string) {
    super(`invalid conformance input: ${detail}`);
    this.name = "InputError";
  }
}

/**
 * Lowest sample rate the engine supports, in hertz (`audio_core::SampleRate::MIN`): a
 * case or an expectation below it is refused, as `audio-cli` refuses the case.
 */
export const MIN_RATE = 8000;

/** Highest sample rate the engine supports, in hertz (`audio_core::SampleRate::MAX`). */
export const MAX_RATE = 384_000;

/** The modes of a case's output stage (`spec/engine-design.md`, decision 12). */
export const OUTPUT_MODES = ["raw", "protected"] as const;

/** The detectors of the limiter's true peak, by name: how precisely each estimates it. */
export const DETECTORS = ["standard", "precise", "mastering"] as const;

/** The range of a limiter's lookahead, in milliseconds (`audio_core::Setting::Lookahead`). */
export const LOOKAHEAD_MS = { lowest: 0.5, highest: 5 } as const;

/** The output stage of a case without a limiter: it carries no setting. */
export interface RawOutput {
  /** Raw: non-finite samples replaced, nothing else. */
  readonly mode: "raw";
}

/** The output stage of a case through the true-peak limiter: its lookahead (its maximum too) and its detector, its other settings the engine's defaults. */
export interface ProtectedOutput {
  /** Through the true-peak limiter. */
  readonly mode: "protected";
  /** The limiter's lookahead and its maximum, in milliseconds. */
  readonly lookahead: number;
  /** The limiter's peak detector. */
  readonly detector: (typeof DETECTORS)[number];
}

/** The output stage of a case: raw, or protected with its limiter's settings. */
export type CaseOutput = RawOutput | ProtectedOutput;

/** What a case declares: its rate, its duration and its output stage. */
export interface Case {
  /** Sample rate, in hertz: a whole number from {@link MIN_RATE} to {@link MAX_RATE}. */
  readonly rate: number;
  /** Duration of the render, in frames (samples per channel), from 1 to `MAX_EXACT_FRAMES`. */
  readonly frames: number;
  /** The output stage; raw when the case names none. */
  readonly output: CaseOutput;
}

/** The output stage of a case that names none: raw. */
const RAW: RawOutput = Object.freeze({ mode: "raw" });

/**
 * What one channel of a case must contain. The allowed distance from it is derived
 * (`sineErrorBound` in `signal.ts`), never declared.
 */
export type ChannelExpectation =
  /** The sine the channel holds, from its first sample on. */
  | { readonly sine: Sine }
  /**
   * The sine the patch plays, rendered through the case's protected output stage: delayed
   * by its latency and limited at the default ceiling (`limited.ts`).
   */
  | { readonly limited: Sine }
  /**
   * A signal the oracle derives no bound for (a sum of sines through the limiter), with the
   * reason in words: the render is held to its reference hash and, through a protected
   * output stage, to silence over the limiter's latency and every sample to the ceiling, and its signal is not measured.
   */
  | { readonly unmeasured: string };

/** What the render of a case must be. */
export interface Expectation {
  /** Sample rate the render must have, in hertz: the one of its case. */
  readonly rate: number;
  /** Duration the render must have, in frames: the one of its case. */
  readonly frames: number;
  /** What each channel must contain, in channel order: the render has exactly as many channels. */
  readonly channels: readonly ChannelExpectation[];
}

/** Reads a field of an object (`undefined` when it has none). */
export function field(value: object, name: string): unknown {
  return Reflect.get(value, name);
}

/** Whether `value` is a finite number (`Number.isFinite` is false for anything but a number). */
function isFiniteNumber(value: unknown): value is number {
  return Number.isFinite(value);
}

/** Reads a finite number, or rejects `where`. */
function number(value: unknown, where: string): number {
  if (!isFiniteNumber(value)) {
    throw new InputError(`${where} is not a finite number`);
  }
  return value;
}

/**
 * Reads a number that is exactly a binary32 value, or rejects `where`, saying `why` it must
 * be. A frequency is the value of a box parameter, and the patch format stores parameters as
 * binary32 (spec/patch-format.md): the engine rounds the decimal text to binary32 once.
 * Rounding here what `JSON.parse` already rounded to binary64 would round twice, and differ
 * by an ulp from a text near a binary32 midpoint. A binary64 that is a binary32 value lies
 * far from every midpoint, so its text, whatever its digits, rounds to it once or twice
 * alike. An amplitude is a gain after the sine, which `sineErrorBound` counts in binary32
 * only when it is exact there.
 */
function binary32(value: unknown, where: string, why: string): number {
  const read = number(value, where);
  if (Math.fround(read) !== read) {
    throw new InputError(`${where} is not a binary32 value (${why})`);
  }
  return read;
}

/**
 * Reads a start phase in [0, 2π), or rejects `where`. `sineErrorBound` allows a few ulps
 * of a sine argument below 4π (a reduced phase plus this one); a larger phase cannot be
 * reduced by 2π exactly, and its own rounding would exceed the bound (about 6e-12 at 1e6).
 */
function phase(value: unknown, where: string): number {
  const read = number(value, where);
  if (!(read >= 0 && read < 2 * Math.PI)) {
    throw new InputError(`${where} is not in [0, 2π)`);
  }
  return read;
}

/** Reads a positive whole number, or rejects `where`. */
function count(value: unknown, where: string): number {
  const read = number(value, where);
  if (!Number.isInteger(read) || read <= 0) {
    throw new InputError(`${where} is not a positive whole number`);
  }
  return read;
}

/** Reads a duration: a positive whole number up to {@link MAX_EXACT_FRAMES}, or rejects `where`. */
function frames(value: unknown, where: string): number {
  const read = count(value, where);
  if (read > MAX_EXACT_FRAMES) {
    throw new InputError(`${where} is above ${String(MAX_EXACT_FRAMES)}, the longest case with an exact reference`);
  }
  return read;
}

/** Reads a sample rate the engine supports, from {@link MIN_RATE} to {@link MAX_RATE} hertz, or rejects `where`. */
function sampleRate(value: unknown, where: string): number {
  const read = count(value, where);
  if (read < MIN_RATE || read > MAX_RATE) {
    throw new InputError(`${where} is not a supported sample rate (${String(MIN_RATE)} to ${String(MAX_RATE)} Hz)`);
  }
  return read;
}

/** `value`, a JSON object (not an array), or rejects `where`. */
function object(value: unknown, where: string): object {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new InputError(`${where} is not an object`);
  }
  return value;
}

/** The entries of a JSON object, or rejects `where`. */
function entries(value: unknown, where: string): [string, unknown][] {
  return Object.entries(object(value, where));
}

/**
 * `value`, a JSON object with no field but `keys`, or rejects `where` (a missing field is
 * rejected by its reader); `why` is added to the message of an unknown field.
 */
export function onlyKeys(value: unknown, keys: readonly string[], where: string, why = ""): object {
  const checked = object(value, where);
  const extra = Object.keys(checked).find((key) => !keys.includes(key));
  if (extra !== undefined) {
    throw new InputError(`${where}: unknown field ${extra}${why}`);
  }
  return checked;
}

/** The integer fields of a case file, whose numbers must be written as whole numbers. */
const WHOLE_FIELDS: readonly string[] = ["sample_rate", "frames"];

/**
 * A `JSON.parse` reviver that refuses a number of an integer field ({@link WHOLE_FIELDS})
 * not written as a whole number, without fraction or exponent (`100.0`, `1e2`), as the
 * engine's reader refuses it: it reads the text of each number (`context.source`), which
 * the parsed value has lost.
 */
function wholeNumbers(name: string): (this: unknown, key: string, value: unknown, ...context: unknown[]) => unknown {
  return (key, value, ...context) => {
    // Run once the shape is checked: the integer fields are numbers by then.
    if (WHOLE_FIELDS.includes(key) && !/^-?\d+$/.test(String(Reflect.get(Object(context[0]), "source")))) {
      throw new InputError(`${name}: ${key} is not a whole number written without fraction or exponent`);
    }
    return value;
  };
}

/**
 * Reads the text of a case file, as `audio-cli` reads it: a JSON object with the fields
 * `sample_rate` (a sample rate the engine supports) and `frames` (1 to `MAX_EXACT_FRAMES`),
 * each a whole number written without fraction or exponent, and optionally `output`, the
 * output stage of its render (an object of `mode`, and `lookahead` and `detector` when
 * protected; raw when absent).
 *
 * The text is parsed twice. First plainly (`JSON.parse` does not recurse, however deep
 * the text), to check the fields and their values; then, once it is known to be an object
 * of two numbers and at most one output object of checked fields, each a number or a
 * string (two levels deep at most), with the reviver that checks how the numbers are
 * written: a reviver walks the value recursively, and a deeply nested text (a few
 * kilobytes of `[`) would overflow the stack. The reviver's fields ({@link WHOLE_FIELDS})
 * are never keys of the output object.
 * @throws {InputError} Naming the case `name`, when the text is not such an object.
 */
export function parseCase(text: string, name: string): Case {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch (error) {
    throw new InputError(`${name}: not JSON (${String(error)})`);
  }
  const checked = onlyKeys(value, ["sample_rate", "frames", "output"], name);
  const output = Object.hasOwn(checked, "output") ? parseOutput(field(checked, "output"), `${name}: output`) : RAW;
  const read = { rate: sampleRate(field(checked, "sample_rate"), `${name}: sample_rate`), frames: frames(field(checked, "frames"), `${name}: frames`), output };
  JSON.parse(text, wholeNumbers(name));
  return read;
}

/** `value` if it is one of `allowed`, or rejects `where`. */
function oneOf<T extends string>(value: unknown, allowed: readonly T[], where: string): T {
  const found = allowed.find((known) => known === value);
  if (found === undefined) {
    throw new InputError(`${where} is not one of ${allowed.join(", ")}`);
  }
  return found;
}

/**
 * Reads the output stage of a case, as `audio-cli` reads it: an object with a `mode` (raw or
 * protected); a raw one with no other field, a protected one with exactly a `lookahead` in
 * milliseconds within {@link LOOKAHEAD_MS} and a `detector`; never `null`.
 */
function parseOutput(value: unknown, where: string): CaseOutput {
  const checked = onlyKeys(value, ["mode", "lookahead", "detector"], where);
  const mode = oneOf(field(checked, "mode"), OUTPUT_MODES, `${where}: mode`);
  if (mode === "raw") {
    const setting = ["lookahead", "detector"].find((name) => Object.hasOwn(checked, name));
    if (setting !== undefined) {
      throw new InputError(`${where}: a raw output stage takes no ${setting}`);
    }
    return RAW;
  }
  const lookahead: unknown = field(checked, "lookahead");
  if (typeof lookahead !== "number" || !(lookahead >= LOOKAHEAD_MS.lowest && lookahead <= LOOKAHEAD_MS.highest)) {
    throw new InputError(`${where}: lookahead is not a number of milliseconds from ${String(LOOKAHEAD_MS.lowest)} to ${String(LOOKAHEAD_MS.highest)}`);
  }
  return { mode, lookahead, detector: oneOf(field(checked, "detector"), DETECTORS, `${where}: detector`) };
}

/**
 * Reads `references.json`: case name → render format → hash, every case with exactly the
 * three render formats of {@link RENDER_FORMATS}.
 */
export function parseReferences(value: unknown): Map<string, ReadonlyMap<RenderFormat, string>> {
  return new Map(
    entries(value, "references.json").map(([name, hashes]) => {
      const where = `references.json: ${name}`;
      const checked = onlyKeys(hashes, RENDER_FORMATS, where);
      return [
        name,
        new Map(
          RENDER_FORMATS.map((format) => {
            const hash = field(checked, format);
            if (typeof hash !== "string" || !/^[0-9a-f]{64}$/.test(hash)) {
              throw new InputError(`${where}: ${format} is not a SHA-256 hash`);
            }
            return [format, hash];
          }),
        ),
      ];
    }),
  );
}

/**
 * Reads one channel expectation: an object with exactly one of a `sine`, a `limited` sine
 * or an `unmeasured` signal. A sine has exactly a frequency, an amplitude and a phase, its
 * frequency lying in [0, 4 × `rate`) and its amplitude exact in binary32, the domain
 * `sineErrorBound` is derived for; an unmeasured signal is the reason the oracle derives no
 * bound for it, in words (a string holding more than white space).
 */
function parseChannel(value: unknown, rate: number, where: string): ChannelExpectation {
  const checked = onlyKeys(value, ["sine", "limited", "unmeasured"], where, " (tolerances are derived, not declared)");
  if (Object.keys(checked).length > 1) {
    throw new InputError(`${where} is not one sine, one limited sine or one unmeasured signal`);
  }
  if (Object.hasOwn(checked, "unmeasured")) {
    const reason = field(checked, "unmeasured");
    if (typeof reason !== "string" || !/\S/.test(reason)) {
      throw new InputError(`${where}: unmeasured is not the reason it is not measured, in words`);
    }
    return { unmeasured: reason };
  }
  const kind = Object.hasOwn(checked, "limited") ? "limited" : "sine";
  const parsed = parseSine(field(checked, kind), rate, `${where}: ${kind}`);
  return kind === "limited" ? { limited: parsed } : { sine: parsed };
}

/** Reads the sine of a channel expectation, named `where`. */
function parseSine(value: unknown, rate: number, where: string): Sine {
  const sine = onlyKeys(value, ["frequency", "amplitude", "phase"], where);
  const frequency = binary32(field(sine, "frequency"), `${where}.frequency`, "parameters are binary32: give the value the engine plays");
  if (!(frequency >= 0 && frequency < 4 * rate)) {
    throw new InputError(`${where}.frequency is not in [0, 4 × sample_rate)`);
  }
  return {
    frequency,
    amplitude: binary32(field(sine, "amplitude"), `${where}.amplitude`, "the error bound of a binary32 render counts a gain only when it is exact in binary32"),
    phase: phase(field(sine, "phase"), `${where}.phase`),
  };
}

/**
 * Reads `expectations.json`: case name → what its render must be, each with exactly a
 * `sample_rate`, `frames` and `channels`, every channel the oracle measures measurable over
 * the frames measured (refused here rather than failed as a correct render, `unmeasurable`).
 */
export function parseExpectations(value: unknown): Map<string, Expectation> {
  return new Map(
    entries(value, "expectations.json").map(([name, expectation]) => {
      const where = `expectations.json: ${name}`;
      const checked = onlyKeys(expectation, ["sample_rate", "frames", "channels"], where);
      const channels = field(checked, "channels");
      if (!Array.isArray(channels) || channels.length === 0) {
        throw new InputError(`${where}: channels is not a non-empty list`);
      }
      const rate = sampleRate(field(checked, "sample_rate"), `${where}: sample_rate`);
      const length = frames(field(checked, "frames"), `${where}: frames`);
      return [
        name,
        {
          rate,
          frames: length,
          channels: channels.map((channel: unknown, index) => {
            const parsed = parseChannel(channel, rate, `${where}: channel ${String(index)}`);
            const refusal = "sine" in parsed ? unmeasurable(parsed.sine, rate, length) : null;
            if (refusal !== null) {
              throw new InputError(`${where}: channel ${String(index)}: sine.frequency cannot be measured over ${String(length)} frames (${refusal}): lengthen the case`);
            }
            return parsed;
          }),
        },
      ];
    }),
  );
}
