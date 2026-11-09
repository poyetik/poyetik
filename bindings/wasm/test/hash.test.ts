import { existsSync, readFileSync } from "node:fs";
import { describe, expect, test } from "vitest";
import { RENDER_FORMATS, type RenderFormat } from "../ts/engine.ts";
import { interleave, interleave64, renderHash } from "../ts/hash.ts";

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

/** A render format, read from a vector. */
function format(json: unknown): RenderFormat {
  const name = text(json, "format");
  const known = RENDER_FORMATS.find((candidate) => candidate === name);
  if (known === undefined) {
    throw new TypeError(`unknown render format ${name}`);
  }
  return known;
}

/** An arithmetic mode, read from a vector: 16 hexadecimal digits. */
function mode(json: unknown): bigint {
  const digits = text(json, "arithmetic_mode");
  if (!/^[0-9a-f]{16}$/u.test(digits)) {
    throw new TypeError(`${digits} is not 16 hexadecimal digits`);
  }
  return BigInt(`0x${digits}`);
}

/** A known-answer vector: a render, the bytes it hashes and its hash. */
interface Vector {
  /** Name of the vector, shown in the test title. */
  readonly name: string;
  /** Its render format. */
  readonly format: RenderFormat;
  /** The arithmetic mode it was computed in. */
  readonly mode: bigint;
  /** Sample rate it was rendered at, in hertz: part of what is hashed. */
  readonly rate: number;
  /** Number of channels: part of what is hashed. */
  readonly channels: number;
  /** The interleaved samples, in the array of their format, from their bit patterns. */
  readonly samples: Float32Array | Float64Array;
  /** The exact bytes the hash covers, as the specification gives them. */
  readonly bytes: Uint8Array<ArrayBuffer>;
  /** The expected SHA-256 of those bytes, in lowercase hexadecimal. */
  readonly sha256: string;
}

/** Every vector of the specification. */
const VECTORS: readonly Vector[] = list(SPEC, "vectors").map((vector) => {
  const bits = list(vector, "samples").map(String);
  const read = format(vector);
  return {
    name: text(vector, "name"),
    format: read,
    mode: mode(vector),
    rate: integer(vector, "rate"),
    channels: integer(vector, "channels"),
    samples:
      read === "binary64"
        ? new Float64Array(BigUint64Array.from(bits, (word) => BigInt(`0x${word}`)).buffer)
        : new Float32Array(Uint32Array.from(bits, (word) => Number.parseInt(word, 16)).buffer),
    bytes: hexBytes(text(vector, "bytes")),
    sha256: text(vector, "sha256"),
  };
});

/** Every arithmetic mode the specification says a reader refuses. */
const REFUSED: readonly { readonly name: string; readonly mode: bigint }[] = list(SPEC, "refused_modes").map((entry) => ({ name: text(entry, "name"), mode: mode(entry) }));

