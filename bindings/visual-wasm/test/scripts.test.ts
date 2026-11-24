// The command-line contract of this package's build and benchmark commands, on their real entry
// points run as processes: one `error:` line on stderr, exit 1 for a failure, 2 for a
// usage error, and a closed stdout never an error. Only the paths that stop before cargo
// runs are taken, so that no build starts; each process is bounded in time, since a
// synchronous spawn cannot be interrupted by the test's own timeout.
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { LIMIT_MS, scriptsOf } from "@poyetik/wasm-build/testing/scripts";
import { expect, test } from "vitest";
import { visualConformance } from "../ts/testing/conformance.ts";

/** This package's scripts. */
const { run, runClosed } = scriptsOf(new URL("../", import.meta.url));

test("refuses an argument to either command with a usage error and exit code 2", () => {
  expect(run("build-wasm.ts", ["--help"], {})).toStrictEqual({ status: 2, stdout: "", stderr: 'error: build:wasm takes no argument but help, not "--help" (see: pnpm build:wasm help)\n' });
  expect(run("reproducible.ts", ["now"], {})).toStrictEqual({ status: 2, stdout: "", stderr: 'error: test:reproducible takes no argument but help, not "now" (see: pnpm test:reproducible help)\n' });
  expect(run("build-wasm.ts", ["help"], {})).toMatchObject({ status: 0, stderr: "" });
  expect(run("benchmarks.ts", ["now"], {})).toStrictEqual({ status: 2, stdout: "", stderr: 'error: unknown command "now" (see: pnpm --filter @poyetik/visual-wasm exec node scripts/benchmarks.ts help)\n' });
  expect(run("benchmarks.ts", ["help"], {})).toMatchObject({ status: 0, stderr: "" });
});

/** The visual conformance directory, found by walking up: mutation testing runs the tests from a copy nested deeper. */
const CONFORMANCE = visualConformance(new URL("./", import.meta.url));

test.each([["the root", new URL("../../", CONFORMANCE)], ["below it", CONFORMANCE], ["the package", new URL("../", import.meta.url)]])("gives as its usage hint a command that runs from %s", (_where, directory) => {
  // The hint of the benchmark command's usage error, followed as written through a shell, from the
  // repository's root, below it, and from the package (finding K7-1 of the sixth deep review of
  // visual block 2 (#34): it ran from the package alone).
  const { stderr } = run("benchmarks.ts", ["now"], {});
  const hint = /\(see: (?<command>[^()]+)\)\n$/u.exec(stderr)?.groups?.["command"];
  expect(hint).toBe("pnpm --filter @poyetik/visual-wasm exec node scripts/benchmarks.ts help");
  const followed = spawnSync("sh", ["-c", hint ?? ""], { cwd: fileURLToPath(directory), encoding: "utf8", timeout: LIMIT_MS });
  expect({ status: followed.status, stderr: followed.stderr }).toStrictEqual({ status: 0, stderr: "" });
  expect(followed.stdout).toContain("usage: pnpm --filter @poyetik/visual-wasm exec node scripts/benchmarks.ts <check|bless|help>");
});

test("explains a failure in one line and exit code 1, never a stack trace", () => {
  expect(run("reproducible.ts", [], { TMPDIR: "/nonexistent-directory-of-the-test" })).toStrictEqual({
    status: 1,
    stdout: "",
    stderr: "error: cannot create a temporary directory in /nonexistent-directory-of-the-test: ENOENT\n",
  });
  expect(run("build-wasm.ts", [], { PATH: "" })).toStrictEqual({
    status: 1,
    stdout: "",
    stderr: "error: rustc --print sysroot could not be run: Error: spawnSync rustc ENOENT\n",
  });
});

test("takes a closed stdout as no error: the exit code stays the command's", async () => {
  for (const script of ["build-wasm.ts", "reproducible.ts", "benchmarks.ts"]) {
    expect(await runClosed(script, ["help"]), script).toStrictEqual({ status: 0, stderr: "" });
  }
});
