import { readFileSync } from "node:fs";
import { assemble } from "@poyetik/wasm-build/testing/assemble";
import { describe, expect, test } from "vitest";
import { VISUAL_MODULE } from "../scripts/module.ts";
import { LAW_FROM, LINEAR_PERCENT, linearityProblems, MEASURED, measure, MODULE_NAME, PLACEMENT, placementProblems, SIZES, sizeName, TOLERANCE_PERCENT, VISUAL_BENCHMARKS, visualLaws } from "../benchmarks/suite.ts";

/** The visual module as `pnpm build:wasm` built it. */
function built(): Uint8Array<ArrayBuffer> {
  return new Uint8Array(readFileSync(new URL(`../dist/${VISUAL_MODULE.file}`, import.meta.url)));
}

/** The names of the measurements, in their order. */
const NAMES = [...SIZES.flatMap(([width, height]) => [...MEASURED.map((what) => sizeName(what, width, height)), sizeName(PLACEMENT, width, height)]), MODULE_NAME];

/**
 * The export of the module each benchmark counts, by the measurement that counts it: the work
 * done for every image a page shows.
 */
const COUNTED: Readonly<Record<string, (typeof MEASURED)[number] | typeof PLACEMENT>> = {
  visual_render: "render",
  visual_display: "render and display",
  visual_hash: "render and hash",
  visual_placement: PLACEMENT,
};

/** Every other export of the module, with why no benchmark counts it: none does work for every image. */
const UNCOUNTED: Readonly<Record<string, string>> = {
  memory: "the module's memory, no function",
  visual_abi_version: "a constant, read once when a driver is created",
  visual_setup: "once per square set up, not per image (its cost is the image's allocation)",
  visual_message_ptr: "where the last refusal's message is, read once after a refused setup",
  visual_message_len: "the length of that message, read once after a refused setup",
  visual_values_len: "a stored length, read after each render to check it",
  visual_display_len: "a stored length, read after each display to check it",
  visual_shader_ptr: "where a shader's source is, read once when a GPU drawing is set up",
  visual_shader_len: "the length of that source, read once when a GPU drawing is set up",
  visual_thresholds: "where the display shader's thresholds are, read once when a GPU drawing is set up",
  visual_encode_values: "the room for values to encode apart from the square, which no page shows: the display corpus's tests",
  visual_encode: "the encoding of those values, which no page shows: the display corpus's tests",
};

