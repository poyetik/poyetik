import { describe, expect, test } from "vitest";
import { RENDER_FORMATS, type RenderFormat } from "../src/format.ts";
import type { CaseOutput } from "../src/inputs.ts";
import {
  BS1770,
  CEILING_DBTP,
  INTERPOLATORS,
  NEED_MARGIN,
  SCAN,
  allowance,
  besselI0,
  controlError,
  delayOf,
  framesOf,
  holdLimited,
  judgeLimited,
  kaiser,
  latency,
  limitedError,
  limitedPlan,
  range,
  spread,
  underRead,
  type LimitedPlan,
} from "../src/limited.ts";
import { measure, sineBounds } from "../src/measure.ts";
import { ideal, sineErrorBound, type Sine } from "../src/signal.ts";
import { limit } from "./limiter-model.ts";

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
    const read = underRead(2 * Math.PI * share, INTERPOLATORS.standard);
    expect(read.low).toBeCloseTo(low, 9);
    expect(read.high).toBeCloseTo(high, 9);
  });

  test("gives the range of a limited sine from its under-read", () => {
    const omega = (2 * Math.PI * 1000) / 48_000;
    expect(range(underRead(omega, INTERPOLATORS.standard), 1).lowest).toBeCloseTo(CEILING / 1.0010159769025224, 9);
    expect(range(underRead(omega, INTERPOLATORS.standard), -1).highest).toBeCloseTo(CEILING / 0.9996831277349645, 9);
  });

  test("counts its frames and its latency as the limiter does", () => {
    expect(framesOf(1.5, 48_000)).toBe(72);
    expect(framesOf(0.5, 8000)).toBe(4);
    expect(framesOf(1 / 96, 48_000)).toBe(1);
    expect(framesOf(0.001, 8000)).toBe(1);
    expect(latency(OUTPUT, 48_000)).toBe(78);
    expect(latency({ ...OUTPUT, detector: "precise" }, 48_000)).toBe(88);
    expect(latency({ ...OUTPUT, detector: "mastering" }, 96_000)).toBe(208);
  });
});

