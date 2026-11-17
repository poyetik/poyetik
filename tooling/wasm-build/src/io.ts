// The real outside world of the build commands (see cli.ts): this process, its files and
// cargo. Shared by every entry point; tested on real files and a real child process.
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import type { BenchIo } from "./bench.ts";
import type { EntryProcess, Io } from "./cli.ts";

/** The real outside world. */
export const PROCESS_IO: Io = {
  env: process.env,
  home: homedir(),
  repository: fileURLToPath(new URL("../../../", import.meta.url)).replace(/\/$/, ""),
  run: (command, args, options) => execFileSync(command, args, { ...options, encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] }),
  read: (path) => readFileSync(path),
  makeDirectory: (path) => mkdirSync(path, { recursive: true }),
  copy: (from, to) => {
    copyFileSync(from, to);
  },
  temporaryRoot: tmpdir(),
  temporaryDirectory: (prefix) => mkdtempSync(join(tmpdir(), prefix)),
  remove: (path) => {
    rmSync(path, { recursive: true, force: true });
  },
  out: (line) => process.stdout.write(`${line}\n`),
  err: (line) => process.stderr.write(`${line}\n`),
};

/** This process as an entry point reports to it: its stdout, its stderr, its exit code. */
export const PROCESS_ENTRY: EntryProcess = {
  stdout: process.stdout,
  err: PROCESS_IO.err,
  exit: (code) => {
    process.exitCode = code;
  },
};

/** This process's files and output, as a benchmark command uses them. */
export const PROCESS_BENCH_IO: BenchIo = {
  repository: PROCESS_IO.repository,
  read: PROCESS_IO.read,
  write: (path, text) => {
    writeFileSync(path, text);
  },
  out: PROCESS_IO.out,
  err: PROCESS_IO.err,
};
