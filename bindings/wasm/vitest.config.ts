import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";

// 100% everywhere: CI fails on any regression.
export default defineConfig({
  // The worklet module is imported through this alias: statically, so that it is
  // instrumented like every other module, and without the worklet globals being
  // type-checked in the test project.
  resolve: { alias: { "#worklet/processors": fileURLToPath(new URL("ts/worklet/processors.ts", import.meta.url)) } },
  test: {
    include: ["test/**/*.test.ts"],
    coverage: {
      provider: "v8",
      // Everything but the wiring shells, which only connect covered code to the real
      // process, files, network or browser: scripts/build-wasm.ts, scripts/reproducible.ts,
      // scripts/io.ts, e2e/serve.ts and e2e/site/harness.ts (run by pnpm build,
      // test:reproducible and test:e2e).
      include: ["ts/**/*.ts", "e2e/routes.ts", "scripts/wasm.ts", "scripts/cli.ts"],
      reporter: ["text", "json-summary", "html"],
      thresholds: { lines: 100, branches: 100, functions: 100, statements: 100 },
    },
  },
});
