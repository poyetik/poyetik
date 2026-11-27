import { describe as group, expect, test } from "vitest";
import { MeasureError, hasMargin, measure, sineFitConditioning, type Bounds, type Measurements } from "../src/measure.ts";
import { describe, fold, hold, judgeQuality, unmeasurable, worst, type Levels } from "../src/quality.ts";
import { ideal, sineError, sineErrorBound } from "../src/signal.ts";
import { sineSamples, sineSamples64 } from "./fixtures.ts";

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
    const quality = judgeQuality(sineSamples(1, 8000, 8000, 1000.5), 8000, { ...SINE, frequency: 1000.5 }, "binary64-as-binary32");
    if (quality.kind !== "measured") {
      throw new Error(JSON.stringify(quality));
    }
    expect(quality.levels.thdPlusN).toBeLessThan(-147);
    expect(quality.levels.sfdr).toBeGreaterThan(141);
    expect(typeof quality.levels.harmonics === "string" ? null : quality.levels.harmonics.snr).toBeGreaterThan(147);
  });

  test("holds each render format to its own bound", () => {
    // A binary64 sine passes its binary64 bound; the same sine rounded to binary32 fails it,
    // and passes the bounds of both binary32 formats; a sine whose angle was rounded to
    // binary32 passes the binary32 bound only.
    const wide = sineSamples64(1, 8000, 8000, 1000.5);
    const sine = { ...SINE, frequency: 1000.5 };
    expect(judgeQuality(wide, 8000, sine, "binary64").kind).toBe("measured");
    const narrow = sineSamples(1, 8000, 8000, 1000.5);
    expect(judgeQuality(narrow, 8000, sine, "binary64").kind).toBe("failed");
    expect(judgeQuality(narrow, 8000, sine, "binary64-as-binary32").kind).toBe("measured");
    expect(judgeQuality(narrow, 8000, sine, "binary32").kind).toBe("measured");
    const angle = Array.from({ length: 8000 }, (_empty, frame) => Math.fround(Math.sin(Math.fround((2 * Math.PI * ((1000.5 * frame) % 8000)) / 8000))));
    expect(judgeQuality(angle, 8000, sine, "binary64-as-binary32").kind).toBe("failed");
    expect(judgeQuality(angle, 8000, sine, "binary32").kind).toBe("measured");
    // Five measurements of 8000 frames: about 0.2 s in a plain run, less than a 20-fold
    // margin under Vitest's 5 s default once Stryker instruments the measurements.
  }, 60_000);

  test("measures a sine above Nyquist at the frequency its samples hold", () => {
    const quality = judgeQuality(sineSamples(1, 8000, 8000, 10_001), 8000, { ...SINE, frequency: 10_001 }, "binary64-as-binary32");
    expect(quality.kind).toBe("measured");
  });

  test.each([
    ["DC", { ...SINE, frequency: 0 }],
    ["Nyquist", { ...SINE, frequency: 4000 }],
    ["the rate", { ...SINE, frequency: 8000 }],
    ["silence", { ...SINE, amplitude: 0 }],
    // At a phase of π/2 they reach full scale, far above their error bound: measured nothing
    // for their frequency alone.
    ["DC, at full scale", { ...SINE, frequency: 0, phase: Math.PI / 2 }],
    ["Nyquist, at full scale", { ...SINE, frequency: 4000, phase: Math.PI / 2 }],
    ["the rate, at full scale", { ...SINE, frequency: 8000, phase: Math.PI / 2 }],
  ])("measures nothing at %s", (_name, sine) => {
    expect(judgeQuality(new Array<number>(64).fill(0), 8000, sine, "binary64-as-binary32")).toStrictEqual({ kind: "unmeasured" });
  });

  test.each([
    // 2^-126 Hz: over 64 frames at 8 kHz, the ideal sine stays below 1e-39, within its
    // error bound (about 2^-25) of silence, where no level can tell it from silence.
    ["2^-126 Hz", 2 ** -126, "unmeasured"],
    // Its peak about half its error bound over 64 frames, then twice it: measured, and too
    // close to DC to be told apart.
    ["half its bound", (0.5 * 2 ** -25 * 8000) / (2 * Math.PI * 63), "unmeasured"],
    // Its peak at its last frame (63) just below its bound, and at frame 64, which is not
    // measured, just above it.
    ["one frame past its last", (2 ** -25 * 8000) / (2 * Math.PI * 63.5), "unmeasured"],
    ["twice its bound", (2 * 2 ** -25 * 8000) / (2 * Math.PI * 63), "failed"],
  ])("measures nothing of a sine that stays within its error bound of silence: %s", (_name, frequency, kind) => {
    expect(judgeQuality(new Array<number>(64).fill(0), 8000, { ...SINE, frequency }, "binary64-as-binary32").kind).toBe(kind);
  });

  group("a binary32 amplitude just above 2^-126, where the error bound nears the amplitude", () => {
    /** The binary32 value of the bits `bits`. */
    const binary32 = (bits: number): number => new Float32Array(new Uint32Array([bits]).buffer)[0] ?? Number.NaN;
    /** The exact samples of a 1 kHz sine of `amplitude` over 4800 frames at 48 kHz: 100 whole cycles. */
    const exact = (amplitude: number): number[] => Array.from({ length: 4800 }, (_frame, frame) => amplitude * Math.sin((2 * Math.PI * 1000 * frame) / 48_000));
    /** The quality of the exact samples of `amplitude`, judged as a binary32 render. */
    const judged = (amplitude: number): string => judgeQuality(exact(amplitude), 48_000, { ...SINE, amplitude }, "binary32").kind;
    // Where its fundamental has a margin, computed apart from the oracle: the bound is
    // 2^-126 + a·k (k = 2^-22 + 2^-25 + 2^-33 + 2π·4800·2^-52 + 2^-48), the fit's basis is
    // orthogonal over whole cycles, so a > √2 · bound: a > 1.41421410 · 2^-126, between the
    // binary32 values 0x00b504f7 and 0x00b504f8.
    test.each([
      ["1.25 · 2^-126", 0x00a0_0000, "unmeasured"],
      ["the last value without a margin", 0x00b5_04f7, "unmeasured"],
      ["the first value with one", 0x00b5_04f8, "measured"],
    ])("%s (bits %i) is %s, never failed for every render", (_name, bits, kind) => {
      expect(judged(binary32(bits))).toBe(kind);
      expect(unmeasurable({ ...SINE, amplitude: binary32(bits) }, 48_000, 4800)).toBeNull();
    });

    test("measures nothing of a sine whose peak equals its error bound", () => {
      // A start phase of the bound itself: sin(x) is x in binary64 for so small an x, so the
      // one frame measured peaks at exactly the bound, which it does not rise above.
      const bound = sineErrorBound(1, 1, "binary64");
      expect(Math.sin(bound)).toBe(bound);
      expect(judgeQuality([bound], 48_000, { ...SINE, phase: bound }, "binary64")).toStrictEqual({ kind: "unmeasured" });
    });

    test("still holds a wrong render to its distance from the sine", () => {
      const amplitude = binary32(0x00b5_04f7);
      expect(judgeQuality(new Array<number>(4800).fill(0), 48_000, { ...SINE, amplitude }, "binary32")).toStrictEqual({ kind: "unmeasured" });
      expect(sineError(new Array<number>(4800).fill(0), 48_000, { ...SINE, amplitude })).toBeGreaterThan(sineErrorBound(amplitude, 4800, "binary32"));
    });
  });

  group("a sine whose harmonic fit has no margin while its sine fit has", () => {
    /** The binary32 value of the bits `bits`. */
    const binary32 = (bits: number): number => new Float32Array(new Uint32Array([bits]).buffer)[0] ?? Number.NaN;
    /** `value` rounded to binary32, then flushed to a zero of its sign below 2^-126 in magnitude, as the engine gives a binary32 sample. */
    const sample = (value: number): number => {
      const rounded = Math.fround(value);
      return Math.abs(rounded) < 2 ** -126 ? rounded * 0 : rounded;
    };
    /** The quality of the exact render of `sine` over `frames` frames at 48 kHz, in binary32. */
    const exactRender = (sine: { readonly frequency: number; readonly amplitude: number; readonly phase: number }, frames: number) =>
      judgeQuality(
        Array.from({ length: frames }, (_frame, frame) => sample(ideal(sine, 48_000, frame))),
        48_000,
        sine,
        "binary32",
      );
    /** What the harmonic levels say when they are not measured for want of a margin. */
    const unmargined = "its error bound leaves the harmonic fit no margin";

    // 3001.5 Hz over 4800 frames is not a whole number of cycles: the harmonic fit is worse
    // conditioned than the sine fit, so their margins differ (each a fact of the expectation).
    test.each([
      ["3001.5 Hz, 4800 frames, 1.5 · 2^-126", 3001.5, 4800, 0x00c0_0000],
      ["46.44 Hz in binary32, 1024 frames, 1.6 · 2^-126", Math.fround(46.44), 1024, 0x00cc_cccd],
    ])("%s: measured, its harmonic levels not", (_name, frequency, frames, bits) => {
      const quality = exactRender({ frequency, amplitude: binary32(bits), phase: 0 }, frames);
      expect(quality.kind).toBe("measured");
      expect(quality.kind === "measured" ? quality.levels.harmonics : null).toBe(unmargined);
    });

    test("measures the harmonic levels with both margins", () => {
      const quality = exactRender({ frequency: 3001.5, amplitude: binary32(0x0180_0000), phase: 0 }, 4800);
      expect(quality.kind === "measured" ? typeof quality.levels.harmonics : quality.kind).toBe("object");
    });

    test("measures nothing without either margin", () => {
      expect(exactRender({ frequency: 3001.5, amplitude: binary32(0x0080_0000), phase: 0 }, 4800).kind).toBe("unmeasured");
    });

    /**
     * The first binary32 amplitude, by its bits, from 2^-126 to 8 · 2^-126 (bits 0x0080_0000
     * to 0x0200_0000), whose exact render over `frames` frames leaves a fit of `conditioning`
     * a margin ({@link hasMargin}, under the binary32 error bound): found by bisection, after
     * checking that the first amplitude has none and the last one has one. The bisection
     * finds the first one only if the margin grows with the amplitude in that range: assumed,
     * not checked here (a scan of every amplitude of the range is too slow for a unit test);
     * the regimes asserted at six more amplitudes below check it there.
     */
    const threshold = (conditioning: number, frames: number): number => {
      const margined = (bits: number): boolean => hasMargin(binary32(bits), sineErrorBound(binary32(bits), frames, "binary32"), conditioning);
      let [low, high] = [0x0080_0000, 0x0200_0000];
      expect([margined(low), margined(high)]).toStrictEqual([false, true]);
      while (high - low > 1) {
        const middle = Math.floor((low + high) / 2);
        if (margined(middle)) {
          high = middle;
        } else {
          low = middle;
        }
      }
      return high;
    };

    // The law the margins exist for: the exact render of an expectation the oracle accepts is
    // never failed, at frequencies that are not whole cycles. Each threshold, where a fit
    // first has a margin, is found from the code and pinned to its bits, measured once by a
    // bisection over the oracle's verdict: one binary32 step below it, at it and one above,
    // the render is judged in the regime of its side, never failed; and so are amplitudes
    // between and beyond the thresholds.
    test.each([
      ["3001.5 Hz, 2400 frames", 3001.5, 2400, 0x00b5_5af2, 0x0101_ffe6],
      ["46.44 Hz in binary32, 1024 frames", Math.fround(46.44), 1024, 0x00bb_7389, 0x00ff_e4bb],
      ["3001.5 Hz, 4800 frames", 3001.5, 4800, 0x00b5_5cff, 0x00ea_39c9],
    ])("%s: never fails the exact render of an accepted expectation, on each side of each threshold", (_name, frequency, frames, sineThreshold, harmonicThreshold) => {
      const at = (bits: number) => exactRender({ frequency, amplitude: binary32(bits), phase: 0 }, frames);
      // The harmonic fit's conditioning, a fact of the frequency and the frames, read from a
      // render with both margins.
      const sine = { frequency, amplitude: binary32(0x0200_0000), phase: 0 };
      const measured = measure(Float64Array.from(Array.from({ length: frames }, (_frame, frame) => sample(ideal(sine, 48_000, frame)))), 48_000, fold(frequency, 48_000));
      if (measured.harmonics instanceof MeasureError) {
        throw measured.harmonics;
      }
      const sineConditioning = sineFitConditioning(fold(frequency, 48_000), 48_000, frames);
      if (sineConditioning instanceof MeasureError) {
        throw sineConditioning;
      }
      expect([threshold(sineConditioning, frames), threshold(measured.harmonics.quality.conditioning, frames)]).toStrictEqual([sineThreshold, harmonicThreshold]);
      /** The regime of a judgement: unmeasured, measured without harmonic levels, or with them; else what failed. */
      const regime = (bits: number): string => {
        expect(unmeasurable({ frequency, amplitude: binary32(bits), phase: 0 }, 48_000, frames)).toBeNull();
        const quality = at(bits);
        if (quality.kind !== "measured") {
          return quality.kind === "failed" ? `failed: ${quality.problems.join("; ")}` : quality.kind;
        }
        return quality.levels.harmonics === unmargined ? "no harmonics" : typeof quality.levels.harmonics === "object" ? "harmonics" : quality.levels.harmonics;
      };
      expect([sineThreshold - 1, sineThreshold, sineThreshold + 1, harmonicThreshold - 1, harmonicThreshold, harmonicThreshold + 1].map(regime)).toStrictEqual([
        "unmeasured",
        "no harmonics",
        "no harmonics",
        "no harmonics",
        "harmonics",
        "harmonics",
      ]);
      // Between and beyond the thresholds: 1, 1.5, 2.5, 3, 4 and 8 · 2^-126, each in the regime
      // of its side of the thresholds pinned above (1 · 2^-126 below every sine threshold,
      // 1.5 · 2^-126 between the two of each case, the others above every harmonic one).
      expect([0x0080_0000, 0x00c0_0000, 0x0120_0000, 0x0140_0000, 0x0180_0000, 0x0200_0000].map(regime)).toStrictEqual([
        "unmeasured",
        "no harmonics",
        "harmonics",
        "harmonics",
        "harmonics",
        "harmonics",
      ]);
      // 12 judgements of up to 4800 frames: well under a second in a plain run, far more
      // under Stryker's instrumentation of the measurements.
    }, 60_000);
  });

  group("a binary32 subnormal amplitude, which the parser accepts", () => {
    /** The binary32 value of the bits `bits`. */
    const binary32 = (bits: number): number => new Float32Array(new Uint32Array([bits]).buffer)[0] ?? Number.NaN;
    /** The regime of the exact render of `sine` over 4800 frames at 48 kHz in `format`: binary64 as computed, or rounded to binary32, its subnormals flushed to zeros of their sign as delivery gives them. */
    const regime = (sine: { readonly frequency: number; readonly amplitude: number; readonly phase: number }, format: "binary64" | "binary64-as-binary32"): string => {
      expect(unmeasurable(sine, 48_000, 4800)).toBeNull();
      const samples = Array.from({ length: 4800 }, (_frame, frame) => {
        const value = ideal(sine, 48_000, frame);
        const rounded = Math.fround(value);
        return format === "binary64" ? value : Math.abs(rounded) < 2 ** -126 ? rounded * 0 : rounded;
      });
      const quality = judgeQuality(samples, 48_000, sine, format);
      if (quality.kind !== "measured") {
        return quality.kind === "failed" ? `failed: ${quality.problems.join("; ")}` : quality.kind;
      }
      return typeof quality.levels.harmonics === "object" ? "harmonics" : quality.levels.harmonics;
    };

    // The law at the parser's smallest amplitudes: the exact render of an accepted
    // expectation is never failed. In binary64 every one is measured with its harmonic
    // levels, derived apart from the oracle's verdict. A fit has a margin when the amplitude
    // a exceeds the error per sample (with the fit's floor, 2^-36 times about 3·a) times
    // √(2(1 + ρ))/(1 − ρ), ρ its conditioning: at most about 1.83 here, the factor of the
    // harmonic fit at 3001.5 Hz over 4800 frames (its threshold 0x00ea_39c9 above, 1.83 ·
    // 2^-126, under an error of 2^-126 and a little more), less at whole cycles. The error
    // is the bound of the format: in binary64, 2^(e - 54) + a·acc, below 2^-37·a, so both
    // fits have a margin in every case. Given as binary32, every sample is a binary32
    // subnormal, flushed to zero at delivery: the bound, 2^-126 + a·acc, exceeds a, and the
    // render is unmeasured, like a binary32 render of these amplitudes.
    test.each([0x0000_0001, 0x007f_ffff])("bits %i in binary64: measured with its harmonic levels, at whole cycles or not, at two phases", (bits) => {
      const amplitude = binary32(bits);
      expect(amplitude).toBeLessThan(2 ** -126);
      expect(sineErrorBound(amplitude, 4800, "binary64") / amplitude).toBeLessThan(2 ** -23);
      expect(
        [3001.5, 1000].flatMap((frequency) => [0, 1].map((phase) => regime({ frequency, amplitude, phase }, "binary64"))),
      ).toStrictEqual(["harmonics", "harmonics", "harmonics", "harmonics"]);
      // 4 judgements of 4800 frames: about 0.1 s in a plain run, over 5 s under Stryker's
      // instrumentation of the measurements.
    }, 60_000);

    test.each([0x0000_0001, 0x007f_ffff])("bits %i given as binary32: flushed at delivery, unmeasured, never failed", (bits) => {
      const amplitude = binary32(bits);
      expect(sineErrorBound(amplitude, 4800, "binary64-as-binary32")).toBeGreaterThan(amplitude);
      expect(
        [3001.5, 1000].flatMap((frequency) => [0, 1].map((phase) => regime({ frequency, amplitude, phase }, "binary64-as-binary32"))),
      ).toStrictEqual(["unmeasured", "unmeasured", "unmeasured", "unmeasured"]);
    }, 60_000);
  });

  test("reports what cannot be measured", () => {
    expect(judgeQuality(new Array<number>(64).fill(0), 8000, SINE, "binary64-as-binary32")).toStrictEqual({
      kind: "failed",
      problems: ["cannot be measured: the fundamental, of amplitude 0, is not above the floor of the measurement"],
    });
  });

  test("reports the levels out of their bounds", () => {
    // A second harmonic at -100 dB: far above the THD bound of a binary32 sine.
    const samples = sineSamples(1, 8000, 8000, 1000).map((value, frame) => value + 1e-5 * Math.sin((2 * Math.PI * 2000 * frame) / 8000));
    const quality = judgeQuality(samples, 8000, SINE, "binary64-as-binary32");
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
    expect(judgeQuality(samples, 8000, { ...SINE, frequency: 1000.5 }, "binary64-as-binary32").kind).toBe("measured");
  }, 60_000);

  test("lets other errors through", () => {
    /** Samples whose copy fails, as a bug would. */
    class Hostile extends Array<number> {
      /** Throws. */
      override slice(): number[] {
        throw new RangeError("boom");
      }
    }
    expect(() => judgeQuality(Hostile.from([1, 2]), 8000, SINE, "binary64-as-binary32")).toThrow(new RangeError("boom"));
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
    expect(hold(MEASURED, BOUNDS, true)).toStrictEqual({ kind: "measured", levels: { thdPlusN: -150, sfdr: 150, harmonics: { thd: -150, aliasing: -150, snr: 150 } } });
    const atBounds = { ...MEASURED, thdPlusN: -140, sfdr: 140, harmonics: { thd: -140, aliasing: -140, snr: 140, quality: { conditioning: 0, floor: 0 } } };
    expect(hold(atBounds, BOUNDS, true).kind).toBe("measured");
  });

  test("reports every level out of its bound", () => {
    const out = { ...MEASURED, thdPlusN: -130, sfdr: 130, harmonics: { thd: -120, aliasing: -110, snr: 135, quality: { conditioning: 0, floor: 0 } } };
    expect(hold(out, BOUNDS, true)).toStrictEqual({
      kind: "failed",
      problems: [
        "THD+N -130.0 dB, above its bound -140.0 dB",
        "SFDR 130.0 dB, below its bound 140.0 dB",
        "SNR 135.0 dB, below its bound 140.0 dB",
        "THD -120.0 dB, above its bound -140.0 dB",
        "aliasing -110.0 dB, above its bound -140.0 dB",
      ],
    });
    expect(hold({ ...MEASURED, thdPlusN: Number.NaN }, BOUNDS, true)).toStrictEqual({ kind: "failed", problems: ["THD+N NaN dB, above its bound -140.0 dB"] });
    expect(hold({ ...MEASURED, sfdr: Number.NaN }, BOUNDS, true)).toStrictEqual({ kind: "failed", problems: ["SFDR NaN dB, below its bound 140.0 dB"] });
  });

  test("checks no THD or aliasing that does not exist", () => {
    const none = { ...MEASURED, harmonics: { thd: null, aliasing: null, snr: 150, quality: { conditioning: 0, floor: 0 } } };
    expect(hold(none, { ...BOUNDS, harmonics: { thd: -200, snr: 140 } }, true)).toStrictEqual({ kind: "measured", levels: { thdPlusN: -150, sfdr: 150, harmonics: { thd: null, aliasing: null, snr: 150 } } });
  });

  test("passes a refused harmonic fit, saying so", () => {
    const refused = { ...MEASURED, harmonics: new MeasureError("Unresolvable", "too close") };
    expect(hold(refused, { ...BOUNDS, harmonics: null }, true)).toStrictEqual({ kind: "measured", levels: { thdPlusN: -150, sfdr: 150, harmonics: "too close" } });
  });

  test("passes harmonic levels without a margin by the expectation, saying so", () => {
    expect(hold(MEASURED, { ...BOUNDS, harmonics: null }, false)).toStrictEqual({
      kind: "measured",
      levels: { thdPlusN: -150, sfdr: 150, harmonics: "its error bound leaves the harmonic fit no margin" },
    });
    expect(hold({ ...MEASURED, thdPlusN: -130 }, BOUNDS, false)).toStrictEqual({ kind: "failed", problems: ["THD+N -130.0 dB, above its bound -140.0 dB"] });
  });

  test("fails without a margin", () => {
    expect(hold(MEASURED, null, true)).toStrictEqual({ kind: "failed", problems: ["its error bound leaves the fundamental no margin to be measured"] });
    expect(hold(MEASURED, { ...BOUNDS, harmonics: null }, true)).toStrictEqual({ kind: "failed", problems: ["its error bound leaves the harmonic fit no margin to be measured"] });
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
