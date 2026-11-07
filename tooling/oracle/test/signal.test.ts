import { expect, test } from "vitest";
import { channel, sineError } from "../src/signal.ts";
import { parseWav } from "../src/wav.ts";
import { makeWav, sineSamples } from "./fixtures.ts";

const SINE = { frequency: 1000, amplitude: 1, phase: 0 };

test("reads each channel of interleaved samples", () => {
  const wav = parseWav(makeWav({ rate: 8, channels: 2, samples: [1, 2, 3, 4, 5, 6] }));
  expect(channel(wav, 0)).toStrictEqual([1, 3, 5]);
  expect(channel(wav, 1)).toStrictEqual([2, 4, 6]);
});

test("measures the distance to the ideal sine", () => {
  const samples = sineSamples(1, 480, 48_000, 1000);
  const error = sineError(samples, 48_000, SINE);
  expect(error).toBeGreaterThan(0);
  expect(error).toBeLessThan(1e-7);
  expect(sineError([0, 0.25], 4000, SINE)).toBe(0.75);
});

test("uses the frequency, the amplitude and the phase", () => {
  const samples = sineSamples(1, 48, 48_000, 1000);
  expect(sineError(samples, 48_000, { ...SINE, frequency: 1001 })).toBeGreaterThan(1e-3);
  expect(sineError(samples, 24_000, SINE)).toBeGreaterThan(0.1);
  expect(sineError(samples, 48_000, { ...SINE, amplitude: 0.5 })).toBeCloseTo(0.5, 2);
  expect(sineError(samples, 48_000, { ...SINE, phase: Math.PI })).toBeCloseTo(2, 2);
});

test("adds the phase", () => {
  const cosine = Array.from({ length: 48 }, (_, index) => Math.cos((2 * Math.PI * 1000 * index) / 48_000));
  expect(sineError(cosine, 48_000, { ...SINE, phase: Math.PI / 2 })).toBeLessThan(1e-12);
});

test("is infinite on a sample that is not finite, wherever it is", () => {
  expect(sineError([Number.NaN, 0], 4000, SINE)).toBe(Number.POSITIVE_INFINITY);
  expect(sineError([0, Number.POSITIVE_INFINITY], 4000, SINE)).toBe(Number.POSITIVE_INFINITY);
});

test("is zero for no samples", () => {
  expect(sineError([], 4000, SINE)).toBe(0);
});
