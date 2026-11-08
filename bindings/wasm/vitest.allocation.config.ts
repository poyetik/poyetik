import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";

// The allocation tests, run without coverage or mutation instrumentation: both allocate
// on their own, which would make "no garbage collection" unmeasurable. `--expose-gc`
// lets a test start from a fully collected heap. Two projects: V8 with every tier, and V8
// with its interpreter only, where a fresh audio thread runs before anything is optimized.
export default defineConfig({
  resolve: { alias: { "#worklet/processors": fileURLToPath(new URL("ts/worklet/processors.ts", import.meta.url)) } },
  test: {
    projects: [
      {
        extends: true,
        test: { name: "every tier", include: ["test/allocation/render.alloc.ts", "test/allocation/processor.alloc.ts"], pool: "forks", execArgv: ["--expose-gc"] },
      },
      {
        extends: true,
        test: {
          name: "interpreter",
          include: ["test/allocation/processor.alloc.ts", "test/allocation/unaligned.alloc.ts"],
          pool: "forks",
          execArgv: ["--expose-gc", "--no-opt", "--no-maglev", "--no-sparkplug"],
        },
      },
    ],
  },
});
