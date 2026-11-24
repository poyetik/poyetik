// Counting garbage collections, for the allocation tests: run without the coverage or
// mutation instrumentation, which allocates by itself and would hide or fake the result.
import { PerformanceObserver } from "node:perf_hooks";

/**
 * Counts the garbage collections that `action` causes. Collections are reported after
 * the fact, so the count is read after the queued reports are delivered.
 */
export async function collections(action: () => void): Promise<number> {
  // Start from a fully collected heap, with the report of that collection delivered:
  // garbage left by earlier code must not be collected during `action`.
  const collect: unknown = Reflect.get(globalThis, "gc");
  if (typeof collect !== "function") {
    throw new Error("run with --expose-gc (see: pnpm --filter @poyetik/audio-wasm test:allocation)");
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
export function repeat(times: number, action: () => void): void {
  for (let index = 0; index < times; index += 1) {
    action();
  }
}
