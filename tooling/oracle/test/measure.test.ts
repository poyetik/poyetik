import { existsSync, readFileSync } from "node:fs";
import { describe, expect, test } from "vitest";
import {
  at,
  basis,
  compensatedSum,
  components,
  CONDITIONING_MAX,
  cycles,
  fft,
  FLOOR,
  hasMargin,
  HARMONICS,
  JACOBI_ITERATIONS,
  MAX_FRAMES,
  measure,
  MeasureError,
  PADDING,
  renderFloor,
  sineBounds,
  sineFitRefusal,
  solve,
  spectrum,
  split,
  unknowns,
  wholeTurns,
  WINDOW,
  window,
  type Measurements,
  type Quality,
} from "../src/measure.ts";
import { RENDER_FORMATS } from "../src/format.ts";
import { sineErrorBound } from "../src/signal.ts";
import { splitMix } from "./split-mix.ts";

/**
 * The known-answer vectors of the specification, found by walking up from this file: the
 * tests also run from a copy of the package (mutation testing sandbox), deeper in the
 * repository.
 */
function vectorsFile(): URL {
  for (let directory = new URL("./", import.meta.url); directory.pathname !== "/"; directory = new URL("../", directory)) {
    const candidate = new URL("spec/measurement-vectors.json", directory);
    if (existsSync(candidate)) {
      return candidate;
    }
  }
  throw new Error("spec/measurement-vectors.json not found");
}

/** The vectors, as parsed JSON. */
const VECTORS: unknown = JSON.parse(readFileSync(vectorsFile(), "utf8"));

/** Reads a field of a JSON object. */
function field(json: unknown, key: string): unknown {
  return typeof json === "object" && json !== null ? Reflect.get(json, key) : undefined;
}

/** Reads a number field. */
function num(json: unknown, key: string): number {
  const value = field(json, key);
  if (typeof value !== "number") {
    throw new TypeError(`${key} is not a number`);
  }
  return value;
}

/** Reads a list field. */
function list(json: unknown, key: string): unknown[] {
  const value = field(json, key);
  if (!Array.isArray(value)) {
    throw new TypeError(`${key} is not a list`);
  }
  return value;
}

/** Reads a text field. */
function text(json: unknown, key: string): string {
  const value = field(json, key);
  if (typeof value !== "string") {
    throw new TypeError(`${key} is not a string`);
  }
  return value;
}

/** The signal a vector describes: `Σ a·cos(2π·((f·n) mod rate)/rate + φ)`, plus `step · (u[n] - 1/2)` with noise. */
function synthesize(vector: unknown): Float64Array {
  const rate = num(vector, "rate");
  const tones = list(vector, "tones").map((tone) => ({ frequency: num(tone, "frequency"), amplitude: num(tone, "amplitude"), phase: num(tone, "phase") }));
  const noise = field(vector, "noise");
  const next = noise === undefined ? null : splitMix(BigInt(num(noise, "seed")));
  const step = noise === undefined ? 0 : num(noise, "step");
  return Float64Array.from({ length: num(vector, "frames") }, (_empty, frame) => {
    const sum = tones.reduce((total, tone) => total + tone.amplitude * Math.cos((2 * Math.PI * ((tone.frequency * frame) % rate)) / rate + tone.phase), 0);
    return next === null ? sum : sum + step * (next() - 0.5);
  });
}

/** The highest sidelobe of the window, -92 dB, as `the window keeps its sidelobes 92 dB down` measures. */
const SIDELOBE = 10 ** (-92 / 20);

/** `20 log10(ratio)`. */
const amplitudeDb = (ratio: number): number => 20 * Math.log10(ratio);

/** The larger floor of a measurement's fits. */
function floorOf(measured: Measurements): number {
  return Math.max(measured.sine.floor, measured.harmonics instanceof MeasureError ? 0 : measured.harmonics.quality.floor);
}

/**
 * Checks a level against a vector's expectation: "floor" (at most `20 log10(8·floor / A)`,
 * at least its negation when `rising`), "none", or a number within the floor's effect on
 * both amplitudes plus `slack` dB.
 */
function checkLevel(what: string, got: number | null, wanted: unknown, floor: number, amplitude: number, rising: boolean, slack = 0): void {
  const limit = amplitudeDb((8 * floor) / amplitude);
  if (wanted === "none") {
    expect(got, what).toBeNull();
  } else if (wanted === "floor") {
    expect(got, what).not.toBeNull();
    if (rising) {
      expect(got, what).toBeGreaterThanOrEqual(-limit);
    } else {
      expect(got, what).toBeLessThanOrEqual(limit);
    }
  } else {
    if (typeof wanted !== "number" || got === null) {
      throw new TypeError(`${what}: a level`);
    }
    const part = amplitude * 10 ** (((rising ? -1 : 1) * wanted) / 20);
    const tolerance = amplitudeDb((1 + floor / part) / (1 - floor / amplitude)) + slack;
    expect(Math.abs(got - wanted), `${what}: ${String(got)} instead of ${String(wanted)}`).toBeLessThanOrEqual(tolerance);
  }
}

