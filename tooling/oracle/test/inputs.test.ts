import { describe, expect, test } from "vitest";
import { InputError, MAX_RATE, MIN_RATE, parseCase, parseExpectations, parseReferences } from "../src/inputs.ts";
import { sineErrorBound } from "../src/signal.ts";

/** A well-formed SHA-256 hash: 64 lowercase hexadecimal digits. */
const HASH = "a".repeat(64);
/** A valid channel expectation, with a non-zero phase so that a dropped field shows. */
const CHANNEL = { sine: { frequency: 440, amplitude: 1, phase: 0.5 } };

describe("parseCase", () => {
  test("reads the rate and the duration, from the lowest supported rate to the highest", () => {
    expect(parseCase('{"sample_rate":48000,"frames":10}', "c")).toStrictEqual({ rate: 48_000, frames: 10 });
    expect(parseCase('{ "frames" : 536870912 , "sample_rate" : 8000 }', "c")).toStrictEqual({ rate: 8000, frames: 2 ** 29 });
    expect(parseCase('{"sample_rate":384000,"frames":1}', "c")).toStrictEqual({ rate: 384_000, frames: 1 });
  });

  test.each([
    ['{"frames":10}', "c: sample_rate is not a finite number"],
    ['{"sample_rate":48000}', "c: frames is not a finite number"],
    ['{"sample_rate":"48000","frames":10}', "c: sample_rate is not a finite number"],
    ['{"sample_rate":48000,"frames":10,"patch":{}}', "c: unknown field patch"],
    ['{"sample_rate":48000,"frames":10,"note":"x"}', "c: unknown field note"],
    ['{"sample_rate":48000.5,"frames":10}', "c: sample_rate is not a positive whole number"],
    ['{"sample_rate":48000.0,"frames":10}', "c: sample_rate is not a whole number written without fraction or exponent"],
    ['{"sample_rate":48000,"frames":100.0}', "c: frames is not a whole number written without fraction or exponent"],
    ['{"sample_rate":48000,"frames":1e2}', "c: frames is not a whole number written without fraction or exponent"],
    ['{"sample_rate":48000,"frames":1E2}', "c: frames is not a whole number written without fraction or exponent"],
    ['{"sample_rate":48000,"frames":0}', "c: frames is not a positive whole number"],
    ['{"sample_rate":48000,"frames":-1}', "c: frames is not a positive whole number"],
    ['{"sample_rate":48000,"frames":-0}', "c: frames is not a positive whole number"],
    // The same domain as the expectations: 1 to 2^29 frames.
    ['{"sample_rate":48000,"frames":536870913}', "c: frames is above 536870912, the longest case with an exact reference"],
    // The engine's range of rates, as audio-cli refuses the case.
    ['{"sample_rate":7999,"frames":10}', "c: sample_rate is not a supported sample rate (8000 to 384000 Hz)"],
    ['{"sample_rate":384001,"frames":10}', "c: sample_rate is not a supported sample rate (8000 to 384000 Hz)"],
    ['{"sample_rate":4294967296,"frames":10}', "c: sample_rate is not a supported sample rate (8000 to 384000 Hz)"],
    ["null", "c is not an object"],
    ['"case"', "c is not an object"],
    ["[48000,10]", "c is not an object"],
    ["{", "c: not JSON (SyntaxError: Expected property name or '}' in JSON at position 1 (line 1 column 2))"],
  ])("rejects %s", (text, detail) => {
    expect(() => parseCase(text, "c")).toThrow(new InputError(detail));
  });

  test("refuses a deeply nested text without overflowing the stack", () => {
    // 20 KB of nesting, within the 64 KiB of a case file: a reviver recursing through it overflows.
    const nested = `${"[".repeat(10_000)}${"]".repeat(10_000)}`;
    expect(() => parseCase(nested, "c")).toThrow(new InputError("c is not an object"));
    expect(() => parseCase(`{"sample_rate":48000,"frames":${nested}}`, "c")).toThrow(new InputError("c: frames is not a finite number"));
  });

  test("states the engine's range of rates", () => {
    expect([MIN_RATE, MAX_RATE]).toStrictEqual([8000, 384_000]);
  });
});

