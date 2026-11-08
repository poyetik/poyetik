/**
 * The conformance files the oracle reads, parsed at the boundary: cases, reference
 * hashes and expectations arrive as `unknown` and leave typed, or are rejected.
 */

import { MAX_FRAMES, type Sine } from "./signal.ts";

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

/** What a case declares: its rate and duration. */
export interface Case {
  readonly rate: number;
  readonly frames: number;
}

/**
 * What one channel of a case must contain. The allowed distance from it is derived
 * (`sineErrorBound` in `signal.ts`), never declared.
 */
export interface ChannelExpectation {
  readonly sine: Sine;
}

/** What the render of a case must be. */
export interface Expectation {
  readonly rate: number;
  readonly frames: number;
  readonly channels: readonly ChannelExpectation[];
}

/** Reads a field of an object. */
function field(value: unknown, name: string): unknown {
  return typeof value === "object" && value !== null ? Reflect.get(value, name) : undefined;
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
 * Reads a number that is exactly a binary32 value, or rejects `where`. A frequency is the
 * value of a box parameter, and the patch format stores parameters as binary32
 * (spec/patch-format.md): the engine rounds the decimal text to binary32 once. Rounding
 * here what `JSON.parse` already rounded to binary64 would round twice, and differ by an
 * ulp from a text near a binary32 midpoint. A binary64 that is a binary32 value lies far
 * from every midpoint, so its text, whatever its digits, rounds to it once or twice alike.
 */
function binary32(value: unknown, where: string): number {
  const read = number(value, where);
  if (Math.fround(read) !== read) {
    throw new InputError(`${where} is not a binary32 value (parameters are binary32: give the value the engine plays)`);
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

/** Reads a duration: a positive whole number up to {@link MAX_FRAMES}, or rejects `where`. */
function frames(value: unknown, where: string): number {
  const read = count(value, where);
  if (read > MAX_FRAMES) {
    throw new InputError(`${where} is above ${String(MAX_FRAMES)}, the longest case with an exact reference`);
  }
  return read;
}

/** The entries of a JSON object, or rejects `where`. */
function entries(value: unknown, where: string): [string, unknown][] {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new InputError(`${where} is not an object`);
  }
  return Object.entries(value);
}

/** Reads a case file: its sample rate and duration (the same domain as the expectations). */
export function parseCase(value: unknown, name: string): Case {
  return { rate: count(field(value, "sample_rate"), `${name}: sample_rate`), frames: frames(field(value, "frames"), `${name}: frames`) };
}

/** Reads `references.json`: case name → hash. */
export function parseReferences(value: unknown): Map<string, string> {
  return new Map(
    entries(value, "references.json").map(([name, hash]) => {
      if (typeof hash !== "string" || !/^[0-9a-f]{64}$/.test(hash)) {
        throw new InputError(`references.json: ${name} is not a SHA-256 hash`);
      }
      return [name, hash];
    }),
  );
}

/** Reads one channel expectation: an object with a `sine` and nothing else. */
function parseChannel(value: unknown, where: string): ChannelExpectation {
  const extra = entries(value, where).find(([key]) => key !== "sine");
  if (extra !== undefined) {
    throw new InputError(`${where}: unknown field ${extra[0]} (tolerances are derived, not declared)`);
  }
  const sine = field(value, "sine");
  return {
    sine: {
      frequency: binary32(field(sine, "frequency"), `${where}: sine.frequency`),
      amplitude: number(field(sine, "amplitude"), `${where}: sine.amplitude`),
      phase: phase(field(sine, "phase"), `${where}: sine.phase`),
    },
  };
}

/** Reads `expectations.json`: case name → what its render must be. */
export function parseExpectations(value: unknown): Map<string, Expectation> {
  return new Map(
    entries(value, "expectations.json").map(([name, expectation]) => {
      const where = `expectations.json: ${name}`;
      const channels = field(expectation, "channels");
      if (!Array.isArray(channels) || channels.length === 0) {
        throw new InputError(`${where}: channels is not a non-empty list`);
      }
      return [
        name,
        {
          rate: count(field(expectation, "sample_rate"), `${where}: sample_rate`),
          frames: frames(field(expectation, "frames"), `${where}: frames`),
          channels: channels.map((channel: unknown, index) => parseChannel(channel, `${where}: channel ${String(index)}`)),
        },
      ];
    }),
  );
}
