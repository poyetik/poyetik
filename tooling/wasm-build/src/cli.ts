// The two build commands of a WebAssembly module, with everything they touch injected:
// building the module into its package's dist/ (pnpm build:wasm) and checking that it is
// reproducible (pnpm test:reproducible). The entry points of each package (its
// scripts/build-wasm.ts and scripts/reproducible.ts) only wire them to the real process,
// files and cargo, with the package's module.
import { createHash } from "node:crypto";
import { join } from "node:path";
import { artifactPath, BuildError, cargoArgs, cargoHomeOf, commandFailure, commitHashOf, encodedRustflags, privatePaths, reproducibilityProblems, sysrootOf, type Toolchain, type WasmModule } from "./wasm.ts";

/** What the commands need from the outside world. */
export interface Io {
  /** The environment of the process, passed on to cargo. */
  readonly env: Readonly<Record<string, string | undefined>>;
  /** The home directory of the user. */
  readonly home: string;
  /** The repository root, without a trailing slash. */
  readonly repository: string;
  /**
   * Runs `command` and returns its stdout (its stderr is shown as it comes); throws what
   * `execFileSync` throws when it fails.
   */
  readonly run: (command: string, args: readonly string[], options: { readonly cwd: string; readonly env: Readonly<Record<string, string | undefined>> }) => string;
  /** Reads a file. */
  readonly read: (path: string) => Uint8Array;
  /** Creates a directory and its parents, if missing. */
  readonly makeDirectory: (path: string) => void;
  /** Copies a file. */
  readonly copy: (from: string, to: string) => void;
  /** Creates a fresh, empty temporary directory whose name starts with `prefix`; returns its path. */
  readonly temporaryDirectory: (prefix: string) => string;
  /** Removes a directory and everything in it. */
  readonly remove: (path: string) => void;
  /** Writes a line on stdout. */
  readonly out: (line: string) => void;
  /** Writes a line on stderr. */
  readonly err: (line: string) => void;
}

/**
 * Runs `rustc` with `args` in the repository, so that it is the toolchain the repository
 * pins, and returns its stdout.
 * @throws {BuildError} when it fails.
 */
function rustc(io: Io, args: readonly string[]): string {
  try {
    return io.run("rustc", args, { cwd: io.repository, env: io.env });
  } catch (error) {
    throw new BuildError(commandFailure(`rustc ${args.join(" ")}`, error));
  }
}

/**
 * The toolchain that builds the module, asked of rustc.
 * @throws {BuildError} when rustc fails, or prints what is not understood.
 */
function toolchainOf(io: Io): Toolchain {
  const sysroot = sysrootOf(rustc(io, ["--print", "sysroot"]));
  return { sysroot, commit: commitHashOf(rustc(io, ["-vV"])) };
}

/**
 * Builds `module` reproducibly with `toolchain` (in `targetDir` when given) and returns
 * its path.
 * @throws {BuildError} when cargo fails, or does not report the module.
 */
function build(io: Io, module: WasmModule, toolchain: Toolchain, targetDir?: string): string {
  const env = {
    ...io.env,
    CARGO_ENCODED_RUSTFLAGS: encodedRustflags(cargoHomeOf(io.env, io.home), io.repository, toolchain),
    ...(targetDir === undefined ? {} : { CARGO_TARGET_DIR: targetDir }),
  };
  let messages: string;
  try {
    messages = io.run("cargo", cargoArgs(module), { cwd: io.repository, env });
  } catch (error) {
    throw new BuildError(commandFailure("cargo build", error));
  }
  return artifactPath(messages, module);
}

/** Runs `action`, turning the failures it explains into one `error:` line and exit code 1. */
function reported(io: Io, action: () => number): number {
  try {
    return action();
  } catch (error) {
    if (error instanceof BuildError) {
      io.err(`error: ${error.message}`);
      return 1;
    }
    throw error;
  }
}

/** `pnpm build:wasm`: builds `module` and copies it to its file in `dist`; returns the exit code. */
export function buildMain(io: Io, dist: string, module: WasmModule): number {
  return reported(io, () => {
    const artifact = build(io, module, toolchainOf(io));
    io.makeDirectory(dist);
    io.copy(artifact, join(dist, module.file));
    return 0;
  });
}

/**
 * `pnpm test:reproducible`: builds `module` twice, in two fresh target directories
 * (removed whatever the outcome), and checks that both builds are the same bytes and that
 * neither holds a path of this machine. Prints the SHA-256 of the module on success, to
 * compare across machines; each problem otherwise, then one `error:` line. Returns the
 * exit code.
 */
export function reproducibleMain(io: Io, module: WasmModule): number {
  const dirs = [io.temporaryDirectory(`${module.package}-a-`), io.temporaryDirectory(`${module.package}-b-`)] as const;
  try {
    return reported(io, () => {
      const toolchain = toolchainOf(io);
      const first = io.read(build(io, module, toolchain, dirs[0]));
      const second = io.read(build(io, module, toolchain, dirs[1]));
      const paths = privatePaths(io.repository, cargoHomeOf(io.env, io.home), io.home, toolchain.sysroot);
      const problems = reproducibilityProblems(first, second, paths, module);
      if (problems.length > 0) {
        problems.forEach(io.out);
        io.err(`error: the ${module.label} is not reproducible (${String(problems.length)} problem(s))`);
        return 1;
      }
      io.out(`the ${module.label} is reproducible: sha256 ${createHash("sha256").update(first).digest("hex")}`);
      return 0;
    });
  } finally {
    dirs.forEach(io.remove);
  }
}

/**
 * Makes a closed `stream` (a reader gone, as with `| head`) harmless: what nobody reads
 * is dropped, and the exit code stays the outcome of the command. Any other stream error
 * is thrown, as it would be without this.
 */
export function ignoreClosedReader(stream: { on: (event: "error", listener: (error: Error) => void) => unknown }): void {
  stream.on("error", (error) => {
    if (!("code" in error) || error.code !== "EPIPE") {
      throw error;
    }
  });
}
