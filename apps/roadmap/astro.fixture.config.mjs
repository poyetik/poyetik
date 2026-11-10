import { fileURLToPath } from "node:url";
import base from "./astro.config.mjs";

// The page built from the end-to-end fixture board (e2e/fixture-board.json) instead of
// board.json, into its own directory: every rendering branch of a column is reached whatever
// the published data holds. The production configuration and page are unchanged, so no
// setting can make the published page show the fixture.
export default {
  ...base,
  outDir: "./dist-fixture",
  vite: {
    resolve: {
      alias: [{ find: /^\.\.\/\.\.\/board\.json$/, replacement: fileURLToPath(new URL("./e2e/fixture-board.json", import.meta.url)) }],
    },
  },
};
