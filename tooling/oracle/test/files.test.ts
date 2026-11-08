import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type * as Fs from "node:fs";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { listNames, readBounded } from "../src/files.ts";

/** The size of every read `readBounded` asked for, recorded by the mocked `readSync` (hoisted with it). */
const asked: number[] = vi.hoisted(() => []);
/** The descriptors opened and not closed yet, recorded by the mocked `openSync` and `closeSync`. */
const open: Set<number> = vi.hoisted(() => new Set<number>());

vi.mock("node:fs", async (original) => {
  const fs = await original<typeof Fs>();
  return {
    ...fs,
    openSync: (path: string, flags: number): number => {
      const descriptor = fs.openSync(path, flags);
      open.add(descriptor);
      return descriptor;
    },
    closeSync: (descriptor: number): void => {
      open.delete(descriptor);
      fs.closeSync(descriptor);
    },
    readSync: (descriptor: number, buffer: Uint8Array): number => {
      asked.push(buffer.length);
      return fs.readSync(descriptor, buffer);
    },
  };
});

/** The temporary directory of the current test. */
let dir = "";

beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), "oracle-files-"));
  asked.length = 0;
});

afterEach(() => {
  rmSync(dir, { recursive: true, force: true });
  // Every file opened is closed, whether it was read or refused.
  expect(open.size).toBe(0);
});

/** A file of `size` bytes, each its index modulo 251, in the test's directory. */
function file(size: number): string {
  const path = join(dir, `${String(size)}.bin`);
  writeFileSync(path, Uint8Array.from({ length: size }, (_empty, index) => index % 251));
  return path;
}

test("reads a file up to its limit, whole and unchanged", () => {
  for (const size of [0, 1, 65_536, 200_000]) {
    const bytes = readBounded(file(size), 200_000);
    expect(bytes).toStrictEqual(new Uint8Array(Uint8Array.from({ length: size }, (_empty, index) => index % 251)));
  }
});

test("refuses a file one byte past its limit, reading no more than that byte", () => {
  const path = file(300_000);
  expect(() => readBounded(path, 100_000)).toThrow(new Error(`cannot read ${path}: larger than 100000 bytes`));
  expect(asked.reduce((sum, size) => sum + size, 0)).toBe(100_001);
  expect(Math.max(...asked)).toBe(65_536);
  const exact = file(100_001);
  expect(() => readBounded(exact, 100_000)).toThrow(new Error(`cannot read ${exact}: larger than 100000 bytes`));
  expect(() => readBounded(file(1), 0)).toThrow(/larger than 0 bytes$/);
});

test("refuses anything but a regular file before reading it", () => {
  const sub = join(dir, "sub");
  mkdirSync(sub);
  expect(() => readBounded(sub, 10)).toThrow(new Error(`cannot read ${sub}: not a regular file`));
  expect(asked).toStrictEqual([]);
});

test("names a missing file by its error code, not by Node's sentence", () => {
  const missing = join(dir, "missing.json");
  expect(() => readBounded(missing, 10)).toThrow(new Error(`cannot read ${missing}: ENOENT`));
  // The system error stays the cause.
  let thrown: unknown;
  try {
    readBounded(missing, 10);
  } catch (error) {
    thrown = error;
  }
  expect(Reflect.get(Object(Reflect.get(Object(thrown), "cause")), "code")).toBe("ENOENT");
  // A path through a file: its parent is not a directory, whatever the permissions.
  const through = join(file(1), "x");
  expect(() => readBounded(through, 10)).toThrow(new Error(`cannot read ${through}: ENOTDIR`));
});

test("lists a directory, or names it by its error code", () => {
  file(3);
  file(4);
  expect(listNames(dir).sort()).toStrictEqual(["3.bin", "4.bin"]);
  const missing = join(dir, "none");
  expect(() => listNames(missing)).toThrow(new Error(`cannot list ${missing}: ENOENT`));
});
