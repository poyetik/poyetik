import { describe, expect, test } from "vitest";
import { InputError, MAX_RATE, MIN_RATE, parseCase, parseExpectations, parseReferences } from "../src/inputs.ts";

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
    // The engine's range of rates, as engine-cli refuses the case.
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
  test("reads case names and hashes", () => {
    expect(parseReferences({ a: HASH, b: "0".repeat(64) })).toStrictEqual(new Map([["a", HASH], ["b", "0".repeat(64)]]));
  });

  test.each([
    [[HASH], "references.json is not an object"],
    [null, "references.json is not an object"],
    ["x", "references.json is not an object"],
    [{ a: 1 }, "references.json: a is not a SHA-256 hash"],
    [{ a: HASH.toUpperCase() }, "references.json: a is not a SHA-256 hash"],
    [{ a: `${HASH}0` }, "references.json: a is not a SHA-256 hash"],
    [{ a: `x${HASH}` }, "references.json: a is not a SHA-256 hash"],
    // Its text is a valid hash, but it is not a string.
    [{ a: [HASH] }, "references.json: a is not a SHA-256 hash"],
  ])("rejects %o", (value, detail) => {
    expect(() => parseReferences(value)).toThrow(new InputError(detail));
  });
});

describe("parseExpectations", () => {
  test("reads every case's rate, duration and channels", () => {
    expect(parseExpectations({ a: { sample_rate: 8000, frames: 4, channels: [CHANNEL] } })).toStrictEqual(
      new Map([["a", { rate: 8000, frames: 4, channels: [{ sine: { frequency: 440, amplitude: 1, phase: 0.5 } }] }]]),
    );
  });

  test.each([
    [[], "expectations.json is not an object"],
    [{ a: null }, "expectations.json: a: channels is not a non-empty list"],
    [{ a: { sample_rate: 8000, frames: 4, channels: [{ sine: null }] } }, "expectations.json: a: channel 0: sine.frequency is not a finite number"],
    [{ a: { sample_rate: 8000, frames: 4 } }, "expectations.json: a: channels is not a non-empty list"],
    [{ a: { sample_rate: 8000, frames: 4, channels: [] } }, "expectations.json: a: channels is not a non-empty list"],
    [{ a: { frames: 4, channels: [CHANNEL] } }, "expectations.json: a: sample_rate is not a finite number"],
    [{ a: { sample_rate: 8000, channels: [CHANNEL] } }, "expectations.json: a: frames is not a finite number"],
    [{ a: { sample_rate: 8000, frames: 4, channels: [{}] } }, "expectations.json: a: channel 0: sine.frequency is not a finite number"],
    [{ a: { sample_rate: 8000, frames: 4, channels: [3] } }, "expectations.json: a: channel 0 is not an object"],
    [{ a: { sample_rate: 8000, frames: 2 ** 29 + 1, channels: [CHANNEL] } }, "expectations.json: a: frames is above 536870912, the longest case with an exact reference"],
    [{ a: { sample_rate: 8000, frames: 4, channels: [CHANNEL, { ...CHANNEL, sine: { frequency: 1, phase: 0 } }] } }, "expectations.json: a: channel 1: sine.amplitude is not a finite number"],
    [{ a: { sample_rate: 8000, frames: 4, channels: [{ ...CHANNEL, sine: { frequency: 1, amplitude: 1 } }] } }, "expectations.json: a: channel 0: sine.phase is not a finite number"],
    // The bound is derived below 4 cycles per sample.
    [{ a: { sample_rate: 8000, frames: 4, channels: [{ sine: { ...CHANNEL.sine, frequency: 32_000 } }] } }, "expectations.json: a: channel 0: sine.frequency is not in [0, 4 × sample_rate)"],
    [{ a: { sample_rate: 8000, frames: 4, channels: [{ sine: { ...CHANNEL.sine, frequency: -(2 ** -149) } }] } }, "expectations.json: a: channel 0: sine.frequency is not in [0, 4 × sample_rate)"],
    // The bound covers a start phase in [0, 2π) only: a larger one cannot be reduced exactly.
    [{ a: { sample_rate: 8000, frames: 4, channels: [{ sine: { ...CHANNEL.sine, phase: 1e6 } }] } }, "expectations.json: a: channel 0: sine.phase is not in [0, 2π)"],
    [{ a: { sample_rate: 8000, frames: 4, channels: [{ sine: { ...CHANNEL.sine, phase: 2 * Math.PI } }] } }, "expectations.json: a: channel 0: sine.phase is not in [0, 2π)"],
    [{ a: { sample_rate: 8000, frames: 4, channels: [{ sine: { ...CHANNEL.sine, phase: -Number.MIN_VALUE } }] } }, "expectations.json: a: channel 0: sine.phase is not in [0, 2π)"],
    [
      { a: { sample_rate: 8000, frames: 4, channels: [{ sine: CHANNEL.sine, max_error: 1e-6 }] } },
      "expectations.json: a: channel 0: unknown field max_error (tolerances are derived, not declared)",
    ],
  ])("rejects %o", (value, detail) => {
    expect(() => parseExpectations(value)).toThrow(new InputError(detail));
  });

  test("names its errors", () => {
    expect(new InputError("x")).toMatchObject({ name: "InputError", message: "invalid conformance input: x" });
  });
});

test("reads frequencies that are exact binary32 values, as the engine plays them", () => {
  const channel = { sine: { frequency: 440.100006103515625, amplitude: 1, phase: 0 } };
  const parsed = parseExpectations({ a: { sample_rate: 8000, frames: 4, channels: [channel] } });
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
    const parsed = parseExpectations({ a: { sample_rate: 8000, frames: 4, channels: [{ sine: { ...CHANNEL.sine, phase } }] } });
    expect(parsed.get("a")?.channels[0]?.sine.phase).toBe(phase);
  }
});

test("accepts a frequency from 0 to just below 4 cycles per sample", () => {
  for (const frequency of [0, Math.fround(32_000 - 2 ** -9)]) {
    const parsed = parseExpectations({ a: { sample_rate: 8000, frames: 4, channels: [{ sine: { ...CHANNEL.sine, frequency } }] } });
    expect(parsed.get("a")?.channels[0]?.sine.frequency).toBe(frequency);
  }
});
