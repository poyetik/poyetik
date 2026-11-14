// How a WebAssembly module of the project is built reproducibly: with no path of the
// machine that built it inside, so that two clean builds give the same bytes. What is proven: two builds on one
// machine, in two fresh target directories (pnpm test:reproducible, which prints the
// SHA-256 of the module); across machines, only by comparing that digest by hand (a
// committed digest is planned, #11 step 3). Pure functions, tested; cli.ts runs them.

<<<<<<<< HEAD:bindings/audio-wasm/scripts/wasm.ts
/** The cargo arguments building the engine module, its JSON messages on stdout. */
export const CARGO_ARGS = [
  "build",
  "--package",
  "audio-wasm",
  "--release",
  "--locked",
  "--target",
  "wasm32-unknown-unknown",
  "--message-format",
  "json-render-diagnostics",
] as const;
========
/** A WebAssembly module of the project: what builds it, and what it is called. */
export interface WasmModule {
  /** The cargo package that builds it (`audio-wasm`), a `cdylib` for WebAssembly. */
  readonly package: string;
  /** What the messages call it (`engine module`). */
  readonly label: string;
  /** The file it is copied to in the package's `dist/` (`engine.wasm`). */
  readonly file: string;
}

/** The name of `module`'s cargo target: its package's, with `_` for `-`. */
export function targetName(module: WasmModule): string {
  return module.package.replaceAll("-", "_");
}

/** The cargo arguments building `module`, its JSON messages on stdout. */
export function cargoArgs(module: WasmModule): readonly string[] {
  return ["build", "--package", module.package, "--release", "--locked", "--target", "wasm32-unknown-unknown", "--message-format", "json-render-diagnostics"];
}
>>>>>>>> b30d7e6a (refactor(wasm-build): share the WebAssembly module build):tooling/wasm-build/src/wasm.ts

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
 * @throws {BuildError} when no message names it.
 */
export function artifactPath(messages: string, module: WasmModule): string {
  for (const line of messages.split("\n")) {
    if (!line.startsWith("{")) {
      continue;
    }
    const message: unknown = JSON.parse(line);
    const reason: unknown = Reflect.get(Object(message), "reason");
    const target: unknown = Reflect.get(Object(message), "target");
    const files: unknown = Reflect.get(Object(message), "filenames");
<<<<<<<< HEAD:bindings/audio-wasm/scripts/wasm.ts
    if (reason === "compiler-artifact" && Reflect.get(Object(target), "name") === "audio_wasm" && Array.isArray(files)) {
========
    if (reason === "compiler-artifact" && Reflect.get(Object(target), "name") === targetName(module) && Array.isArray(files)) {
>>>>>>>> b30d7e6a (refactor(wasm-build): share the WebAssembly module build):tooling/wasm-build/src/wasm.ts
      const wasm = files.find((file: unknown): file is string => typeof file === "string" && file.endsWith(".wasm"));
      if (wasm !== undefined) {
        return wasm;
      }
    }
  }
<<<<<<<< HEAD:bindings/audio-wasm/scripts/wasm.ts
  throw new BuildError("cargo did not report the engine module (audio_wasm .wasm)");
========
  throw new BuildError(`cargo did not report the ${module.label} (${targetName(module)} .wasm)`);
>>>>>>>> b30d7e6a (refactor(wasm-build): share the WebAssembly module build):tooling/wasm-build/src/wasm.ts
}

/**
 * The paths of `paths` (non-empty) that appear in `bytes`, read as UTF-8: a module is
 * mostly not text, and invalid sequences are replaced rather than refused.
 */
export function leakedPaths(bytes: Uint8Array, paths: readonly string[]): string[] {
  const text = new TextDecoder().decode(bytes);
  return paths.filter((path) => path.length > 0 && text.includes(path));
}

/** Whether two builds are the same bytes. */
export function identical(first: Uint8Array, second: Uint8Array): boolean {
  return first.length === second.length && first.every((byte, index) => byte === second[index]);
}

/** The Cargo home of a machine: `CARGO_HOME` when set, else `~/.cargo`. */
export function cargoHomeOf(env: Readonly<Record<string, string | undefined>>, home: string): string {
  return env["CARGO_HOME"] ?? `${home}/.cargo`;
}

/**
 * The paths of the building machine that must never appear in the module: the sysroot
 * too, which `RUSTUP_HOME` may put outside the home directory.
 */
export function privatePaths(repository: string, cargo: string, home: string, sysroot: string): string[] {
  return [repository, cargo, home, sysroot];
}

/**
 * What makes two builds of `module` not reproducible: their bytes differ, or the first
 * contains a path of the building machine. Empty when they are reproducible.
 */
export function reproducibilityProblems(first: Uint8Array, second: Uint8Array, paths: readonly string[], module: WasmModule): string[] {
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