/** The SFDR's slack for a vector: its spectrum's scalloping loss, and two images of every other tone at the sidelobe level, relative to the spur. */
function sfdrSlack(vector: unknown, expected: number, fundamental: number): number {
  const spur = fundamental * 10 ** (-expected / 20);
  const others = list(vector, "tones")
    .slice(1)
    .reduce((total: number, tone) => total + num(tone, "amplitude"), 0);
  const loss = spectrum(new Float64Array(num(vector, "frames"))).scalloping;
  return -amplitudeDb(1 - loss - (2 * SIDELOBE * others) / spur);
}

describe("the shared known answers", () => {
  test.each(list(VECTORS, "vectors").map((vector) => [text(vector, "name"), vector] as const))("measures %s", (name, vector) => {
    const measured = measure(synthesize(vector), num(vector, "rate"), num(vector, "fundamental"));
    const expected = field(vector, "expected");
    const amplitude = num(expected, "fundamental");
    const floor = floorOf(measured);
    if (field(vector, "noise") !== undefined) {
      expect(Math.abs(measured.fundamental - amplitude)).toBeLessThanOrEqual(1e-5);
      noiseWithinItsInterval(vector, measured);
      return;
    }
    expect(Math.abs(measured.fundamental - amplitude), name).toBeLessThanOrEqual(floor);
    checkLevel(`${name}: THD+N`, measured.thdPlusN, field(expected, "thd_plus_n"), floor, amplitude, false);
    const sfdr = field(expected, "sfdr");
    checkLevel(`${name}: SFDR`, measured.sfdr, sfdr, floor, amplitude, true, typeof sfdr === "number" ? sfdrSlack(vector, sfdr, amplitude) : 0);
    const harmonics = field(expected, "harmonics");
    if (typeof harmonics === "string") {
      expect(measured.harmonics, name).toBeInstanceOf(MeasureError);
      expect(measured.harmonics instanceof MeasureError ? measured.harmonics.kind : null).toBe(harmonics);
    } else {
      if (measured.harmonics instanceof MeasureError) {
        throw measured.harmonics;
      }
      checkLevel(`${name}: THD`, measured.harmonics.thd, field(harmonics, "thd"), floor, amplitude, false);
      checkLevel(`${name}: aliasing`, measured.harmonics.aliasing, field(harmonics, "aliasing"), floor, amplitude, false);
      checkLevel(`${name}: SNR`, measured.harmonics.snr, field(harmonics, "snr"), floor, amplitude, true);
    }
  });

  test("has every vector", () => {
    expect(text(VECTORS, "scheme")).toBe("measurements/1");
    expect(list(VECTORS, "vectors")).toHaveLength(9);
    expect(list(VECTORS, "refusals")).toHaveLength(7);
    expect(list(VECTORS, "sine_bounds")).toHaveLength(7);
    expect(list(VECTORS, "sine_error_bounds")).toHaveLength(20);
  });

  test.each(list(VECTORS, "refusals").map((vector) => [text(vector, "name"), vector] as const))("refuses %s", (_name, vector) => {
    expect(() => measure(synthesize(vector), num(vector, "rate"), num(vector, "fundamental"))).toThrow(
      expect.objectContaining({ name: "MeasureError", kind: text(vector, "error") }),
    );
  });

  test.each(list(VECTORS, "sine_bounds").map((vector) => [text(vector, "name"), vector] as const))("bounds %s", (_name, vector) => {
    const quality = (value: unknown): Quality => ({ conditioning: num(value, "conditioning"), floor: num(value, "floor") });
    const harmonic = field(vector, "harmonic");
    const measured = {
      sine: quality(field(vector, "sine")),
      enbw: num(vector, "enbw"),
      harmonics: harmonic === null ? new MeasureError("Unresolvable", "refused") : { thd: null, aliasing: null, snr: 0, quality: quality(harmonic) },
    };
    const got = sineBounds(num(vector, "amplitude"), num(vector, "error"), measured);
    const expected = field(vector, "expected");
    if (expected === null) {
      expect(got).toBeNull();
      return;
    }
    // A few roundings deep: within 1e-9 dB of the independent values.
    expect(got?.thdPlusN).toBeCloseTo(num(expected, "thd_plus_n"), 9);
    expect(got?.sfdr).toBeCloseTo(num(expected, "sfdr"), 9);
    const harmonics = field(expected, "harmonics");
    if (harmonics === null) {
      expect(got?.harmonics).toBeNull();
    } else {
      expect(got?.harmonics?.thd).toBeCloseTo(num(harmonics, "thd"), 9);
      expect(got?.harmonics?.snr).toBeCloseTo(num(harmonics, "snr"), 9);
    }
  });

  test("bounds binary32 sines only for amplitudes exact in binary32, the domain of the bound", () => {
    const binary32 = list(VECTORS, "sine_error_bounds").filter((vector) => text(vector, "format") === "binary32");
    expect(binary32.length).toBeGreaterThan(0);
    for (const vector of binary32) {
      expect(Math.fround(num(vector, "amplitude"))).toBe(num(vector, "amplitude"));
    }
  });

  test.each(list(VECTORS, "sine_error_bounds").map((vector) => [`${text(vector, "format")}, ${text(vector, "description")}`, vector] as const))(
    "bounds the error of a sine: %s",
    (_name, vector) => {
      const format = RENDER_FORMATS.find((candidate) => candidate === text(vector, "format"));
      if (format === undefined) {
        throw new Error(`unknown format ${text(vector, "format")}`);
      }
      const wanted = num(vector, "bound");
      expect(Math.abs(sineErrorBound(num(vector, "amplitude"), num(vector, "frames"), format) - wanted)).toBeLessThanOrEqual(wanted * 1e-15);
    },
  );

  test("has bound vectors of every render format", () => {
    expect(new Set(list(VECTORS, "sine_error_bounds").map((vector) => text(vector, "format")))).toStrictEqual(new Set(RENDER_FORMATS));
  });
});

