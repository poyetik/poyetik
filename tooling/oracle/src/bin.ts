/** Entry point: `node src/bin.ts <conformance dir> <dir of exported WAV files>`, or `node src/bin.ts help`. */

import { readFileSync, readdirSync } from "node:fs";
import { ignoreClosedReader, main } from "./cli.ts";

ignoreClosedReader(process.stdout);
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
