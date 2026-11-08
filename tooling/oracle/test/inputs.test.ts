import { describe, expect, test } from "vitest";
import { InputError, parseCase, parseExpectations, parseReferences } from "../src/inputs.ts";

/** A well-formed SHA-256 hash: 64 lowercase hexadecimal digits. */
const HASH = "a".repeat(64);
/** A valid channel expectation, with a non-zero phase so that a dropped field shows. */
const CHANNEL = { sine: { frequency: 440, amplitude: 1, phase: 0.5 } };

describe("parseCase", () => {
  test("reads the rate and the duration", () => {
    expect(parseCase({ sample_rate: 48_000, frames: 10, patch: {} }, "c")).toStrictEqual({ rate: 48_000, frames: 10 });
    expect(parseCase({ sample_rate: 48_000, frames: 2 ** 29 }, "c")).toStrictEqual({ rate: 48_000, frames: 2 ** 29 });
  });

  test.each([
    [{ frames: 10 }, "c: sample_rate is not a finite number"],
    [{ sample_rate: 48_000 }, "c: frames is not a finite number"],
    [{ sample_rate: "48000", frames: 10 }, "c: sample_rate is not a finite number"],
    [{ sample_rate: 48_000, frames: Number.POSITIVE_INFINITY }, "c: frames is not a finite number"],
    [{ sample_rate: 48_000.5, frames: 10 }, "c: sample_rate is not a positive whole number"],
    [{ sample_rate: 48_000, frames: 0 }, "c: frames is not a positive whole number"],
    [{ sample_rate: 48_000, frames: -1 }, "c: frames is not a positive whole number"],
    // The same domain as the expectations: 1 to 2^29 frames.
    [{ sample_rate: 48_000, frames: 2 ** 29 + 1 }, "c: frames is above 536870912, the longest case with an exact reference"],
    [null, "c: sample_rate is not a finite number"],
    ["case", "c: sample_rate is not a finite number"],
  ])("rejects %o", (value, detail) => {
    expect(() => parseCase(value, "c")).toThrow(new InputError(detail));
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
    [{ a: { sample_rate: 8000, frames: 4 } }, "expectations.json: a: channels is not a non-empty list"],
    [{ a: { sample_rate: 8000, frames: 4, channels: [] } }, "expectations.json: a: channels is not a non-empty list"],
    [{ a: { frames: 4, channels: [CHANNEL] } }, "expectations.json: a: sample_rate is not a finite number"],
    [{ a: { sample_rate: 8000, channels: [CHANNEL] } }, "expectations.json: a: frames is not a finite number"],
    [{ a: { sample_rate: 8000, frames: 4, channels: [{}] } }, "expectations.json: a: channel 0: sine.frequency is not a finite number"],
    [{ a: { sample_rate: 8000, frames: 4, channels: [3] } }, "expectations.json: a: channel 0 is not an object"],
    [{ a: { sample_rate: 8000, frames: 2 ** 29 + 1, channels: [CHANNEL] } }, "expectations.json: a: frames is above 536870912, the longest case with an exact reference"],
    [{ a: { sample_rate: 8000, frames: 4, channels: [CHANNEL, { ...CHANNEL, sine: { frequency: 1, phase: 0 } }] } }, "expectations.json: a: channel 1: sine.amplitude is not a finite number"],
    [{ a: { sample_rate: 8000, frames: 4, channels: [{ ...CHANNEL, sine: { frequency: 1, amplitude: 1 } }] } }, "expectations.json: a: channel 0: sine.phase is not a finite number"],
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
