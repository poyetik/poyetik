// The real outside world of the build commands, on real files in a temporary directory
// and a real child process: what the fakes of build-scripts.test.ts stand in for.
import { existsSync, fstatSync, readdirSync, realpathSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, expect, test, vi } from "vitest";
import { PROCESS_BENCH_IO, PROCESS_ENTRY, PROCESS_IO } from "../src/io.ts";

afterEach(() => {
  vi.restoreAllMocks();
});

test("is this process: its environment, its user's home and the repository root, without a trailing slash", () => {
  expect(PROCESS_IO.env).toBe(process.env);
  expect(PROCESS_IO.home).toBe(homedir());
  expect(PROCESS_IO.repository).toBe(resolve(import.meta.dirname, "../../.."));
  expect(PROCESS_IO.repository.endsWith("/")).toBe(false);
});

test("works on real files: a fresh directory, nested directories, a copy, a read, a removal", () => {
  const dir = PROCESS_IO.temporaryDirectory("poyetik-io-");
  try {
    expect(dir.startsWith(join(tmpdir(), "poyetik-io-"))).toBe(true);
    expect(readdirSync(dir)).toStrictEqual([]);
    const nested = join(dir, "a", "b");
    PROCESS_IO.makeDirectory(nested);
    // Again: an existing directory is not an error.
    PROCESS_IO.makeDirectory(nested);
    writeFileSync(join(dir, "from.bin"), Uint8Array.of(1, 2, 3));
    PROCESS_IO.copy(join(dir, "from.bin"), join(nested, "to.bin"));
    expect(PROCESS_IO.read(join(nested, "to.bin"))).toStrictEqual(Buffer.from([1, 2, 3]));
  } finally {
    PROCESS_IO.remove(dir);
  }
  expect(existsSync(dir)).toBe(false);
  // Removing what is already gone is not an error.
  PROCESS_IO.remove(dir);
});

test("runs a command in its directory, with its environment, and returns its stdout as text", () => {
  const dir = PROCESS_IO.temporaryDirectory("poyetik-io-");
  try {
    const script = "process.stdout.write(process.cwd() + ' ' + process.env.POYETIK_IO_TEST)";
    const output = PROCESS_IO.run(process.execPath, ["-e", script], { cwd: dir, env: { POYETIK_IO_TEST: "seen" } });
    expect(output).toBe(`${realpathSync(dir)} seen`);
  } finally {
    PROCESS_IO.remove(dir);
  }
});

test("gives the command no input, and shows its stderr as it comes: this process's own", () => {
  // The child describes its stdin and its stderr; inherited, its stderr is this process's.
  const script = "const { fstatSync, statSync } = require('node:fs'); const input = fstatSync(0); const nul = statSync('/dev/null'); process.stdout.write(JSON.stringify([input.isCharacterDevice() && input.rdev === nul.rdev, fstatSync(2).ino]))";
  const output = PROCESS_IO.run(process.execPath, ["-e", script], { cwd: tmpdir(), env: {} });
  expect(output).toBe(JSON.stringify([true, fstatSync(2).ino]));
});

test("throws what the command's failure is", () => {
  expect(() => PROCESS_IO.run(process.execPath, ["-e", "process.exit(3)"], { cwd: tmpdir(), env: {} })).toThrow(/Command failed/);
});

test("writes whole lines on stdout and stderr", () => {
  const out = vi.spyOn(process.stdout, "write").mockReturnValue(true);
  const err = vi.spyOn(process.stderr, "write").mockReturnValue(true);
  PROCESS_IO.out("built");
  PROCESS_IO.err("error: failed");
  expect(out.mock.calls).toStrictEqual([["built\n"]]);
  expect(err.mock.calls).toStrictEqual([["error: failed\n"]]);
});

test("is this process's entry: its stdout, its stderr, and its exit code", () => {
  expect(PROCESS_ENTRY.stdout).toBe(process.stdout);
  expect(PROCESS_ENTRY.err).toBe(PROCESS_IO.err);
  const before = process.exitCode;
  try {
    PROCESS_ENTRY.exit(3);
    expect(process.exitCode).toBe(3);
  } finally {
    process.exitCode = before;
  }
});

test("is this process's files and output for a benchmark command, a write replacing the file", () => {
  expect([PROCESS_BENCH_IO.repository, PROCESS_BENCH_IO.read, PROCESS_BENCH_IO.out, PROCESS_BENCH_IO.err]).toStrictEqual([PROCESS_IO.repository, PROCESS_IO.read, PROCESS_IO.out, PROCESS_IO.err]);
  const dir = PROCESS_IO.temporaryDirectory("poyetik-io-");
  try {
    const file = join(dir, "baseline.json");
    PROCESS_BENCH_IO.write(file, "old");
    PROCESS_BENCH_IO.write(file, "{}\n");
    expect(new TextDecoder().decode(PROCESS_IO.read(file))).toBe("{}\n");
  } finally {
    PROCESS_IO.remove(dir);
  }
});
