import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { expect, test } from "vitest";
import { RENDER_FORMATS, sampleBits, type RenderFormat } from "../src/format.ts";
import { KNOWN_OPTIONS, renderHash, STRICT } from "../src/hash.ts";
import { parseWav, type Wav } from "../src/wav.ts";
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

/** Reads a render format field. */
function format(json: unknown): RenderFormat {
  const value = text(json, "format");
  const known = RENDER_FORMATS.find((candidate) => candidate === value);
  if (known === undefined) {
    throw new TypeError(`unknown format ${value}`);
  }
  return known;
}

/** Reads an arithmetic mode: 16 hexadecimal digits. */
function mode(json: unknown): bigint {
  const digits = text(json, "arithmetic_mode");
  if (!/^[0-9a-f]{16}$/.test(digits)) {
    throw new TypeError(`${digits} is not 16 hexadecimal digits`);
  }
  return BigInt(`0x${digits}`);
}

/** A known-answer vector: a render as WAV data, the bytes it hashes and its hash. */
interface Vector {
  /** Name of the vector, shown in the test title. */
  readonly name: string;
  /** The render, as a WAV file would hold it. */
  readonly wav: Wav;
  /** Its render format. */
  readonly format: RenderFormat;
  /** Its arithmetic mode. */
  readonly mode: bigint;
  /** The exact bytes the hash covers, as the specification gives them. */
  readonly bytes: Buffer;
  /** The expected SHA-256 of those bytes, in lowercase hexadecimal. */
  readonly sha256: string;
}

/** Every vector of the specification. */
const VECTORS: readonly Vector[] = list(SPEC, "vectors").map((vector) => {
  const samples = list(vector, "samples");
  const bits = sampleBits(format(vector));
  const data = new Uint8Array((bits / 8) * samples.length);
  const view = new DataView(data.buffer);
  samples.forEach((digits, index) => {
    if (bits === 64) {
      view.setBigUint64(8 * index, BigInt(`0x${String(digits)}`), true);
    } else {
      view.setUint32(4 * index, Number.parseInt(String(digits), 16), true);
    }
  });
  const channels = integer(vector, "channels");
  return {
    name: text(vector, "name"),
    wav: { rate: integer(vector, "rate"), channels, frames: samples.length / channels, bits, data },
    format: format(vector),
    mode: mode(vector),
    bytes: Buffer.from(text(vector, "bytes"), "hex"),
    sha256: text(vector, "sha256"),
  };
});

test("reads every known-answer vector of the specification, in every render format", () => {
  expect(text(SPEC, "scheme")).toBe("render-hash/2");
  expect(VECTORS.length).toBeGreaterThanOrEqual(7);
  expect(new Set(VECTORS.map((vector) => vector.format))).toStrictEqual(new Set(RENDER_FORMATS));
});

test.each(VECTORS)("reproduces the known-answer vector $name", (vector) => {
  expect(renderHash(vector.wav, vector.format, vector.mode)).toBe(vector.sha256);
  // The vector is consistent: its bytes give its hash.
  expect(createHash("sha256").update(vector.bytes).digest("hex")).toBe(vector.sha256);
});

test.each(list(SPEC, "refused_modes").map((refused) => ({ name: text(refused, "name"), mode: mode(refused) })))("refuses the unknown mode $name", ({ mode: refused }) => {
  const [vector] = VECTORS;
  if (vector === undefined) {
    throw new Error("no vector");
  }
  expect(() => renderHash(vector.wav, vector.format, refused)).toThrow(new RangeError(`unknown arithmetic options: 0x${refused.toString(16).padStart(16, "0")}`));
});

test("accepts every known option and the strict mode, and refuses what is not a 64-bit mode", () => {
  const stereo = parseWav(makeWav({ rate: 48_000, channels: 2, samples: [0, 0.5, -0.5, 1] }));
  const hashes = [0n, 1n, 2n, 4n, 7n].map((option) => renderHash(stereo, "binary32", option));
  expect(new Set(hashes).size).toBe(5);
  expect(STRICT).toBe(0n);
  expect(KNOWN_OPTIONS).toBe(7n);
  expect(() => renderHash(stereo, "binary32", -1n)).toThrow(new RangeError("the arithmetic mode must be an unsigned 64-bit integer, not -1"));
  expect(() => renderHash(stereo, "binary32", 2n ** 64n)).toThrow(new RangeError("the arithmetic mode must be an unsigned 64-bit integer, not 18446744073709551616"));
  expect(() => renderHash(stereo, "binary32", 2n ** 64n - 1n)).toThrow(new RangeError("unknown arithmetic options: 0xffffffffffffffff"));
});

test("hashes the WAV data it reads, in its format", () => {
  const stereo = parseWav(makeWav({ rate: 48_000, channels: 2, samples: [0, 0.5, -0.5, 1], bits: 64 }));
  const vector = VECTORS.find((candidate) => candidate.name === "stereo-48000");
  expect(renderHash(stereo, "binary64", STRICT)).toBe(vector?.sha256);
  const narrow = parseWav(makeWav({ rate: 48_000, channels: 2, samples: [0, 0.5, -0.5, 1] }));
  expect(renderHash(narrow, "binary32", STRICT)).not.toBe(renderHash(narrow, "binary64-as-binary32", STRICT));
});

test("refuses samples of another width than the format's", () => {
  const narrow = parseWav(makeWav({ rate: 48_000, channels: 1, samples: [0.5] }));
  expect(() => renderHash(narrow, "binary64", STRICT)).toThrow(new RangeError("binary64 samples are 64-bit, not 32-bit"));
  const wide = parseWav(makeWav({ rate: 48_000, channels: 1, samples: [0.5], bits: 64 }));
  expect(() => renderHash(wide, "binary32", STRICT)).toThrow(new RangeError("binary32 samples are 32-bit, not 64-bit"));
  expect(() => renderHash(wide, "binary64-as-binary32", STRICT)).toThrow(new RangeError("binary64-as-binary32 samples are 32-bit, not 64-bit"));
});

test("counts samples, not frames", () => {
  const stereo = parseWav(makeWav({ rate: 48_000, channels: 2, samples: [0, 0.5, -0.5, 1] }));
  expect(renderHash({ ...stereo, frames: 1 }, "binary32", STRICT)).not.toBe(renderHash(stereo, "binary32", STRICT));
  expect(renderHash({ ...stereo, channels: 4, frames: 1 }, "binary32", STRICT)).not.toBe(renderHash(stereo, "binary32", STRICT));
});