/**
 * Checks the THD+N and SNR of a noisy vector against the specification's confidence
 * interval: the mean square of N uniform samples of step s lies within 10 standard
 * deviations (s²/√(180N)) of s²/12 (Chebyshev, 99 %), and a fit of K unknowns removes at
 * most 100·K·σ²/N of it (Markov, 99 %).
 */
function noiseWithinItsInterval(vector: unknown, measured: Measurements): void {
  const step = num(field(vector, "noise"), "step");
  const frames = num(vector, "frames");
  const variance = (step * step) / 12;
  const spread = (10 * step * step) / Math.sqrt(180 * frames);
  const signal = (measured.fundamental * measured.fundamental) / 2;
  const interval = (count: number): [number, number] => [10 * Math.log10(signal / (variance + spread)), 10 * Math.log10(signal / (variance - spread - (100 * count * variance) / frames))];
  const [low, high] = interval(3);
  expect(-measured.thdPlusN).toBeGreaterThanOrEqual(low);
  expect(-measured.thdPlusN).toBeLessThanOrEqual(high);
  if (measured.harmonics instanceof MeasureError) {
    throw measured.harmonics;
  }
  const [lowest, highest] = interval(unknowns(components(num(vector, "fundamental"), num(vector, "rate"), HARMONICS)).length);
  expect(measured.harmonics.snr).toBeGreaterThanOrEqual(lowest);
  expect(measured.harmonics.snr).toBeLessThanOrEqual(highest);
  const expectation = num(field(field(vector, "expected"), "harmonics"), "snr");
  expect(expectation).toBeGreaterThanOrEqual(lowest);
  expect(expectation).toBeLessThanOrEqual(highest);
}

describe("the arithmetic", () => {
  test("reads an array within its bounds only", () => {
    expect(at([1, 2, 3], 2)).toBe(3);
    expect(() => at([1, 2, 3], 3)).toThrow(new RangeError("index 3 outside 3 values"));
    expect(() => at([1], -1)).toThrow(new RangeError("index -1 outside 1 values"));
  });

  test.each([
    [[2 ** 53, 1, -(2 ** 53)], 1],
    [[1, 2 ** 53, 1, -(2 ** 53)], 2],
    [[1e16, 1, 1, 1, -1e16], 3],
    [[-0.5, 1e-17, 0.5], 1e-17],
  ])("sums %j without losing a term", (terms, wanted) => {
    expect(compensatedSum(terms)).toBe(wanted);
  });

  test.each([1000.1, Math.PI, 0.1, 1e-300, 123_456_789.123, -7.25])("splits %d into two halves of 26 bits", (value) => {
    const [high, low] = split(value);
    expect(high + low).toBe(value);
    const bits = new BigUint64Array(new Float64Array([high]).buffer)[0] ?? 0n;
    expect(bits & ((1n << 27n) - 1n)).toBe(0n);
    expect(Math.abs(low)).toBeLessThanOrEqual(Math.abs(high) * 2 ** -26);
  });

  /** `frequency · frame / rate` modulo 1, exactly, in BigInt: `(m · frame) mod (rate · 2^k)` over `rate · 2^k`, to 2^-64. */
  function exactCycles(frequency: number, frame: number, rate: number): number {
    const bits = new BigUint64Array(new Float64Array([frequency]).buffer)[0] ?? 0n;
    const down = 1075n - (bits >> 52n);
    const mantissa = (bits & ((1n << 52n) - 1n)) | (1n << 52n);
    const modulus = BigInt(rate) << down;
    const remainder = (mantissa * BigInt(frame)) % modulus;
    return Number((remainder << 64n) / modulus) * 2 ** -64;
  }

  test.each([
    [1000.1, MAX_FRAMES - 1, 44_100],
    [1000.1, 1, 44_100],
    [0.1, MAX_FRAMES - 1, 8000],
    [191_999.9, MAX_FRAMES - 1, 384_000],
    [440.25, 96_000, 96_000],
  ])("reduces the phase of %d Hz exactly at frame %d of %d Hz", (frequency, frame, rate) => {
    const difference = cycles(frequency, frame, rate) - exactCycles(frequency, frame, rate);
    const turns = Math.round(difference);
    expect(Math.abs(turns)).toBeLessThanOrEqual(1);
    expect(Math.abs(difference - turns)).toBeLessThanOrEqual(2 ** -51);
  });

  test("loses the phase without the reduction (the control)", () => {
    const plain = (1000.1 * (MAX_FRAMES - 1)) / 44_100;
    expect(Math.abs(plain - Math.floor(plain) - exactCycles(1000.1, MAX_FRAMES - 1, 44_100))).toBeGreaterThan(2 ** -45);
    expect(MAX_FRAMES).toBeLessThan(2 ** 26);
  });

  test.each([
    [0, 1000, true],
    [8, 1000, true],
    [-8, 1000, true],
    [7, 1000, false],
    [2, 4000, true],
    [3, 2666.666748046875, false],
    [16, 1000.5, false],
    [16, 1500, true],
  ])("tells whether %d times %d Hz is whole turns of 8 kHz", (multiple, fundamental, whole) => {
    expect(wholeTurns(multiple, fundamental, 8000)).toBe(whole);
  });
});

