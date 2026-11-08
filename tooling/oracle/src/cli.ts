/** The `oracle` command, with its file access injected. */

import { join } from "node:path";
import { judge } from "./oracle.ts";

/** Usage of the command, printed on stdout by `oracle help`. */
export const USAGE = "usage: oracle <conformance dir> <dir of exported WAV files>\n       oracle help";

/** What the command needs from the outside world. */
export interface Io {
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
 * Reads and parses the JSON file at `path`, as strict UTF-8: a byte that is not UTF-8 is
 * refused, never replaced, as the engine's own reader refuses it.
 * @throws {Error} Naming `path` when the file is not UTF-8 or not JSON, with the decoder's
 * or the parser's error as its cause.
 */
export function readJson(io: Io, path: string): unknown {
  const bytes = io.readBytes(path);
  try {
    return JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
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

/** A stream, as far as its errors go. */
interface Failing {
  readonly on: (event: "error", listener: (error: Error) => void) => unknown;
}

/**
 * Applies the stream part of the command-line contract, as the Rust tools do:
 * - a closed `stdout` (a reader gone, as with `| head`) is never an error: what nobody
 *   reads is dropped, and the exit code stays the outcome of the judgement;
 * - any other `stdout` error is one `error:` line on `stderr`, then `failed` (which
 *   makes the exit code tell a failure), never a stack trace;
 * - a broken `stderr` is ignored: nothing more can be said, and the exit code still tells.
 */
export function guardStreams(stdout: Failing, stderr: Failing & { readonly write: (text: string) => unknown }, failed: () => void): void {
  stderr.on("error", () => {
    // Nothing to do: the exit code still tells.
  });
  stdout.on("error", (error) => {
    const code = "code" in error && typeof error.code === "string" ? error.code : undefined;
    if (code === "EPIPE") {
      return;
    }
    stderr.write(`error: cannot write to stdout: ${code ?? error.message}\n`);
    failed();
  });
}
