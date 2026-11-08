import { describe, expect, test } from "vitest";
import { HISTORY_SECONDS } from "../src/settings.ts";
import { WINDOW_SECONDS } from "../src/Oscilloscope.tsx";
import { frequency, MAX_FREQUENCY_RATIO, MIN_PERIODS, peak, risingCrossings, scopeScene } from "../src/scope.ts";
import { SAMPLE_RATES } from "./matrix.ts";

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
    // One rising crossing every 40 samples, four periods.
    const square = Float32Array.from({ length: 161 }, (_, index) => (index % 40 < 20 ? -1 : 1));
    expect(frequency(square, 4000)).toBe(100);
    expect(frequency(square, 8000)).toBe(200);
  });

  test("needs three periods", () => {
    expect(MIN_PERIODS).toBe(3);
    // A square of period 40 samples: three crossings are two periods, four are three.
    const periods = (count: number): Float32Array => Float32Array.from({ length: count * 40 + 21 }, (_, index) => (index % 40 < 20 ? -1 : 1));
    expect(frequency(periods(2), 4000)).toBeUndefined();
    expect(frequency(periods(3), 4000)).toBe(100);
    expect(frequency(new Float32Array(16), 48_000)).toBeUndefined();
  });

  test("measures nothing above a twentieth of the rate", () => {
    expect(MAX_FREQUENCY_RATIO).toBe(1 / 20);
    const at = (hertz: number): number | undefined => frequency(sine(hertz, 48_000, 4096, 0.3), 48_000);
    expect(at(2399)).toBeDefined();
    expect(at(2401)).toBeUndefined();
    // Exactly a twentieth (a square of period 20 samples at 48 kHz) is still measured.
    const edge = Float32Array.from({ length: 101 }, (_, index) => (index % 20 < 10 ? -1 : 1));
    expect(frequency(edge, 48_000)).toBe(2400);
  });

  test("is within 0.05 Hz everywhere in its domain, at every rate of the matrix", () => {
    // Measured worst case 0.011 Hz: from three periods in the history to a twentieth of the
    // rate, at four phases.
    for (const rate of SAMPLE_RATES) {
      const count = Math.round(HISTORY_SECONDS * rate);
      const lowest = (MIN_PERIODS + 1) / HISTORY_SECONDS;
      for (const hertz of [lowest, 100, 440, 1000, rate * MAX_FREQUENCY_RATIO * 0.99].filter((f) => f <= rate * MAX_FREQUENCY_RATIO)) {
        for (const phase of [0, 0.7, 2, 4.5]) {
          const measured = frequency(sine(hertz, rate, count, phase), rate);
          expect(measured, `${String(hertz)} Hz at ${String(rate)} Hz, phase ${String(phase)}`).toBeDefined();
          expect(Math.abs((measured ?? 0) - hertz)).toBeLessThan(0.05);
        }
      }
    }
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

  test("shifts the trace so that the interpolated crossing is at the left edge", () => {
    // Crossing at 0.5: the trace starts at sample 1, half a sample to the right.
    const scene = scopeScene(new Float32Array([-1, 1, 0.5, -0.5, -1]), 4, 30, 2);
    expect(scene.trace.map((point) => point.x)).toStrictEqual([5, 15, 25, 35]);
  });

  test("triggers at every rate of the matrix: the trace is the sine from its crossing", () => {
    // Every period up to 75 ms (85 ms of history, 10 ms shown) finds a crossing, and the
    // trace, shifted by the fraction of a sample, is the sine starting at phase 0 at the
    // left edge. The crossing comes from the line through two samples h radians apart,
    // which misplaces the zero of a sine by up to h³/16 radians (measured over the matrix
    // at 64 phases: at most half of the h³/8 allowed), plus the binary32 rounding of the
    // samples (2^-24): never the whole sample of jitter of an integer trigger.
    for (const rate of SAMPLE_RATES) {
      const count = Math.round(HISTORY_SECONDS * rate);
      const window = Math.round(WINDOW_SECONDS * rate);
      const step = 900 / (window - 1);
      for (const hertz of [13.4, 440, 5000].filter((f) => f <= rate / 4)) {
        const h = (2 * Math.PI * hertz) / rate;
        const bound = 135 * (h ** 3 / 8 + 2 ** -24);
        for (const phase of [0, 1, 2.5, 4, 5.5]) {
          const { trace } = scopeScene(sine(hertz, rate, count, phase), window, 900, 300);
          expect(trace).toHaveLength(window);
          for (const point of trace) {
            const ideal = 150 - 135 * Math.sin((2 * Math.PI * hertz * (point.x / step)) / rate);
            expect(Math.abs(point.y - ideal), `${String(hertz)} Hz at ${String(rate)} Hz, phase ${String(phase)}`).toBeLessThanOrEqual(bound);
          }
        }
      }
    }
  });

  test("puts a one-sample window at the left edge", () => {
    expect(scopeScene(new Float32Array([0.5]), 1, 30, 2).trace).toStrictEqual([{ x: 0, y: 1 - 0.45 }]);
  });
});