describe("the fits", () => {
  test("keeps each folded frequency once, with its lowest order", () => {
    expect(components(1000, 8000, HARMONICS)).toStrictEqual([
      { order: 0, edge: true },
      { order: 1, edge: false },
      { order: 2, edge: false },
      { order: 3, edge: false },
      { order: 4, edge: true },
    ]);
    expect(components(7000, 48_000, HARMONICS)).toHaveLength(11);
    expect(components(7000, 48_000, HARMONICS).slice(1).filter((component) => component.edge)).toStrictEqual([]);
    expect(components(2000, 8000, HARMONICS)).toStrictEqual([
      { order: 0, edge: true },
      { order: 1, edge: false },
      { order: 2, edge: true },
    ]);
    expect(components(16_000, 48_000, HARMONICS).map((component) => component.order)).toStrictEqual([0, 1]);
    expect(components(440, 48_000, 1)).toHaveLength(2);
    expect(HARMONICS).toBe(10);
  });

  test("gives each unknown the mean square of its function", () => {
    expect(unknowns(components(1000, 8000, HARMONICS)).map((unknown) => [unknown.order, unknown.weight])).toStrictEqual([
      [0, 1],
      [1, 0.5],
      [1, 0.5],
      [2, 0.5],
      [2, 0.5],
      [3, 0.5],
      [3, 0.5],
      [4, 1],
    ]);
  });

  test("evaluates the basis at a phase, reduced first", () => {
    const found = components(1000, 8000, HARMONICS);
    const half = Math.SQRT1_2;
    const row = basis(found, 0.125, 8);
    [1, half, half, 0, 1, -half, half, -1].forEach((wanted, index) => {
      expect(at(row, index)).toBeCloseTo(wanted, 15);
    });
    expect(row).toHaveLength(8);
    basis(found, 7.125, 8).forEach((value, index) => {
      expect(value).toBeCloseTo(at(row, index), 14);
    });
  });

  test("accepts a conditioning of one half and no more", () => {
    const one = solve([[1.5]], [3], [1]);
    expect(one.conditioning).toBe(0.5);
    expect(at(one.coefficients, 0)).toBeCloseTo(2, 15);
    expect(() => solve([[1.5 + 2 ** -52]], [3], [1])).toThrow(expect.objectContaining({ kind: "Unresolvable" }));
    const two = solve(
      [
        [4, 1],
        [1, 1],
      ],
      [2, 2],
      [0.5, 1],
    );
    expect(two.conditioning).toBe(0.5);
    // Rows of different deviations: the largest decides, 0.6 here, refused.
    const uneven = [
      [1, 0.3, 0.3],
      [0.3, 1, 0],
      [0.3, 0, 1],
    ];
    expect(() => solve(uneven, [1, 1, 1], [1, 1, 1])).toThrow(expect.objectContaining({ kind: "Unresolvable", message: "the components are too close to be told apart (conditioning 0.6, at most 0.5)" }));
    expect(at(two.coefficients, 0)).toBeCloseTo(0, 15);
    expect(at(two.coefficients, 1)).toBeCloseTo(2, 15);
  });

  test("bounds its own error by the floor, term by term", () => {
    const basisError = 2 * Math.PI * (HARMONICS * 2 ** -51 + 2 ** -53) + 2 ** -52;
    const count = 2 * (HARMONICS + 1);
    const evaluation = (count + 1) * 2 ** -53;
    const rows = count * 2 * (2 * basisError + 2 ** -52);
    const solution = (rows + 2 * basisError) / (1 - CONDITIONING_MAX) + CONDITIONING_MAX ** JACOBI_ITERATIONS * 3;
    const transform = (23 * 5 * 2 ** -53 * 2 * Math.sqrt(8)) / 0.358;
    expect(basisError + evaluation + solution + transform).toBeLessThanOrEqual(FLOOR);
    expect(FLOOR).toBe(2 ** -36);
    expect(CONDITIONING_MAX ** JACOBI_ITERATIONS).toBeLessThanOrEqual(2 ** -60);
  });
});

