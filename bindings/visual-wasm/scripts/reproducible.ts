/** Entry point of `pnpm test:reproducible`: checks that the visual module is reproducible (see `@poyetik/wasm-build`). */
import { reproducibleMain, runEntryPoint } from "@poyetik/wasm-build/cli";
import { PROCESS_ENTRY, PROCESS_IO } from "@poyetik/wasm-build/io";
import { VISUAL_MODULE } from "./module.ts";

runEntryPoint(PROCESS_ENTRY, () => reproducibleMain(PROCESS_IO, process.argv.slice(2), VISUAL_MODULE));
