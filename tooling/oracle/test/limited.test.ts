import { describe, expect, test } from "vitest";
import { BS1770, CEILING_DBTP, SCAN, STANDARD_DELAY, framesOf, holdLimited, judgeLimited, latency, limitedError, range, rounding, underRead } from "../src/limited.ts";
import { measure, sineBounds } from "../src/measure.ts";

/** The protected output of the oracle's cases: the standard detector, 1.5 ms. */
const OUTPUT = { mode: "protected", lookahead: 1.5, detector: "standard" } as const;

/** 10^(-1/20): the default ceiling as an amplitude, computed apart. */
const CEILING = 0.891_250_938_133_745_6;

/**
 * Frames of every rendered sine: many periods to measure past the latency and the two
 * windows. What the judge decides does not depend on it, the floor of its measurements
 * included, while their cost grows with it, many times over under Stryker's
 * instrumentation, whose initial run times every test.
 */
const FRAMES = 2048;

/** The problems of a verdict, one per line; empty when it passed. */
function problemsOf(verdict: ReturnType<typeof holdLimited>): string {
  return "problems" in verdict ? verdict.problems.join("\n") : "";
}

/** `frames` frames at 48 kHz of `frequency` hertz: `silence` frames of silence, then `amplitude · sin(2π f n / rate)`, plus `wobble(n)`. */
function render(frames: number, amplitude: number, silence = 78, wobble: (frame: number) => number = () => 0, frequency = 1000): number[] {
  return Array.from({ length: frames }, (_value, frame) => (frame < silence ? 0 : amplitude * Math.sin((2 * Math.PI * frequency * frame) / 48_000) + wobble(frame)));
}

describe("the standard detector's under-read", () => {
  test("holds the standard's table and the facts derived from it", () => {
    expect(BS1770).toHaveLength(12);
    expect(BS1770[5]).toStrictEqual([0.1373291015625, 0.465087890625, 0.77978515625, 0.97216796875]);
    expect(CEILING_DBTP).toBe(-1);
    expect(STANDARD_DELAY).toBe(6);
    expect(SCAN).toBe(4000);
  });

  test.each([
    // Computed apart (the same scan, written in Python): R lowered by its scan error, and G.
    [0.1, 0.999153265747489, 1.022160300599261],
    [0.3, 0.9823070689777499, 1],
    [0.45, 0.7780685528902791, 1],
    // Where the interval's own samples decide, every sign of the candidates matters.
    [0.49, 0.183082949592645, 1],
    [1000 / 48_000, 0.9996831277349645, 1.0010159769025224],
  ])("reads a sine at %f of the rate low by its computed worst case", (share, low, high) => {
    const read = underRead(2 * Math.PI * share);
    expect(read.low).toBeCloseTo(low, 9);
    expect(read.high).toBeCloseTo(high, 9);
  });

  test("gives the range of a limited sine from its under-read", () => {
    const omega = (2 * Math.PI * 1000) / 48_000;
    expect(range(omega).lowest).toBeCloseTo(CEILING / 1.0010159769025224, 9);
    expect(range(omega).highest).toBeCloseTo(CEILING / 0.9996831277349645, 9);
  });

  test("counts its frames and its latency as the limiter does", () => {
    expect(framesOf(1.5, 48_000)).toBe(72);
    expect(framesOf(0.5, 8000)).toBe(4);
    expect(framesOf(1 / 96, 48_000)).toBe(1);
    expect(framesOf(0.001, 8000)).toBe(1);
    expect(latency(OUTPUT, 48_000)).toBe(78);
  });
});

