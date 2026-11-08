import { existsSync, readFileSync } from "node:fs";
import { describe, expect, test } from "vitest";
import { interleave, renderHash } from "../ts/hash.ts";

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

/** Decodes hexadecimal digits, two per byte. */
function hexBytes(digits: string): Uint8Array<ArrayBuffer> {
  return Uint8Array.from(digits.match(/../gu) ?? [], (pair) => Number.parseInt(pair, 16));
}

/** Writes bytes as lowercase hexadecimal digits. */
function hexText(bytes: Uint8Array): string {
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

/** The vectors file, parsed. */
const SPEC: unknown = JSON.parse(readFileSync(vectorsFile(), "utf8"));

/** A known-answer vector: a render, the bytes it hashes and its hash. */
interface Vector {
  readonly name: string;
  readonly rate: number;
  readonly channels: number;
  /** The interleaved samples, as bit patterns. */
  readonly samples: Uint32Array;
  readonly bytes: Uint8Array<ArrayBuffer>;
  readonly sha256: string;
}

/** Every vector of the specification. */
const VECTORS: readonly Vector[] = list(SPEC, "vectors").map((vector) => ({
  name: text(vector, "name"),
  rate: integer(vector, "rate"),
  channels: integer(vector, "channels"),
  samples: Uint32Array.from(list(vector, "samples"), (bits) => Number.parseInt(String(bits), 16)),
  bytes: hexBytes(text(vector, "bytes")),
  sha256: text(vector, "sha256"),
}));

describe("renderHash", () => {
  test("reads every known-answer vector of the specification", () => {
    expect(text(SPEC, "scheme")).toBe("render-hash/1");
    expect(VECTORS.length).toBeGreaterThanOrEqual(3);
  });

  test.each(VECTORS)("reproduces the known-answer vector $name", async (vector) => {
    expect(await renderHash(vector.rate, vector.channels, new Float32Array(vector.samples.buffer))).toBe(vector.sha256);
    // The vector is consistent: its bytes give its hash.
    expect(hexText(new Uint8Array(await crypto.subtle.digest("SHA-256", vector.bytes)))).toBe(vector.sha256);
  });

  test("hashes the rate and the channel count of an empty render", async () => {
    expect(await renderHash(48_000, 1, new Float32Array())).not.toBe(await renderHash(44_100, 1, new Float32Array()));
    expect(await renderHash(48_000, 1, new Float32Array())).not.toBe(await renderHash(48_000, 2, new Float32Array()));
  });

  test("covers every bit of every sample, NaN payloads included", async () => {
    const bits = new Uint32Array([0x7fc00000, 0x3f000000]);
    const other = new Uint32Array([0x7fc00001, 0x3f000000]);
    expect(await renderHash(48_000, 1, new Float32Array(bits.buffer))).not.toBe(await renderHash(48_000, 1, new Float32Array(other.buffer)));
    const ulp = new Uint32Array([0x7fc00000, 0x3f000001]);
    expect(await renderHash(48_000, 1, new Float32Array(bits.buffer))).not.toBe(await renderHash(48_000, 1, new Float32Array(ulp.buffer)));
  });

  test("reads only the samples of a view", async () => {
    const whole = new Float32Array([9, 0, 0.5, -0.5, 1, 9]);
    expect(await renderHash(48_000, 2, whole.subarray(1, 5))).toBe(await renderHash(48_000, 2, new Float32Array([0, 0.5, -0.5, 1])));
  });

  test("is 64 lowercase hex digits", async () => {
    expect(await renderHash(1, 1, new Float32Array([1e-45]))).toMatch(/^[0-9a-f]{64}$/);
  });
});

describe("interleave", () => {
  test("puts frames one after the other", () => {
    expect(interleave([new Float32Array([1, 2]), new Float32Array([3, 4])])).toStrictEqual(new Float32Array([1, 3, 2, 4]));
  });

  test("takes its length from the first channel, padding shorter ones with zeros", () => {
    expect(interleave([new Float32Array([1, 2]), new Float32Array([3])])).toStrictEqual(new Float32Array([1, 3, 2, 0]));
  });

  test("ignores samples beyond the length of the first channel", () => {
    expect(interleave([new Float32Array([1]), new Float32Array([2, 3])])).toStrictEqual(new Float32Array([1, 2]));
  });

  test("keeps every bit, NaN payloads included", () => {
    const signaling = new Float32Array(new Uint32Array([0x7f800001, 0xffa00000]).buffer);
    const quiet = new Float32Array(new Uint32Array([0x7fc00001]).buffer);
    const out = interleave([signaling.subarray(0, 1), quiet]);
    expect(Array.from(new Uint32Array(out.buffer))).toStrictEqual([0x7f800001, 0x7fc00001]);
    const offset = interleave([signaling.subarray(1)]);
    expect(Array.from(new Uint32Array(offset.buffer))).toStrictEqual([0xffa00000]);
  });

  test("accepts no channels", () => {
    expect(interleave([])).toStrictEqual(new Float32Array());
  });
});
