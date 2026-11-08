// Allocation tests: run alone (pnpm test:allocation), without the coverage or mutation
// instrumentation, which allocates by itself and would hide or fake the result.
import { PerformanceObserver } from "node:perf_hooks";
import { expect, test } from "vitest";
import { WasmEngine } from "../../ts/engine.ts";
import { SINE_PATCH, wasmBytes } from "../fixtures.ts";

/**
 * Counts the garbage collections that `action` causes. Collections are reported after
 * the fact, so the count is read after the queued reports are delivered.
 */
async function collections(action: () => void): Promise<number> {
  // Start from a fully collected heap, with the report of that collection delivered:
  // garbage left by earlier code must not be collected during `action`.
  const collect: unknown = Reflect.get(globalThis, "gc");
  if (typeof collect !== "function") {
    throw new Error("run with --expose-gc (pnpm test:allocation)");
  }
  Reflect.apply(collect, undefined, []);
  for (let turn = 0; turn < 3; turn += 1) {
    await new Promise((resolve) => setImmediate(resolve));
  }
  let count = 0;
  const observer = new PerformanceObserver((list) => {
    count += list.getEntries().length;
  });
  observer.observe({ entryTypes: ["gc"] });
  action();
  for (let turn = 0; turn < 3; turn += 1) {
    await new Promise((resolve) => setImmediate(resolve));
  }
  count += observer.takeRecords().length;
  observer.disconnect();
  return count;
}

/** Calls `action` `times` times. */
function repeat(times: number, action: () => void): void {
  for (let index = 0; index < times; index += 1) {
    action();
  }
}

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

test("rendering does not allocate: no garbage collection over 500 000 renders", async () => {
  const engine = WasmEngine.create(new WebAssembly.Module(wasmBytes()), SINE_PATCH, 48_000);
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
