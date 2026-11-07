import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [react()],
  // The engine processors run in an AudioWorklet: bundled as one ES module.
  worker: { format: "es" },
  test: {
    environment: "jsdom",
    include: ["test/**/*.test.{ts,tsx}"],
    setupFiles: ["test/setup.ts"],
    // 100% everywhere: CI fails on any regression.
    coverage: {
      provider: "v8",
      include: ["src/**/*.{ts,tsx}"],
      reporter: ["text", "json-summary", "html"],
      thresholds: { lines: 100, branches: 100, functions: 100, statements: 100 },
    },
  },
});
