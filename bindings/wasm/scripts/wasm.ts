// How the engine module is built reproducibly: the same bytes on every machine, with no
// path of the machine that built it inside. Pure functions, tested; the entry points
// (build-wasm.ts, reproducible.ts) only run them.

/** The cargo arguments building the engine module, its JSON messages on stdout. */
export const CARGO_ARGS = [
  "build",
  "--package",
  "engine-wasm",
  "--release",
  "--locked",
  "--target",
  "wasm32-unknown-unknown",
  "--message-format",
  "json-render-diagnostics",
] as const;

/**
 * The rustc flags of a reproducible build, as `CARGO_ENCODED_RUSTFLAGS` (separated by
 * U+001F): the paths of the Cargo home and of the repository, embedded in panic and
 * debug strings, are replaced by fixed ones.
 */
export function encodedRustflags(cargoHome: string, repository: string): string {
  return ["--remap-path-prefix", `${cargoHome}=/cargo`, "--remap-path-prefix", `${repository}=/poyetik`].join("\u001f");
}

/** Thrown when the build output cannot be understood. */
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
 * The path of the engine module, from cargo's JSON messages (one per line): asked of
 * cargo rather than assumed, so that `CARGO_TARGET_DIR` and any layout work.
 * @throws {BuildError} when no message names it.
 */
export function artifactPath(messages: string): string {
  for (const line of messages.split("\n")) {
    if (!line.startsWith("{")) {
      continue;
    }
    const message: unknown = JSON.parse(line);
    const reason: unknown = Reflect.get(Object(message), "reason");
    const target: unknown = Reflect.get(Object(message), "target");
    const files: unknown = Reflect.get(Object(message), "filenames");
    if (reason === "compiler-artifact" && Reflect.get(Object(target), "name") === "engine_wasm" && Array.isArray(files)) {
      const wasm = files.find((file: unknown): file is string => typeof file === "string" && file.endsWith(".wasm"));
      if (wasm !== undefined) {
        return wasm;
      }
    }
  }
  throw new BuildError("cargo did not report the engine module (engine_wasm .wasm)");
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