describe("the windowed detectors", () => {
  test.each([
    // I0 from published tables, as the nearest binary64.
    [0, 1],
    [1, 1.266_065_877_752_008_4],
    [5, 27.239_871_823_604_442],
    [10, 2815.716_628_466_254_4],
  ])("sums I0(%f) to the precision of binary64", (x, expected) => {
    expect(Math.abs(besselI0(x) - expected)).toBeLessThanOrEqual(4 * Number.EPSILON * expected);
  });

  test("delays each detector by half its taps", () => {
    expect([delayOf(INTERPOLATORS.standard), delayOf(INTERPOLATORS.precise), delayOf(INTERPOLATORS.mastering)]).toStrictEqual([6, 16, 64]);
    expect(INTERPOLATORS.standard.phases).toHaveLength(4);
    expect(INTERPOLATORS.standard.phases[3]).toStrictEqual(BS1770.map(([, , , fourth]) => fourth));
  });

  test.each<[string, number, number, number, number, number]>([
    // Computed apart, in 60-digit decimal arithmetic (sin and I0 by their series): phase, tap, coefficient.
    ["precise", 0, 0, -0.002_337_656_472_999_305_5, 4, 32],
    ["precise", 0, 15, 0.139_122_943_038_199_72, 4, 32],
    ["precise", 0, 16, 0.976_637_344_266_209_4, 4, 32],
    ["precise", 3, 0, -0.002_820_208_509_173_462, 4, 32],
    ["precise", 3, 31, -0.002_337_656_472_999_305_5, 4, 32],
    ["mastering", 0, 0, -0.000_035_656_639_766_639_76, 8, 128],
    ["mastering", 0, 63, 0.066_209_650_614_578_85, 8, 128],
    ["mastering", 0, 64, 0.993_619_669_188_794_9, 8, 128],
    ["mastering", 7, 0, -0.000_042_559_165_446_621_81, 8, 128],
    ["mastering", 7, 127, -0.000_035_656_639_766_639_76, 8, 128],
  ])("computes the %s interpolator's phase %i, tap %i, as its definition", (detector, phase, tap, expected, phases, taps) => {
    const interpolator = detector === "precise" ? INTERPOLATORS.precise : INTERPOLATORS.mastering;
    expect(interpolator.phases).toHaveLength(phases);
    expect(interpolator.taps).toBe(taps);
    interpolator.phases.forEach((coefficients) => {
      expect(coefficients).toHaveLength(taps);
    });
    // Within (taps + 14) · 2^-53 of it, as the doc of `kaiser` derives.
    expect(Math.abs((interpolator.phases[phase]?.[tap] ?? 0) - expected)).toBeLessThanOrEqual((taps + 14) * 2 ** -53);
  });

  test("windows the sinc over every point, from one end to the other", () => {
    // Two phases of two taps (β 1): points at -1.5, -0.5, 0.5, 1.5 from the centre, r = ±1 at the ends.
    const ends = Math.sin(0.75 * Math.PI) / (0.75 * Math.PI);
    const inner = (Math.sin(0.25 * Math.PI) / (0.25 * Math.PI)) * besselI0(Math.sqrt(1 - 1 / 9));
    const tiny = kaiser(2, 2, 1);
    expect(tiny.taps).toBe(2);
    expect(tiny.phases[0]?.[0]).toBeCloseTo(ends / (ends + inner), 15);
    expect(tiny.phases[0]?.[1]).toBeCloseTo(inner / (ends + inner), 15);
    expect(tiny.phases[1]?.[0]).toBeCloseTo(inner / (ends + inner), 15);
  });

  test.each<[string, number, number, number]>([
    // Computed apart (the same scan, written in Python, its own Kaiser window): R lowered by its scan error, and G.
    ["precise", 0.1, 1.000_582_933_384_771_3, 1.009_938_710_539_364],
    ["precise", 0.45, 0.987_287_357_129_571_3, 1.021_096_010_289_353_4],
    ["precise", 0.49, 0.443_634_000_870_577_97, 1],
    ["precise", 997 / 44_100, 1.007_593_495_792_693_3, 1.009_020_038_294_543_8],
    ["mastering", 0.3, 0.996_594_160_400_176_9, 1.000_126_249_552_488],
    ["mastering", 0.49, 0.940_792_781_564_091, 1],
    ["mastering", 30_011 / 96_000, 0.994_776_652_325_753_6, 1],
  ])("reads a sine at %s %f of the rate low by its computed worst case", (detector, share, low, high) => {
    const read = underRead(2 * Math.PI * share, detector === "precise" ? INTERPOLATORS.precise : INTERPOLATORS.mastering);
    expect(read.low).toBeCloseTo(low, 9);
    expect(read.high).toBeCloseTo(high, 9);
  });
});

