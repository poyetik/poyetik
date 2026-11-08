/**
 * The conformance files the oracle reads, parsed at the boundary: cases, reference
 * hashes and expectations arrive as `unknown` and leave typed, or are rejected.
 */

import type { Sine } from "./signal.ts";

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

/** What one channel of a case must contain. */
export interface ChannelExpectation {
  readonly sine: Sine;
  /** Largest allowed distance from the ideal sine. */
  readonly maxError: number;
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

/** Reads a positive whole number, or rejects `where`. */
function count(value: unknown, where: string): number {
  const read = number(value, where);
  if (!Number.isInteger(read) || read <= 0) {
    throw new InputError(`${where} is not a positive whole number`);
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

/** Reads a case file: its sample rate and duration. */
export function parseCase(value: unknown, name: string): Case {
  return { rate: count(field(value, "sample_rate"), `${name}: sample_rate`), frames: count(field(value, "frames"), `${name}: frames`) };
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

/** Reads one channel expectation. */
function parseChannel(value: unknown, where: string): ChannelExpectation {
  const sine = field(value, "sine");
  return {
    sine: {
      // The frequency is the value of a box parameter, and the patch format stores
      // parameters as binary32 (spec/patch-format.md): the engine plays the nearest
      // binary32 value, so the ideal signal uses it too.
      frequency: Math.fround(number(field(sine, "frequency"), `${where}: sine.frequency`)),
      amplitude: number(field(sine, "amplitude"), `${where}: sine.amplitude`),
      phase: number(field(sine, "phase"), `${where}: sine.phase`),
    },
    maxError: number(field(value, "max_error"), `${where}: max_error`),
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
          frames: count(field(expectation, "frames"), `${where}: frames`),
          channels: channels.map((channel: unknown, index) => parseChannel(channel, `${where}: channel ${String(index)}`)),
        },
      ];
    }),
  );
}