describe("parseReferences", () => {
  /** A hash per render format. */
  const HASHES = { binary64: "0".repeat(64), "binary64-as-binary32": HASH, binary32: "1".repeat(64) };

  test("reads case names and the hash of each render format", () => {
    expect(parseReferences({ a: HASHES, b: { binary32: HASH, binary64: HASH, "binary64-as-binary32": HASH } })).toStrictEqual(
      new Map([
        [
          "a",
          new Map([
            ["binary64", "0".repeat(64)],
            ["binary64-as-binary32", HASH],
            ["binary32", "1".repeat(64)],
          ]),
        ],
        [
          "b",
          new Map([
            ["binary64", HASH],
            ["binary64-as-binary32", HASH],
            ["binary32", HASH],
          ]),
        ],
      ]),
    );
  });

  test("lists the hashes in the order of the render formats, whatever the order of the file", () => {
    const read = parseReferences({ a: { binary32: HASH, "binary64-as-binary32": HASH, binary64: HASH } }).get("a");
    expect([...(read?.keys() ?? [])]).toStrictEqual(["binary64", "binary64-as-binary32", "binary32"]);
  });

  test.each([
    [[HASH], "references.json is not an object"],
    [null, "references.json is not an object"],
    ["x", "references.json is not an object"],
    [{ a: HASH }, "references.json: a is not an object"],
    [{ a: [HASH, HASH, HASH] }, "references.json: a is not an object"],
    [{ a: { ...HASHES, binary16: HASH } }, "references.json: a: unknown field binary16"],
    [{ a: { "binary64-as-binary32": HASH, binary32: HASH } }, "references.json: a: binary64 is not a SHA-256 hash"],
    [{ a: { binary64: HASH, binary32: HASH } }, "references.json: a: binary64-as-binary32 is not a SHA-256 hash"],
    [{ a: { binary64: HASH, "binary64-as-binary32": HASH } }, "references.json: a: binary32 is not a SHA-256 hash"],
    [{ a: { ...HASHES, binary32: 1 } }, "references.json: a: binary32 is not a SHA-256 hash"],
    [{ a: { ...HASHES, binary32: HASH.toUpperCase() } }, "references.json: a: binary32 is not a SHA-256 hash"],
    [{ a: { ...HASHES, binary32: `${HASH}0` } }, "references.json: a: binary32 is not a SHA-256 hash"],
    [{ a: { ...HASHES, binary32: `x${HASH}` } }, "references.json: a: binary32 is not a SHA-256 hash"],
    // Its text is a valid hash, but it is not a string.
    [{ a: { ...HASHES, binary32: [HASH] } }, "references.json: a: binary32 is not a SHA-256 hash"],
  ])("rejects %o", (value, detail) => {
    expect(() => parseReferences(value)).toThrow(new InputError(detail));
  });
});