describe("the visual benchmarks", () => {
  test("count every export doing work for each image, and say why each other is not counted", () => {
    const wasm = built();
    const exports = WebAssembly.Module.exports(new WebAssembly.Module(wasm)).map(({ name }) => name).sort();
    expect(exports, "every export, counted or not, once").toStrictEqual([...Object.keys(COUNTED), ...Object.keys(UNCOUNTED)].sort());
    const names = measure(wasm).map(({ name }) => name);
    for (const [exported, what] of Object.entries(COUNTED)) {
      expect(SIZES.map(([width, height]) => sizeName(what, width, height)).filter((name) => !names.includes(name)), `${exported}: ${what} at every size, none missing`).toStrictEqual([]);
    }
  });

  test("measure every size of the ladder, the same counts every time, proportional to the pixels", () => {
    const wasm = built();
    const first = measure(wasm);
    expect(first.map(({ name }) => name)).toStrictEqual(NAMES);
    expect(first.map(({ unit }) => unit)).toStrictEqual([...NAMES.slice(0, -1).map(() => "instructions"), "bytes"]);
    expect(first.at(-1)?.value).toBe(wasm.length);
    expect(first.filter(({ value }) => !(Number.isSafeInteger(value) && value > 0))).toStrictEqual([]);
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
    // Finding K16-7 of the sixth deep review of visual block 2 (#34): 2.00002% above was
    // refused as 88.76 a pixel against 87.02, 1.9995% as printed. Exact in integers, at the
    // bound on each side and one instruction beyond, the counts said.
    const exact = (count: number, largest = 80_200_000) =>
      linearityProblems([
        { name: "render 320x180", value: count, unit: "instructions" },
        { name: "render 1280x720", value: largest, unit: "instructions" },
      ]).filter((problem) => problem.startsWith("render 320x180"));
    expect(exact(5_112_750)).toStrictEqual([]);
    expect(exact(4_912_250)).toStrictEqual([]);
    const beyond = (count: string) => [`render 320x180: ${count} instructions for 57600 pixels, 80200000 for 921600 at the largest size: not proportional to its pixels within 2%`];
    expect(exact(5_112_751)).toStrictEqual(beyond("5112751"));
    expect(exact(4_912_249)).toStrictEqual(beyond("4912249"));
    // Finding MI-1 of the seventh: exactly 2% above, where a quotient of floats reads beyond.
    expect(exact(5_112_801, 80_200_800)).toStrictEqual([]);
    expect(exact(5_112_802, 80_200_800)).toStrictEqual(["render 320x180: 5112802 instructions for 57600 pixels, 80200800 for 921600 at the largest size: not proportional to its pixels within 2%"]);
    expect(at(LINEAR_PERCENT), "at the bound").toStrictEqual([]);
    expect(at(LINEAR_PERCENT + 1)).toStrictEqual(["render 64x36: 237312 instructions for 2304 pixels, 92160000 for 921600 at the largest size: not proportional to its pixels within 2%"]);
    expect(at(-(LINEAR_PERCENT + 1))).toStrictEqual(["render 64x36: 223488 instructions for 2304 pixels, 92160000 for 921600 at the largest size: not proportional to its pixels within 2%"]);
    expect(linearityProblems(linear.filter(({ name }) => name !== "render and hash 640x360"))).toStrictEqual(["render and hash 640x360: not measured"]);
    expect(linearityProblems(linear.filter(({ name }) => name !== "render and display 64x36"))).toStrictEqual(["render and display 64x36: not measured"]);
    // The display encoding's law holds from the second size: its smallest may cost more a
    // pixel, never the others.
    const display = (size: number, percent: number) =>
      linearityProblems(linear.map((entry) => (entry.name === sizeName("render and display", SIZES[size]?.[0] ?? 0, SIZES[size]?.[1] ?? 0) ? { ...entry, value: Math.round(entry.value * (1 + percent / 100)) } : entry)));
    expect(display(0, 10), "the smallest, not held").toStrictEqual([]);
    expect(display(1, LINEAR_PERCENT + 1)).toStrictEqual(["render and display 320x180: 17798400 instructions for 57600 pixels, 276480000 for 921600 at the largest size: not proportional to its pixels within 2%"]);
    expect(LAW_FROM).toStrictEqual({ render: 0, "render and display": 1, "render and hash": 0 });
    // The largest size missing: it alone is named, the others measured and compared with nothing.
    expect(linearityProblems(linear.filter(({ name }) => name !== "render 1280x720"))).toStrictEqual(["render 1280x720: not measured"]);
  });

  test("refuse a placement costing otherwise at a size, and a size not measured, naming it", () => {
    const placed = (values: readonly number[]) => SIZES.map(([width, height], at) => ({ name: sizeName(PLACEMENT, width, height), value: values[at] ?? 0, unit: "instructions" }));
    expect(placementProblems(placed([681, 681, 681, 681]))).toStrictEqual([]);
    expect(placementProblems(placed([682, 681, 680, 681]))).toStrictEqual([
      "placement 64x36: 682 instructions, 681 at the largest size: an image's placement costs the same at every size",
      "placement 640x360: 680 instructions, 681 at the largest size: an image's placement costs the same at every size",
    ]);
    expect(placementProblems(placed([681, 681, 681, 681]).filter(({ name }) => name !== "placement 320x180"))).toStrictEqual(["placement 320x180: not measured"]);
    expect(placementProblems(placed([681, 681, 681, 681]).filter(({ name }) => name !== "placement 1280x720")), "the largest alone named").toStrictEqual(["placement 1280x720: not measured"]);
    expect(placementProblems(placed([681, 681, 681, 681]).filter(({ name }) => name !== "placement 1280x720" && name !== "placement 320x180")), "each missing size").toStrictEqual(["placement 320x180: not measured", "placement 1280x720: not measured"]);
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
      packageName: "@poyetik/visual-wasm",
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
