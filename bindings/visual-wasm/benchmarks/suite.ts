// The cost benchmarks of the visual library's module (spec/benchmarks.md, "Visual library"):
// the WebAssembly instructions the CPU reference executes to render one image of the square,
// to render it and encode it for display, and to render and hash it, at each size of a ladder of image sizes,
// the instructions of placing the square in an image (what a GPU drawing it asks for every
// image), and the module's size.
// Counted by the shared instrumenter (@poyetik/wasm-build), compared with
// spec/visual-benchmark-baseline.json by the shared command; the laws are the visual library's:
// the cost of an image is proportional to its pixels, and its placement's does not depend on
// its size.
import { type Benchmarks, call, countedInstance, exact, forBenchmark, type Measurement } from "@poyetik/wasm-build/bench";
import { instrument } from "@poyetik/wasm-build/instrument";

/**
 * The image sizes measured, smallest first, each four times the pixels of the one before
 * (both sides doubled, 16:9): from a thumbnail to HD, the largest an instrumented render
 * and hash measure in well under a second.
 */
export const SIZES: readonly (readonly [number, number])[] = [
  [64, 36],
  [320, 180],
  [640, 360],
  [1280, 720],
];

/**
 * The image measured, the same in every run: the work of an image may depend on where the
 * square is in it (the display encoding's does, on its edges: {@link LAW_FROM}), so that
 * the counts are those of this one image.
 */
export const IMAGE = 7;

/**
 * The square measured: 60 images a second, a side of 39 322 / 65 536 of the height (the
 * matrix's), one sweep per 1001/1000 s, so that the image holds edges, inside and outside.
 */
export const SQUARE = { rate: [60, 1], side: 39_322, cycle: [1001, 1000] } as const;

/**
 * How far the cost per pixel at a size may be from the largest size's, in percent: the cost
 * of an image is proportional to its pixels, plus what an image costs once (setting the
 * render up), which is below 1.4% of the smallest size's (measured 2026-11-16: 88.19 against
 * 87.04 instructions a pixel for the render, 1373.82 against 1369.99 for render and hash).
 */
export const LINEAR_PERCENT = 2;

/**
 * The first size of {@link SIZES} each measurement's law holds from: every size, but the
 * display encoding from the second. Its cost per pixel depends on the image: a value of 0 or
 * 1 (outside or inside the square) is encoded at once, an edge pixel's partial coverage
 * through a bucket and a comparison or two, and the edges' share of the pixels falls as
 * 1 / height (the measured costs a pixel: `spec/benchmarks.md`, "The law: an image costs its
 * pixels"; the counts: `spec/visual-benchmark-baseline.json`).
 */
export const LAW_FROM: Readonly<Record<(typeof MEASURED)[number], number>> = { render: 0, "render and display": 1, "render and hash": 0 };

/** The tolerance of the comparison with the baseline, in percent: counts have no noise, so this decides which change needs a reviewed bless (spec/benchmarks.md). */
export const TOLERANCE_PERCENT = 2;

/** The name of the module's size in the reports and the baseline. */
export const MODULE_NAME = "visual module";

/** What is measured at each size: the render alone, then with the display encoding, then with the hash. */
export const MEASURED = ["render", "render and display", "render and hash"] as const;

/** What is measured at each size besides {@link MEASURED}: the square's placement in the image, whose cost the size does not change. */
export const PLACEMENT = "placement";

/** The name of a measurement of `what` at `width` × `height`. */
export function sizeName(what: (typeof MEASURED)[number] | typeof PLACEMENT, width: number, height: number): string {
  return `${what} ${String(width)}x${String(height)}`;
}

/**
 * Measures the visual module `wasm`: at each size, in a fresh instance with the square set
 * up (not counted), the instructions of rendering image {@link IMAGE}, of rendering it and
 * encoding it for display, then of rendering and hashing it, then of placing the square in
 * it ({@link PLACEMENT}, `visual_placement`); and the module's size, before
 * instrumentation.
 * @throws {Error} naming the benchmark that could not be measured.
 */
