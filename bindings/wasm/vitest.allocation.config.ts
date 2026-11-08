import { defineConfig } from "vitest/config";

// The allocation tests, run without coverage or mutation instrumentation: both allocate
// on their own, which would make "no garbage collection" unmeasurable. `--expose-gc`
// lets a test start from a fully collected heap.
export default defineConfig({
  test: {
    include: ["test/allocation/**/*.alloc.ts"],
    pool: "forks",
    execArgv: ["--expose-gc"],
  },
});
