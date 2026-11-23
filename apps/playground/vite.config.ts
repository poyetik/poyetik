import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";
import { withPolicy } from "./csp.ts";

export default defineConfig({
  plugins: [
    react(),
    // The built page declares its Content Security Policy. Not in development: the dev
    // server injects inline styles and scripts for hot reloading.
    {
      name: "content-security-policy",
      apply: "build",
      transformIndexHtml: withPolicy,
    },
  ],
  // The engine processors run in an AudioWorklet: bundled as one ES module.
  worker: { format: "es" },
  test: {
    // Refuse `.only` locally too: Vitest allows it unless `CI` is set (see test/runner.test.ts).
    allowOnly: false,
    environment: "jsdom",
    include: ["test/**/*.test.{ts,tsx}"],
    setupFiles: ["test/setup.ts"],
    // 100% everywhere: CI fails on any regression.
    coverage: {
      provider: "v8",
      include: ["src/**/*.{ts,tsx}", "csp.ts", "gates/**/*.ts"],
      reporter: ["text", "json-summary", "html"],
      thresholds: { lines: 100, branches: 100, functions: 100, statements: 100 },
    },
  },
});
