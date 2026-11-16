import { describe, expect, test } from "vitest";
import { RENDER_FORMATS, type RenderFormat } from "../src/format.ts";
import type { ProtectedOutput } from "../src/inputs.ts";
import {
  BS1770,
  CEILING_DBTP,
  INTERPOLATORS,
  needMargin,
  RELEASE_MS,
  SCAN,
  allowance,
  besselI0,
  controlError,
  delayOf,
  fastKeep,
  fastRelease,
  framesOf,
  holdLimited,
  judgeLimited,
  kaiser,
  latency,
  limitedError,
  keep,
  limitedPlan,
  LANES,
  range,
  reductionRoundings,
  released,
  slowKeep,
  spread,
  TREE_LEVELS,
  underRead,
  type LimitedPlan,
} from "../src/limited.ts";
import { MAX_FRAMES, measure, sineBounds } from "../src/measure.ts";
import { ideal, type Sine } from "../src/signal.ts";
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
    // Where the interval's later end decides.
    [0.1892, 0.978_492_891_919_088, 1.002_970_559_938_670_2],
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
    const bound = sineBounds((within.highest + within.lowest) / 2, limitedError(within, allowed), sine)?.thdPlusN ?? 0;
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

  test.each<[RenderFormat, number, number]>([
    // Computed apart (Python, exact powers of two and 80-digit decimals, from the list of
    // rounding steps: the input sine in the computing precision, the margin below only,
    // the ceiling rounded toward zero in the format given, the gain and its product in the
    // computing precision, the delivery, the flush), never from `allowance`.
    ["binary64", 8.011_169_034_021_527_33e-9, 2.011_168_978_510_375_72e-9],
    ["binary64-as-binary32", 1.123_192_972_799_328_15e-7, 4.671_465_256_005_334_74e-8],
    ["binary32", 7.980_055_424_797_564_03e-7, 6.876_974_142_616_006_6e-7],
  ])("counts each term of %s", (format, below, above) => {
    const allowed = allowance(within, -2, 1e-9, 3e-9, 4000, format);
    expect(allowed.below / below).toBeCloseTo(1, 12);
    expect(allowed.above / above).toBeCloseTo(1, 12);
    // Silence through no limiter: the flush alone.
    const flushed = format === "binary64" ? 2 ** -1022 : 2 ** -126;
    expect(allowance({ lowest: 0, highest: 0 }, 0, 0, 0, 4000, format)).toStrictEqual({ below: flushed, above: flushed });
  });

  test("adds the engine's margin to every need: 64 units of 2^-53 per frame of the case's lookahead, its maximum too", () => {
    // Written apart: 1.5 ms at 48 kHz is 72 frames, 5 ms at 384 kHz 1,920, 0.5 ms at 8 kHz 4;
    // 64 × 2^-53 is 2^-47.
    expect(needMargin(OUTPUT, 48_000)).toBe(72 * 2 ** -47);
    expect(needMargin({ ...OUTPUT, lookahead: 5 }, 384_000)).toBe(1920 * 2 ** -47);
    expect(needMargin({ ...OUTPUT, lookahead: 0.5 }, 8000)).toBe(4 * 2 ** -47);
    // A lookahead of a fraction of a frame is one frame, as the engine counts it.
    expect(needMargin({ ...OUTPUT, lookahead: 0.5 }, 1000)).toBe(2 ** -47);
  });

  test("counts the roundings of a term of the engine's estimate in the reduction order", () => {
    // Decision 28: 16 lanes and a tree of 4 levels; a term's product, the additions of its
    // lane and the tree. Written apart: 12 taps, 1 + 0 + 4; 32, 1 + 1 + 4; 128, 1 + 7 + 4.
    expect([LANES, TREE_LEVELS]).toStrictEqual([16, 4]);
    expect([reductionRoundings(12), reductionRoundings(16), reductionRoundings(17), reductionRoundings(32), reductionRoundings(128)]).toStrictEqual([5, 5, 6, 6, 12]);
  });

  test("measures how far a phase's coefficients can move an estimate", () => {
    const spreading = BS1770.reduce((sum, row) => sum + Math.abs(row[0]), 0);
    expect(spread(INTERPOLATORS.standard)).toBeCloseTo(2.0228, 4);
    expect(spread(INTERPOLATORS.standard)).toBeGreaterThanOrEqual(spreading);
    expect(spread({ taps: 2, phases: [[0.25, 0.25]] })).toBe(1);
  });

  test.each<[string, number, number, number, readonly [number, number, number]]>([
    // Computed apart (Python, exact powers of two and 80-digit decimals, each interpolator
    // from its definition, from the list of the engine's rounding steps per format: the
    // estimate in the computing precision in decision 28's order, a windowed interpolator's
    // coefficients rounded to binary32 in a binary32 render, ρ = (43L + 3)u, the slow
    // stage), 4,000 frames: binary64, binary64 given as binary32, binary32.
    ["standard", 48_000, 1, 72, [1.416_678_419_877_369_14e-11, 1.416_678_419_877_369_14e-11, 1.488_376_489_591_235_1e-6]],
    ["precise", 44_100, 2, 66, [3.511_168_609_423_195_61e-11, 3.511_168_609_423_195_61e-11, 4.587_707_644_348_575_57e-6]],
    ["mastering", 96_000, 1, 144, [2.856_824_187_466_280_61e-11, 2.856_824_187_466_280_61e-11, 4.291_874_934_133_069_16e-6]],
  ])("pins the need's error through the %s detector at %i Hz per format", (detector, rate, amplitude, lookahead, expected) => {
    const interpolator = detector === "standard" ? INTERPOLATORS.standard : detector === "precise" ? INTERPOLATORS.precise : INTERPOLATORS.mastering;
    RENDER_FORMATS.forEach((format, index) => {
      expect(controlError(interpolator, amplitude, 4000, format, lookahead, rate) / (expected[index] ?? 0)).toBeCloseTo(1, 12);
    });
  });
});

