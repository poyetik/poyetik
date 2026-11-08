import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { BoardError } from "../lib/board.ts";
import { ignoreClosedReader, localDate, run } from "./cli.ts";

/** Path of the board file: `ROADMAP_FILE` when set (tests, other boards), otherwise the package's own board.json. */
const file = process.env["ROADMAP_FILE"] ?? fileURLToPath(new URL("../../board.json", import.meta.url));

/** Runs a file operation, turning its failure into a message naming the file. */
function onFile<T>(what: string, action: () => T): T {
  try {
    return action();
  } catch (error) {
    throw new BoardError(`cannot ${what} ${file}: ${String(error)}`);
  }
}

ignoreClosedReader(process.stdout);
process.exitCode = run(process.argv.slice(2), {
  file,
  read: () => onFile("read", () => new TextDecoder().decode(readFileSync(file))),
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
