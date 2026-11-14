// Allocation tests: run alone (pnpm test:allocation), without the coverage or mutation
// instrumentation, which allocates by itself and would hide or fake the result.
import { expect, test } from "vitest";
import { WasmEngine } from "../../ts/engine.ts";
import { SINE_PATCH, wasmBytes } from "../fixtures.ts";
import { collections, repeat } from "./collections.ts";

test("the collection counter sees garbage: a loop allocating 64 MB collects", async () => {
  let kept: number[] = [];
  expect(
    await collections(() => {
      repeat(1_000_000, () => {
        kept = [kept.length, 1, 2, 3, 4, 5, 6, 7];
      });
    }),
  ).toBeGreaterThan(0);
  expect(kept).toHaveLength(8);
});

test.each(["binary64-as-binary32", "binary32"] as const)("rendering in %s does not allocate: no garbage collection over 500 000 renders", async (format) => {
  const engine = WasmEngine.create(new WebAssembly.Module(wasmBytes()), SINE_PATCH, 48_000, format);
  const outputs = [new Float32Array(128), new Float32Array(128)];
  // Warm up first: compiling the code allocates once.
  repeat(10_000, () => {
    engine.render(outputs);
  });
  expect(
    await collections(() => {
      repeat(500_000, () => {
        engine.render(outputs);
      });
    }),
  ).toBe(0);
  expect(engine.blocks).toBe(1_020_000);
});