describe("the release from the onset", () => {
  test("keeps each stage's share a frame, moved by the engine's rounding toward its slowest return", () => {
    expect(fastRelease()).toBe(55);
    expect(keep(100, 48_000)).toBe(Math.exp(-1 / 4800));
    expect(fastKeep(48_000)).toBe(Math.exp(-1 / 2640) + 4 * 2 ** -53);
    expect(slowKeep(48_000)).toBe(Math.exp(-1 / 4800) - 4 * 2 ** -53);
    expect(RELEASE_MS).toBe(100);
  });

  test("needs no release when the onset needs no more than the steady sine", () => {
    expect(released(0.5, 0.5, 0.1, 48_000, 84)).toBe(0);
    expect(released(0, 0, 0.1, 48_000, 84)).toBe(0);
  });

  test("releases the fast stage to the steady need, with the error on both", () => {
    expect(released(1, 0.5, 0.25, 48_000, 84)).toBe(Math.ceil(Math.log(1.25 / 0.75) / -Math.log(fastKeep(48_000))));
  });

  test("refuses an onset the steady sine does not need, however small", () => {
    // The slow stage rises far below the error: only the steady sine's zero refuses it.
    expect(released(1e-3, 0, 1, 48_000, 84)).toBeNull();
  });

  test("refuses a slow stage above the steady need and the error, and only above", () => {
    const slow = (onset: number, error: number, settled: number): number => (onset + error) * (1 - slowKeep(48_000) ** settled);
    // Exactly at it, accepted.
    expect(released(1, slow(1, 0, 84), 0, 48_000, 84)).not.toBeNull();
    expect(released(1, slow(1, 0, 84) - 0.005, 0.01, 48_000, 84)).not.toBeNull();
    expect(released(1, slow(1, 0, 84) * (1 + 1e-12), 0, 48_000, 84)).not.toBeNull();
    expect(released(1, slow(1, 0, 84) * (1 - 1e-12), 0, 48_000, 84)).toBeNull();
    // The error raises the onset too: a long settling brings the slow stage to 1.5.
    expect(slow(1, 0.5, 100_000)).toBeCloseTo(1.5, 6);
    expect(released(1, 0.6, 0.5, 48_000, 100_000)).toBeNull();
  });
});

