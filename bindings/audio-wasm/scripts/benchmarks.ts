/** Entry point of `pnpm benchmarks:check` and of the bless `cargo xtask bless-benchmarks` runs: the benchmark command (see benchmarks/cli.ts) on the real process and files. */
import { benchMain } from "../benchmarks/cli.ts";
import { BENCH_IO } from "../benchmarks/io.ts";
import { ignoreClosedReader } from "@poyetik/wasm-build/cli";

ignoreClosedReader(process.stdout);
process.exitCode = benchMain(BENCH_IO, process.argv.slice(2));
