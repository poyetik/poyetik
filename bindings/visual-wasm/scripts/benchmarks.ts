/** Entry point of `pnpm benchmarks:check` and of the visual bless: measures the visual module (see `benchmarks/suite.ts`). */
import { benchMain } from "@poyetik/wasm-build/bench";
import { runEntryPoint } from "@poyetik/wasm-build/cli";
import { PROCESS_BENCH_IO, PROCESS_ENTRY } from "@poyetik/wasm-build/io";
import { VISUAL_BENCHMARKS } from "../benchmarks/suite.ts";

runEntryPoint(PROCESS_ENTRY, () => benchMain(PROCESS_BENCH_IO, process.argv.slice(2), VISUAL_BENCHMARKS));
