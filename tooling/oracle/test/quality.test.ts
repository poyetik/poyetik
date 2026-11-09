import { describe as group, expect, test } from "vitest";
import { MeasureError, type Bounds, type Measurements } from "../src/measure.ts";
import { describe, fold, hold, judgeQuality, worst, type Levels } from "../src/quality.ts";
import { sineSamples } from "./fixtures.ts";

/** A 1 kHz full-scale sine. */
const SINE = { frequency: 1000, amplitude: 1, phase: 0 };

group("fold", () => {
  test.each([
    [1000, 1000],
    [4000, 4000],
    [5000, 3000],
    [10_001, 2001],
    [19_984, 3984],
    [8000, 0],
  ])("folds %d Hz to %d Hz at 8 kHz", (frequency, folded) => {
    expect(fold(frequency, 8000)).toBe(folded);
  });
});

group("judgeQuality", () => {
  test("holds a binary32 sine within its derived bounds", () => {
    const quality = judgeQuality(sineSamples(1, 8000, 8000, 1000.5), 8000, { ...SINE, frequency: 1000.5 });
    if (quality.kind !== "measured") {
      throw new Error(JSON.stringify(quality));
    }
    expect(quality.levels.thdPlusN).toBeLessThan(-147);
    expect(quality.levels.sfdr).toBeGreaterThan(141);
    expect(typeof quality.levels.harmonics === "string" ? null : quality.levels.harmonics.snr).toBeGreaterThan(147);
  });

  test("measures a sine above Nyquist at the frequency its samples hold", () => {
    const quality = judgeQuality(sineSamples(1, 8000, 8000, 10_001), 8000, { ...SINE, frequency: 10_001 });
    expect(quality.kind).toBe("measured");
  });

  test.each([
    ["DC", { ...SINE, frequency: 0 }],
    ["Nyquist", { ...SINE, frequency: 4000 }],
    ["the rate", { ...SINE, frequency: 8000 }],
    ["silence", { ...SINE, amplitude: 0 }],
  ])("measures nothing at %s", (_name, sine) => {
    expect(judgeQuality(new Array<number>(64).fill(0), 8000, sine)).toStrictEqual({ kind: "unmeasured" });
  });

  test.each([
    // 2^-126 Hz: over 64 frames at 8 kHz, the ideal sine stays below 1e-39, within its
    // error bound (about 2^-25) of silence, where no level can tell it from silence.
    ["2^-126 Hz", 2 ** -126, "unmeasured"],
    // Its peak about half its error bound over 64 frames, then twice it: measured, and too
    // close to DC to be told apart.
    ["half its bound", (0.5 * 2 ** -25 * 8000) / (2 * Math.PI * 63), "unmeasured"],
    ["twice its bound", (2 * 2 ** -25 * 8000) / (2 * Math.PI * 63), "failed"],
  ])("measures nothing of a sine that stays within its error bound of silence: %s", (_name, frequency, kind) => {
    expect(judgeQuality(new Array<number>(64).fill(0), 8000, { ...SINE, frequency }).kind).toBe(kind);
  });

  test("reports what cannot be measured", () => {
    expect(judgeQuality(new Array<number>(64).fill(0), 8000, SINE)).toStrictEqual({
      kind: "failed",
      problems: ["cannot be measured: the fundamental, of amplitude 0, is not above the floor of the measurement"],
    });
  });

  test("reports the levels out of their bounds", () => {
    // A second harmonic at -100 dB: far above the THD bound of a binary32 sine.
    const samples = sineSamples(1, 8000, 8000, 1000).map((value, frame) => value + 1e-5 * Math.sin((2 * Math.PI * 2000 * frame) / 8000));
    const quality = judgeQuality(samples, 8000, SINE);
    expect(quality).toStrictEqual({
      kind: "failed",
      problems: [
        expect.stringMatching(/^THD\+N -100\.0 dB, above its bound -14\d\.\d dB$/),
        expect.stringMatching(/^SFDR 100\.0 dB, below its bound 14\d\.\d dB$/),
        expect.stringMatching(/^THD -100\.0 dB, above its bound -14\d\.\d dB$/),
      ],
    });
  });

  test("measures at most the frames the measurement takes", () => {
    const samples = sineSamples(1, 2 ** 17 + 5, 8000, 1000.5);
    expect(judgeQuality(samples, 8000, { ...SINE, frequency: 1000.5 }).kind).toBe("measured");
  }, 60_000);

  test("lets other errors through", () => {
    /** Samples whose copy fails, as a bug would. */
    class Hostile extends Array<number> {
      /** Throws. */
      override slice(): number[] {
        throw new RangeError("boom");
      }
    }
    expect(() => judgeQuality(Hostile.from([1, 2]), 8000, SINE)).toThrow(new RangeError("boom"));
  });
});

/** Measured levels, all at -150 dB or 150 dB. */
const MEASURED: Measurements = {
  fundamental: 1,
  thdPlusN: -150,
  sfdr: 150,
  sine: { conditioning: 0, floor: 0 },
  enbw: 2,
  harmonics: { thd: -150, aliasing: -150, snr: 150, quality: { conditioning: 0, floor: 0 } },
};

