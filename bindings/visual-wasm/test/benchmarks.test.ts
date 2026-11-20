import { readFileSync } from "node:fs";
import { assemble } from "@poyetik/wasm-build/testing/assemble";
import { describe, expect, test } from "vitest";
import { VISUAL_MODULE } from "../scripts/module.ts";
import { LAW_FROM, LINEAR_PERCENT, linearityProblems, MEASURED, measure, MODULE_NAME, PLACEMENT, placementProblems, SIZES, sizeName, TOLERANCE_PERCENT, VISUAL_BENCHMARKS, visualLaws } from "../benchmarks/suite.ts";

/** The visual module as `pnpm build:wasm` built it. */
function built(): Uint8Array {
  return new Uint8Array(readFileSync(new URL(`../dist/${VISUAL_MODULE.file}`, import.meta.url)));
}

/** The names of the measurements, in their order. */
const NAMES = [...SIZES.flatMap(([width, height]) => [...MEASURED.map((what) => sizeName(what, width, height)), sizeName(PLACEMENT, width, height)]), MODULE_NAME];

describe("the visual benchmarks", () => {
  test("measure every size of the ladder, the same counts every time, proportional to the pixels", () => {
    const wasm = built();
    const first = measure(wasm);
    expect(first.map(({ name }) => name)).toStrictEqual(NAMES);
    expect(first.map(({ unit }) => unit)).toStrictEqual([...NAMES.slice(0, -1).map(() => "instructions"), "bytes"]);
    expect(first.at(-1)?.value).toBe(wasm.length);
    expect(first.every(({ value }) => Number.isSafeInteger(value) && value > 0)).toBe(true);
    expect(measure(wasm), "deterministic").toStrictEqual(first);
    expect(visualLaws(first)).toStrictEqual([]);
    // The placement, a few integer operations: the same count at every size, and far below
    // a render's.
    const placements = SIZES.map(([width, height]) => first.find(({ name }) => name === sizeName(PLACEMENT, width, height))?.value);
    expect(new Set(placements).size, `one count: ${String(placements)}`).toBe(1);
    expect((placements[0] ?? Number.POSITIVE_INFINITY) < 1000, `${String(placements[0])} instructions`).toBe(true);
    // Encoding an image, or hashing it, costs more than rendering it: their counts include
    // the render; the hash costs more than the encoding.
    for (const [width, height] of SIZES) {
      const value = (name: string): number => first.find((entry) => entry.name === name)?.value ?? 0;
      const [render = 0, display = 0, hash = 0] = MEASURED.map((what) => value(sizeName(what, width, height)));
      expect(render < display && display < hash, `${String(width)}x${String(height)}: ${String(render)} < ${String(display)} < ${String(hash)}`).toBe(true);
    }
    expect(MEASURED).toStrictEqual(["render", "render and display", "render and hash"]);
  });

  test("refuse a cost not proportional to the pixels, and a size not measured, naming it", () => {
    const linear = SIZES.flatMap(([width, height]) => [
      { name: sizeName("render", width, height), value: 100 * width * height, unit: "instructions" },
      { name: sizeName("render and display", width, height), value: 300 * width * height, unit: "instructions" },
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
    expect(linearityProblems(linear.filter(({ name }) => name !== "render and display 64x36"))).toStrictEqual(["render and display 64x36: not measured"]);
    // The display encoding's law holds from the second size: its smallest may cost more a
    // pixel, never the others.
    const display = (size: number, percent: number) =>
      linearityProblems(linear.map((entry) => (entry.name === sizeName("render and display", SIZES[size]?.[0] ?? 0, SIZES[size]?.[1] ?? 0) ? { ...entry, value: Math.round(entry.value * (1 + percent / 100)) } : entry)));
    expect(display(0, 10), "the smallest, not held").toStrictEqual([]);
    expect(display(1, LINEAR_PERCENT + 1)).toStrictEqual(["render and display 320x180: 309.00 instructions a pixel, 300.00 at the largest size: not proportional to its pixels within 2%"]);
    expect(LAW_FROM).toStrictEqual({ render: 0, "render and display": 1, "render and hash": 0 });
    expect(linearityProblems(linear.filter(({ name }) => name !== "render 1280x720"))).toStrictEqual(SIZES.map(([width, height]) => `${sizeName("render", width, height)}: not measured`));
  });

  test("refuse a placement costing otherwise at a size, and a size not measured, naming it", () => {
    const placed = (values: readonly number[]) => SIZES.map(([width, height], at) => ({ name: sizeName(PLACEMENT, width, height), value: values[at] ?? 0, unit: "instructions" }));
    expect(placementProblems(placed([681, 681, 681, 681]))).toStrictEqual([]);
    expect(placementProblems(placed([682, 681, 680, 681]))).toStrictEqual([
      "placement 64x36: 682 instructions, 681 at the largest size: an image's placement costs the same at every size",
      "placement 640x360: 680 instructions, 681 at the largest size: an image's placement costs the same at every size",
    ]);
    expect(placementProblems(placed([681, 681, 681, 681]).filter(({ name }) => name !== "placement 320x180"))).toStrictEqual(["placement 320x180: not measured"]);
    expect(placementProblems(placed([681, 681, 681, 681]).filter(({ name }) => name !== "placement 1280x720"))).toStrictEqual(SIZES.map(([width, height]) => `${sizeName(PLACEMENT, width, height)}: not measured`));
    // Every law, the linearity's first.
    const linear = SIZES.flatMap(([width, height]) => MEASURED.map((what) => ({ name: sizeName(what, width, height), value: 100 * width * height, unit: "instructions" })));
    expect(visualLaws([...linear, ...placed([681, 681, 681, 682])])).toStrictEqual(SIZES.slice(0, -1).map(([width, height]) => `${sizeName(PLACEMENT, width, height)}: 681 instructions, 682 at the largest size: an image's placement costs the same at every size`));
    expect(visualLaws(placed([681, 681, 681, 681]))[0]).toBe("render 64x36: not measured");
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
    expect(() => measure(rendering)).toThrow(new Error("render and display 64x36: no function visual_display"));
    const displaying = assemble({
      functions: [
        { params: 7, result: true, locals: [0], code: [0x41, 0x01, 0x0b], exported: "visual_setup" },
        { params: 2, result: true, locals: [0], code: [0x41, 0x00, 0x0b], exported: "visual_render" },
        { params: 2, result: true, locals: [0], code: [0x41, 0x00, 0x0b], exported: "visual_display" },
      ],
    });
    expect(() => measure(displaying)).toThrow(new Error("render and hash 64x36: no function visual_hash"));
    const hashing = assemble({
      functions: [
        { params: 7, result: true, locals: [0], code: [0x41, 0x01, 0x0b], exported: "visual_setup" },
        { params: 2, result: true, locals: [0], code: [0x41, 0x00, 0x0b], exported: "visual_render" },
        { params: 2, result: true, locals: [0], code: [0x41, 0x00, 0x0b], exported: "visual_display" },
        { params: 3, result: true, locals: [0], code: [0x41, 0x00, 0x0b], exported: "visual_hash" },
      ],
    });
    expect(() => measure(hashing)).toThrow(new Error("placement 64x36: no function visual_placement"));
  });

  test("are the visual module's, compared within their tolerance and checked by their law", () => {
    expect(VISUAL_BENCHMARKS).toMatchObject({
      baselineFile: "spec/visual-benchmark-baseline.json",
      moduleFile: `bindings/visual-wasm/dist/${VISUAL_MODULE.file}`,
      buildCommand: "pnpm build:wasm, in bindings/visual-wasm",
      command: "node scripts/benchmarks.ts",
      tolerancePercent: TOLERANCE_PERCENT,
      measure,
      laws: visualLaws,
    });
    expect([TOLERANCE_PERCENT, LINEAR_PERCENT]).toStrictEqual([2, 2]);
    // The name the committed baseline holds for the module's size (the benchmark command
    // compares every name of it with the measurements).
    expect(MODULE_NAME).toBe("visual module");
  });
});
