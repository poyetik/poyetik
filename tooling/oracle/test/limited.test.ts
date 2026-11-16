import { describe, expect, test } from "vitest";
import { BS1770, CEILING_DBTP, INTERPOLATORS, SCAN, besselI0, delayOf, framesOf, holdLimited, judgeLimited, kaiser, latency, limitedError, range, rounding, underRead } from "../src/limited.ts";
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
    expect(range(omega, INTERPOLATORS.standard).lowest).toBeCloseTo(CEILING / 1.0010159769025224, 9);
    expect(range(omega, INTERPOLATORS.standard).highest).toBeCloseTo(CEILING / 0.9996831277349645, 9);
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
  const { lowest, highest } = range((2 * Math.PI * 1000) / 48_000, INTERPOLATORS.standard);

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
    const within = range((2 * Math.PI * 600) / 48_000, INTERPOLATORS.standard);
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

  test("judges every detector from its own interpolator and delay", () => {
    for (const [detector, silence] of [["precise", 88], ["mastering", 136]] as const) {
      const within = range((2 * Math.PI * 1000) / 48_000, INTERPOLATORS[detector]);
      const middle = (within.lowest + within.highest) / 2;
      const output = { ...OUTPUT, detector };
      expect(judgeLimited(render(FRAMES, middle, silence), 48_000, sine, output, "binary64")).not.toHaveProperty("problems");
      // One frame more of latency than the detector's: a sample during it.
      expect(judgeLimited(render(FRAMES, middle, silence - 1), 48_000, sine, output, "binary64")).toStrictEqual({ problems: [`not silent over its latency, ${String(silence)} frames`] });
    }
  });

  test("judges a sine whose period fits the window only", () => {
    expect(judgeLimited(render(FRAMES, 0.89), 48_000, { ...sine, frequency: 500 }, OUTPUT, "binary64")).toStrictEqual({
      problems: ["a period of the sine (96 frames) does not fit the limiter's window (74 frames)"],
    });
  });
});
