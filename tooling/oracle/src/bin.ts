/** Entry point: `node src/bin.ts <conformance dir> <dir of exported WAV files>`, or `node src/bin.ts help`. */

import { guardStreams, main } from "./cli.ts";
import { listNames, readBounded } from "./files.ts";

guardStreams(process.stdout, process.stderr, () => {
  // Stream errors come after `main` has set the exit code: a 1 or a 2 already tells a
  // failure and is kept; only a success becomes one.
  if (process.exitCode === 0) {
    process.exitCode = 1;
  }
});
process.exitCode = main(process.argv.slice(2), {
  readBytes: readBounded,
  list: listNames,
  out: (line) => {
    process.stdout.write(`${line}\n`);
  },
  err: (line) => {
    process.stderr.write(`${line}\n`);
  },
});
