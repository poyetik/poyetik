/** Entry point of `pnpm build:wasm`: builds the visual module into dist/visual.wasm (see `@poyetik/wasm-build`). */
import { fileURLToPath } from "node:url";
import { buildMain, runEntryPoint } from "@poyetik/wasm-build/cli";
import { PROCESS_ENTRY, PROCESS_IO } from "@poyetik/wasm-build/io";
import { VISUAL_MODULE } from "./module.ts";

runEntryPoint(PROCESS_ENTRY, () => buildMain(PROCESS_IO, process.argv.slice(2), fileURLToPath(new URL("../dist/", import.meta.url)), [VISUAL_MODULE]));
