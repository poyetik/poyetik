// The command-line contract of a package's scripts, on their real entry points run as
// processes: the helpers the script tests of each package that builds a module share
// (`@poyetik/wasm-build/testing/scripts`), so that every package runs its scripts alike.
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

/** The longest a process of these tests may run, in milliseconds: Node starts in well under a second. */
export const LIMIT_MS = 30_000;

/** What a script run printed, and its exit code (null when a signal ended it). */
export interface Ran {
  /** Its exit code. */
  readonly status: number | null;
  /** What it wrote on stdout. */
  readonly stdout: string;
  /** What it wrote on stderr. */
  readonly stderr: string;
}

/** Runs the scripts of one package, by the name of each entry point in its `scripts/`. */
export interface Scripts {
  /** The path of the entry point `script`. */
  readonly entryPoint: (script: string) => string;
  /** Runs `script` with `args` and the environment `env` added to the test's. */
  readonly run: (script: string, args: readonly string[], env: Readonly<Record<string, string>>) => Ran;
  /**
   * Runs `script` with `args`, its stdout's reader gone before it writes (as with
   * `| head -0`); resolves to its exit code and its stderr.
   */
  readonly runClosed: (script: string, args: readonly string[]) => Promise<{ status: number | null; stderr: string }>;
}

/**
 * The scripts of the package whose directory is `root`, each process bounded in time, since
 * a synchronous spawn cannot be interrupted by a test's own timeout.
 * @param root The package's directory, as a URL ending with `/`.
 * @returns How to run its scripts.
 */
export function scriptsOf(root: URL): Scripts {
  const entryPoint = (script: string): string => fileURLToPath(new URL(`scripts/${script}`, root));
  return {
    entryPoint,
    run: (script, args, env) => {
      const { status, stdout, stderr } = spawnSync(process.execPath, [entryPoint(script), ...args], {
        env: { ...process.env, ...env },
        encoding: "utf8",
        timeout: LIMIT_MS,
      });
      return { status, stdout, stderr };
    },
    runClosed: (script, args) =>
      new Promise((resolve, reject) => {
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
      }),
  };
}
