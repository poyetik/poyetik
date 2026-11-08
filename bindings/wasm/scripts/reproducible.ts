// Checks that the engine module is reproducible: two builds in two fresh target
// directories give the same bytes, and neither contains a path of this machine.
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { buildWasm, PRIVATE_PATHS } from "./build-wasm.ts";
import { identical, leakedPaths } from "./wasm.ts";

const dirs = [mkdtempSync(join(tmpdir(), "engine-wasm-a-")), mkdtempSync(join(tmpdir(), "engine-wasm-b-"))];
try {
  const [first, second] = dirs.map((dir) => new Uint8Array(readFileSync(buildWasm(dir))));
  const problems = [
    ...(first !== undefined && second !== undefined && identical(first, second) ? [] : ["two builds of the engine module differ"]),
    ...leakedPaths(first ?? new Uint8Array(), PRIVATE_PATHS).map((path) => `the engine module contains the path ${path}`),
  ];
  for (const problem of problems) {
    process.stderr.write(`error: ${problem}\n`);
  }
  process.exitCode = problems.length === 0 ? 0 : 1;
} finally {
  for (const dir of dirs) {
    rmSync(dir, { recursive: true, force: true });
  }
}