/** `amplitude · sin(2π·cycles·n + phase)`, the phase reduced exactly. */
function sine(frames: number, cyclesPerSample: number, amplitude: number, phase: number): Float64Array {
  return Float64Array.from({ length: frames }, (_empty, frame) => {
    const turns = cyclesPerSample * frame;
    return amplitude * Math.sin(2 * Math.PI * (turns - Math.floor(turns)) + phase);
  });
}

describe("measure", () => {
  test("measures at the longest length and refuses one frame more", () => {
    const samples = sine(MAX_FRAMES, 0.25, 0.5, 0.3);
    const measured = measure(samples, 8000, 2000);
    expect(Math.abs(measured.fundamental - 0.5)).toBeLessThanOrEqual(measured.sine.floor);
    expect(measured.thdPlusN).toBeLessThan(-200);
    const longer = new Float64Array(MAX_FRAMES + 1);
    expect(() => measure(longer, 8000, 2000)).toThrow(expect.objectContaining({ kind: "TooLong", message: `the signal has ${String(MAX_FRAMES + 1)} frames, more than the ${String(MAX_FRAMES)} measured` }));
  }, 60_000);

  test.each([0, -0, -1, 4000, 4000.5, Number.NaN, Number.POSITIVE_INFINITY])("refuses a fundamental of %d Hz at 8 kHz", (fundamental) => {
    expect(() => measure(sine(64, 0.1, 1, 0), 8000, fundamental)).toThrow(
      expect.objectContaining({ kind: "Fundamental", message: `the fundamental, ${String(fundamental)} Hz, is not strictly between 0 and Nyquist` }),
    );
  });

  test("refuses what it cannot measure, saying why", () => {
    const tone = sine(64, 0.1, 1, 0);
    expect(measure(tone, 8000, 800).fundamental).toBeCloseTo(1, 12);
    expect(() => measure(new Float64Array(0), 8000, 800)).toThrow(expect.objectContaining({ kind: "NoSamples", message: "the signal has no sample" }));
    const broken = Float64Array.from(tone);
    broken[5] = Number.POSITIVE_INFINITY;
    expect(() => measure(broken, 8000, 800)).toThrow(expect.objectContaining({ kind: "NotFinite", message: "sample 5 is not finite" }));
    expect(() => measure(tone.subarray(0, 2), 8000, 800)).toThrow(
      expect.objectContaining({ kind: "TooShort", message: "the signal has 2 frames, fewer than the 3 unknowns of the fit" }),
    );
    expect(() => measure(tone.subarray(0, 3), 8000, 800)).toThrow(expect.objectContaining({ kind: "Unresolvable" }));
    expect(() => measure(new Float64Array(64), 8000, 800)).toThrow(
      expect.objectContaining({ kind: "NoFundamental", message: "the fundamental, of amplitude 0, is not above the floor of the measurement" }),
    );
    expect(() => measure(new Float64Array(64).fill(0.5), 8000, 800)).toThrow(expect.objectContaining({ kind: "NoFundamental" }));
    expect(new MeasureError("NoSamples", "why").name).toBe("MeasureError");
  });

  test("lets an error that is not a refusal through", () => {
    /** A signal whose samples break on the second reading: an error that is not a refusal, as a bug would raise. */
    class Hostile extends Float64Array {
      /** Counts the readings. */
      #reads = 0;
      /** Throws from the third reading on: after the sine fit, in the harmonic fit. */
      override forEach(callback: (value: number, index: number, array: this) => void): void {
        this.#reads += 1;
        if (this.#reads > 1) {
          throw new RangeError("boom");
        }
        super.forEach(callback);
      }
    }
    expect(() => measure(new Hostile(sine(64, 0.1, 1, 0)), 8000, 800)).toThrow(new RangeError("boom"));
  });

  test("tells apart components only when it can", () => {
    expect(() => measure(sine(1024, 3 / 48_000, 1, 0), 48_000, 3)).toThrow(expect.objectContaining({ kind: "Unresolvable" }));
    expect(measure(sine(1024, 10 / 1024, 1, 0), 48_000, 468.75).sine.conditioning).toBeLessThan(0.1);
    const coherent = measure(sine(8000, 3999 / 8000, 1, 0), 8000, 3999).harmonics;
    expect(coherent instanceof MeasureError ? 1 : coherent.quality.conditioning).toBeLessThan(1e-12);
    const refused = measure(sine(4096, 16_001 / 48_000, 1, 0), 48_000, 16_001);
    expect(refused.harmonics).toBeInstanceOf(MeasureError);
    expect(refused.harmonics instanceof MeasureError ? refused.harmonics.message : "").toMatch(/^the components are too close to be told apart \(conditioning [\d.]+, at most 0\.5\)$/);
    expect(refused.sine.conditioning).toBeLessThanOrEqual(CONDITIONING_MAX);
  });

  test("scales its floor by the signal and the fit", () => {
    const samples = sine(1000, 0.125, 0.5, 0).map((value) => value + 0.25);
    const measured = measure(samples, 8000, 1000);
    expect(measured.sine.floor / (FLOOR * 1.5)).toBeCloseTo(1, 11);
    expect(measured.harmonics instanceof MeasureError ? 0 : measured.harmonics.quality.floor / (FLOOR * 1.5)).toBeCloseTo(1, 11);
  });

  test("meets the bounds for every signal within the error, and reaches them", () => {
    const error = 1e-6;
    const next = splitMix(7n);
    for (const [rate, frames, perSample] of [
      [8000, 1000, 0.123],
      [48_000, 4096, 0.021],
      [44_100, 3000, 0.41],
    ] as const) {
      const ideal = sine(frames, perSample, 1, 0.4);
      const shapes: ((frame: number) => number)[] = [
        (frame) => Math.sign(Math.cos(2 * Math.PI * 2 * perSample * frame)),
        (frame) => Math.sign(Math.cos(2 * Math.PI * 7 * perSample * frame)),
        (frame) => Math.sign(Math.cos(2 * Math.PI * 0.3171 * frame)),
        (frame) => (frame % 2 === 0 ? 1 : -1),
        () => 2 * next() - 1,
      ];
      shapes.forEach((shape, index) => {
        const measured = measure(
          ideal.map((value, frame) => value + error * shape(frame)),
          rate,
          perSample * rate,
        );
        const bounds = sineBounds(1, error, measured);
        if (bounds?.harmonics === null || bounds === null || measured.harmonics instanceof MeasureError) {
          throw new Error("bounded and resolvable");
        }
        expect(measured.thdPlusN).toBeLessThanOrEqual(bounds.thdPlusN);
        expect(measured.sfdr).toBeGreaterThanOrEqual(bounds.sfdr);
        expect(measured.harmonics.snr).toBeGreaterThanOrEqual(bounds.harmonics.snr);
        for (const level of [measured.harmonics.thd, measured.harmonics.aliasing]) {
          expect(level ?? -Infinity).toBeLessThanOrEqual(bounds.harmonics.thd);
        }
        if (index === 0) {
          expect(measured.thdPlusN).toBeGreaterThan(bounds.thdPlusN - 4);
        }
      });
    }
  }, 60_000);

  test("gives no bound without a margin, and no harmonic bound without a harmonic fit", () => {
    const sineQuality = { conditioning: 0.1, floor: 0 };
    expect(sineBounds(1, 0.7, { sine: sineQuality, enbw: 2, harmonics: new MeasureError("Unresolvable", "refused") })).toBeNull();
    expect(sineBounds(1, 2 ** -25, { sine: sineQuality, enbw: 2, harmonics: new MeasureError("Unresolvable", "refused") })?.harmonics).toBeNull();
  });
});