describe("the bounds of a limited sine", () => {
  /** The measurements of a 1 kHz sine of `amplitude` at 48 kHz, {@link FRAMES} frames. */
  const measured = (amplitude: number): ReturnType<typeof measure> => measure(Float64Array.from(render(FRAMES, amplitude, 0)), 48_000, 1000);
  /** No allowance at all. */
  const NONE = { below: 0, above: 0 };

  test("holds the fundamental to the range at its ends, within the fit's floor", () => {
    const sine = measured(0.89);
    const floor = sine.sine.floor;
    const at = (lowest: number, highest: number): boolean => !("problems" in holdLimited(sine, { lowest, highest }, NONE));
    expect(at(sine.fundamental - floor - 0.01, sine.fundamental - floor)).toBe(true);
    expect(at(sine.fundamental + floor, sine.fundamental + floor + 0.01)).toBe(true);
    expect(problemsOf(holdLimited(sine, { lowest: sine.fundamental - floor - 0.02, highest: sine.fundamental - floor - 0.01 }, NONE))).toMatch(/^fundamental 0\.8\d+, not between/);
    expect(at(sine.fundamental + floor + 0.01, sine.fundamental + floor + 0.02)).toBe(false);
  });

  test("moves each end by its side of the allowance, as the sine fit carries it", () => {
    const sine = measured(0.89);
    const carried = Math.sqrt(2 * (1 + sine.sine.conditioning)) / (1 - sine.sine.conditioning);
    const passes = (within: { lowest: number; highest: number }, allowed: { below: number; above: number }): boolean => !("problems" in holdLimited(sine, within, allowed));
    // The fundamental 0.004 below the range: an allowance below of 0.004 / carried, and the floor, reach it.
    const above = { lowest: sine.fundamental + 0.004, highest: sine.fundamental + 0.01 };
    expect(passes(above, { below: (0.004 * 1.001) / carried, above: 0 })).toBe(true);
    expect(passes(above, { below: (0.004 * 0.999) / carried, above: 0.1 })).toBe(false);
    const below = { lowest: sine.fundamental - 0.01, highest: sine.fundamental - 0.004 };
    expect(passes(below, { below: 0, above: (0.004 * 1.001) / carried })).toBe(true);
    expect(passes(below, { below: 0.1, above: (0.004 * 0.999) / carried })).toBe(false);
  });

  test("holds THD+N to the bound of the range, and refuses a range too wide to bound it", () => {
    const sine = measured(0.89);
    const within = { lowest: 0.88, highest: 0.9 };
    const allowed = { below: 1e-3, above: 2e-3 };
    expect(holdLimited(sine, within, allowed)).toStrictEqual({ fundamental: sine.fundamental, thdPlusN: sine.thdPlusN });
    // Exactly at its bound, a THD+N passes; the bound takes the larger side of the allowance.
    const bound = sineBounds((within.highest + within.lowest) / 2, 0.01 + 2e-3, sine)?.thdPlusN ?? 0;
    expect(holdLimited({ ...sine, thdPlusN: bound }, within, allowed)).toStrictEqual({ fundamental: sine.fundamental, thdPlusN: bound });
    expect(holdLimited({ ...sine, thdPlusN: bound }, within, { below: 2e-3, above: 1e-3 })).toStrictEqual({ fundamental: sine.fundamental, thdPlusN: bound });
    expect(problemsOf(holdLimited({ ...sine, thdPlusN: bound + 1e-9 }, within, allowed))).toMatch(/^THD\+N /);
    const tight = { lowest: sine.fundamental, highest: sine.fundamental };
    expect(problemsOf(holdLimited({ ...sine, thdPlusN: -100 }, tight, NONE))).toMatch(/THD\+N -100 dB, above -\d+/);
    expect(problemsOf(holdLimited({ ...sine, thdPlusN: Number.NaN }, within, NONE))).toMatch(/^THD\+N NaN dB/);
    expect(problemsOf(holdLimited(sine, { lowest: 0, highest: 50 }, NONE))).toBe("its range leaves the fundamental no margin to be measured");
  });

  test("allows each sample half the range and the larger side of its allowance", () => {
    expect(limitedError({ lowest: 0.5, highest: 0.75 }, { below: 1e-6, above: 3e-6 })).toBe(0.125 + 3e-6);
    expect(limitedError({ lowest: 0.5, highest: 0.75 }, { below: 3e-6, above: 1e-6 })).toBe(0.125 + 3e-6);
  });
});

describe("the allowance of each render format", () => {
  const within = { lowest: 0.5, highest: 0.75 };
  /** The unit roundoff of binary64. */
  const unit = 2 ** -53;

  test.each<[RenderFormat, number, number, number]>([
    // The sample precision's unit, the rounding at delivery, the flush.
    ["binary64", unit, 0, 2 ** -1022],
    ["binary64-as-binary32", unit, 2 ** -24, 2 ** -126],
    ["binary32", 2 ** -24, 0, 2 ** -126],
  ])("counts each term of %s", (format, sample, delivered, flushed) => {
    const input = sineErrorBound(2, 4000, format);
    const allowed = allowance(within, -2, 1e-9, 4000, format);
    // The engine's margin on every need, read from its one constant: A times it, below only.
    expect(allowed.below).toBe(input + 2 * (1e-9 + NEED_MARGIN) + 0.5 * (5 * unit + 2 * sample) + 0.75 * (3 * sample + delivered) + flushed);
    expect(allowed.above).toBe(input + 2 * 1e-9 + 0.75 * (5 * unit + 2 * sample + delivered) + flushed);
  });

  test("errs on the need by the estimate's error over the ceiling, and the control path's roundings", () => {
    // The standard detector (12 taps, H its largest sum of magnitudes), 1.5 ms at 48 kHz (L = 72), a release of 4,800 frames.
    const spreading = BS1770.reduce((sum, row) => sum + Math.abs(row[0]), 0);
    expect(spread(INTERPOLATORS.standard)).toBeCloseTo(2.0228, 4);
    expect(spread(INTERPOLATORS.standard)).toBeGreaterThanOrEqual(spreading);
    expect(spread({ taps: 2, phases: [[0.25, 0.25]] })).toBe(1);
    const most = spread(INTERPOLATORS.standard);
    for (const format of RENDER_FORMATS) {
      const estimated = most * sineErrorBound(2, 4000, format) + (2 * 13 * most + 2 * 12 * 26) * unit * 2;
      expect(controlError(INTERPOLATORS.standard, -2, 4000, format, 72, 48_000)).toBeCloseTo(estimated / CEILING + 2 * unit + 2 * unit * 4801 + 16 * 73 * unit, 25);
    }
  });

  test("pins the need's error of a corpus-like case per format", () => {
    // Computed apart (Python, the same terms): standard detector, 1 kHz at 48 kHz, A = 1, 4,000 frames.
    for (const [format, expected] of [
      ["binary64", 1.395_438_604_923_123_9e-11],
      ["binary64-as-binary32", 6.765_477_710_009_013e-8],
      ["binary32", 6.766_864_046_245_947e-7],
    ] as const) {
      expect(controlError(INTERPOLATORS.standard, 1, 4000, format, 72, 48_000) / expected).toBeCloseTo(1, 12);
    }
  });
});

