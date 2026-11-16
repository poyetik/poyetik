// How a WebAssembly module of the project is built reproducibly: with no path of the
// machine that built it inside, so that two clean builds give the same bytes. What is proven: two builds on one
// machine, in two fresh target directories shared by a package's modules, the second in
// the reverse order of the first, so that bytes depending on what was built before them
// differ (pnpm test:reproducible, which prints the SHA-256 of each module); across machines, only by comparing that digest by hand (a
// committed digest is planned, #11 step 3). Pure functions, tested; cli.ts runs them.

import { resolve } from "node:path";

/** A WebAssembly module of the project: what builds it, and what it is called. */
export interface WasmModule {
  /** The cargo package that builds it (`audio-wasm`), a `cdylib` for WebAssembly. */
  readonly package: string;
  /** What the messages call it (`engine module`). */
  readonly label: string;
  /** The file it is copied to in the package's `dist/` (`engine.wasm`). */
  readonly file: string;
  /** Other files of `dist/` it is copied to as well: none when absent. */
  readonly copies?: readonly string[];
  /**
   * Variables its build is given, whatever the environment holds (`POYETIK_ENGINE_BLOCK`,
   * which names the engine block a module is built for): none when absent.
   */
  readonly env?: Readonly<Record<string, string>>;
}

/** The files of `dist/` `module` is copied to: its own, then its copies. */
export function filesOf(module: WasmModule): readonly string[] {
  return [module.file, ...(module.copies ?? [])];
}

/** The name of `module`'s cargo target: its package's, with `_` for `-`. */
export function targetName(module: WasmModule): string {
  return module.package.replaceAll("-", "_");
}

/** The cargo arguments building `module`, its JSON messages on stdout. */
export function cargoArgs(module: WasmModule): readonly string[] {
  return ["build", "--package", module.package, "--release", "--locked", "--target", "wasm32-unknown-unknown", "--message-format", "json-render-diagnostics"];
}

/** The toolchain that builds the module, as rustc reports it. */
export interface Toolchain {
  /** Its sysroot (`rustc --print sysroot`), where the rust-src component installs the standard library's sources. */
  readonly sysroot: string;
  /** The commit it was built from (`rustc -vV`): 40 lowercase hexadecimal digits. */
  readonly commit: string;
}

/** Where the rust-src component puts the standard library's sources, below the sysroot. */
const STANDARD_SOURCES = "lib/rustlib/src/rust";

/**
 * The rustc flags of a reproducible build, as `CARGO_ENCODED_RUSTFLAGS` (separated by
 * U+001F): the paths of the Cargo home and of the repository, embedded in panic and
 * debug strings, are replaced by fixed ones. So are the standard library's sources in the
 * sysroot: the standard library is built with its paths remapped to `/rustc/<commit>`,
 * and rustc translates them back to the local sources when the rust-src component is
 * installed; remapping those sources to `/rustc/<commit>` again makes the module the same
 * bytes with or without rust-src.
 */
export function encodedRustflags(cargoHome: string, repository: string, toolchain: Toolchain): string {
  return [
    "--remap-path-prefix",
    `${cargoHome}=/cargo`,
    "--remap-path-prefix",
    `${repository}=/poyetik`,
    "--remap-path-prefix",
    `${toolchain.sysroot}/${STANDARD_SOURCES}=/rustc/${toolchain.commit}`,
  ].join("\u001f");
}

/** Thrown when the build fails, or its output cannot be understood: one line explaining why. */
export class BuildError extends Error {
  /**
   * Describes what went wrong.
   * @param message What went wrong.
   */
  constructor(message: string) {
    super(message);
    this.name = "BuildError";
  }
}

/**
 * The sysroot `rustc --print sysroot` printed: one non-empty line, ended by a newline, with
 * no control character.
 * @throws {BuildError} when it printed anything else.
 */
export function sysrootOf(printed: string): string {
  const path = printed.endsWith("\n") ? printed.slice(0, -1) : "";
  if (path.length === 0 || /\p{Cc}/u.test(path)) {
    throw new BuildError(`rustc --print sysroot printed no single path: ${JSON.stringify(printed)}`);
  }
  return path;
}

/**
 * The commit hash `rustc -vV` printed, on its line `commit-hash: <hash>`.
 * @throws {BuildError} when no such line holds exactly 40 lowercase hexadecimal digits.
 */
export function commitHashOf(verbose: string): string {
  const commit = /^commit-hash: ([0-9a-f]{40})$/mu.exec(verbose)?.[1];
  if (commit === undefined) {
    throw new BuildError("rustc -vV reported no commit hash (40 hexadecimal digits)");
  }
  return commit;
}

