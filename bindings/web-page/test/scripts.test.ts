// The command-line contract of this package's two build commands, on their real entry
// points run as processes: one `error:` line on stderr, exit 1 for a failure, 2 for a
// usage error, and a closed stdout never an error. Only the paths that stop before cargo
// runs are taken, so that no build starts; each process is bounded in time, since a
// synchronous spawn cannot be interrupted by the test's own timeout.
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { expect, test } from "vitest";

/** The longest a process of these tests may run, in milliseconds: Node starts in well under a second. */
const LIMIT_MS = 30_000;

/** The path of the entry point `script` of this package. */
function entryPoint(script: string): string {
  return fileURLToPath(new URL(`../scripts/${script}`, import.meta.url));
}

/**
 * Runs the entry point `script` with `args`, its stdout's reader gone before it writes (as
 * with `| head -0`); resolves to its exit code and its stderr.
 */
function runClosed(script: string, args: readonly string[]): Promise<{ status: number | null; stderr: string }> {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [entryPoint(script), ...args], { stdio: ["ignore", "pipe", "pipe"], timeout: LIMIT_MS });
    child.stdout.destroy();
    let stderr = "";
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk: string) => {
      stderr += chunk;
    });
    child.on("error", reject);
    child.on("close", (status) => {
      resolve({ status, stderr });
    });
  });
}

/** Runs the entry point `script` of this package with `args` and the environment `env`. */
function run(script: string, args: readonly string[], env: Readonly<Record<string, string>>): { status: number | null; stdout: string; stderr: string } {
  const { status, stdout, stderr } = spawnSync(process.execPath, [entryPoint(script), ...args], {
    env: { ...process.env, ...env },
    encoding: "utf8",
    timeout: LIMIT_MS,
  });
  return { status, stdout, stderr };
}

test("refuses an argument to either command with a usage error and exit code 2", () => {
  expect(run("build-wasm.ts", ["--help"], {})).toStrictEqual({ status: 2, stdout: "", stderr: 'error: build:wasm takes no argument but help, not "--help" (see: pnpm build:wasm help)\n' });
  expect(run("reproducible.ts", ["now"], {})).toStrictEqual({ status: 2, stdout: "", stderr: 'error: test:reproducible takes no argument but help, not "now" (see: pnpm test:reproducible help)\n' });
  expect(run("build-wasm.ts", ["help"], {})).toMatchObject({ status: 0, stderr: "" });
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
  for (const script of ["build-wasm.ts", "reproducible.ts"]) {
    expect(await runClosed(script, ["help"]), script).toStrictEqual({ status: 0, stderr: "" });
  }
});
