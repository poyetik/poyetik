/** Entry point of `pnpm test:reproducible`: checks that the visual module is reproducible (see `@poyetik/wasm-build`). */
import { ignoreClosedReader, reproducibleMain } from "@poyetik/wasm-build/cli";
import { PROCESS_IO } from "@poyetik/wasm-build/io";
import { VISUAL_MODULE } from "./module.ts";

ignoreClosedReader(process.stdout);
process.exitCode = reproducibleMain(PROCESS_IO, VISUAL_MODULE);