describe("the spectrum", () => {
  /** The window from its definition. */
  const reference = (frames: number): number[] =>
    Array.from({ length: frames }, (_empty, index) => {
      const angle = (2 * Math.PI * index) / frames;
      return 0.35875 - 0.48829 * Math.cos(angle) + 0.14128 * Math.cos(2 * angle) - 0.01168 * Math.cos(3 * angle);
    });

  /** `|W(ν)| / Σw` by direct summation. */
  const transform = (weights: readonly number[], nu: number): number => {
    const [re, im] = weights.reduce(([real, imaginary], weight, index) => [real + weight * Math.cos(2 * Math.PI * nu * index), imaginary - weight * Math.sin(2 * Math.PI * nu * index)], [0, 0]);
    return Math.sqrt(re * re + im * im) / weights.reduce((total, weight) => total + weight, 0);
  };

  /** `a·cos(2π·cycles·n + phase)`, the phase reduced exactly. */
  const tone = (frames: number, perSample: number, amplitude: number, phase: number): Float64Array =>
    Float64Array.from({ length: frames }, (_empty, frame) => {
      const turns = perSample * frame;
      return amplitude * Math.cos(2 * Math.PI * (turns - Math.floor(turns)) + phase);
    });

  test("has the window's closed-form values", () => {
    expect(WINDOW).toStrictEqual([0.35875, 0.48829, 0.14128, 0.01168]);
    [6e-5, 0.21747, 1, 0.21747].forEach((wanted, index) => {
      expect(at(window(4), index)).toBeCloseTo(wanted, 15);
    });
    for (const frames of [1, 2, 3, 7, 64, 1000]) {
      const weights = window(frames);
      expect(weights).toHaveLength(frames);
      reference(frames).forEach((wanted, index) => {
        expect(at(weights, index)).toBeCloseTo(wanted, 15);
      });
      expect(Array.from(weights).filter((weight) => !(weight > 0))).toStrictEqual([]);
    }
  });

  test("keeps the window's sidelobes 92 dB down", () => {
    const weights = Array.from(window(256));
    let highest = 0;
    for (let step = 0; step <= 62_000; step++) {
      highest = Math.max(highest, transform(weights, (4 + (124 * step) / 62_000) / 256));
    }
    expect(amplitudeDb(highest)).toBeLessThan(-92);
    expect(amplitudeDb(highest)).toBeGreaterThan(-92.1);
  }, 60_000);

  test.each([1, 2, 4, 8, 64, 512])("transforms %d points like the definition", (points) => {
    const re = Float64Array.from({ length: points }, (_empty, index) => Math.sin(index * 1.3) + 0.25);
    const im = Float64Array.from({ length: points }, (_empty, index) => Math.cos(index * 0.7) - 0.5);
    const [fastRe, fastIm] = fft(Float64Array.from(re), Float64Array.from(im));
    for (let point = 0; point < points; point++) {
      let [slowRe, slowIm] = [0, 0];
      for (let index = 0; index < points; index++) {
        const angle = (-2 * Math.PI * ((point * index) % points)) / points;
        slowRe += at(re, index) * Math.cos(angle) - at(im, index) * Math.sin(angle);
        slowIm += at(re, index) * Math.sin(angle) + at(im, index) * Math.cos(angle);
      }
      expect(Math.abs(at(fastRe, point) - slowRe)).toBeLessThan(1e-12);
      expect(Math.abs(at(fastIm, point) - slowIm)).toBeLessThan(1e-12);
    }
  });

  test.each([
    [1, 4],
    [2, 8],
    [3, 16],
    [4, 16],
    [5, 32],
    [1000, 4096],
  ])("pads %d frames to %d points", (frames, points) => {
    expect(PADDING).toBe(4);
    const measured = spectrum(new Float64Array(frames).fill(0.5));
    expect(measured.points).toBe(points);
    expect(measured.amplitudes).toHaveLength(points / 2 + 1);
  });

  test.each([1, 2, 5, 100, 1001])("measures DC and Nyquist exactly over %d frames", (frames) => {
    expect(Math.abs(at(spectrum(new Float64Array(frames).fill(0.75)).amplitudes, 0) - 0.75)).toBeLessThan(1e-13);
    const nyquist = spectrum(tone(frames, 0.5, -0.5, 0)).amplitudes;
    expect(Math.abs(at(nyquist, nyquist.length - 1) - 0.5)).toBeLessThan(1e-13);
  });

  test("measures a sine on a point exactly", () => {
    const measured = spectrum(tone(1000, 0.25, 0.5, 1));
    expect(Math.abs(at(measured.amplitudes, 1024) - 0.5)).toBeLessThan(1e-12);
  });

  test.each([64, 1000, 4096])("loses at most the scalloping bound between points, over %d frames", (frames) => {
    const weights = reference(frames);
    const total = weights.reduce((sum, weight) => sum + weight, 0);
    const moment = weights.reduce((sum, weight, index) => sum + weight * (index - frames / 2) ** 2, 0) / total;
    const measured = spectrum(tone(frames, 0.1, 1, 0));
    const bound = (Math.PI * Math.PI * moment) / (2 * measured.points * measured.points);
    expect(Math.abs(measured.scalloping - bound)).toBeLessThanOrEqual(1e-15);
    expect(bound).toBeLessThan(0.006);
    const perSample = (measured.points / 8 + 0.5) / measured.points;
    const near = at(spectrum(tone(frames, perSample, 1, 0.3)).amplitudes, measured.points / 8);
    const image = transform(weights, 2 * perSample - 0.5 / measured.points);
    expect(near).toBeGreaterThanOrEqual(1 - bound - image);
    expect(near).toBeLessThanOrEqual(1 + image);
    expect(near).toBeLessThan(1 - bound / 1.5 + image);
  });

  test.each([1, 4, 1000])("has the window's equivalent noise bandwidth over %d frames", (frames) => {
    const weights = reference(frames);
    const sum = weights.reduce((total, weight) => total + weight, 0);
    const squares = weights.reduce((total, weight) => total + weight * weight, 0);
    expect(spectrum(tone(frames, 0.1, 1, 0)).enbw).toBeCloseTo((frames * squares) / (sum * sum), 12);
  });

  test("has Harris's noise bandwidth, and every point bounded by Cauchy-Schwarz", () => {
    expect(spectrum(new Float64Array(1000)).enbw).toBeCloseTo(2.00435, 5);
    const measured = spectrum(tone(4096, 0.125, 1, 0));
    expect(Math.max(...measured.amplitudes.subarray(0, 4096))).toBeLessThanOrEqual(Math.sqrt(2 * measured.enbw));
  });

  test("reads up to twice a component beside an edge", () => {
    const measured = spectrum(tone(1000, 0.5, 0.5, 0));
    const beside = at(measured.amplitudes, measured.points / 2 - 1);
    expect(beside).toBeCloseTo(2 * 0.5 * transform(reference(1000), 1 / measured.points), 12);
    expect(beside).toBeGreaterThan(0.97);
    expect(beside).toBeLessThanOrEqual(1);
  });

  test("refuses what it cannot measure", () => {
    expect(() => spectrum(new Float64Array(0))).toThrow(expect.objectContaining({ kind: "NoSamples" }));
    expect(() => spectrum(new Float64Array(MAX_FRAMES + 1))).toThrow(expect.objectContaining({ kind: "TooLong" }));
    expect(() => spectrum(Float64Array.from([0, 1, Number.NaN]))).toThrow(expect.objectContaining({ kind: "NotFinite", message: "sample 2 is not finite" }));
    expect(() => spectrum(Float64Array.from([Number.NEGATIVE_INFINITY, Number.NaN]))).toThrow(expect.objectContaining({ message: "sample 0 is not finite" }));
  });
});

