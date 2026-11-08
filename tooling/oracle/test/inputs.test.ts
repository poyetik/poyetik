import { describe, expect, test } from "vitest";
import { InputError, parseCase, parseExpectations, parseReferences } from "../src/inputs.ts";

const HASH = "a".repeat(64);
const CHANNEL = { sine: { frequency: 440, amplitude: 1, phase: 0.5 }, max_error: 1e-6 };

describe("parseCase", () => {
  test("reads the rate and the duration", () => {
    expect(parseCase({ sample_rate: 48_000, frames: 10, patch: {} }, "c")).toStrictEqual({ rate: 48_000, frames: 10 });
  });

  test.each([
    [{ frames: 10 }, "c: sample_rate is not a finite number"],
    [{ sample_rate: 48_000 }, "c: frames is not a finite number"],
    [{ sample_rate: "48000", frames: 10 }, "c: sample_rate is not a finite number"],
    [{ sample_rate: 48_000, frames: Number.POSITIVE_INFINITY }, "c: frames is not a finite number"],
    [{ sample_rate: 48_000.5, frames: 10 }, "c: sample_rate is not a positive whole number"],
    [{ sample_rate: 48_000, frames: 0 }, "c: frames is not a positive whole number"],
    [{ sample_rate: 48_000, frames: -1 }, "c: frames is not a positive whole number"],
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
      new Map([["a", { rate: 8000, frames: 4, channels: [{ sine: { frequency: 440, amplitude: 1, phase: 0.5 }, maxError: 1e-6 }] }]]),
    );
  });

  test.each([
    [[], "expectations.json is not an object"],
    [{ a: { sample_rate: 8000, frames: 4 } }, "expectations.json: a: channels is not a non-empty list"],
    [{ a: { sample_rate: 8000, frames: 4, channels: [] } }, "expectations.json: a: channels is not a non-empty list"],
    [{ a: { frames: 4, channels: [CHANNEL] } }, "expectations.json: a: sample_rate is not a finite number"],
    [{ a: { sample_rate: 8000, channels: [CHANNEL] } }, "expectations.json: a: frames is not a finite number"],
    [{ a: { sample_rate: 8000, frames: 4, channels: [{ max_error: 1 }] } }, "expectations.json: a: channel 0: sine.frequency is not a finite number"],
    [{ a: { sample_rate: 8000, frames: 4, channels: [CHANNEL, { ...CHANNEL, sine: { frequency: 1, phase: 0 } }] } }, "expectations.json: a: channel 1: sine.amplitude is not a finite number"],
    [{ a: { sample_rate: 8000, frames: 4, channels: [{ ...CHANNEL, sine: { frequency: 1, amplitude: 1 } }] } }, "expectations.json: a: channel 0: sine.phase is not a finite number"],
    [{ a: { sample_rate: 8000, frames: 4, channels: [{ sine: CHANNEL.sine }] } }, "expectations.json: a: channel 0: max_error is not a finite number"],
  ])("rejects %o", (value, detail) => {
    expect(() => parseExpectations(value)).toThrow(new InputError(detail));
  });

  test("names its errors", () => {
    expect(new InputError("x")).toMatchObject({ name: "InputError", message: "invalid conformance input: x" });
  });
});

test("reads frequencies as the binary32 parameters the engine plays", () => {
  const channel = { sine: { frequency: 440.1, amplitude: 1, phase: 0 }, max_error: 1e-6 };
  const parsed = parseExpectations({ a: { sample_rate: 8000, frames: 4, channels: [channel] } });
  const frequency = parsed.get("a")?.channels[0]?.sine.frequency;
  expect(frequency).toBe(Math.fround(440.1));
  expect(frequency).not.toBe(440.1);
});
