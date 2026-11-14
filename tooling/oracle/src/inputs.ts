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

/** What a case declares: its rate and duration. */
export interface Case {
  /** Sample rate, in hertz: a whole number from {@link MIN_RATE} to {@link MAX_RATE}. */
  readonly rate: number;
  /** Duration of the render, in frames (samples per channel), from 1 to `MAX_EXACT_FRAMES`. */
  readonly frames: number;
}

/**
 * What one channel of a case must contain. The allowed distance from it is derived
 * (`sineErrorBound` in `signal.ts`), never declared.
 */
export interface ChannelExpectation {
  /** The sine the channel holds, from its first sample on. */
  readonly sine: Sine;
}

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

/**
 * A `JSON.parse` reviver that refuses any number not written as a whole number, without
 * fraction or exponent (`100.0`, `1e2`), as the engine's reader refuses it for an integer
 * field: it reads the text of each number (`context.source`), which the parsed value
 * has lost.
 */
function wholeNumbers(name: string): (this: unknown, key: string, value: unknown, ...context: unknown[]) => unknown {
  return (key, value, ...context) => {
    if (typeof value === "number" && !/^-?\d+$/.test(String(Reflect.get(Object(context[0]), "source")))) {
      throw new InputError(`${name}: ${key} is not a whole number written without fraction or exponent`);
    }
    return value;
  };
}

/**
 * Reads the text of a case file, as `audio-cli` reads it: a JSON object with exactly the
 * fields `sample_rate` (a sample rate the engine supports) and `frames` (1 to
 * `MAX_EXACT_FRAMES`), each a whole number written without fraction or exponent.
 *
 * The text is parsed twice. First plainly (`JSON.parse` does not recurse, however deep
 * the text), to check the fields and their values; then, once it is known to be a flat
 * object of two numbers, with the reviver that checks how the numbers are written: a
 * reviver walks the value recursively, and a deeply nested text (a few kilobytes of `[`)
 * would overflow the stack.
 * @throws {InputError} Naming the case `name`, when the text is not such an object.
 */
export function parseCase(text: string, name: string): Case {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch (error) {
    throw new InputError(`${name}: not JSON (${String(error)})`);
  }
  const checked = onlyKeys(value, ["sample_rate", "frames"], name);
  const read = { rate: sampleRate(field(checked, "sample_rate"), `${name}: sample_rate`), frames: frames(field(checked, "frames"), `${name}: frames`) };
  JSON.parse(text, wholeNumbers(name));
  return read;
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
 * Reads one channel expectation: an object with a `sine` and nothing else, the sine
 * having exactly a frequency, an amplitude and a phase, its frequency lying in
 * [0, 4 × `rate`) and its amplitude exact in binary32, the domain `sineErrorBound` is
 * derived for.
 */
function parseChannel(value: unknown, rate: number, where: string): ChannelExpectation {
  const sine = onlyKeys(field(onlyKeys(value, ["sine"], where, " (tolerances are derived, not declared)"), "sine"), ["frequency", "amplitude", "phase"], `${where}: sine`);
  const frequency = binary32(field(sine, "frequency"), `${where}: sine.frequency`, "parameters are binary32: give the value the engine plays");
  if (!(frequency >= 0 && frequency < 4 * rate)) {
    throw new InputError(`${where}: sine.frequency is not in [0, 4 × sample_rate)`);
  }
  return {
    sine: {
      frequency,
      amplitude: binary32(field(sine, "amplitude"), `${where}: sine.amplitude`, "the error bound of a binary32 render counts a gain only when it is exact in binary32"),
      phase: phase(field(sine, "phase"), `${where}: sine.phase`),
    },
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
            const refusal = unmeasurable(parsed.sine, rate, length);
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
