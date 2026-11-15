import { readFileSync } from "node:fs";
import { assemble } from "@poyetik/wasm-build/testing/assemble";
import { describe, expect, test } from "vitest";
import { VISUAL_MODULE } from "../scripts/module.ts";
import { LINEAR_PERCENT, linearityProblems, measure, MODULE_NAME, SIZES, sizeName, TOLERANCE_PERCENT, VISUAL_BENCHMARKS } from "../benchmarks/suite.ts";

/** The visual module as `pnpm build:wasm` built it. */
function built(): Uint8Array {
  return new Uint8Array(readFileSync(new URL(`../dist/${VISUAL_MODULE.file}`, import.meta.url)));
}

/** The names of the measurements, in their order. */
const NAMES = [...SIZES.flatMap(([width, height]) => [sizeName("render", width, height), sizeName("render and hash", width, height)]), MODULE_NAME];

describe("the visual benchmarks", () => {
  test("measure every size of the ladder, the same counts every time, proportional to the pixels", () => {
    const wasm = built();
    const first = measure(wasm);
    expect(first.map(({ name }) => name)).toStrictEqual(NAMES);
    expect(first.map(({ unit }) => unit)).toStrictEqual([...NAMES.slice(0, -1).map(() => "instructions"), "bytes"]);
    expect(first.at(-1)?.value).toBe(wasm.length);
    expect(first.every(({ value }) => Number.isSafeInteger(value) && value > 0)).toBe(true);
    expect(measure(wasm), "deterministic").toStrictEqual(first);
    expect(linearityProblems(first)).toStrictEqual([]);
    // Hashing an image costs more than rendering it: its counts include the render.
    for (const [width, height] of SIZES) {
      const value = (name: string): number => first.find((entry) => entry.name === name)?.value ?? 0;
      expect(value(sizeName("render and hash", width, height)), `${String(width)}x${String(height)}`).toBeGreaterThan(value(sizeName("render", width, height)));
    }
  });

  test("refuse a cost not proportional to the pixels, and a size not measured, naming it", () => {
    const linear = SIZES.flatMap(([width, height]) => [
      { name: sizeName("render", width, height), value: 100 * width * height, unit: "instructions" },
      { name: sizeName("render and hash", width, height), value: 1500 * width * height, unit: "instructions" },
    ]);
    expect(linearityProblems(linear)).toStrictEqual([]);
    const [small] = SIZES;
    if (small === undefined) {
      throw new Error("no size");
    }
    const at = (percent: number) =>
      linearityProblems(linear.map((entry) => (entry.name === sizeName("render", small[0], small[1]) ? { ...entry, value: Math.round(entry.value * (1 + percent / 100)) } : entry)));
    expect(at(LINEAR_PERCENT), "at the bound").toStrictEqual([]);
    expect(at(LINEAR_PERCENT + 1)).toStrictEqual(["render 64x36: 103.00 instructions a pixel, 100.00 at the largest size: not proportional to its pixels within 2%"]);
    expect(at(-(LINEAR_PERCENT + 1))).toStrictEqual(["render 64x36: 97.00 instructions a pixel, 100.00 at the largest size: not proportional to its pixels within 2%"]);
    expect(linearityProblems(linear.filter(({ name }) => name !== "render and hash 640x360"))).toStrictEqual(["render and hash 640x360: not measured"]);
    expect(linearityProblems(linear.filter(({ name }) => name !== "render 1280x720"))).toStrictEqual(SIZES.map(([width, height]) => `${sizeName("render", width, height)}: not measured`));
  });

  test("name the benchmark a module cannot measure", () => {
    const empty = assemble({ functions: [] });
    expect(() => measure(empty)).toThrow(new Error("render 64x36: no function visual_setup"));
    const refusing = assemble({ functions: [{ params: 7, result: true, locals: [0], code: [0x41, 0x00, 0x0b], exported: "visual_setup" }] });
    expect(() => measure(refusing)).toThrow(new Error("render 64x36: the module refused the square"));
    const accepting = assemble({ functions: [{ params: 7, result: true, locals: [0], code: [0x41, 0x01, 0x0b], exported: "visual_setup" }] });
    expect(() => measure(accepting)).toThrow(new Error("render 64x36: no function visual_render"));
    const rendering = assemble({
      functions: [
        { params: 7, result: true, locals: [0], code: [0x41, 0x01, 0x0b], exported: "visual_setup" },
        { params: 2, result: true, locals: [0], code: [0x41, 0x00, 0x0b], exported: "visual_render" },
      ],
    });
    expect(() => measure(rendering)).toThrow(new Error("render and hash 64x36: no function visual_hash"));
  });

  test("are the visual module's, compared within their tolerance and checked by their law", () => {
    expect(VISUAL_BENCHMARKS).toMatchObject({
      baselineFile: "spec/visual-benchmark-baseline.json",
      moduleFile: `bindings/visual-wasm/dist/${VISUAL_MODULE.file}`,
      buildCommand: "pnpm build:wasm, in bindings/visual-wasm",
      command: "node scripts/benchmarks.ts (in bindings/visual-wasm)",
      tolerancePercent: TOLERANCE_PERCENT,
      measure,
      laws: linearityProblems,
    });
    expect([TOLERANCE_PERCENT, LINEAR_PERCENT]).toStrictEqual([2, 2]);
    // The name the committed baseline holds for the module's size (the benchmark command
    // compares every name of it with the measurements).
    expect(MODULE_NAME).toBe("visual module");
  });
});
