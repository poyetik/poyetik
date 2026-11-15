/** Entry point of `pnpm test:reproducible`: checks that the visual module is reproducible (see `@poyetik/wasm-build`). */
import { guardStdout, reproducibleMain } from "@poyetik/wasm-build/cli";
import { PROCESS_IO } from "@poyetik/wasm-build/io";
import { VISUAL_MODULE } from "./module.ts";

guardStdout(process.stdout, PROCESS_IO.err, (code) => {
  process.exitCode = code;
});
process.exitCode = reproducibleMain(PROCESS_IO, process.argv.slice(2), VISUAL_MODULE);
