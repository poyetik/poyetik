/** Entry point of `pnpm build:wasm`: builds the engine module into dist/engine.wasm (see cli.ts). */
import { fileURLToPath } from "node:url";
import { buildMain } from "./cli.ts";
import { PROCESS_IO } from "./io.ts";

process.exitCode = buildMain(PROCESS_IO, fileURLToPath(new URL("../dist/", import.meta.url)));
