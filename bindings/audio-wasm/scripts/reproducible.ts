/** Entry point of `pnpm test:reproducible`: checks that the engine module is reproducible (see `@poyetik/wasm-build`). */
import { ignoreClosedReader, reproducibleMain } from "@poyetik/wasm-build/cli";
import { PROCESS_IO } from "@poyetik/wasm-build/io";
import { ENGINE_MODULE } from "./module.ts";

ignoreClosedReader(process.stdout);
process.exitCode = reproducibleMain(PROCESS_IO, ENGINE_MODULE);
