import { defineConfig } from "vitest/config";

// 100% everywhere: CI fails on any regression.
export default defineConfig({
  test: {
    // Refuse `.only` locally too: Vitest allows it unless `CI` is set (see test/runner.test.ts).
    allowOnly: false,
    include: ["test/**/*.test.ts"],
    // A guard against a hang, not a tolerance on what the tests check: each test's work is
    // fixed, and Stryker's instrumented dry run on a loaded machine can take it past
    // Vitest's 5 s default; 15 s (maintainer decision, 2026-11-16) keeps a hang bounded.
    // The tests that do more say their own, longer time.
    testTimeout: 15_000,
    coverage: {
      provider: "v8",
      include: ["src/**/*.ts"],
      reporter: ["text", "json-summary", "html"],
      thresholds: { lines: 100, branches: 100, functions: 100, statements: 100 },
    },
  },
});
