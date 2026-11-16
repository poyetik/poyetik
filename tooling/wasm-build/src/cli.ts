// The two build commands of a package's WebAssembly modules, with everything they touch
// injected: building the modules into its dist/ (pnpm build:wasm) and checking that each is
// reproducible (pnpm test:reproducible). The entry points of each package (its
// scripts/build-wasm.ts and scripts/reproducible.ts) only wire them to the real process,
// files and cargo, with the package's modules.
import { createHash } from "node:crypto";
import { oneLine } from "@poyetik/config/one-line";
import { join, resolve } from "node:path";
import { artifactPath, BuildError, cargoArgs, cargoHomeOf, commandFailure, commitHashOf, encodedRustflags, filesOf, privatePaths, reproducibilityProblems, sysrootOf, type Toolchain, type WasmModule } from "./wasm.ts";

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
  /** The directory temporary directories are created in, named when one cannot be. */
  readonly temporaryRoot: string;
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
 * Builds `module` reproducibly with `toolchain` in the target directory `targetDir`, given
 * its own variables whatever the environment holds, and returns its path.
 * @throws {BuildError} when cargo fails, or does not report the module.
 */
function build(io: Io, module: WasmModule, toolchain: Toolchain, targetDir: string): string {
  const env = {
    ...io.env,
    ...module.env,
    CARGO_ENCODED_RUSTFLAGS: encodedRustflags(cargoHomeOf(io.env, io.home, io.repository), io.repository, toolchain),
    CARGO_TARGET_DIR: targetDir,
  };
  let messages: string;
  try {
    messages = io.run("cargo", cargoArgs(module), { cwd: io.repository, env });
  } catch (error) {
    throw new BuildError(commandFailure(`cargo build of the ${module.label}`, error));
  }
  return artifactPath(messages, module);
}

/** Why the file operation that threw `error` failed: its error code (`ENOSPC`), else its message. */
function why(error: unknown): string {
  const code: unknown = Reflect.get(Object(error), "code");
  return typeof code === "string" ? code : String(error);
}

/**
 * Runs the file operation `action`.
 * @throws {BuildError} saying it could not `what`, and why, when it fails.
 */
function onFiles<T>(what: string, action: () => T): T {
  try {
    return action();
  } catch (error) {
    throw new BuildError(`cannot ${what}: ${why(error)}`);
  }
}

/**
 * Runs `action`, turning any failure into one `error:` line and exit code 1: the
 * command-line contract allows no stack trace, whatever failed.
 */
function reported(io: Io, action: () => number): number {
  try {
    return action();
  } catch (error) {
    io.err(`error: ${oneLine(error instanceof BuildError ? error.message : `unexpected failure: ${String(error)}`)}`);
    return 1;
  }
}

/**
 * Reads the arguments of `command`, which takes none but `help`: `undefined` when there
 * are none, else its exit code, after printing `usage` on stdout for `help`, or one usage
 * error naming the argument refused (the first, or the one after `help`), made safe on one
 * line and pointing to the help.
 */
function usage(io: Io, command: string, args: readonly string[], text: string): number | undefined {
  const [first, second] = args;
  if (first === undefined) {
    return undefined;
  }
  const refuse = (refused: string): number => {
    io.err(`error: ${oneLine(`${command} takes no argument but help, not "${refused}"`)} (see: pnpm ${command} help)`);
    return 2;
  };
  if (first !== "help") {
    return refuse(first);
  }
  if (second !== undefined) {
    return refuse(second);
  }
  io.out(text);
  return 0;
}

/** A package's modules: at least one. */
export type Modules = readonly [WasmModule, ...WasmModule[]];

/**
 * The target directory `pnpm build:wasm` builds `module` in: one per module, named after
 * its file, in `wasm-build/` of the directory cargo would use (`CARGO_TARGET_DIR`, relative
 * to the repository, or the repository's `target`). A module's variables are inputs of the
 * crates that read them (`POYETIK_ENGINE_BLOCK`): sharing one directory, every module would
 * rebuild them all, every time.
 */
function targetDirOf(io: Io, module: WasmModule): string {
  return join(resolve(io.repository, io.env["CARGO_TARGET_DIR"] ?? "target"), "wasm-build", module.file);
}

/**
 * `pnpm build:wasm`: builds each of `modules` in turn, each in a target directory of its
 * own ({@link targetDirOf}), and copies each into `dist`, to its file and its copies, as it
 * is built; returns the exit code.
 */
