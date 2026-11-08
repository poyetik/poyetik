import { getViteConfig } from "astro/config";
import type { ViteUserConfig } from "vitest/config";

/**
 * The test options, apart so that a test can read them: Astro builds the configuration
 * itself. 100% coverage everywhere: CI fails on any regression.
 */
export const TESTS: NonNullable<ViteUserConfig["test"]> = {
  // Refuse `.only` locally too: Vitest allows it unless `CI` is set (see test/runner.test.ts).
  allowOnly: false,
  include: ["test/**/*.test.ts"],
  coverage: {
    provider: "v8",
    include: ["src/**/*.ts", "src/**/*.astro"],
    reporter: ["text", "json-summary", "html"],
    thresholds: { lines: 100, branches: 100, functions: 100, statements: 100 },
  },
};

export default getViteConfig({ test: TESTS });
