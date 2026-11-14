import { defineConfig } from "vitest/config";

// 100% everywhere: CI fails on any regression.
export default defineConfig({
  test: {
    // Refuse `.only` locally too: Vitest allows it unless `CI` is set (see test/runner.test.ts).
    allowOnly: false,
    include: ["test/**/*.test.ts"],
    coverage: {
      provider: "v8",
      include: ["src/**/*.ts"],
      reporter: ["text", "json-summary", "html"],
      thresholds: { lines: 100, branches: 100, functions: 100, statements: 100 },
    },
  },
});
