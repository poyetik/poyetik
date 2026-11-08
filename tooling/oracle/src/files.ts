/**
 * The oracle's access to the disk: files read with a size limit, never more than one byte
 * past it, and directories listed; every failure is an `Error` naming the path and its
 * cause in stable words (a system error code, never Node's sentence, which repeats the path).
 */

import { closeSync, constants, fstatSync, openSync, readdirSync, readSync } from "node:fs";

/** The size of each read: a case file is read in one, a WAV file in as many as it needs. */
const CHUNK_BYTES = 64 * 1024;

/** Why a file operation failed: a system error by its code (`ENOENT`, `EISDIR`), anything else by its message. */
function cause(error: unknown): string {
  const code: unknown = Reflect.get(Object(error), "code");
  return typeof code === "string" ? code : String(Reflect.get(Object(error), "message"));
}

/** Runs `action`, turning its failure into an `Error` saying what could not be done to `path` and why. */
function naming<T>(what: string, path: string, action: () => T): T {
  try {
    return action();
  } catch (error) {
    throw new Error(`cannot ${what} ${path}: ${cause(error)}`, { cause: error });
  }
}

/**
 * The bytes of the regular file at `path`, of at most `limit` bytes. Reads at most
 * `limit` + 1 bytes, whatever the file holds or claims: a file that grows while it is
 * read, or a huge one, is refused, never loaded whole; and anything but a regular file (a
 * directory, a FIFO that would block, a device that never ends) is refused before a read.
 * @throws {Error} Naming `path`, when it cannot be opened or read, is not a regular file,
 * or holds more than `limit` bytes.
 */
export function readBounded(path: string, limit: number): Uint8Array {
  return naming("read", path, () => {
    // Without waiting: opening a FIFO for reading would otherwise block until a writer comes.
    const descriptor = openSync(path, constants.O_RDONLY | constants.O_NONBLOCK);
    try {
      if (!fstatSync(descriptor).isFile()) {
        throw new Error("not a regular file");
      }
      const chunks: Uint8Array[] = [];
      let total = 0;
      for (;;) {
        const chunk = new Uint8Array(Math.min(CHUNK_BYTES, limit + 1 - total));
        const read = readSync(descriptor, chunk);
        if (read === 0) {
          break;
        }
        chunks.push(chunk.subarray(0, read));
        total += read;
        if (total > limit) {
          throw new Error(`larger than ${String(limit)} bytes`);
        }
      }
      const bytes = new Uint8Array(total);
      let at = 0;
      for (const chunk of chunks) {
        bytes.set(chunk, at);
        at += chunk.length;
      }
      return bytes;
    } finally {
      closeSync(descriptor);
    }
  });
}

/**
 * The names of the entries of the directory `dir`, in any order.
 * @throws {Error} Naming `dir`, when it cannot be listed.
 */
export function listNames(dir: string): string[] {
  return naming("list", dir, () => readdirSync(dir));
}
