import { defineConfig } from "astro/config";

// Static site: board.json is read and validated at build time, no client-side JS.
export default defineConfig({
  // Stylesheets as files, never inlined: the Content Security Policy allows no inline style.
  build: { format: "file", inlineStylesheets: "never" },
  devToolbar: { enabled: false },
  server: { port: 4321 },
});
