// Builds the engine module reproducibly and copies it to dist/engine.wasm.
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { homedir } from "node:os";
import { fileURLToPath } from "node:url";
import { artifactPath, CARGO_ARGS, encodedRustflags } from "./wasm.ts";

/** The repository root, and the Cargo home whose path is remapped. */
const REPOSITORY = fileURLToPath(new URL("../../../", import.meta.url)).replace(/\/$/, "");
const CARGO_HOME = process.env["CARGO_HOME"] ?? `${homedir()}/.cargo`;

/** Builds the module (in `targetDir` when given) and returns its path. */
export function buildWasm(targetDir?: string): string {
  const messages = execFileSync("cargo", CARGO_ARGS, {
    cwd: REPOSITORY,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "inherit"],
    env: {
      ...process.env,
      CARGO_ENCODED_RUSTFLAGS: encodedRustflags(CARGO_HOME, REPOSITORY),
      ...(targetDir === undefined ? {} : { CARGO_TARGET_DIR: targetDir }),
    },
  });
  return artifactPath(messages);
}

/** The paths that must never appear in the module. */
export const PRIVATE_PATHS = [REPOSITORY, CARGO_HOME, homedir()];

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  mkdirSync(new URL("../dist/", import.meta.url), { recursive: true });
  copyFileSync(buildWasm(), new URL("../dist/engine.wasm", import.meta.url));
}