describe("parseExpectations", () => {
  test("reads every case's rate, duration and channels", () => {
    expect(parseExpectations({ a: { sample_rate: 8000, frames: 4000, channels: [CHANNEL] } })).toStrictEqual(
      new Map([["a", { rate: 8000, frames: 4000, channels: [{ sine: { frequency: 440, amplitude: 1, phase: 0.5 } }] }]]),
    );
  });

  test.each([
    [[], "expectations.json is not an object"],
    [{ a: null }, "expectations.json: a is not an object"],
    [{ a: { sample_rate: 8000, frames: 4000, channels: [{ sine: null }] } }, "expectations.json: a: channel 0: sine is not an object"],
    // Exactly the documented fields, at every level.
    [{ a: { sample_rate: 8000, frames: 4000, channels: [CHANNEL], max_error: 1e-6 } }, "expectations.json: a: unknown field max_error"],
    [{ a: { sample_rate: 8000, frames: 4000, channels: [{ sine: { ...CHANNEL.sine, offset: 0 } }] } }, "expectations.json: a: channel 0: sine: unknown field offset"],
    [{ a: { sample_rate: 8000, frames: 4 } }, "expectations.json: a: channels is not a non-empty list"],
    [{ a: { sample_rate: 8000, frames: 4000, channels: [] } }, "expectations.json: a: channels is not a non-empty list"],
    [{ a: { frames: 4, channels: [CHANNEL] } }, "expectations.json: a: sample_rate is not a finite number"],
    [{ a: { sample_rate: 8000, channels: [CHANNEL] } }, "expectations.json: a: frames is not a finite number"],
    [{ a: { sample_rate: 8000, frames: 4000, channels: [{}] } }, "expectations.json: a: channel 0: sine is not an object"],
    [{ a: { sample_rate: 8000, frames: 4000, channels: [3] } }, "expectations.json: a: channel 0 is not an object"],
    [{ a: { sample_rate: 8000, frames: 2 ** 29 + 1, channels: [CHANNEL] } }, "expectations.json: a: frames is above 536870912, the longest case with an exact reference"],
    [{ a: { sample_rate: 8000, frames: 4000, channels: [CHANNEL, { ...CHANNEL, sine: { frequency: 1, phase: 0 } }] } }, "expectations.json: a: channel 1: sine.amplitude is not a finite number"],
    [{ a: { sample_rate: 8000, frames: 4000, channels: [{ sine: { ...CHANNEL.sine, amplitude: 1e300 } }] } }, "expectations.json: a: channel 0: sine.amplitude is not a binary32 value (the error bound of a binary32 render counts a gain only when it is exact in binary32)"],
    [{ a: { sample_rate: 8000, frames: 4000, channels: [{ ...CHANNEL, sine: { frequency: 1, amplitude: 1 } }] } }, "expectations.json: a: channel 0: sine.phase is not a finite number"],
    // The bound is derived below 4 cycles per sample.
    [{ a: { sample_rate: 8000, frames: 4000, channels: [{ sine: { ...CHANNEL.sine, frequency: 32_000 } }] } }, "expectations.json: a: channel 0: sine.frequency is not in [0, 4 × sample_rate)"],
    [{ a: { sample_rate: 8000, frames: 4000, channels: [{ sine: { ...CHANNEL.sine, frequency: -(2 ** -149) } }] } }, "expectations.json: a: channel 0: sine.frequency is not in [0, 4 × sample_rate)"],
    // The binary32 bound counts a gain only when it is exact in binary32.
    [
      { a: { sample_rate: 8000, frames: 4000, channels: [{ sine: { ...CHANNEL.sine, amplitude: 0.1 } }] } },
      "expectations.json: a: channel 0: sine.amplitude is not a binary32 value (the error bound of a binary32 render counts a gain only when it is exact in binary32)",
    ],
    [
      { a: { sample_rate: 8000, frames: 4000, channels: [{ sine: { ...CHANNEL.sine, amplitude: 1e-35 } }] } },
      "expectations.json: a: channel 0: sine.amplitude is not a binary32 value (the error bound of a binary32 render counts a gain only when it is exact in binary32)",
    ],
    // A sine the oracle would measure must have enough frames for its fit.
    [
      { a: { sample_rate: 8000, frames: 2, channels: [CHANNEL] } },
      "expectations.json: a: channel 0: sine.frequency cannot be measured over 2 frames (the signal has 2 frames, fewer than the 3 unknowns of the fit): lengthen the case",
    ],
    // The bound covers a start phase in [0, 2π) only: a larger one cannot be reduced exactly.
    [{ a: { sample_rate: 8000, frames: 4000, channels: [{ sine: { ...CHANNEL.sine, phase: 1e6 } }] } }, "expectations.json: a: channel 0: sine.phase is not in [0, 2π)"],
    [{ a: { sample_rate: 8000, frames: 4000, channels: [{ sine: { ...CHANNEL.sine, phase: 2 * Math.PI } }] } }, "expectations.json: a: channel 0: sine.phase is not in [0, 2π)"],
    [{ a: { sample_rate: 8000, frames: 4000, channels: [{ sine: { ...CHANNEL.sine, phase: -Number.MIN_VALUE } }] } }, "expectations.json: a: channel 0: sine.phase is not in [0, 2π)"],
    [
      { a: { sample_rate: 8000, frames: 4000, channels: [{ sine: CHANNEL.sine, max_error: 1e-6 }] } },
      "expectations.json: a: channel 0: unknown field max_error (tolerances are derived, not declared)",
    ],
  ])("rejects %o", (value, detail) => {
    expect(() => parseExpectations(value)).toThrow(new InputError(detail));
  });

  test("names its errors", () => {
    expect(new InputError("x")).toMatchObject({ name: "InputError", message: "invalid conformance input: x" });
  });
});

