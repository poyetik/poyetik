import { defineConfig } from "vitest/config";

// 100% everywhere: CI fails on any regression.
export default defineConfig({
  test: {
    // Refuse `.only` locally too: Vitest allows it unless `CI` is set (see test/runner.test.ts).
    allowOnly: false,
    include: ["test/**/*.test.ts"],
    coverage: {
      provider: "v8",
      // Everything but the entry points scripts/build-wasm.ts and scripts/reproducible.ts,
      // which only wire measured code to the real process (pnpm build and
      // test:reproducible run them), each reviewed in proof-bypasses.txt.
      include: ["ts/**/*.ts", "scripts/module.ts", "benchmarks/**/*.ts"],
      reporter: ["text", "json-summary", "html"],
      thresholds: { lines: 100, branches: 100, functions: 100, statements: 100 },
    },
  },
});