// Each plan scans the under-read and conditions a fit: slow under Stryker's instrumentation, given its time.
describe("the plan of a limited channel", { timeout: 60_000 }, () => {
  /** The plan of a sine at 48 kHz through the standard detector, 1.5 ms, over 4,000 frames. */
  const planOf = (frequency: number, amplitude: number, phase: number, frames = 4000, output: CaseOutput = OUTPUT, rate = 48_000): LimitedPlan | string => limitedPlan(output, rate, frames, { frequency, amplitude, phase });

  test.each<[string, number, number, number, number, number, number, number, number, CaseOutput]>([
    // Computed apart (Python, its own interpolators and scan): start, onset need, steady need, the range.
    ["1 kHz, phase 0", 48_000, 1000, 1, 0, 154, 0, 0.109_653_633_210_157_67, 0.890_346_366_789_842_4, OUTPUT],
    ["1 kHz, phase π/2", 48_000, 1000, 1, Math.PI / 2, 1798, 0.204_369_065_907_539_48, 0.109_653_633_210_157_67, 0.890_346_366_789_842_4, OUTPUT],
    ["15 kHz, where G = 1", 48_000, 15_000, 1, 0, 1044, 0.152_305_458_726_026, 0.108_749_061_866_254_44, CEILING, OUTPUT],
    ["47 kHz, folded to 1 kHz", 48_000, 47_000, 1, 0, 154, 0, 0.109_653_633_210_157_67, 0.890_346_366_789_842_4, OUTPUT],
    ["997 Hz at 44.1 kHz, precise", 44_100, 997, 2, 1, 197, 0.566_452_396_692_490_3, 0.558_358_157_267_051_3, 0.883_283_685_465_897_4, { ...OUTPUT, detector: "precise" }],
    ["30,011 Hz at 96 kHz, mastering", 96_000, 30_011, 1, 0, 2890, 0.173_792_773_894_501_9, 0.108_749_061_866_254_44, CEILING, { ...OUTPUT, detector: "mastering" }],
  ])("derives when %s has settled", (_case, rate, frequency, amplitude, phase, start, onset, steady, lowest, output) => {
    const plan = planOf(frequency, amplitude, phase, 4000, output, rate);
    expect(typeof plan === "string" ? plan : { start: plan.start, frequency: plan.frequency }).toStrictEqual({ start, frequency: frequency % rate === 47_000 ? 1000 : frequency });
    if (typeof plan !== "string") {
      expect(plan.onset).toBeCloseTo(onset, 12);
      expect(plan.steady).toBeCloseTo(steady, 12);
      expect(plan.within.lowest).toBeCloseTo(lowest, 12);
      expect(plan.rate).toBe(rate);
    }
  });

  test("bounds the amplitude by the sine's own below the ceiling", () => {
    const plan = planOf(1000, -0.5, 0);
    expect(typeof plan === "string" ? plan : plan.within).toStrictEqual({ lowest: 0.5, highest: 0.5 });
    // Between c/G (0.89035) and c/R (0.89153): the lowest end is c/G, the highest the sine's own.
    const read = { low: 0.999_683_127_734_964_5, high: 1.001_015_976_902_522_4 };
    expect(range(read, 0.890_5)).toStrictEqual({ lowest: CEILING / read.high, highest: 0.8905 });
    expect(range(read, -2)).toStrictEqual({ lowest: CEILING / read.high, highest: CEILING / read.low });
  });

  test.each<[string, number, number, number, number, string]>([
    ["DC once folded", 48_000, 1, 0, 4000, "the sine, folded to 0 Hz, is not strictly between DC and Nyquist"],
    ["Nyquist once folded", 72_000, 1, 1, 4000, "the sine, folded to 24000 Hz, is not strictly between DC and Nyquist"],
    ["a period longer than the window", 500, 1, 0, 4000, "a period of the sine (96 frames) does not fit the limiter's window (74 frames)"],
    ["an onset needing more where the steady sine needs nothing", 1000, 0.89, Math.PI / 2, 4000, "its onset needs a reduction of 0.10603265832307807, the steady sine 0: the slow stage's return is not derived"],
    ["an onset raising the slow stage above the steady need", 1000, 0.8906, Math.PI / 2, 4000, "its onset needs a reduction of 0.10663492691167697, the steady sine 0.00028478914232836386: the slow stage's return is not derived"],
    ["a case ending where it settles", 1000, 1, 0, 154, "it cannot be measured from frame 154, where the limiter has settled: the signal has 0 frames, fewer than the 3 unknowns of the fit"],
    ["a case ending two frames later", 1000, 1, 0, 156, "it cannot be measured from frame 154, where the limiter has settled: the signal has 2 frames, fewer than the 3 unknowns of the fit"],
    ["an amplitude below its error", 1000, 2 ** -125, 0, 4000, "its range leaves the fundamental no margin to be measured"],
  ])("refuses %s", (_case, frequency, amplitude, phase, frames, refusal) => {
    expect(planOf(frequency, amplitude, phase, frames)).toBe(refusal);
  });

  test("settles the slow stage below the steady need just short of refusing it", () => {
    // A little louder than the refused 0.8906: the slow stage's peak falls under the steady need.
    const plan = planOf(1000, 0.8925, Math.PI / 2, 20_000);
    expect(typeof plan === "string" ? plan : plan.start).toBe(10_202);
  });
});

