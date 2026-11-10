import { strict } from "@poyetik/config/eslint";

export default strict(import.meta.dirname, ["dist", "dist-fixture", ".astro"]);
