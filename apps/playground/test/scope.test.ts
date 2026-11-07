import { describe, expect, test } from "vitest";
import { frequency, peak, risingCrossings, scopeScene } from "../src/scope.ts";

/** `count` samples of a sine at `hertz`, sampled at `rate`. */
function sine(hertz: number, rate: number, count: number, phase = 0): Float32Array {
  return Float32Array.from({ length: count }, (_, index) => Math.sin(phase + (2 * Math.PI * hertz * index) / rate));
}

describe("risingCrossings", () => {
  test("finds upward zero crossings, interpolated", () => {
    expect(risingCrossings(new Float32Array([-1, 1, 2, -2, -1, 3]))).toStrictEqual([0.5, 4.25]);
  });

  test("counts a start at zero, and not a downward crossing", () => {
    expect(risingCrossings(new Float32Array([0, 1, 0, -1, 0]))).toStrictEqual([0]);
  });

  test("never counts the first sample as a crossing", () => {
    expect(risingCrossings(new Float32Array([1, -1, 1]))).toStrictEqual([1.5]);
  });

  test("finds nothing in silence or a single sample", () => {
    expect(risingCrossings(new Float32Array(8))).toStrictEqual([]);
    expect(risingCrossings(new Float32Array([1]))).toStrictEqual([]);
    expect(risingCrossings(new Float32Array())).toStrictEqual([]);
  });
});

describe("frequency", () => {
  test.each([440, 1000, 55.5])("measures a %f Hz sine to a hundredth of a hertz", (hertz) => {
    const measured = frequency(sine(hertz, 48_000, 4096, 0.3), 48_000);
    expect(measured).toBeDefined();
    expect(Math.abs((measured ?? 0) - hertz)).toBeLessThan(0.01);
  });

  test("scales with the sample rate", () => {
    // One rising crossing every 4 samples.
    const square = new Float32Array([-1, 1, 1, -1, -1, 1, 1, -1, -1, 1]);
    expect(frequency(square, 400)).toBe(100);
    expect(frequency(square, 800)).toBe(200);
  });

  test("needs two crossings", () => {
    expect(frequency(new Float32Array([-1, 1, 1]), 48_000)).toBeUndefined();
    expect(frequency(new Float32Array(16), 48_000)).toBeUndefined();
  });
});

describe("peak", () => {
  test("is the largest absolute value", () => {
    expect(peak(new Float32Array([0.5, -0.75, 0.25]))).toBe(0.75);
    expect(peak(new Float32Array([0.5, 0.75]))).toBe(0.75);
    expect(peak(new Float32Array())).toBe(0);
  });
});

describe("scopeScene", () => {
  test("lays the grid out at +1, 0 and -1 with a margin", () => {
    expect(scopeScene(new Float32Array(), 0, 100, 200).grid).toStrictEqual([10, 100, 190]);
  });

  test("maps samples to pixels: time across, amplitude up", () => {
    const scene = scopeScene(new Float32Array([0, 1, 0, -1, 0]), 5, 100, 200);
    expect(scene).toMatchObject({ width: 100, height: 200 });
    expect(scene.trace).toStrictEqual([
      { x: 0, y: 100 },
      { x: 25, y: 10 },
      { x: 50, y: 100 },
      { x: 75, y: 190 },
      { x: 100, y: 100 },
    ]);
  });

  test("starts at the latest rising crossing that leaves a full window", () => {
    // Rising crossings at 1 and 5 (rounded up); with 4 samples shown, 5 does not fit.
    const samples = new Float32Array([-1, 1, 0.5, -0.5, -1, 1, 0.5, -0.5]);
    expect(scopeScene(samples, 4, 3, 2).trace.map((point) => point.y)).toStrictEqual([1 - 0.9, 1 - 0.45, 1 + 0.45, 1 + 0.9]);
    expect(scopeScene(samples, 3, 2, 2).trace.map((point) => point.y)).toStrictEqual([1 - 0.9, 1 - 0.45, 1 + 0.45]);
  });

  test("prefers a crossing that ends exactly at the last sample", () => {
    // Rising crossings at 1 and 3; with 2 samples shown, 3 just fits.
    const samples = new Float32Array([-1, 1, -1, 1, 0.5]);
    expect(scopeScene(samples, 2, 1, 2).trace.map((point) => point.y)).toStrictEqual([1 - 0.9, 1 - 0.45]);
  });

  test("rounds an interpolated crossing up to the first positive sample", () => {
    const samples = new Float32Array([-1, 0.5, 1, 0.5, 0, -0.5]);
    expect(scopeScene(samples, 2, 1, 2).trace[0]?.y).toBeCloseTo(1 - 0.45);
  });

  test("shows the most recent window when nothing triggers", () => {
    const samples = new Float32Array([0.1, 0.2, 0.3, 0.4]);
    expect(scopeScene(samples, 2, 1, 2).trace.map((point) => point.y)).toStrictEqual([expect.closeTo(1 - 0.3 * 0.9), expect.closeTo(1 - 0.4 * 0.9)]);
  });

  test("shows what there is when the window is longer than the samples", () => {
    expect(scopeScene(new Float32Array([0.5]), 4, 30, 2).trace).toStrictEqual([{ x: 0, y: 1 - 0.45 }]);
  });

  test("puts a one-sample window at the left edge", () => {
    expect(scopeScene(new Float32Array([0.5]), 1, 30, 2).trace).toStrictEqual([{ x: 0, y: 1 - 0.45 }]);
  });
});
