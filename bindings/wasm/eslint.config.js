import { strict } from "@poyetik/config/eslint";

export default strict(import.meta.dirname, ["dist", "e2e/site/harness.js"]);