describe("the bounds of a limited sine", () => {
  test("rounds each format as it gives its samples", () => {
    expect(rounding("binary64")).toBe(2 * Number.EPSILON);
    expect(rounding("binary64-as-binary32")).toBe(2 ** -23);
    expect(rounding("binary32")).toBe(2 ** -23);
  });

  test("allows each sample half the range and the rounding at its top", () => {
    const within = { lowest: 0.5, highest: 0.75 };
    expect(limitedError(within, "binary64")).toBe(0.125 + 2 * Number.EPSILON * 0.75);
    expect(limitedError(within, "binary32")).toBe(0.125 + 2 ** -23 * 0.75);
  });

  /** The measurements of a 1 kHz sine of `amplitude` at 48 kHz, {@link FRAMES} frames. */
  const measured = (amplitude: number): ReturnType<typeof measure> => measure(Float64Array.from(render(FRAMES, amplitude, 0)), 48_000, 1000);

  test("holds the true peak to the range at its ends, within the fit's floor", () => {
    const sine = measured(0.89);
    const floor = sine.sine.floor;
    const at = (lowest: number, highest: number): boolean => !("problems" in holdLimited(sine, { lowest, highest }, "binary64"));
    expect(at(sine.fundamental - floor - 0.01, sine.fundamental - floor)).toBe(true);
    expect(at(sine.fundamental + floor, sine.fundamental + floor + 0.01)).toBe(true);
    expect(problemsOf(holdLimited(sine, { lowest: sine.fundamental - floor - 0.02, highest: sine.fundamental - floor - 0.01 }, "binary64"))).toMatch(/^true peak 0\.8\d+, not between/);
    expect(at(sine.fundamental + floor + 0.01, sine.fundamental + floor + 0.02)).toBe(false);
  });

  test("holds THD+N to the bound of the range, and refuses a range too wide to bound it", () => {
    const sine = measured(0.89);
    const within = { lowest: 0.88, highest: 0.9 };
    expect(holdLimited(sine, within, "binary32")).toStrictEqual({ peak: sine.fundamental, thdPlusN: sine.thdPlusN });
    // Exactly at its bound, a THD+N passes.
    const bound = sineBounds((within.highest + within.lowest) / 2, limitedError(within, "binary32"), sine)?.thdPlusN ?? 0;
    expect(holdLimited({ ...sine, thdPlusN: bound }, within, "binary32")).toStrictEqual({ peak: sine.fundamental, thdPlusN: bound });
    const tight = { lowest: sine.fundamental, highest: sine.fundamental };
    expect(problemsOf(holdLimited({ ...sine, thdPlusN: -100 }, tight, "binary64"))).toMatch(/THD\+N -100 dB, above -\d+/);
    expect(problemsOf(holdLimited({ ...sine, thdPlusN: Number.NaN }, within, "binary64"))).toMatch(/^THD\+N NaN dB/);
    expect(problemsOf(holdLimited(sine, { lowest: 0, highest: 50 }, "binary64"))).toBe("its range leaves the fundamental no margin to be measured");
  });
});

describe("judgeLimited", () => {
  const sine = { frequency: 1000, amplitude: 1, phase: 0 };
  const { lowest, highest } = range((2 * Math.PI * 1000) / 48_000);

  test("accepts a sine limited within the derived range, silent over its latency", () => {
    for (const amplitude of [lowest, (lowest + highest) / 2, highest]) {
      const judged = judgeLimited(render(FRAMES, amplitude), 48_000, sine, OUTPUT, "binary64");
      expect("peak" in judged ? judged.peak : 0).toBeCloseTo(amplitude, 9);
    }
    expect(judgeLimited(render(FRAMES, highest), 48_000, sine, OUTPUT, "binary32")).not.toHaveProperty("problems");
    // Several judgements: slow under Stryker's instrumentation, given its time.
  }, 60_000);

  test("measures only once two windows have passed after the latency", () => {
    // A transient over the two windows after the latency (78 + 148 frames) is the
    // limiter's attack, never measured; one frame later, it is.
    const middle = (lowest + highest) / 2;
    const attack = render(FRAMES, middle, 78, (frame) => (frame < 78 + 148 ? 0.5 : 0));
    expect(judgeLimited(attack, 48_000, sine, OUTPUT, "binary64")).not.toHaveProperty("problems");
    const late = render(FRAMES, middle, 78, (frame) => (frame === 78 + 148 ? 0.5 : 0));
    expect(judgeLimited(late, 48_000, sine, OUTPUT, "binary64")).toHaveProperty("problems");
  });

  test("accepts a period exactly as long as the window", () => {
    // 1.625 ms at 48 kHz: 78 frames of lookahead, a window of 80, a period of 80 at 600 Hz.
    const output = { ...OUTPUT, lookahead: 1.625 };
    const within = range((2 * Math.PI * 600) / 48_000);
    const samples = render(FRAMES, (within.lowest + within.highest) / 2, 84, () => 0, 600);
    expect(judgeLimited(samples, 48_000, { ...sine, frequency: 600 }, output, "binary64")).not.toHaveProperty("problems");
  });

  test.each<[string, number[], string]>([
    ["a sample during the latency", render(FRAMES, 0.89, 77), "not silent over its latency, 78 frames"],
    ["a true peak above the range", render(FRAMES, highest + 0.01), "true peak"],
    ["a true peak below the range", render(FRAMES, lowest - 0.01), "true peak"],
    ["a distorted sine", render(FRAMES, 0.89, 78, (frame) => 0.05 * Math.sin((2 * Math.PI * 3000 * frame) / 48_000)), "THD+N"],
    ["silence", render(FRAMES, 0), "cannot be measured: MeasureError"],
  ])("refuses %s", (_case, samples, problem) => {
    const judged = judgeLimited(samples, 48_000, sine, OUTPUT, "binary64");
    expect("problems" in judged ? judged.problems.join("\n") : "").toContain(problem);
  });

  test("judges the standard detector only, and a sine whose period fits the window", () => {
    expect(judgeLimited(render(FRAMES, 0.89), 48_000, sine, { ...OUTPUT, detector: "precise" }, "binary64")).toStrictEqual({
      problems: ["the oracle judges limited channels with the standard detector only, not precise"],
    });
    expect(judgeLimited(render(FRAMES, 0.89), 48_000, { ...sine, frequency: 500 }, OUTPUT, "binary64")).toStrictEqual({
      problems: ["a period of the sine (96 frames) does not fit the limiter's window (74 frames)"],
    });
  });
});
