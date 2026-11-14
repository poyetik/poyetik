/** Entry point of `pnpm build:wasm`: builds the visual module into dist/visual.wasm (see `@poyetik/wasm-build`). */
import { fileURLToPath } from "node:url";
import { buildMain } from "@poyetik/wasm-build/cli";
import { PROCESS_IO } from "@poyetik/wasm-build/io";
import { VISUAL_MODULE } from "./module.ts";

process.exitCode = buildMain(PROCESS_IO, fileURLToPath(new URL("../dist/", import.meta.url)), VISUAL_MODULE);
