/** Entry point of `pnpm test:reproducible`: checks that the page module is reproducible (see `@poyetik/wasm-build`). */
import { reproducibleMain, runEntryPoint } from "@poyetik/wasm-build/cli";
import { PROCESS_ENTRY, PROCESS_IO } from "@poyetik/wasm-build/io";
import { PAGE_MODULE } from "./module.ts";

runEntryPoint(PROCESS_ENTRY, () => reproducibleMain(PROCESS_IO, process.argv.slice(2), [PAGE_MODULE]));
