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
      include: ["ts/**/*.ts", "e2e/routes.ts"],
      reporter: ["text", "json-summary", "html"],
      thresholds: { lines: 100, branches: 100, functions: 100, statements: 100 },
    },
  },
});
