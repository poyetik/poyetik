// The cost benchmarks of the visual library's module (spec/benchmarks.md, "Visual library"):
// the WebAssembly instructions the CPU reference executes to render one image of the square,
// to render it and encode it for display, and to render and hash it, at each size of a ladder of image sizes, and the module's size.
// Counted by the shared instrumenter (@poyetik/wasm-build), compared with
// spec/visual-benchmark-baseline.json by the shared command; the law is the video one: the
// cost of an image is proportional to its pixels.
import { type Benchmarks, countedInstance, exact, forBenchmark, type Measurement } from "@poyetik/wasm-build/bench";
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

/** The image rendered: any index gives the same work, the square's place being exact. */
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
 * 1 / height (measured 2026-11-18: 215.8 instructions a pixel at 64×36, 209.9 at 320×180,
 * 208.8 at 1280×720 for the render and display: 3.4%, then 0.5%, from the largest; the baseline holds the counts, `spec/visual-benchmark-baseline.json`).
 */
export const LAW_FROM: Readonly<Record<(typeof MEASURED)[number], number>> = { render: 0, "render and display": 1, "render and hash": 0 };

/** The tolerance of the comparison with the baseline, in percent: counts have no noise, so this decides which change needs a reviewed bless (spec/benchmarks.md). */
export const TOLERANCE_PERCENT = 2;

/** The name of the module's size in the reports and the baseline. */
export const MODULE_NAME = "visual module";

/** What is measured at each size: the render alone, then with the display encoding, then with the hash. */
export const MEASURED = ["render", "render and display", "render and hash"] as const;

/** The name of a measurement of `what` at `width` × `height`. */
export function sizeName(what: (typeof MEASURED)[number], width: number, height: number): string {
  return `${what} ${String(width)}x${String(height)}`;
}

/**
 * Calls the export `name` of `exports` with `args` and returns what it returns.
 * @throws {TypeError} when it is not a function.
 */
function call(exports: Readonly<Record<string, unknown>>, name: string, ...args: readonly number[]): unknown {
  const exported = exports[name];
  if (typeof exported !== "function") {
    throw new TypeError(`no function ${name}`);
  }
  return Reflect.apply(exported, undefined, args);
}

/**
 * Measures the visual module `wasm`: at each size, in a fresh instance with the square set
 * up (not counted), the instructions of rendering image {@link IMAGE}, of rendering it and
 * encoding it for display, then of rendering and hashing it; and the module's size, before
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
    measurements.push(
      { name: render, value: rendered, unit: "instructions" },
      { name: sizeName("render and display", width, height), value: displayed, unit: "instructions" },
      { name: sizeName("render and hash", width, height), value: hashed, unit: "instructions" },
    );
  }
  measurements.push({ name: MODULE_NAME, value: wasm.length, unit: "bytes" });
  return measurements;
}

/**
 * The law of the measurements: for each of {@link MEASURED}, the cost per
 * pixel at every size from {@link LAW_FROM}'s within {@link LINEAR_PERCENT} of the largest
 * size's. One problem a size that breaks it, or a size not measured.
 */
export function linearityProblems(measurements: readonly Measurement[]): string[] {
  const values = new Map(measurements.map(({ name, value }) => [name, value]));
  const problems: string[] = [];
  for (const what of MEASURED) {
    const perPixel = SIZES.map(([width, height]) => {
      const value = values.get(sizeName(what, width, height));
      return value === undefined ? undefined : value / (width * height);
    });
    const largest = perPixel.at(-1);
    SIZES.forEach(([width, height], at) => {
      const cost = perPixel[at];
      if (at < LAW_FROM[what] && cost !== undefined) {
        return;
      }
      if (cost === undefined || largest === undefined) {
        problems.push(`${sizeName(what, width, height)}: not measured`);
      } else if (Math.abs(cost - largest) * 100 > largest * LINEAR_PERCENT) {
        problems.push(`${sizeName(what, width, height)}: ${cost.toFixed(2)} instructions a pixel, ${largest.toFixed(2)} at the largest size: not proportional to its pixels within ${String(LINEAR_PERCENT)}%`);
      }
    });
  }
  return problems;
}

/** The visual library's benchmarks, for the shared command. */
export const VISUAL_BENCHMARKS: Benchmarks = {
  baselineFile: "spec/visual-benchmark-baseline.json",
  moduleFile: "bindings/visual-wasm/dist/visual.wasm",
  buildCommand: "pnpm build:wasm, in bindings/visual-wasm",
  command: "node scripts/benchmarks.ts (in bindings/visual-wasm)",
  tolerancePercent: TOLERANCE_PERCENT,
  measure,
  laws: linearityProblems,
};
