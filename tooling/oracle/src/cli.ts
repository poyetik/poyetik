/** The `oracle` command, with its file access injected. */

import { join } from "node:path";
import { oneLine } from "@poyetik/config/one-line";
import { judge } from "./oracle.ts";

/** Usage of the command, printed on stdout by `node src/bin.ts help`, run in tooling/oracle. */
export const USAGE = "usage: node src/bin.ts <conformance dir> <dir of exported WAV files>\n       node src/bin.ts help\n(in tooling/oracle)";

/** Largest case file, in bytes: 64 KiB, as `audio-cli` reads it (`MAX_CASE_BYTES`). */
export const MAX_CASE_BYTES = 64 * 1024;

/** Largest `references.json`, in bytes: 1 MiB, as `audio-cli` reads it (`MAX_REFERENCES_BYTES`). */
export const MAX_REFERENCES_BYTES = 1024 * 1024;

/** Largest `expectations.json`, in bytes: 1 MiB, the limit of `references.json`, which lists the same cases. */
export const MAX_EXPECTATIONS_BYTES = 1024 * 1024;

/**
 * Largest WAV file, in bytes: what a RIFF header can declare, its 32-bit size field
 * (2^32 − 1) plus the 8 bytes of the `RIFF` tag and that field; a larger file cannot be a
 * valid WAV file.
 */
export const MAX_WAV_BYTES = 2 ** 32 + 7;

/** What the command needs from the outside world. */
export interface Io {
  /**
   * The bytes of the regular file at `path`, of at most `limit` bytes, reading no more
   * than one byte past it; throws when it cannot be read or is larger (the message names it).
   */
  readonly readBytes: (path: string, limit: number) => Uint8Array;
  /** The names of the entries of the directory `dir`, in any order; throws when it cannot be listed. */
  readonly list: (dir: string) => readonly string[];
  /** Prints one line on stdout: a verdict or the usage. */
  readonly out: (line: string) => void;
  /** Prints one line on stderr: an `error:` line, the only thing the oracle writes there. */
  readonly err: (line: string) => void;
}

/**
 * What a case name may hold, as `audio-cli` reads it (`conformance/README.md`), so that both
 * readers of the corpus accept the same files.
 */
const CASE_NAME = /^[a-z0-9][a-z0-9.-]*$/u;

/**
 * The case files of `dir`, by name, each read by `read`.
 * @throws {Error} Naming the first file whose name is not a case name.
 */
function caseFiles(io: Io, dir: string, read: (path: string) => string): Map<string, string> {
  const files = byName(io, dir, ".json", (path) => path);
  for (const [name, path] of files) {
    if (!CASE_NAME.test(name)) {
      throw new Error(`${path}: not a case name (lowercase ASCII letters, digits, dots and hyphens, starting with a letter or a digit)`);
    }
  }
  return new Map([...files].map(([name, path]) => [name, read(path)]));
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
 * Reads the text file at `path`, of at most `limit` bytes, as strict UTF-8: a byte that is
 * not UTF-8 is refused, never replaced, as the engine's own reader refuses it.
 * @throws {Error} Naming `path` when the file cannot be read, is larger than `limit` or is
 * not UTF-8, with the decoder's error as its cause.
 */
function readText(io: Io, path: string, limit: number): string {
  const bytes = io.readBytes(path, limit);
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch (error) {
    throw new Error(`${path}: ${String(error)}`, { cause: error });
  }
}

/**
 * Reads and parses the JSON file at `path`, of at most `limit` bytes, as strict UTF-8 (see
 * {@link readText}).
 * @throws {Error} Naming `path` when the file cannot be read, is larger than `limit`, or is
 * not UTF-8 or not JSON, with the decoder's or the parser's error as its cause.
 */
export function readJson(io: Io, path: string, limit: number): unknown {
  const text = readText(io, path, limit);
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
 * usage. Every verdict and error line is made safe by `oneLine`: what it quotes from the
 * corpus (a case name, a JSON key, a chunk id, the JSON parser's excerpt of a file) can
 * neither drive the terminal nor forge a line, and is bounded.
 */
export function main(args: readonly string[], io: Io): number {
  if (args.length === 1 && args[0] === "help") {
    io.out(USAGE);
    return 0;
  }
  if (!isPair(args)) {
    io.err(`error: expected <conformance dir> <dir of exported WAV files>, got ${String(args.length)} argument(s) (see: node src/bin.ts help)`);
    return 2;
  }
  const [conformance, wavs] = args;
  try {
    const verdict = judge({
      cases: caseFiles(io, join(conformance, "cases"), (path) => readText(io, path, MAX_CASE_BYTES)),
      references: readJson(io, join(conformance, "references.json"), MAX_REFERENCES_BYTES),
      expectations: readJson(io, join(conformance, "expectations.json"), MAX_EXPECTATIONS_BYTES),
      wavs: byName(io, wavs, ".wav", (path) => io.readBytes(path, MAX_WAV_BYTES)),
    });
    for (const line of verdict.lines) {
      io.out(oneLine(line));
    }
    if (verdict.failures > 0) {
      io.err(`error: ${oneLine(`oracle failed on ${conformance} against ${wavs}: ${String(verdict.failures)} problem(s)`)}`);
      return 1;
    }
    return 0;
  } catch (error) {
    io.err(`error: ${oneLine(error instanceof Error ? error.message : String(error))}`);
    return 1;
  }
}

/** A stream, as far as its errors go. */
interface Failing {
  /** Registers `listener` for the stream's `error` events (its result is not used). */
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