export function measure(wasm: Uint8Array): Measurement[] {
  const module = new WebAssembly.Module(instrument(wasm));
  const measurements: Measurement[] = [];
  for (const [width, height] of SIZES) {
    const render = sizeName("render", width, height);
    const { exports, counted } = forBenchmark(render, () => {
      const instance = countedInstance(module);
      const { rate, side, cycle } = SQUARE;
      if (call(instance.exports, "visual_setup", width, height, rate[0], rate[1], side, cycle[0], cycle[1]) !== 1) {
        throw new Error("the module refused the square");
      }
      return instance;
    });
    const rendered = forBenchmark(render, () => exact(counted(() => call(exports, "visual_render", IMAGE, 0))));
    const displayed = forBenchmark(sizeName("render and display", width, height), () => exact(counted(() => call(exports, "visual_display", IMAGE, 0))));
    const hashed = forBenchmark(sizeName("render and hash", width, height), () => exact(counted(() => call(exports, "visual_hash", IMAGE, 0, 1))));
    const placed = forBenchmark(sizeName(PLACEMENT, width, height), () => exact(counted(() => call(exports, "visual_placement", IMAGE, 0))));
    measurements.push(
      { name: render, value: rendered, unit: "instructions" },
      { name: sizeName("render and display", width, height), value: displayed, unit: "instructions" },
      { name: sizeName("render and hash", width, height), value: hashed, unit: "instructions" },
      { name: sizeName(PLACEMENT, width, height), value: placed, unit: "instructions" },
    );
  }
  measurements.push({ name: MODULE_NAME, value: wasm.length, unit: "bytes" });
  return measurements;
}

/**
 * The law of the measurements: for each of {@link MEASURED}, the cost per
 * pixel at every size from {@link LAW_FROM}'s within {@link LINEAR_PERCENT} of the largest
 * size's. One problem a size that breaks it, or a size not measured, naming only the sizes
 * missing (the largest missing, the others are compared with nothing).
 */
export function linearityProblems(measurements: readonly Measurement[]): string[] {
  const values = new Map(measurements.map(({ name, value }) => [name, value]));
  const problems: string[] = [];
  for (const what of MEASURED) {
    const perPixel = SIZES.map(([width, height]) => {
      const value = values.get(sizeName(what, width, height));
      return value === undefined ? undefined : value / (width * height);
    });
    // The largest size not measured is named once, at its own size, and the others compare
    // with nothing: every comparison with NaN is false.
    const largest = perPixel.at(-1) ?? Number.NaN;
    SIZES.forEach(([width, height], at) => {
      const cost = perPixel[at];
      if (cost === undefined) {
        problems.push(`${sizeName(what, width, height)}: not measured`);
      } else if (at >= LAW_FROM[what] && Math.abs(cost - largest) * 100 > largest * LINEAR_PERCENT) {
        problems.push(`${sizeName(what, width, height)}: ${cost.toFixed(2)} instructions a pixel, ${largest.toFixed(2)} at the largest size: not proportional to its pixels within ${String(LINEAR_PERCENT)}%`);
      }
    });
  }
  return problems;
}

/**
 * The law of the placement: the square's placement in an image costs the same at every
 * size, the largest's ({@link PLACEMENT}: a few integer operations on its edges, whatever
 * the pixels; measured 2026-11-20: 681 instructions at every size of {@link SIZES} and at
 * 1×1 and 16 384×2048, for image {@link IMAGE}). One problem a size that costs otherwise, or
 * a size not measured, naming only the sizes missing.
 */
export function placementProblems(measurements: readonly Measurement[]): string[] {
  const values = new Map(measurements.map(({ name, value }) => [name, value]));
  const counts = SIZES.map(([width, height]) => values.get(sizeName(PLACEMENT, width, height)));
  const largest = counts.at(-1);
  return SIZES.flatMap(([width, height], at) => {
    const count = counts[at];
    if (count === undefined) {
      return [`${sizeName(PLACEMENT, width, height)}: not measured`];
    }
    // The largest size not measured is named once, at its own size: nothing to compare with.
    return largest === undefined || count === largest ? [] : [`${sizeName(PLACEMENT, width, height)}: ${String(count)} instructions, ${String(largest)} at the largest size: an image's placement costs the same at every size`];
  });
}

/** Every law of the visual benchmarks: {@link linearityProblems}, then {@link placementProblems}. */
export function visualLaws(measurements: readonly Measurement[]): string[] {
  return [...linearityProblems(measurements), ...placementProblems(measurements)];
}

/** The visual library's benchmarks, for the shared command. */
export const VISUAL_BENCHMARKS: Benchmarks = {
  baselineFile: "spec/visual-benchmark-baseline.json",
  moduleFile: "bindings/visual-wasm/dist/visual.wasm",
  packageName: "@poyetik/visual-wasm",
  tolerancePercent: TOLERANCE_PERCENT,
  measure,
  laws: visualLaws,
};
