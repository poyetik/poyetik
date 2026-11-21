// The built modules the tests read, each read in one place: the page module as
// `pnpm build:wasm` built it into this package's dist/, and the visual library's own module
// as its package built it. Read again by each call, so that a wrong file name fails the test
// that reads it rather than the loading of a test file.
import { readFileSync } from "node:fs";
import { PAGE_MODULE } from "../scripts/module.ts";

/**
 * The page module's bytes, as `pnpm build:wasm` built it.
 * @returns Its bytes.
 */
export function pageBytes(): Uint8Array<ArrayBuffer> {
  return new Uint8Array(readFileSync(new URL(`../dist/${PAGE_MODULE.file}`, import.meta.url)));
}

/**
 * The visual library's own module's bytes, as its package built it.
 * @returns Its bytes.
 */
export function visualBytes(): Uint8Array<ArrayBuffer> {
  return new Uint8Array(readFileSync(new URL(import.meta.resolve("@poyetik/visual-wasm/visual.wasm"))));
}
