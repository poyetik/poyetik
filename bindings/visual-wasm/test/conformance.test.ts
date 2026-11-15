// The visual conformance cases rendered by the real WebAssembly module, built into dist/ by
// `pnpm build:wasm`: every case of conformance/visual/cases must give the image hash of
// conformance/visual/references.json, which the native renderer (visual-cli) recorded.
// Same bits natively and in WebAssembly.
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { parseVisualCase } from "@poyetik/oracle/visual-inputs";
import { expect, test } from "vitest";
import { VISUAL_MODULE } from "../scripts/module.ts";
import { VisualModule, VisualSetupError } from "../ts/visual.ts";

/**
 * The visual conformance directory, found by walking up from `from`: the tests also run
 * from a copy of the package, deeper in the repository (Stryker's sandbox).
 * @throws {Error} When no directory up to the root holds it.
 */
function visualConformance(from: URL = new URL(import.meta.url)): URL {
  for (let directory = new URL("./", from); ; directory = new URL("../", directory)) {
    const candidate = new URL("conformance/visual/", directory);
    if (existsSync(candidate)) {
      return candidate;
    }
    // The root, where going up changes nothing: `/` here, `/C:/` on Windows.
    if (new URL("../", directory).href === directory.href) {
      throw new Error("conformance/visual not found");
    }
  }
}

/** The visual conformance directory. */
const CONFORMANCE = visualConformance();

/** The module, as `pnpm build:wasm` built it. */
const MODULE = new URL(`../dist/${VISUAL_MODULE.file}`, import.meta.url);

test("gives every case its reference hash, as the native renderer does", async () => {
  const references: unknown = JSON.parse(readFileSync(new URL("references.json", CONFORMANCE), "utf8"));
  const names = readdirSync(new URL("cases/", CONFORMANCE)).filter((name) => name.endsWith(".json"));
  expect(names, "the eleven cases of the corpus").toHaveLength(11);
  for (const file of names) {
    const name = file.slice(0, -".json".length);
    const read = parseVisualCase(readFileSync(new URL(`cases/${file}`, CONFORMANCE), "utf8"), name);
    // A fresh instance per case, as the native renderer starts each case afresh.
    const module = await VisualModule.instantiate(readFileSync(MODULE), { width: read.width, height: read.height, rate: read.rate, side: read.square.side, cycle: read.square.cycle });
    expect(module.hash(read.first, read.images), name).toBe(Reflect.get(Object(references), name));
  }
  expect(typeof references === "object" && references !== null ? Object.keys(references).sort() : []).toStrictEqual(names.map((file) => file.slice(0, -".json".length)).sort());
});

/** The pairs of integers of `value`, or none when it is not a list of such pairs. */
function pairs(value: unknown): (readonly [number, number])[] | undefined {
  if (!Array.isArray(value)) {
    return undefined;
  }
  const read: (readonly [number, number])[] = [];
  for (const pair of value.values()) {
    const first: unknown = Reflect.get(Object(pair), 0);
    const second: unknown = Reflect.get(Object(pair), 1);
    if (!Array.isArray(pair) || pair.length !== 2 || typeof first !== "number" || typeof second !== "number" || !Number.isSafeInteger(first) || !Number.isSafeInteger(second)) {
      return undefined;
    }
    read.push([first, second]);
  }
  return read;
}

/**
 * The shared visual matrix (`conformance/visual/test-matrix.json`): its rates and sizes as
 * pairs, its indices read from the text, exact past 2^53.
 */
function visualMatrix(): { rates: (readonly [number, number])[]; sizes: (readonly [number, number])[]; indices: bigint[] } {
  const text = readFileSync(new URL("test-matrix.json", CONFORMANCE), "utf8");
  const json: unknown = JSON.parse(text);
  const rates = pairs(Reflect.get(Object(json), "image_rates"));
  const sizes = pairs(Reflect.get(Object(json), "image_sizes"));
  const indices = /"image_indices": \[(?<digits>[\d, ]+)\]/u.exec(text)?.groups?.["digits"]?.split(", ").map((digits) => BigInt(digits));
  if (rates === undefined || sizes === undefined || indices === undefined) {
    throw new Error("test-matrix.json is not the visual matrix");
  }
  return { rates, sizes, indices };
}