test("bounds nothing without a margin above zero", () => {
  // `a = A - b·√2` exactly zero: no bound, as below zero.
  const orthogonal = { conditioning: 0, floor: 0 };
  const measured = { sine: orthogonal, enbw: 2, harmonics: { thd: null, aliasing: null, snr: 0, quality: orthogonal } };
  expect(sineBounds(0.5 * Math.sqrt(2), 0.5, measured)).toBeNull();
  expect(sineBounds(0.5 * Math.sqrt(2), 0.499, measured)).not.toBeNull();
  expect(sineBounds(1, 0.4, { ...measured, harmonics: { ...measured.harmonics, quality: { conditioning: 0.5, floor: 0 } } })?.harmonics).toBeNull();
});

describe("sineFitRefusal", () => {
  test("refuses as measure does, before any sample: too few frames, then components too close", () => {
    // Three unknowns (DC, and the fundamental's cosine and sine): two frames are too few,
    // three are not.
    expect(sineFitRefusal(1000, 8000, 2)).toStrictEqual(new MeasureError("TooShort", "the signal has 2 frames, fewer than the 3 unknowns of the fit"));
    expect(sineFitRefusal(1000, 8000, 2)?.kind).toBe("TooShort");
    expect(sineFitRefusal(1000, 8000, 3)?.kind).not.toBe("TooShort");
    // The same refusal, conditioning included, as measuring a signal of those frames: the
    // Gram matrix is summed over exactly these frames.
    for (const frames of [3, 5, 100]) {
      const refused = (() => {
        try {
          measure(sine(frames, 1 / 48_000, 1, 0.3), 48_000, 1);
        } catch (error) {
          return error;
        }
        return null;
      })();
      expect(sineFitRefusal(1, 48_000, frames)).toStrictEqual(refused);
    }
    // Two whole cycles over 1000 frames: orthogonal, no refusal.
    expect(sineFitRefusal(96, 48_000, 1000)).toBeNull();
  });
});

