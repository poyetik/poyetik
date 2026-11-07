/// <reference types="vitest/config" />
import { getViteConfig } from "astro/config";

// 100% everywhere: CI fails on any regression.
export default getViteConfig({
  test: {
    include: ["test/**/*.test.ts"],
    coverage: {
      provider: "v8",
      include: ["src/**/*.ts"],
      reporter: ["text", "json-summary", "html"],
      thresholds: { lines: 100, branches: 100, functions: 100, statements: 100 },
    },
  },
});
