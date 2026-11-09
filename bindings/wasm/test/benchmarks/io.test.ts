// The real outside world of the benchmark command, on real files in a temporary directory.
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, expect, test, vi } from "vitest";
import { BENCH_IO } from "../../benchmarks/io.ts";

afterEach(() => {
  vi.restoreAllMocks();
});

test("is the repository root, without a trailing slash", () => {
  expect(BENCH_IO.repository).toBe(resolve(import.meta.dirname, "../../../.."));
  expect(BENCH_IO.repository.endsWith("/")).toBe(false);
});

test("writes and reads real files, and throws when it cannot", () => {
  const dir = mkdtempSync(join(tmpdir(), "poyetik-benchmarks-"));
  try {
    const file = join(dir, "baseline.json");
    BENCH_IO.write(file, "{}\n");
    expect(readFileSync(file, "utf8")).toBe("{}\n");
    expect(BENCH_IO.read(file)).toStrictEqual(Buffer.from("{}\n"));
    expect(() => BENCH_IO.read(join(dir, "missing"))).toThrow(/ENOENT/);
    expect(() => {
      BENCH_IO.write(join(dir, "missing", "file"), "");
    }).toThrow(/ENOENT/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("prints lines on stdout and stderr", () => {
  const out = vi.spyOn(process.stdout, "write").mockImplementation(() => true);
  const err = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
  BENCH_IO.out("a");
  BENCH_IO.err("b");
  expect(out.mock.calls).toStrictEqual([["a\n"]]);
  expect(err.mock.calls).toStrictEqual([["b\n"]]);
});
