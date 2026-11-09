// The real outside world of the benchmark command (see cli.ts): this process and its files.
// Tested on real files.
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import type { BenchIo } from "./cli.ts";

/** The real outside world. */
export const BENCH_IO: BenchIo = {
  repository: fileURLToPath(new URL("../../../", import.meta.url)).replace(/\/$/, ""),
  read: (path) => readFileSync(path),
  write: (path, text) => {
    writeFileSync(path, text);
  },
  out: (line) => process.stdout.write(`${line}\n`),
  err: (line) => process.stderr.write(`${line}\n`),
};
