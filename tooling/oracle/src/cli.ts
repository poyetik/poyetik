/** The `oracle` command, with its file access injected. */

import { join } from "node:path";
import { judge } from "./oracle.ts";

/** Usage of the command. */
export const USAGE = "usage: oracle <conformance dir> <dir of exported WAV files>";

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

/** Whether there are exactly two arguments. */
function isPair(args: readonly string[]): args is readonly [string, string] {
  return args.length === 2;
}

/** Runs the oracle; returns the exit code: 0 when every case passes. */
export function main(args: readonly string[], io: Io): number {
  if (!isPair(args)) {
    io.err(USAGE);
    return 2;
  }
  const [conformance, wavs] = args;
  try {
    const json = (path: string): unknown => JSON.parse(io.readText(path));
    const verdict = judge({
      cases: byName(io, join(conformance, "cases"), ".json", json),
      references: json(join(conformance, "references.json")),
      expectations: json(join(conformance, "expectations.json")),
      wavs: byName(io, wavs, ".wav", io.readBytes),
    });
    verdict.lines.forEach(io.out);
    if (verdict.failures > 0) {
      io.err(`oracle failed: ${String(verdict.failures)} problem(s)`);
      return 1;
    }
    return 0;
  } catch (error) {
    io.err(error instanceof Error ? error.message : String(error));
    return 1;
  }
}