/** Bounds all at -140 dB or 140 dB. */
const BOUNDS: Bounds = { thdPlusN: -140, sfdr: 140, harmonics: { thd: -140, snr: 140 } };

group("hold", () => {
  test("passes levels within their bounds, at them included", () => {
    expect(hold(MEASURED, BOUNDS)).toStrictEqual({ kind: "measured", levels: { thdPlusN: -150, sfdr: 150, harmonics: { thd: -150, aliasing: -150, snr: 150 } } });
    const atBounds = { ...MEASURED, thdPlusN: -140, sfdr: 140, harmonics: { thd: -140, aliasing: -140, snr: 140, quality: { conditioning: 0, floor: 0 } } };
    expect(hold(atBounds, BOUNDS).kind).toBe("measured");
  });

  test("reports every level out of its bound", () => {
    const out = { ...MEASURED, thdPlusN: -130, sfdr: 130, harmonics: { thd: -120, aliasing: -110, snr: 135, quality: { conditioning: 0, floor: 0 } } };
    expect(hold(out, BOUNDS)).toStrictEqual({
      kind: "failed",
      problems: [
        "THD+N -130.0 dB, above its bound -140.0 dB",
        "SFDR 130.0 dB, below its bound 140.0 dB",
        "SNR 135.0 dB, below its bound 140.0 dB",
        "THD -120.0 dB, above its bound -140.0 dB",
        "aliasing -110.0 dB, above its bound -140.0 dB",
      ],
    });
    expect(hold({ ...MEASURED, thdPlusN: Number.NaN }, BOUNDS)).toStrictEqual({ kind: "failed", problems: ["THD+N NaN dB, above its bound -140.0 dB"] });
    expect(hold({ ...MEASURED, sfdr: Number.NaN }, BOUNDS)).toStrictEqual({ kind: "failed", problems: ["SFDR NaN dB, below its bound 140.0 dB"] });
  });

  test("checks no THD or aliasing that does not exist", () => {
    const none = { ...MEASURED, harmonics: { thd: null, aliasing: null, snr: 150, quality: { conditioning: 0, floor: 0 } } };
    expect(hold(none, { ...BOUNDS, harmonics: { thd: -200, snr: 140 } })).toStrictEqual({ kind: "measured", levels: { thdPlusN: -150, sfdr: 150, harmonics: { thd: null, aliasing: null, snr: 150 } } });
  });

  test("passes a refused harmonic fit, saying so", () => {
    const refused = { ...MEASURED, harmonics: new MeasureError("Unresolvable", "too close") };
    expect(hold(refused, { ...BOUNDS, harmonics: null })).toStrictEqual({ kind: "measured", levels: { thdPlusN: -150, sfdr: 150, harmonics: "too close" } });
  });

  test("fails without a margin", () => {
    expect(hold(MEASURED, null)).toStrictEqual({ kind: "failed", problems: ["its error bound leaves the fundamental no margin to be measured"] });
    expect(hold(MEASURED, { ...BOUNDS, harmonics: null })).toStrictEqual({ kind: "failed", problems: ["its error bound leaves the harmonic fit no margin to be measured"] });
  });
});

group("describe and worst", () => {
  /** Levels of one channel. */
  const levels = (thd: number | null, aliasing: number | null, snr: number, thdPlusN = -150, sfdr = 150): Levels => ({ thdPlusN, sfdr, harmonics: { thd, aliasing, snr } });

  test("describes the levels in one line", () => {
    expect(describe(levels(-160.04, null, 150.06))).toBe("THD+N -150.0 dB, SFDR 150.0 dB, THD -160.0 dB, aliasing none, SNR 150.1 dB");
    expect(describe(levels(null, -170, 150))).toBe("THD+N -150.0 dB, SFDR 150.0 dB, THD none, aliasing -170.0 dB, SNR 150.0 dB");
    expect(describe({ thdPlusN: -150, sfdr: 150, harmonics: "too close" })).toBe("THD+N -150.0 dB, SFDR 150.0 dB, harmonics not measured: too close");
  });

  test("keeps the worst of every level", () => {
    expect(worst([])).toBeNull();
    expect(worst([levels(-160, null, 150)])).toStrictEqual(levels(-160, null, 150));
    expect(worst([levels(-160, null, 150, -151, 152), levels(-170, -180, 140, -149, 155), levels(null, -175, 145)])).toStrictEqual(levels(-160, -175, 140, -149, 150));
    const refused: Levels = { thdPlusN: -140, sfdr: 140, harmonics: "too close" };
    expect(worst([levels(-160, null, 150), refused])).toStrictEqual({ thdPlusN: -140, sfdr: 140, harmonics: "too close" });
    expect(worst([refused, levels(-160, null, 150)])).toStrictEqual({ thdPlusN: -140, sfdr: 140, harmonics: "too close" });
  });
});
