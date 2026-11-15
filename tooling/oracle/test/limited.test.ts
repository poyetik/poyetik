import { describe, expect, test } from "vitest";
import { BS1770, CEILING_DBTP, SCAN, STANDARD_DELAY, framesOf, judgeLimited, latency, underRead } from "../src/limited.ts";

/** The protected output of the oracle's cases: the standard detector, 1.5 ms. */
const OUTPUT = { mode: "protected", lookahead: 1.5, detector: "standard" } as const;

/** 10^(-1/20): the default ceiling as an amplitude, computed apart. */
const CEILING = 0.891_250_938_133_745_6;

/** `frames` frames at 48 kHz: `latency` of silence, then `amplitude · sin(2π f n / rate)`, plus `wobble(n)`. */
function render(frames: number, amplitude: number, silence = 78, wobble: (frame: number) => number = () => 0): number[] {
  return Array.from({ length: frames }, (_value, frame) => (frame < silence ? 0 : amplitude * Math.sin((2 * Math.PI * 1000 * frame) / 48_000) + wobble(frame)));
}

describe("the standard detector's under-read", () => {
  test("holds the standard's table and the facts derived from it", () => {
    expect(BS1770).toHaveLength(12);
    expect(CEILING_DBTP).toBe(-1);
    expect(STANDARD_DELAY).toBe(6);
    expect(SCAN).toBe(4000);
  });

  test("reads a slow sine nearly whole, and rolls off near half the rate", () => {
    // As the engine's measurements find from its own copy of the table: under 0.45 dB up to
    // 0.417 of the rate, 2.2 dB at 0.45 (decision 12).
    const slow = underRead(2 * Math.PI * 0.01);
    expect(-20 * Math.log10(slow.low)).toBeLessThan(0.05);
    expect(slow.high).toBeGreaterThanOrEqual(1);
    for (const share of [0.1, 0.25, 0.34, 0.4, 0.417]) {
      expect(-20 * Math.log10(underRead(2 * Math.PI * share).low)).toBeLessThan(0.45);
    }
    const rolled = -20 * Math.log10(underRead(2 * Math.PI * 0.45).low);
    expect(rolled).toBeGreaterThan(2);
    expect(rolled).toBeLessThan(2.5);
  });

  test("counts its frames and its latency as the limiter does", () => {
    expect(framesOf(1.5, 48_000)).toBe(72);
    expect(framesOf(0.5, 8000)).toBe(4);
    expect(framesOf(1 / 96, 48_000)).toBe(1);
    expect(framesOf(0.001, 8000)).toBe(1);
    expect(latency(OUTPUT, 48_000)).toBe(78);
  });
});

describe("judgeLimited", () => {
  const sine = { frequency: 1000, amplitude: 1, phase: 0 };
  const { low, high } = underRead((2 * Math.PI * 1000) / 48_000);

  test("accepts a sine limited within the derived range, silent over its latency", () => {
    for (const amplitude of [CEILING / high, (CEILING / high + CEILING / low) / 2, CEILING / low]) {
      const judged = judgeLimited(render(16_384, amplitude), 48_000, sine, OUTPUT, "binary64");
      expect("peak" in judged ? judged.peak : 0).toBeCloseTo(amplitude, 9);
    }
    expect(judgeLimited(render(16_384, CEILING / low), 48_000, sine, OUTPUT, "binary32")).not.toHaveProperty("problems");
  });

  test.each<[string, number[], string]>([
    ["a sample during the latency", render(16_384, 0.89, 77), "not silent over its latency, 78 frames"],
    ["a true peak above the range", render(16_384, CEILING / low + 0.01), "true peak"],
    ["a true peak below the range", render(16_384, CEILING / high - 0.01), "true peak"],
    ["a distorted sine", render(16_384, 0.89, 78, (frame) => 0.05 * Math.sin((2 * Math.PI * 3000 * frame) / 48_000)), "THD+N"],
    ["silence", render(16_384, 0), "cannot be measured"],
  ])("refuses %s", (_case, samples, problem) => {
    const judged = judgeLimited(samples, 48_000, sine, OUTPUT, "binary64");
    expect("problems" in judged ? judged.problems.join("\n") : "").toContain(problem);
  });

  test("judges the standard detector only, and a sine whose period fits the window", () => {
    expect(judgeLimited(render(16_384, 0.89), 48_000, sine, { ...OUTPUT, detector: "precise" }, "binary64")).toStrictEqual({
      problems: ["the oracle judges limited channels with the standard detector only, not precise"],
    });
    expect(judgeLimited(render(16_384, 0.89), 48_000, { ...sine, frequency: 500 }, OUTPUT, "binary64")).toStrictEqual({
      problems: ["a period of the sine (96 frames) does not fit the limiter's window (74 frames)"],
    });
  });
});
