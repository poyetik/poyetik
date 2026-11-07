import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { localDate, run } from "./cli.ts";

const file = process.env["ROADMAP_FILE"] ?? fileURLToPath(new URL("../../board.json", import.meta.url));

process.exitCode = run(process.argv.slice(2), {
  read: () => new TextDecoder().decode(readFileSync(file)),
  write: (text) => {
    writeFileSync(file, text);
  },
  out: (line) => {
    process.stdout.write(`${line}\n`);
  },
  err: (line) => {
    process.stderr.write(`${line}\n`);
  },
  today: () => localDate(new Date()),
});
