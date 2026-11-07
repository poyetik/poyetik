/** Entry point: `node src/bin.ts <conformance dir> <dir of exported WAV files>`. */

import { readFileSync, readdirSync } from "node:fs";
import { main } from "./cli.ts";

process.exitCode = main(process.argv.slice(2), {
  readText: (path) => new TextDecoder().decode(readFileSync(path)),
  readBytes: (path) => readFileSync(path),
  list: (dir) => readdirSync(dir),
  out: (line) => {
    process.stdout.write(`${line}\n`);
  },
  err: (line) => {
    process.stderr.write(`${line}\n`);
  },
});
