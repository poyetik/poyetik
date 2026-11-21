import { configDefaults, defineConfig } from "vitest/config";

// 100% everywhere: CI fails on any regression. Typed linting starts a TypeScript project
// per run: the policy test lints every fixture in one run, and may take a few seconds.
export default defineConfig({
  test: {
    // Refuse `.only` locally too: Vitest allows it unless `CI` is set (see test/runner.test.ts).
    allowOnly: false,
    include: ["test/**/*.test.ts"],
    // The lint policy's fixtures are files to lint, named as test code on purpose (a sleep in a
    // `.test.ts` file), never tests to run.
    exclude: [...configDefaults.exclude, "test/policy/**"],
    testTimeout: 60_000,
    coverage: {
      provider: "v8",
      include: ["eslint.ts", "one-line.ts", "playwright.ts"],
      reporter: ["text", "json-summary", "html"],
      thresholds: { lines: 100, branches: 100, functions: 100, statements: 100 },
    },
  },
});