// Each test renders through the model and measures: slow under Stryker's instrumentation, given its time.
describe("judgeLimited", { timeout: 60_000 }, () => {
  /** The exact render of `sine` at `rate` through the limiter model with `output`'s detector, `frames` long. */
  const rendered = (sine: Sine, frames: number, output: CaseOutput = OUTPUT, rate = 48_000): number[] => limit((frame) => ideal(sine, rate, frame), frames, rate, output.lookahead, INTERPOLATORS[output.detector].phases);
  /** The plan of `sine` at `rate` through `output`, `frames` long, which must be accepted. */
  const planned = (sine: Sine, frames: number, output: CaseOutput = OUTPUT, rate = 48_000): LimitedPlan => {
    const plan = limitedPlan(output, rate, frames, sine);
    if (typeof plan === "string") {
      throw new Error(plan);
    }
    return plan;
  };

  test.each<[string, Sine, CaseOutput, number, number]>([
    ["a 1 kHz sine", { frequency: 1000, amplitude: 1, phase: 0 }, OUTPUT, 48_000, FRAMES],
    ["a 1 kHz sine from its peak", { frequency: 1000, amplitude: 1, phase: Math.PI / 2 }, OUTPUT, 48_000, 1798 + FRAMES],
    ["a 15 kHz sine, where G = 1", { frequency: 15_000, amplitude: 1, phase: 0 }, OUTPUT, 48_000, 1044 + FRAMES],
    ["a 47 kHz sine, folded", { frequency: 47_000, amplitude: 1, phase: 0 }, OUTPUT, 48_000, FRAMES],
    ["a sine below the ceiling", { frequency: 1000, amplitude: 0.5, phase: 0 }, OUTPUT, 48_000, FRAMES],
    ["a precise render at 44.1 kHz", { frequency: 997, amplitude: 2, phase: 1 }, { ...OUTPUT, detector: "precise" }, 44_100, FRAMES],
    ["a mastering render at 96 kHz", { frequency: 30_011, amplitude: 1, phase: 0 }, { ...OUTPUT, detector: "mastering" }, 96_000, 2890 + FRAMES],
  ])("accepts the exact render of %s, in every format", (_case, sine, output, rate, frames) => {
    const plan = planned(sine, frames, output, rate);
    const samples = rendered(sine, frames, output, rate);
    expect(judgeLimited(samples, plan, "binary64")).not.toHaveProperty("problems");
    expect(judgeLimited(samples.map(Math.fround), plan, "binary64-as-binary32")).not.toHaveProperty("problems");
  });

  test("fails a render measured before it has settled", () => {
    // From its peak, the onset's need takes the fast stage frames to release: measured from
    // where it starts in two windows after the latency (226), the gain still moves.
    const sine = { frequency: 1000, amplitude: 1, phase: Math.PI / 2 };
    const plan = planned(sine, 1798 + FRAMES);
    const samples = rendered(sine, 1798 + FRAMES);
    expect(judgeLimited(samples, { ...plan, start: 226 }, "binary64")).toHaveProperty("problems");
    expect(judgeLimited(samples, plan, "binary64")).not.toHaveProperty("problems");
  });

  test("measures from its start on, not a frame before", () => {
    const sine = { frequency: 1000, amplitude: 1, phase: 0 };
    const plan = planned(sine, FRAMES);
    const at = (frame: number): number[] => rendered(sine, FRAMES).map((sample, index) => (index === frame ? sample + 0.5 : sample));
    expect(judgeLimited(at(plan.start - 1), plan, "binary64")).not.toHaveProperty("problems");
    expect(judgeLimited(at(plan.start), plan, "binary64")).toHaveProperty("problems");
  });

  test("allows a binary32 render the rounding of its format where G = 1, and no more", () => {
    // At 15 kHz the steady output lies on the range's lowest end, c: a binary32 render lies
    // below it by its roundings, which its allowance covers (about 1.6e-6 of the ceiling in
    // binary32, the sine's own error and the need's carried); 2^-17 below, it fails.
    const sine = { frequency: 15_000, amplitude: 1, phase: 0 };
    const plan = planned(sine, 1044 + FRAMES);
    const samples = rendered(sine, 1044 + FRAMES);
    expect(judgeLimited(samples.map((sample) => Math.fround(sample * (1 - 2 ** -23))), plan, "binary32")).not.toHaveProperty("problems");
    expect(judgeLimited(samples.map((sample) => Math.fround(sample * (1 - 2 ** -17))), plan, "binary32")).toMatchObject({ problems: [expect.stringMatching(/^fundamental /)] });
  });

  test.each<[string, (samples: number[]) => number[], string]>([
    ["a sample during the latency", (samples) => samples.map((sample, frame) => (frame === 77 ? 1e-30 : sample)), "not silent over its latency, 78 frames"],
    ["a fundamental above the range", (samples) => samples.map((sample) => sample * 1.01), "fundamental"],
    ["a fundamental below the range", (samples) => samples.map((sample) => sample * 0.99), "fundamental"],
    ["a distorted sine", (samples) => samples.map((sample, frame) => (frame < 78 ? 0 : sample + 0.05 * Math.sin((2 * Math.PI * 3000 * frame) / 48_000))), "THD+N"],
    ["silence", (samples) => samples.map(() => 0), "cannot be measured: MeasureError"],
  ])("refuses %s", (_case, change, problem) => {
    const sine = { frequency: 1000, amplitude: 1, phase: 0 };
    const judged = judgeLimited(change(rendered(sine, FRAMES)), planned(sine, FRAMES), "binary64");
    expect("problems" in judged ? judged.problems.join("\n") : "").toContain(problem);
  });

  test("judges every detector after its own latency", () => {
    for (const [detector, delay] of [["precise", 88], ["mastering", 136]] as const) {
      const sine = { frequency: 1000, amplitude: 1, phase: 0 };
      const output = { ...OUTPUT, detector };
      const plan = planned(sine, FRAMES, output);
      expect(plan.latency).toBe(delay);
      const early = rendered(sine, FRAMES, output).map((sample, frame) => (frame === delay - 1 ? 1e-30 : sample));
      expect(judgeLimited(early, plan, "binary64")).toStrictEqual({ problems: [`not silent over its latency, ${String(delay)} frames`] });
    }
  });
});
