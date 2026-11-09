import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";

// 100% everywhere: CI fails on any regression.
export default defineConfig({
  // The worklet module is imported through this alias: statically, so that it is
  // instrumented like every other module, and without the worklet globals being
  // type-checked in the test project.
  resolve: { alias: { "#worklet/processors": fileURLToPath(new URL("ts/worklet/processors.ts", import.meta.url)) } },
  test: {
    // Refuse `.only` locally too: Vitest allows it unless `CI` is set (see test/runner.test.ts).
    allowOnly: false,
    include: ["test/**/*.test.ts"],
    coverage: {
      provider: "v8",
      // Everything but the wiring shells, which only connect covered code to the real
      // process or browser: the entry points scripts/build-wasm.ts and scripts/reproducible.ts,
      // the bench server e2e/serve.ts and the bench page e2e/site/harness.ts (run by pnpm
      // build, test:reproducible and test:e2e), each reviewed in proof-bypasses.txt.
      include: ["ts/**/*.ts", "benchmarks/**/*.ts", "e2e/routes.ts", "e2e/site/fetch.ts", "scripts/wasm.ts", "scripts/cli.ts", "scripts/io.ts"],
      reporter: ["text", "json-summary", "html"],
      thresholds: { lines: 100, branches: 100, functions: 100, statements: 100 },
    },
  },
});