describe("the floor of a render within an error of a sine", () => {
  test("bounds the peak and the coefficients, doubled", () => {
    // 2 · 2^-36 · ((1 + √2)·|A| + (1 + √(2·21) · √(1 + ρ)/(1 - ρ))·ε), computed here by hand.
    expect(renderFloor(1, 0, 0)).toBe(2 * 2 ** -36 * (1 + Math.SQRT2));
    expect(renderFloor(-1, 0, 0)).toBe(2 * 2 ** -36 * (1 + Math.SQRT2));
    expect(renderFloor(0, 1, 0)).toBe(2 * 2 ** -36 * (1 + Math.sqrt(42)));
    expect(renderFloor(0, 1, 0.5)).toBe(2 * 2 ** -36 * (1 + (Math.sqrt(42) * Math.sqrt(1.5)) / 0.5));
  });

  test("decides a margin within the floor of the render above the error", () => {
    // Without the floor, a margin from √2 · error up; with it, about 2^-31.5 of the error
    // more: an amplitude 2^-33 above √2 has none, one 2^-29 above has one.
    expect(hasMargin(Math.SQRT2 * (1 + 2 ** -33), 1, 0)).toBe(false);
    expect(hasMargin(Math.SQRT2 * (1 + 2 ** -29), 1, 0)).toBe(true);
    expect(hasMargin(0, 0, 0)).toBe(false);
  });

  test("is never exceeded by the fits of an exact render", () => {
    for (const [frequency, frames, rate] of [
      [3001.5, 4800, 48_000],
      [Math.fround(46.44), 1024, 48_000],
      [1000.5, 8000, 8000],
    ] as const) {
      for (const amplitude of [1, 2 ** -100, 1.5 * 2 ** -126]) {
        const error = 2 ** -24 * amplitude + 2 ** -149;
        const samples = Float64Array.from({ length: frames }, (_frame, frame) => Math.fround(amplitude * Math.sin((2 * Math.PI * ((frequency * frame) % rate)) / rate)));
        const measured = measure(samples, rate, frequency);
        expect(measured.sine.floor).toBeLessThanOrEqual(renderFloor(amplitude, error, measured.sine.conditioning));
        if (!(measured.harmonics instanceof MeasureError)) {
          expect(measured.harmonics.quality.floor).toBeLessThanOrEqual(renderFloor(amplitude, error, measured.harmonics.quality.conditioning));
        }
      }
    }
  }, 60_000);
});
