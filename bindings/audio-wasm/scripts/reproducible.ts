/** Entry point of `pnpm test:reproducible`: checks that the engine module is reproducible (see cli.ts). */
import { ignoreClosedReader, reproducibleMain } from "./cli.ts";
import { PROCESS_IO } from "./io.ts";

ignoreClosedReader(process.stdout);
process.exitCode = reproducibleMain(PROCESS_IO);
