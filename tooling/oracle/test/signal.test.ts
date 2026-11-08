import { expect, test } from "vitest";
import { channel, ideal, MAX_FRAMES, sineError, sineErrorBound } from "../src/signal.ts";
import { parseWav } from "../src/wav.ts";
import { makeWav, sineSamples } from "./fixtures.ts";

/** A 1 kHz sine of unit amplitude and zero phase, which `sineSamples` produces. */
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

test("reduces the phase exactly, however far into the case", () => {
  // 1 kHz at 8 kHz: every sample is a multiple of π/4. Far into a case, the unreduced
  // argument 2π × 1000 × n / 8000 has lost the low bits of the phase; the reduced one
  // has not.
  const last = MAX_FRAMES - 1;
  expect(ideal(SINE, 8000, last)).toBe(Math.sin((2 * Math.PI * 7) / 8));
  expect(Math.abs(ideal(SINE, 8000, last) - Math.sin((2 * Math.PI * 1000 * last) / 8000))).toBeGreaterThan(1e-9);
  expect(ideal({ frequency: 1000, amplitude: 2, phase: 1 }, 8000, 2)).toBe(2 * Math.sin((2 * Math.PI * 2000) / 8000 + 1));
});

test("derives the error bound term by term", () => {
  expect(sineErrorBound(1, 0)).toBe(2 ** -25 + 2 ** -48);
  expect(sineErrorBound(-1, 0)).toBe(sineErrorBound(1, 0));
  expect(sineErrorBound(0.75, 0)).toBe(2 ** -25 + 0.75 * 2 ** -48);
  expect(sineErrorBound(1.5, 0)).toBe(2 ** -24 + 1.5 * 2 ** -48);
  expect(sineErrorBound(0, 1000)).toBe(0);
  expect(sineErrorBound(1, 2 ** 29) - sineErrorBound(1, 0)).toBeCloseTo(2 * Math.PI * 2 ** -23, 20);
});

/** A binary32 sine oscillator: phase accumulated in `phase` arithmetic, output rounded. */
function oscillator(frames: number, rate: number, frequency: number, step: (phase: number, increment: number) => number, gain = 1): number[] {
  const increment = Math.fround(frequency) / rate;
  let phase = 0;
  return Array.from({ length: frames }, () => {
    const sample = Math.fround(gain * Math.sin(2 * Math.PI * phase));
    phase = step(phase, increment);
    return sample;
  });
}

/** The engine's accumulation: binary64, wrapped below one cycle. */
const double = (phase: number, increment: number): number => {
  const next = phase + (increment - Math.floor(increment));
  return next >= 1 ? next - 1 : next;
};

test("accepts a correct oscillator at the edges of the rate matrix", () => {
  // Measured worst cases: near Nyquist at 8 kHz over 1 s, and 19 999 Hz at 384 kHz over 10 s.
  for (const [rate, frequency, frames] of [
    [8000, 3999, 8000],
    [384_000, 19_999, 3_840_000],
    [44_100, 440.1, 44_100],
  ] as const) {
    const samples = oscillator(frames, rate, frequency, double);
    const sine = { frequency: Math.fround(frequency), amplitude: 1, phase: 0 };
    expect(sineError(samples, rate, sine)).toBeLessThanOrEqual(sineErrorBound(1, frames));
  }
});

test("rejects oscillators measurably worse than the engine's", () => {
  const sine = { frequency: 440, amplitude: 1, phase: 0 };
  const single = (phase: number, increment: number): number => {
    const next = Math.fround(phase + Math.fround(increment));
    return next >= 1 ? Math.fround(next - 1) : next;
  };
  const bound = sineErrorBound(1, 48_000);
  expect(sineError(oscillator(48_000, 48_000, 440, single), 48_000, sine)).toBeGreaterThan(bound);
  expect(sineError(oscillator(48_000, 48_000, 440, double, 0.9999995), 48_000, sine)).toBeGreaterThan(bound);
});
