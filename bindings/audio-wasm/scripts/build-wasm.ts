/** Entry point of `pnpm build:wasm`: builds one engine module per engine block into dist/engine-<frames>.wasm, the default one also as dist/engine.wasm (see `@poyetik/wasm-build`). */
import { fileURLToPath } from "node:url";
import { buildMain } from "@poyetik/wasm-build/cli";
import { PROCESS_IO } from "@poyetik/wasm-build/io";
import { ENGINE_MODULES } from "./module.ts";

process.exitCode = buildMain(PROCESS_IO, process.argv.slice(2), fileURLToPath(new URL("../dist/", import.meta.url)), ENGINE_MODULES);
