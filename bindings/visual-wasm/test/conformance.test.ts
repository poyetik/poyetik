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