test("refuses a sine the oracle would measure but cannot tell from DC over its frames, and accepts one it can", () => {
  const at = (frequency: number): unknown => ({ a: { sample_rate: 48_000, frames: 1000, channels: [CHANNEL, { sine: { ...CHANNEL.sine, frequency } }] } });
  expect(() => parseExpectations(at(1))).toThrow(
    /^invalid conformance input: expectations\.json: a: channel 1: sine\.frequency cannot be measured over 1000 frames \(the components are too close to be told apart \(conditioning \d+\.\d+, at most 0\.5\)\): lengthen the case$/,
  );
  // Two whole cycles over the 1000 frames: the sine and DC are orthogonal there.
  expect(parseExpectations(at(96)).get("a")?.channels[1]?.sine.frequency).toBe(96);
});

test("refuses a sine measured in one render format only when it cannot be told from DC", () => {
  // At 2^-30 of full scale and about 7.6e-9 Hz, the sine rises above the error bound of
  // binary64 over 1000 frames, never above that of binary32: measured in binary64 only, where
  // it cannot be told from DC.
  const amplitude = 2 ** -30;
  const frequency = Math.fround(7.6e-9);
  const peak = amplitude * Math.sin((2 * Math.PI * frequency * 999) / 48_000);
  expect(peak).toBeGreaterThan(sineErrorBound(amplitude, 1000, "binary64"));
  expect(peak).toBeLessThan(sineErrorBound(amplitude, 1000, "binary32"));
  expect(() => parseExpectations({ a: { sample_rate: 48_000, frames: 1000, channels: [{ sine: { frequency, amplitude, phase: 0 } }] } })).toThrow(
    /^invalid conformance input: expectations\.json: a: channel 0: sine\.frequency cannot be measured over 1000 frames/,
  );
});

test("reads frequencies that are exact binary32 values, as the engine plays them", () => {
  const channel = { sine: { frequency: 440.100006103515625, amplitude: 1, phase: 0 } };
  const parsed = parseExpectations({ a: { sample_rate: 8000, frames: 4000, channels: [channel] } });
  expect(parsed.get("a")?.channels[0]?.sine.frequency).toBe(Math.fround(440.1));
});

test.each([
  // Not binary32. The engine rounds this decimal text to binary32 once, upwards; JSON.parse
  // rounds it to binary64 first, onto a binary32 midpoint, which then rounds downwards.
  "1.00000005960464477539062500001",
  "440.1",
  "0.1",
])("rejects the frequency %s, which is not a binary32 value", (frequency) => {
  const expectations: unknown = JSON.parse(`{"a":{"sample_rate":8000,"frames":4,"channels":[{"sine":{"frequency":${frequency},"amplitude":1,"phase":0}}]}}`);
  expect(() => parseExpectations(expectations)).toThrow(
    new InputError("expectations.json: a: channel 0: sine.frequency is not a binary32 value (parameters are binary32: give the value the engine plays)"),
  );
});

test("accepts the longest case with an exact reference", () => {
  const parsed = parseExpectations({ a: { sample_rate: 8000, frames: 2 ** 29, channels: [CHANNEL] } });
  expect(parsed.get("a")?.frames).toBe(2 ** 29);
});

test("accepts a start phase from 0 to just below 2π", () => {
  for (const phase of [0, -0, 2 * Math.PI - 2 ** -50]) {
    const parsed = parseExpectations({ a: { sample_rate: 8000, frames: 4000, channels: [{ sine: { ...CHANNEL.sine, phase } }] } });
    expect(parsed.get("a")?.channels[0]?.sine.phase).toBe(phase);
  }
});

test("accepts a frequency from 0 to just below 4 cycles per sample", () => {
  // One frame from phase 0: no level measured, so the domain check alone decides.
  for (const frequency of [0, Math.fround(32_000 - 2 ** -9)]) {
    const parsed = parseExpectations({ a: { sample_rate: 8000, frames: 1, channels: [{ sine: { ...CHANNEL.sine, frequency, phase: 0 } }] } });
    expect(parsed.get("a")?.channels[0]?.sine.frequency).toBe(frequency);
  }
});
