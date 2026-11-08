import { defineConfig } from "vitest/config";

// 100% everywhere: CI fails on any regression. Typed linting starts a TypeScript project
// per run: the policy test lints every fixture in one run, and may take a few seconds.
export default defineConfig({
  test: {
    include: ["test/**/*.test.ts"],
    testTimeout: 60_000,
    coverage: {
      provider: "v8",
      include: ["eslint.ts"],
      reporter: ["text", "json-summary", "html"],
      thresholds: { lines: 100, branches: 100, functions: 100, statements: 100 },
    },
  },
});
