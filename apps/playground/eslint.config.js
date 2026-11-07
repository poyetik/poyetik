import { strict } from "@poyetik/config/eslint";
import reactHooks from "eslint-plugin-react-hooks";

// The shared strict policy, plus the rules of React hooks.
export default strict(import.meta.dirname, ["dist"], reactHooks.configs.flat["recommended-latest"]);
