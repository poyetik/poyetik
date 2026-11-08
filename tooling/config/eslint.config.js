import { strict } from "./eslint.ts";

// The policy lints its own source and tests; the fixtures break it on purpose.
export default strict(import.meta.dirname, ["test/policy", "test/compiler"]);