/**
 * The path of `module`, from cargo's JSON messages (one per line): asked of cargo rather
 * than assumed, so that `CARGO_TARGET_DIR` and any layout work.
 * @throws {BuildError} when no message names it, or a line that should be one is not JSON.
 */
export function artifactPath(messages: string, module: WasmModule): string {
  for (const line of messages.split("\n")) {
    if (!line.startsWith("{")) {
      continue;
    }
    let message: unknown;
    try {
      message = JSON.parse(line);
    } catch (error) {
      throw new BuildError(`cargo printed a line that is not JSON (${String(error)})`);
    }
    const reason: unknown = Reflect.get(Object(message), "reason");
    const target: unknown = Reflect.get(Object(message), "target");
    const files: unknown = Reflect.get(Object(message), "filenames");
    if (reason === "compiler-artifact" && Reflect.get(Object(target), "name") === targetName(module) && Array.isArray(files)) {
      const wasm = files.find((file: unknown): file is string => typeof file === "string" && file.endsWith(".wasm"));
      if (wasm !== undefined) {
        return wasm;
      }
    }
  }
  throw new BuildError(`cargo did not report the ${module.label} (${targetName(module)} .wasm)`);
}

/**
 * A path of the building machine, and what follows it wherever a build would leak it: a
 * leak is the path followed by one of `children`. The path alone is no proof, since it may
 * be part of the module's own relative paths (a repository at `/src`, whose files are in
 * `crates/visual-core/src/`) or of any path (a home directory at `/`).
 */
export interface PrivatePath {
  /** The path, without a trailing slash. */
  readonly path: string;
  /** What follows it in a leak, each starting with `/`; none when nothing can tell one. */
  readonly children: readonly string[];
}

/**
 * The private paths of `paths` (non-empty) that appear in `bytes`, each followed by one of
 * its children, read as UTF-8: a module is mostly not text, and invalid sequences are
 * replaced rather than refused.
 */
export function leakedPaths(bytes: Uint8Array, paths: readonly PrivatePath[]): string[] {
  const text = new TextDecoder().decode(bytes);
  return paths.filter(({ path, children }) => path.length > 0 && children.some((child) => text.includes(`${path}${child}`))).map(({ path }) => path);
}

/** Whether two builds are the same bytes. */
export function identical(first: Uint8Array, second: Uint8Array): boolean {
  return first.length === second.length && first.every((byte, index) => byte === second[index]);
}

/**
 * The Cargo home of a machine, as cargo reads it: `CARGO_HOME` when it is set and not
 * empty, resolved from the directory cargo runs in (`repository`) when it is relative,
 * else `~/.cargo`.
 */
export function cargoHomeOf(env: Readonly<Record<string, string | undefined>>, home: string, repository: string): string {
  const given = env["CARGO_HOME"];
  return given === undefined || given === "" ? `${home}/.cargo` : resolve(repository, given);
}

/**
 * The paths of the building machine that must never appear in the module, each with what
 * follows it in a leak: the repository's sources, the Cargo home's crates, anything of the
 * home directory (unless it is `/`, where nothing can tell a leak from any path) and the
 * sysroot's standard library, which `RUSTUP_HOME` may put outside the home directory.
 */
export function privatePaths(repository: string, cargo: string, home: string, sysroot: string): PrivatePath[] {
  return [
    { path: repository, children: ["/crates/", "/bindings/"] },
    { path: cargo, children: ["/registry/", "/git/"] },
    ...(home === "/" ? [] : [{ path: home, children: ["/"] }]),
    { path: sysroot, children: ["/lib/rustlib/"] },
  ];
}

/**
 * What makes two builds of `module` not reproducible: their bytes differ, or the first
 * contains a path of the building machine. Empty when they are reproducible.
 */
export function reproducibilityProblems(first: Uint8Array, second: Uint8Array, paths: readonly PrivatePath[], module: WasmModule): string[] {
  return [
    ...(identical(first, second) ? [] : [`two builds of the ${module.label} differ`]),
    ...leakedPaths(first, paths).map((path) => `the ${module.label} contains the path ${path}`),
  ];
}

/**
 * Why a command run with `execFileSync` failed, from what it threw: its exit code, the
 * signal that stopped it, or why it could not start (such as `ENOENT`, not installed).
 */
export function commandFailure(command: string, thrown: unknown): string {
  const status: unknown = Reflect.get(Object(thrown), "status");
  const signal: unknown = Reflect.get(Object(thrown), "signal");
  if (typeof status === "number") {
    return `${command} failed (exit ${String(status)})`;
  }
  if (typeof signal === "string") {
    return `${command} was stopped by signal ${signal}`;
  }
  return `${command} could not be run: ${String(thrown)}`;
}
