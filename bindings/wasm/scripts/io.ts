// The real outside world of the build commands (see cli.ts): this process, its files and
// cargo. Wiring only, shared by the two entry points.
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import type { Io } from "./cli.ts";

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
  temporaryDirectory: (prefix) => mkdtempSync(join(tmpdir(), prefix)),
  remove: (path) => {
    rmSync(path, { recursive: true, force: true });
  },
  out: (line) => process.stdout.write(`${line}\n`),
  err: (line) => process.stderr.write(`${line}\n`),
};
