import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { BoardError } from "../lib/board.ts";
import { exitAfterStreamFailure, failure, guardStreams, localDate, run } from "./cli.ts";

/** The board file named by the environment, when `ROADMAP_FILE` is set (tests, other boards). */
const configured = process.env["ROADMAP_FILE"];

/** Path of the board file: `ROADMAP_FILE` when set, otherwise the package's own board.json. */
const file = configured ?? fileURLToPath(new URL("../../board.json", import.meta.url));

/** Runs a file operation, turning its failure into a message naming the file and the cause (see `failure`). */
function onFile<T>(what: string, action: () => T): T {
  try {
    return action();
  } catch (error) {
    throw new BoardError(`cannot ${what} ${file}: ${failure(error)}`);
  }
}

guardStreams(process.stdout, process.stderr, () => {
  process.exitCode = exitAfterStreamFailure(process.exitCode);
});
// An empty ROADMAP_FILE names no file: it was used as a path ("cannot read : ENOENT"). It
// is refused as what it is, a variable set by mistake, never taken for the default board.
if (configured === "") {
  process.stderr.write("error: ROADMAP_FILE is set but empty: unset it or name a board file\n");
  process.exitCode = 1;
} else {
  process.exitCode = run(process.argv.slice(2), {
    file,
    // Fatal decoding: a byte that is not UTF-8 is an error, never silently replaced by U+FFFD
    // and then written back by the next command.
    read: () => onFile("read", () => new TextDecoder("utf-8", { fatal: true }).decode(readFileSync(file))),
    write: (text) => {
      onFile("write", () => {
        writeFileSync(file, text);
      });
    },
    out: (line) => {
      process.stdout.write(`${line}\n`);
    },
    err: (line) => {
      process.stderr.write(`${line}\n`);
    },
    today: () => localDate(new Date()),
  });
}