export function buildMain(io: Io, args: readonly string[], dist: string, modules: Modules): number {
  const lines = modules.map((module) => `\n  build the ${module.label} reproducibly into ${filesOf(module).map((file) => join(dist, file)).join(" and ")}`);
  const answered = usage(io, "build:wasm", args, `usage: pnpm build:wasm [help]${lines.join("")}`);
  if (answered !== undefined) {
    return answered;
  }
  return reported(io, () => {
    const toolchain = toolchainOf(io);
    onFiles(`create ${dist}`, () => {
      io.makeDirectory(dist);
    });
    for (const module of modules) {
      const artifact = build(io, module, toolchain, targetDirOf(io, module));
      for (const file of filesOf(module)) {
        const target = join(dist, file);
        onFiles(`copy ${artifact} to ${target}`, () => {
          io.copy(artifact, target);
        });
      }
    }
    return 0;
  });
}

/**
 * `pnpm test:reproducible`: builds each of `modules` twice, in two fresh target
 * directories shared by the modules (removed whatever the outcome), and checks that both
 * builds are the same bytes and that neither holds a path of this machine. Prints the
 * SHA-256 of each module on success, to compare across machines; at the first module that
 * is not, each of its problems, then one `error:` line. Returns the exit code.
 */
export function reproducibleMain(io: Io, args: readonly string[], modules: Modules): number {
  const answered = usage(
    io,
    "test:reproducible",
    args,
    `usage: pnpm test:reproducible [help]\n  build the ${modules.map((module) => module.label).join(", the ")} twice, in two fresh target directories, and check that both builds are the same bytes, holding no path of this machine`,
  );
  if (answered !== undefined) {
    return answered;
  }
  const dirs: string[] = [];
  const temporary = (side: string): string => {
    const dir = onFiles(`create a temporary directory in ${io.temporaryRoot}`, () => io.temporaryDirectory(`${modules[0].package}-${side}-`));
    dirs.push(dir);
    return dir;
  };
  const outcome = reported(io, () => {
    const targets = [temporary("a"), temporary("b")] as const;
    const toolchain = toolchainOf(io);
    const paths = privatePaths(io.repository, cargoHomeOf(io.env, io.home, io.repository), io.home, toolchain.sysroot);
    for (const module of modules) {
      const built = (target: string): Uint8Array => {
        const path = build(io, module, toolchain, target);
        return onFiles(`read ${path}`, () => io.read(path));
      };
      const first = built(targets[0]);
      const second = built(targets[1]);
      const problems = reproducibilityProblems(first, second, paths, module);
      if (problems.length > 0) {
        problems.forEach(io.out);
        io.err(`error: the ${module.label} is not reproducible (${String(problems.length)} problem(s))`);
        return 1;
      }
      io.out(`the ${module.label} is reproducible: sha256 ${createHash("sha256").update(first).digest("hex")}`);
    }
    return 0;
  });
  const removed = dirs.map((dir) =>
    reported(io, () => {
      onFiles(`remove ${dir}`, () => {
        io.remove(dir);
      });
      return 0;
    }),
  );
  return Math.max(outcome, ...removed);
}

/**
 * Makes a closed `stream` (a reader gone, as with `| head`) harmless: what nobody reads
 * is dropped, and the exit code stays the outcome of the command. Any other stream error
 * is given to `fail` as one `error:` line, never thrown: thrown from an event, it would
 * end the process with a stack trace.
 */
export function ignoreClosedReader(stream: ErrorEvents, fail: (line: string) => void): void {
  stream.on("error", (error) => {
    if (why(error) !== "EPIPE") {
      fail(`error: cannot write to stdout: ${why(error)}`);
    }
  });
}

/**
 * Guards stdout for an entry point: a closed reader is harmless, and any other stream
 * error is one `error:` line given to `err` (the entry point's stderr) and exit code 1,
 * given to `exit` ({@link ignoreClosedReader}).
 */
export function guardStdout(stream: ErrorEvents, err: (line: string) => void, exit: (code: number) => void): void {
  ignoreClosedReader(stream, (line) => {
    err(line);
    exit(1);
  });
}

/** A stream that reports its errors as events, as stdout does. */
export interface ErrorEvents {
  /** Adds the listener of its `error` events. */
  readonly on: (event: "error", listener: (error: Error) => void) => unknown;
}

/** What an entry point reports to: its process's stdout, stderr and exit code. */
export interface EntryProcess {
  /** The stdout the command writes to. */
  readonly stdout: ErrorEvents;
  /** Writes a line on stderr. */
  readonly err: (line: string) => void;
  /** Sets the exit code of the process. */
  readonly exit: (code: number) => void;
}

/**
 * Runs `main` as the whole of an entry point: guards stdout first ({@link guardStdout}),
 * so that no entry point can forget it, then sets the exit code to what `main` returns.
 */
export function runEntryPoint(entry: EntryProcess, main: () => number): void {
  guardStdout(entry.stdout, entry.err, entry.exit);
  entry.exit(main());
}
