/** Entry point of `pnpm benchmarks:check` and `pnpm benchmarks:bless`: the benchmark command (see benchmarks/cli.ts) on the real process and files. */
import { benchMain } from "../benchmarks/cli.ts";
import { BENCH_IO } from "../benchmarks/io.ts";
import { ignoreClosedReader } from "./cli.ts";

ignoreClosedReader(process.stdout);
process.exitCode = benchMain(BENCH_IO, process.argv.slice(2));