// Each plan scans the under-read and conditions a fit: slow under Stryker's instrumentation, given its time.
describe("the plan of a limited channel", { timeout: 60_000 }, () => {
  /** The plan of a sine at 48 kHz through the standard detector, 1.5 ms, over 4,000 frames. */
  const planOf = (frequency: number, amplitude: number, phase: number, frames = 4000, output: ProtectedOutput = OUTPUT, rate = 48_000): LimitedPlan | string =>
    limitedPlan(output, rate, frames, { frequency, amplitude, phase }, needMargin(output, rate));

  test.each<[string, number, number, number, number, number, number, number, number, ProtectedOutput]>([
    // Computed apart (Python, its own interpolators and scan): start, onset need, steady need, the range.
    ["1 kHz, phase 0", 48_000, 1000, 1, 0, 154, 0, 0.109_653_633_210_157_67, 0.890_346_366_789_842_4, OUTPUT],
    ["1 kHz, phase π/2", 48_000, 1000, 1, Math.PI / 2, 1798, 0.204_369_065_907_539_48, 0.109_653_633_210_157_67, 0.890_346_366_789_842_4, OUTPUT],
    ["15 kHz, where G = 1", 48_000, 15_000, 1, 0, 1044, 0.152_305_458_726_026, 0.108_749_061_866_254_44, CEILING, OUTPUT],
    ["47 kHz, folded to 1 kHz", 48_000, 47_000, 1, 0, 154, 0, 0.109_653_633_210_157_67, 0.890_346_366_789_842_4, OUTPUT],
    // Its peak where the onset's last estimates read it: frames 0 to 10 only.
    ["1 kHz, phase π/4", 48_000, 1000, 1, Math.PI / 4, 154, 0.101_058_492_654_074_18, 0.109_653_633_210_157_67, 0.890_346_366_789_842_4, OUTPUT],
    // Near Nyquist, where the interval's ends decide: its peak on sample 5, the later end of frame 10.
    ["0.49 of the rate", 48_000, 23_520, 1, 5.026_548_245_743_669, 2035, 0.221_691_317_975_142_13, 0.108_749_061_866_254_44, CEILING, OUTPUT],
    ["997 Hz at 44.1 kHz, precise", 44_100, 997, 2, 1, 197, 0.566_452_396_692_490_3, 0.558_358_157_267_051_3, 0.883_283_685_465_897_4, { ...OUTPUT, detector: "precise" }],
    ["30,011 Hz at 96 kHz, mastering", 96_000, 30_011, 1, 0, 2890, 0.173_792_773_894_501_9, 0.108_749_061_866_254_44, CEILING, { ...OUTPUT, detector: "mastering" }],
  ])("derives when %s has settled", (_case, rate, frequency, amplitude, phase, start, onset, steady, lowest, output) => {
    const plan = planOf(frequency, amplitude, phase, 4000, output, rate);
    expect(typeof plan === "string" ? plan : { start: plan.start, frequency: plan.frequency }).toStrictEqual({ start, frequency: frequency === 47_000 ? 1000 : frequency });
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

  test("accepts a period exactly as long as the window", () => {
    // 1.625 ms at 48 kHz: 78 frames of lookahead, a window of 80, a period of 80 at 600 Hz.
    const plan = planOf(600, 1, 0, 4000, { ...OUTPUT, lookahead: 1.625 });
    expect(typeof plan === "string" ? plan : plan.latency).toBe(84);
  });

  test("lowers the amplitude's range by the engine's margin on the need, and settles with it", () => {
    const sine = { frequency: 1000, amplitude: 1, phase: Math.PI / 2 };
    const without = limitedPlan(OUTPUT, 48_000, 4000, sine, 0);
    const margined = limitedPlan(OUTPUT, 48_000, 4000, sine, 0.01);
    if (typeof without === "string" || typeof margined === "string") {
      throw new Error("refused");
    }
    expect(margined.allowances.binary32.below - without.allowances.binary32.below).toBeCloseTo(0.01, 12);
    expect(margined.allowances.binary32.above).toBe(without.allowances.binary32.above);
    // (onset + ε) / (steady + ε) is smaller: the fast stage reaches it sooner.
    expect(margined.start).toBeLessThan(without.start);
  });

  test("settles the slow stage below the steady need just short of refusing it", () => {
    // A little louder than the refused 0.8906: the slow stage's peak falls under the steady need.
    const plan = planOf(1000, 0.8925, Math.PI / 2, 20_000);
    // Computed apart (Python, its own estimates of the onset, from the smallest error).
    expect(typeof plan === "string" ? plan : plan.start).toBe(10_203);
  });

  test("starts where the limiter has settled for the smallest error, which holds for every format's", () => {
    // Near the slow stage's edge at 15 kHz, the start each format's own error derives: the
    // fast stage released to within the smallest error is released to within every larger
    // one, so the plan's start is the latest of them, binary64's. Sweeps the amplitudes
    // where the start moves with the error.
    const margin = needMargin(OUTPUT, 48_000);
    const starts = new Set<number>();
    for (let step = 0; step <= 90; step++) {
      const amplitude = 0.8921 + step * 1e-5;
      const plan = planOf(15_000, amplitude, 0, 20_000);
      if (typeof plan === "string") {
        throw new Error(plan);
      }
      const own = RENDER_FORMATS.map((format) => {
        const decayed = released(plan.onset, plan.steady, controlError(INTERPOLATORS.standard, amplitude, 20_000, format, 72, 48_000) + margin, 48_000, 84) ?? Number.NaN;
        return Math.max(plan.latency, 83 + decayed + 71);
      });
      expect(plan.start, String(amplitude)).toBe(Math.max(...own));
      starts.add(plan.start);
    }
    // The sweep crosses starts that differ: the start follows the amplitude.
    expect(starts.size).toBeGreaterThan(1);
  });

  test("pins the allowance of each format of the 15 kHz case", () => {
    // Computed apart (Python, its own scan of the under-read, R = 0.98599515926 and G = 1,
    // and the list of rounding steps), 3,092 frames: below, then above.
    const plan = planOf(15_000, 1, 0, 1044 + FRAMES);
    if (typeof plan === "string") {
      throw new Error(plan);
    }
    for (const [format, below, above] of [
      ["binary64", 1.612_159_479_205_930_98e-11, 1.560_971_279_789_534_61e-11],
      ["binary64-as-binary32", 1.601_387_502_498_968_7e-7, 5.389_284_741_940_907_1e-8],
      ["binary32", 2.054_395_187_312_361_81e-6, 1.894_272_046_875_616_97e-6],
    ] as const) {
      expect(plan.allowances[format].below / below, format).toBeCloseTo(1, 12);
      expect(plan.allowances[format].above / above, format).toBeCloseTo(1, 12);
    }
  });
});

// Each test renders through the model and measures: slow under Stryker's instrumentation, given its time.
describe("judgeLimited", { timeout: 60_000 }, () => {
  /** The exact render of `sine` at `rate` through the limiter model with `output`'s detector, `frames` long. */
  const rendered = (sine: Sine, frames: number, output: ProtectedOutput = OUTPUT, rate = 48_000): number[] => limit((frame) => ideal(sine, rate, frame), frames, rate, output.lookahead, INTERPOLATORS[output.detector].phases);
  /** The plan of `sine` at `rate` through `output`, `frames` long, which must be accepted. */
  const planned = (sine: Sine, frames: number, output: ProtectedOutput = OUTPUT, rate = 48_000): LimitedPlan => {
    const plan = limitedPlan(output, rate, frames, sine, needMargin(output, rate));
    if (typeof plan === "string") {
      throw new Error(plan);
    }
    return plan;
  };

  test.each<[string, Sine, ProtectedOutput, number, number]>([
    ["a 1 kHz sine", { frequency: 1000, amplitude: 1, phase: 0 }, OUTPUT, 48_000, FRAMES],
    ["a 1 kHz sine from its peak", { frequency: 1000, amplitude: 1, phase: Math.PI / 2 }, OUTPUT, 48_000, 1798 + FRAMES],
    ["a 15 kHz sine, where G = 1", { frequency: 15_000, amplitude: 1, phase: 0 }, OUTPUT, 48_000, 1044 + FRAMES],
    ["a 47 kHz sine, folded", { frequency: 47_000, amplitude: 1, phase: 0 }, OUTPUT, 48_000, FRAMES],
    ["a sine below the ceiling", { frequency: 1000, amplitude: 0.5, phase: 0 }, OUTPUT, 48_000, FRAMES],
    ["a precise render at 44.1 kHz", { frequency: 997, amplitude: 2, phase: 1 }, { ...OUTPUT, detector: "precise" }, 44_100, FRAMES],
    ["a mastering render at 96 kHz", { frequency: 30_011, amplitude: 1, phase: 0 }, { ...OUTPUT, detector: "mastering" }, 96_000, 2890 + FRAMES],
  ])("accepts the model's render of %s, in every format, its binary32 estimate read high too", (_case, sine, output, rate, frames) => {
    const plan = planned(sine, frames, output, rate);
    for (const format of RENDER_FORMATS) {
      const samples = limit((frame) => ideal(sine, rate, frame), frames, rate, output.lookahead, INTERPOLATORS[output.detector].phases, format);
      expect(judgeLimited(samples, plan, format), format).not.toHaveProperty("problems");
    }
    // Every product and sum of the binary32 estimate rounded toward its sign: the worst a
    // rounding can read it high.
    const outward = limit((frame) => ideal(sine, rate, frame), frames, rate, output.lookahead, INTERPOLATORS[output.detector].phases, "binary32", "outward");
    expect(judgeLimited(outward, plan, "binary32")).not.toHaveProperty("problems");
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
    // At 15 kHz the steady output lies on the range's lowest end, c: a render below it by
    // 0.9 of its allowance as the fit carries it, plus the fit's floor, passes; by 1.1 of
    // it, it fails.
    const sine = { frequency: 15_000, amplitude: 1, phase: 0 };
    const plan = planned(sine, 1044 + FRAMES);
    const samples = rendered(sine, 1044 + FRAMES);
    const levels = measure(Float64Array.from(samples.slice(plan.start, plan.start + MAX_FRAMES)), 48_000, 15_000);
    const carried = Math.sqrt(2 * (1 + levels.sine.conditioning)) / (1 - levels.sine.conditioning);
    const reach = carried * plan.allowances.binary32.below + levels.sine.floor;
    // The allowance is the derived one, about 2.05e-6 of the ceiling (pinned above).
    expect(reach).toBeGreaterThan(2e-6);
    const below = (share: number): number[] => {
      const scale = (plan.within.lowest - share * reach) / levels.fundamental;
      return samples.map((sample) => Math.fround(sample * scale));
    };
    expect(judgeLimited(below(0.9), plan, "binary32")).not.toHaveProperty("problems");
    expect(judgeLimited(below(1.1), plan, "binary32")).toMatchObject({ problems: [expect.stringMatching(/^fundamental /)] });
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