test("covers exactly the square's area across the visual matrix, in WebAssembly", async () => {
  // The exact law of antialiasing, as the box's own tests check it natively: the red values
  // sum to the area of the square, clipped to the width. Both are exact as numbers: every
  // value is a multiple of 2^-24 and the sum stays below 2^28; the edge in 4096ths of a
  // pixel is below 2^27.
  const side = 39_322;
  const { rates, sizes, indices } = visualMatrix();
  expect([rates.length, sizes.length, indices.length], "the whole matrix read").toStrictEqual([7, 6, 6]);
  expect(indices.at(-1), "exact past 2^53").toBe(18_446_744_073_709_551_615n);
  for (const rate of rates) {
    for (const [width, height] of sizes) {
      const module = await VisualModule.instantiate(readFileSync(MODULE), { width, height, rate, side, cycle: [1001, 1000] });
      const edge = Math.floor((height * 4096 * side) / 65_536);
      const area = (edge * Math.min(edge, width * 4096)) / 2 ** 24;
      for (const index of indices) {
        const values = module.render(index);
        let covered = 0;
        for (let red = 0; red < values.length; red += 4) {
          covered += values[red] ?? Number.NaN;
        }
        expect(covered, `${String(rate)} ${String(width)}x${String(height)} image ${String(index)}`).toBe(area);
      }
    }
  }
});

test("explains a refused square and keeps the one it had", async () => {
  const square = { rate: [60, 1], side: 65_536, cycle: [1, 2] } as const;
  await expect(VisualModule.instantiate(readFileSync(MODULE), { ...square, width: 0, height: 2 })).rejects.toThrow(new VisualSetupError("invalid image size 0x2: each side must be from 1 to 16384 pixels"));
  const module = await VisualModule.instantiate(readFileSync(MODULE), { ...square, width: 3, height: 2 });
  const before = module.render(1n);
  expect(before).toHaveLength(3 * 2 * 4);
  for (const [refused, reason] of [
    [{ ...square, width: 3, height: 2, side: 65_537 }, "invalid square side 65537: at most 65536, a square as tall as the image (a side is in 65536ths of the image height)"],
    [{ ...square, width: 3, height: 2, rate: [2, 4] }, "invalid image rate 2/4: a reduced fraction from 1 to 1000 images per second is required"],
    [{ ...square, width: 3, height: 2, cycle: [0, 1] }, "invalid duration 0/1 s: a reduced, nonzero fraction is required"],
  ] as const) {
    expect(() => { module.setup(refused); }, reason).toThrow(new VisualSetupError(reason));
    expect(module.render(1n), reason).toStrictEqual(before);
  }
});

test("sets up the largest image and refuses one pixel row more, as every target does", async () => {
  const square = { rate: [60, 1], side: 65_536, cycle: [1, 2] } as const;
  const module = await VisualModule.instantiate(readFileSync(MODULE), { ...square, width: 1, height: 1 });
  expect(() => { module.setup({ ...square, width: 16_384, height: 2049 }); }).toThrow(new VisualSetupError("invalid image size 16384x2049: an image holds at most 33554432 pixels"));
  module.setup({ ...square, width: 16_384, height: 2048 });
  expect(module.render(0n)).toHaveLength(16_384 * 2048 * 4);
  module.setup({ ...square, width: 2048, height: 16_384 });
  expect(module.render(1n)).toHaveLength(2048 * 16_384 * 4);
});

test("releases the image it replaces before allocating the next one", async () => {
  const { instance } = await WebAssembly.instantiate(readFileSync(MODULE), {});
  const memory: unknown = instance.exports["memory"];
  if (!(memory instanceof WebAssembly.Memory)) {
    throw new Error("no memory");
  }
  const square = { rate: [60, 1], side: 65_536, cycle: [1, 2] } as const;
  const module = new VisualModule(instance.exports, { ...square, width: 16_384, height: 2048 });
  const image = 16_384 * 2048 * 4 * 4;
  module.setup({ ...square, width: 2048, height: 16_384 });
  expect(memory.buffer.byteLength, "one image of 512 MiB at a time, not two").toBeLessThan(image + image / 2);
});

test("stops looking for the conformance directory at the root, a Windows drive's included", () => {
  for (const root of ["file:///", "file:///C:/", "file:///C:/Users/nobody/"]) {
    expect(() => visualConformance(new URL(root)), root).toThrow(new Error("conformance/visual not found"));
  }
});
