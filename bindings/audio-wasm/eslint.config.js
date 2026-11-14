import { strict } from "@poyetik/config/eslint";

// test/types/ holds compiler fixtures, some of them wrong on purpose (see test/types.test.ts).
export default strict(import.meta.dirname, ["dist", "test/types"]);
