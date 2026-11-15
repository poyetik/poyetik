/** Entry point of `pnpm test:reproducible`: checks that the visual module is reproducible (see `@poyetik/wasm-build`). */
import { ignoreClosedReader, reproducibleMain } from "@poyetik/wasm-build/cli";
import { PROCESS_IO } from "@poyetik/wasm-build/io";
import { VISUAL_MODULE } from "./module.ts";

ignoreClosedReader(process.stdout, (line) => {
  PROCESS_IO.err(line);
  process.exitCode = 1;
});
process.exitCode = reproducibleMain(PROCESS_IO, process.argv.slice(2), VISUAL_MODULE);
