/** The `oracle` command, with its file access injected. */

import { join } from "node:path";
import { judge } from "./oracle.ts";

/** Usage of the command, printed on stdout by `oracle help`. */
export const USAGE = "usage: oracle <conformance dir> <dir of exported WAV files>\n       oracle help";

/** What the command needs from the outside world. */
export interface Io {
  readonly readText: (path: string) => string;
  readonly readBytes: (path: string) => Uint8Array;
  readonly list: (dir: string) => readonly string[];
  readonly out: (line: string) => void;
  readonly err: (line: string) => void;
}

/** The files of `dir` with `extension`, by name without it. */
function byName<T>(io: Io, dir: string, extension: string, read: (path: string) => T): Map<string, T> {
  return new Map(
    io
      .list(dir)
      .filter((file) => file.endsWith(extension))
      .map((file) => [file.slice(0, -extension.length), read(join(dir, file))]),
  );
}

/**
 * Reads and parses the JSON file at `path`.
 * @throws {Error} Naming `path` when the file is not JSON, with the parser's error as its cause.
 */
export function readJson(io: Io, path: string): unknown {
  const text = io.readText(path);
  try {
    return JSON.parse(text);
  } catch (error) {
    throw new Error(`${path}: ${String(error)}`, { cause: error });
  }
}

/** Whether there are exactly two arguments. */
function isPair(args: readonly string[]): args is readonly [string, string] {
  return args.length === 2;
}

/**
 * Runs the oracle; returns the exit code: 0 when every case passes (or for `help`), 1
 * when a case fails or an input cannot be read, 2 for a usage error. Every error is one
 * `error:` line, naming the file, or for a usage error the reason and where to read the
 * usage.
 */
export function main(args: readonly string[], io: Io): number {
  if (args.length === 1 && args[0] === "help") {
    io.out(USAGE);
    return 0;
  }
  if (!isPair(args)) {
    io.err(`error: expected <conformance dir> <dir of exported WAV files>, got ${String(args.length)} argument(s) (see: oracle help)`);
    return 2;
  }
  const [conformance, wavs] = args;
  try {
    const json = (path: string): unknown => readJson(io, path);
    const verdict = judge({
      cases: byName(io, join(conformance, "cases"), ".json", json),
      references: json(join(conformance, "references.json")),
      expectations: json(join(conformance, "expectations.json")),
      wavs: byName(io, wavs, ".wav", io.readBytes),
    });
    verdict.lines.forEach(io.out);
    if (verdict.failures > 0) {
      io.err(`error: oracle failed: ${String(verdict.failures)} problem(s)`);
      return 1;
    }
    return 0;
  } catch (error) {
    io.err(`error: ${error instanceof Error ? error.message : String(error)}`);
    return 1;
  }
}

/**
 * Makes a closed `stream` (a reader gone, as with `| head`) harmless: what nobody reads
 * is dropped, and the exit code stays the outcome of the judgement. Any other stream
 * error is thrown, as it would be without this.
 */
export function ignoreClosedReader(stream: { on: (event: "error", listener: (error: Error) => void) => unknown }): void {
  stream.on("error", (error) => {
    if (!("code" in error) || error.code !== "EPIPE") {
      throw error;
    }
  });
}
