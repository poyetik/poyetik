import { defineConfig } from "astro/config";

// Static site: board.json is read and validated at build time, no client-side JS.
export default defineConfig({
  build: { format: "file" },
  devToolbar: { enabled: false },
  server: { port: 4321 },
});