describe("renderHash", () => {
  test("reads every known-answer vector of the specification", () => {
    expect(text(SPEC, "scheme")).toBe("render-hash/2");
    expect(VECTORS.length).toBeGreaterThanOrEqual(7);
    expect(new Set(VECTORS.map((vector) => vector.format))).toStrictEqual(new Set(RENDER_FORMATS));
    expect(REFUSED.length).toBeGreaterThanOrEqual(3);
  });

  test.each(VECTORS)("reproduces the known-answer vector $name", async (vector) => {
    expect(await renderHash(vector.rate, vector.channels, vector.samples, vector.format, vector.mode)).toBe(vector.sha256);
    // The vector is consistent: its bytes give its hash.
    expect(hexText(new Uint8Array(await crypto.subtle.digest("SHA-256", vector.bytes)))).toBe(vector.sha256);
  });

  test.each(REFUSED)("refuses the arithmetic mode $name, whose flags no option defines", async (refused) => {
    const unknown = refused.mode & ~0b111n;
    await expect(renderHash(48_000, 1, new Float32Array(), "binary32", refused.mode)).rejects.toThrow(
      new RangeError(`unknown arithmetic options: 0x${unknown.toString(16).padStart(16, "0")}`),
    );
  });

  test.each([-1n, 2n ** 64n])("refuses the arithmetic mode %s, which is not 64 bits of flags", async (wrong) => {
    await expect(renderHash(48_000, 1, new Float32Array(), "binary32", wrong)).rejects.toThrow(new RangeError(`the arithmetic mode must be 64 bits of flags, not ${wrong.toString()}`));
  });

  test("accepts every known option, alone and together, each changing the hash", async () => {
    const hashes = await Promise.all([0n, 1n, 2n, 4n, 7n].map((flags) => renderHash(48_000, 1, new Float32Array([1]), "binary32", flags)));
    expect(new Set(hashes).size).toBe(5);
    expect(hashes[0]).toBe(await renderHash(48_000, 1, new Float32Array([1]), "binary32"));
  });

  test("refuses samples of another width than the render format's", async () => {
    await expect(renderHash(48_000, 1, new Float32Array(1), "binary64")).rejects.toThrow(new RangeError("binary64 samples are 64-bit, not 32-bit"));
    await expect(renderHash(48_000, 1, new Float64Array(1), "binary64-as-binary32")).rejects.toThrow(new RangeError("binary64-as-binary32 samples are 32-bit, not 64-bit"));
    await expect(renderHash(48_000, 1, new Float64Array(1), "binary32")).rejects.toThrow(new RangeError("binary32 samples are 32-bit, not 64-bit"));
  });

  test("hashes the rate and the channel count of an empty render", async () => {
    expect(await renderHash(48_000, 1, new Float32Array(), "binary32")).not.toBe(await renderHash(44_100, 1, new Float32Array(), "binary32"));
    expect(await renderHash(48_000, 1, new Float32Array(), "binary32")).not.toBe(await renderHash(48_000, 2, new Float32Array(), "binary32"));
  });

  test("covers every bit of every sample, NaN payloads included, in both widths", async () => {
    const bits = new Uint32Array([0x7fc00000, 0x3f000000]);
    const other = new Uint32Array([0x7fc00001, 0x3f000000]);
    expect(await renderHash(48_000, 1, new Float32Array(bits.buffer), "binary32")).not.toBe(await renderHash(48_000, 1, new Float32Array(other.buffer), "binary32"));
    const ulp = new Uint32Array([0x7fc00000, 0x3f000001]);
    expect(await renderHash(48_000, 1, new Float32Array(bits.buffer), "binary32")).not.toBe(await renderHash(48_000, 1, new Float32Array(ulp.buffer), "binary32"));
    const wide = new BigUint64Array([0x7ff8_0000_0000_0000n, 0x3fe0_0000_0000_0000n]);
    const highest = new BigUint64Array([0xfff8_0000_0000_0000n, 0x3fe0_0000_0000_0000n]);
    const lowest = new BigUint64Array([0x7ff8_0000_0000_0000n, 0x3fe0_0000_0000_0001n]);
    const hash64 = (words: BigUint64Array): Promise<string> => renderHash(48_000, 1, new Float64Array(words.buffer), "binary64");
    expect(await hash64(wide)).not.toBe(await hash64(highest));
    expect(await hash64(wide)).not.toBe(await hash64(lowest));
  });

  test("reads only the samples of a view", async () => {
    const whole = new Float32Array([9, 0, 0.5, -0.5, 1, 9]);
    expect(await renderHash(48_000, 2, whole.subarray(1, 5), "binary32")).toBe(await renderHash(48_000, 2, new Float32Array([0, 0.5, -0.5, 1]), "binary32"));
    const wide = new Float64Array([9, 0, 0.5, -0.5, 1, 9]);
    expect(await renderHash(48_000, 2, wide.subarray(1, 5), "binary64")).toBe(await renderHash(48_000, 2, new Float64Array([0, 0.5, -0.5, 1]), "binary64"));
  });

  test.each([44_100.5, -1, 2 ** 32, Number.NaN])("refuses the sample rate %d, which is not a 32-bit unsigned integer", async (rate) => {
    await expect(renderHash(rate, 1, new Float32Array(), "binary32")).rejects.toThrow(new RangeError(`the sample rate must be a whole number from 0 to 4294967295, not ${String(rate)}`));
  });

  test.each([1.5, -1, Number.NaN, 2 ** 53])("refuses the channel count %d", async (channels) => {
    await expect(renderHash(48_000, channels, new Float32Array(), "binary32")).rejects.toThrow(
      new RangeError(`the channel count must be a whole number from 0 to 2^53 - 1, not ${String(channels)}`),
    );
  });

  test.each([
    [3, 2],
    [2, 3],
    [0, 1],
  ])("refuses %i channels over %i samples, not whole frames", async (channels, samples) => {
    await expect(renderHash(48_000, channels, new Float32Array(samples), "binary32")).rejects.toThrow(
      new RangeError(`${String(samples)} samples are not whole frames of ${String(channels)} channels`),
    );
  });

  test("accepts the edges of each domain", async () => {
    // Independent values (Python hashlib).
    expect(await renderHash(0, 0, new Float64Array(), "binary64")).toBe("6447e2f7a6ae9ef2eff5242dc39b9f760b005aca865abd0dfccdabaa00072dc5");
    expect(await renderHash(0xff_ff_ff_ff, 2 ** 53 - 1, new Float32Array(), "binary32", 2n ** 3n - 1n)).toBe("ee57da60938c3aded8540a4fa8aed5839e765e6b2ba4029b13929dcd641f4f7b");
    expect(await renderHash(48_000, 2, new Float32Array(4), "binary64-as-binary32")).toBe("4fcbb6a883cfb323f730917e171b7490b147b772d184ced277258bfaeea76d07");
    expect(await renderHash(48_000, 2, new Float64Array(4), "binary64")).toBe("73710d443a2641f3299af7a7782a6b16c2ff1e7482ceb638517662df14cbfbc6");
  });

  test("is 64 lowercase hex digits", async () => {
    expect(await renderHash(1, 1, new Float32Array([1e-45]), "binary32")).toMatch(/^[0-9a-f]{64}$/);
  });
});

