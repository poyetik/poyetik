// The visual conformance cases rendered by the real WebAssembly module, built into dist/ by
// `pnpm build:wasm`: every case of conformance/visual/cases must give the image hash of
// conformance/visual/references.json, which the native renderer (visual-cli) recorded.
// Same bits natively and in WebAssembly.
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { parseVisualCase } from "@poyetik/oracle/visual-inputs";
import { expect, test } from "vitest";
import { VisualModule } from "../ts/visual.ts";

/** The visual conformance directory, found by walking up: the tests also run from a copy of the package, deeper in the repository (Stryker's sandbox). */
function visualConformance(): URL {
  for (let directory = new URL("./", import.meta.url); directory.pathname !== "/"; directory = new URL("../", directory)) {
    const candidate = new URL("conformance/visual/", directory);
    if (existsSync(candidate)) {
      return candidate;
    }
  }
  throw new Error("conformance/visual not found");
}

/** The visual conformance directory. */
const CONFORMANCE = visualConformance();

/** The module, as `pnpm build:wasm` built it. */
const MODULE = new URL("../dist/visual.wasm", import.meta.url);

test("gives every case its reference hash, as the native renderer does", async () => {
  const references: unknown = JSON.parse(readFileSync(new URL("references.json", CONFORMANCE), "utf8"));
  const names = readdirSync(new URL("cases/", CONFORMANCE)).filter((name) => name.endsWith(".json"));
  expect(names.length).toBeGreaterThanOrEqual(4);
  for (const file of names) {
    const name = file.slice(0, -".json".length);
    const read = parseVisualCase(readFileSync(new URL(`cases/${file}`, CONFORMANCE), "utf8"), name);
    // A fresh instance per case, as the native renderer starts each case afresh.
    const module = await VisualModule.instantiate(readFileSync(MODULE));
    expect(module.setup({ width: read.width, height: read.height, rate: read.rate, side: read.square.side, cycle: read.square.cycle }), name).toBe(true);
    expect(module.hash(read.first, read.images), name).toBe(Reflect.get(Object(references), name));
  }
  expect(typeof references === "object" && references !== null ? Object.keys(references).sort() : []).toStrictEqual(names.map((file) => file.slice(0, -".json".length)).sort());
});

test("renders nothing and hashes zeros before a square is set up, and keeps its square when refused", async () => {
  const module = await VisualModule.instantiate(readFileSync(MODULE));
  expect(module.render(3n)).toHaveLength(0);
  expect(module.hash(0n, 2)).toBe("0".repeat(64));
  expect(module.setup({ width: 3, height: 2, rate: [60, 1], side: 65_536, cycle: [1, 2] })).toBe(true);
  const before = module.render(1n);
  expect(before).toHaveLength(3 * 2 * 4);
  expect(module.setup({ width: 0, height: 2, rate: [60, 1], side: 65_536, cycle: [1, 2] })).toBe(false);
  expect(module.render(1n)).toStrictEqual(before);
});

test("sets up the largest image and refuses one pixel row more, as every target does", async () => {
  const module = await VisualModule.instantiate(readFileSync(MODULE));
  const square = { rate: [60, 1], side: 65_536, cycle: [1, 2] } as const;
  expect(module.setup({ ...square, width: 16_384, height: 2049 })).toBe(false);
  expect(module.setup({ ...square, width: 16_384, height: 2048 })).toBe(true);
  expect(module.render(0n)).toHaveLength(16_384 * 2048 * 4);
  expect(module.setup({ ...square, width: 2048, height: 16_384 })).toBe(true);
  expect(module.render(1n)).toHaveLength(2048 * 16_384 * 4);
});

test("releases the image it replaces before allocating the next one", async () => {
  const { instance } = await WebAssembly.instantiate(readFileSync(MODULE), {});
  const memory: unknown = instance.exports["memory"];
  if (!(memory instanceof WebAssembly.Memory)) {
    throw new Error("no memory");
  }
  const module = new VisualModule(instance.exports);
  const square = { rate: [60, 1], side: 65_536, cycle: [1, 2] } as const;
  const image = 16_384 * 2048 * 4 * 4;
  expect(module.setup({ ...square, width: 16_384, height: 2048 })).toBe(true);
  expect(module.setup({ ...square, width: 2048, height: 16_384 })).toBe(true);
  expect(memory.buffer.byteLength, "one image of 512 MiB at a time, not two").toBeLessThan(image + image / 2);
});
