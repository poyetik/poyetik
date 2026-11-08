import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { expect, test } from "vitest";
import { renderHash } from "../src/hash.ts";
import { parseWav } from "../src/wav.ts";
import { makeWav } from "./fixtures.ts";

/**
 * The known-answer vectors of the specification, found by walking up from this file: the
 * tests also run from a copy of the package (mutation testing sandbox), deeper in the
 * repository.
 */
function vectorsFile(): URL {
  for (let directory = new URL("./", import.meta.url); directory.pathname !== "/"; directory = new URL("../", directory)) {
    const candidate = new URL("spec/render-hash-vectors.json", directory);
    if (existsSync(candidate)) {
      return candidate;
    }
  }
  throw new Error("spec/render-hash-vectors.json not found");
}

/** Reads a field of a JSON object. */
function field(json: unknown, key: string): unknown {
  return typeof json === "object" && json !== null ? Reflect.get(json, key) : undefined;
}

/** Reads a text field. */
function text(json: unknown, key: string): string {
  const value = field(json, key);
  if (typeof value !== "string") {
    throw new TypeError(`${key} is not a string`);
  }
  return value;
}

/** Reads a whole-number field. */
function integer(json: unknown, key: string): number {
  const value = field(json, key);
  if (typeof value !== "number" || !Number.isSafeInteger(value)) {
    throw new TypeError(`${key} is not a whole number`);
  }
  return value;
}

/** Reads a list field. */
function list(json: unknown, key: string): unknown[] {
  const value = field(json, key);
  if (!Array.isArray(value)) {
    throw new TypeError(`${key} is not a list`);
  }
  return value;
}

/** The vectors file, parsed. */
const SPEC: unknown = JSON.parse(readFileSync(vectorsFile(), "utf8"));

/** A known-answer vector: a render as WAV data, the bytes it hashes and its hash. */
interface Vector {
  /** Name of the vector, shown in the test title. */
  readonly name: string;
  /** Sample rate it was rendered at, in hertz: part of what is hashed. */
  readonly rate: number;
  /** Number of channels: part of what is hashed. */
  readonly channels: number;
  /** The interleaved samples as stored in a WAV file: the little-endian bits of each. */
  readonly data: Uint8Array;
  /** The exact bytes the hash covers, as the specification gives them. */
  readonly bytes: Buffer;
  /** The expected SHA-256 of those bytes, in lowercase hexadecimal. */
  readonly sha256: string;
}

/** Every vector of the specification. */
const VECTORS: readonly Vector[] = list(SPEC, "vectors").map((vector) => {
  const samples = list(vector, "samples");
  const data = new Uint8Array(4 * samples.length);
  const view = new DataView(data.buffer);
  samples.forEach((bits, index) => {
    view.setUint32(4 * index, Number.parseInt(String(bits), 16), true);
  });
  return {
    name: text(vector, "name"),
    rate: integer(vector, "rate"),
    channels: integer(vector, "channels"),
    data,
    bytes: Buffer.from(text(vector, "bytes"), "hex"),
    sha256: text(vector, "sha256"),
  };
});

test("reads every known-answer vector of the specification", () => {
  expect(text(SPEC, "scheme")).toBe("render-hash/1");
  expect(VECTORS.length).toBeGreaterThanOrEqual(3);
});

test.each(VECTORS)("reproduces the known-answer vector $name", (vector) => {
  const frames = vector.data.length / 4 / vector.channels;
  expect(renderHash({ rate: vector.rate, channels: vector.channels, frames, data: vector.data })).toBe(vector.sha256);
  // The vector is consistent: its bytes give its hash.
  expect(createHash("sha256").update(vector.bytes).digest("hex")).toBe(vector.sha256);
});

test("hashes the WAV data it reads", () => {
  const stereo = parseWav(makeWav({ rate: 48_000, channels: 2, samples: [0, 0.5, -0.5, 1] }));
  const vector = VECTORS.find((candidate) => candidate.name === "stereo-48000");
  expect(renderHash(stereo)).toBe(vector?.sha256);
});

test("counts samples, not frames", () => {
  const stereo = parseWav(makeWav({ rate: 48_000, channels: 2, samples: [0, 0.5, -0.5, 1] }));
  expect(renderHash({ ...stereo, frames: 1 })).not.toBe(renderHash(stereo));
  expect(renderHash({ ...stereo, channels: 4, frames: 1 })).not.toBe(renderHash(stereo));
});