describe("interleave", () => {
  test("puts frames one after the other, in either width", () => {
    expect(interleave([new Float32Array([1, 2]), new Float32Array([3, 4])])).toStrictEqual(new Float32Array([1, 3, 2, 4]));
    expect(interleave64([new Float64Array([1, 2, 5]), new Float64Array([3, 4, 6])])).toStrictEqual(new Float64Array([1, 3, 2, 4, 5, 6]));
  });

  test.each([
    [[new Float32Array([1, 2]), new Float32Array([3])], "every channel must have the same length: 1 instead of 2"],
    [[new Float32Array([1]), new Float32Array([2, 3])], "every channel must have the same length: 2 instead of 1"],
  ])("refuses channels of different lengths: %o", (channels, message) => {
    expect(() => interleave(channels)).toThrow(new RangeError(message));
  });

  test("refuses binary64 channels of different lengths", () => {
    expect(() => interleave64([new Float64Array(2), new Float64Array(1)])).toThrow(new RangeError("every channel must have the same length: 1 instead of 2"));
  });

  test("keeps every bit, NaN payloads included", () => {
    const signaling = new Float32Array(new Uint32Array([0x7f800001, 0xffa00000]).buffer);
    const quiet = new Float32Array(new Uint32Array([0x7fc00001]).buffer);
    const out = interleave([signaling.subarray(0, 1), quiet]);
    expect(Array.from(new Uint32Array(out.buffer))).toStrictEqual([0x7f800001, 0x7fc00001]);
    const offset = interleave([signaling.subarray(1)]);
    expect(Array.from(new Uint32Array(offset.buffer))).toStrictEqual([0xffa00000]);
    const wide = new Float64Array(new BigUint64Array([0x7ff0_0000_0000_0001n, 0xfff4_0000_0000_0002n]).buffer);
    const together = interleave64([wide.subarray(1), wide.subarray(0, 1)]);
    expect(Array.from(new BigUint64Array(together.buffer))).toStrictEqual([0xfff4_0000_0000_0002n, 0x7ff0_0000_0000_0001n]);
  });

  test("accepts no channels", () => {
    expect(interleave([])).toStrictEqual(new Float32Array());
    expect(interleave64([])).toStrictEqual(new Float64Array());
  });
});
