/** Entry point of `pnpm build:wasm`: builds the engine module into dist/engine.wasm (see `@poyetik/wasm-build`). */
import { fileURLToPath } from "node:url";
import { buildMain } from "@poyetik/wasm-build/cli";
import { PROCESS_IO } from "@poyetik/wasm-build/io";
import { ENGINE_MODULE } from "./module.ts";

process.exitCode = buildMain(PROCESS_IO, process.argv.slice(2), fileURLToPath(new URL("../dist/", import.meta.url)), ENGINE_MODULE);
