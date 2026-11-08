import { loadPatch } from "@poyetik/engine-wasm/web";
import { existsSync, readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
import { expect, test } from "vitest";
import { SINE_440 } from "../src/patch.ts";

/**
 * The conformance patch file of sine-440, found by walking up from the package directory
 * (under jsdom, module URLs are not file URLs): the tests also run from a copy of the
 * package (mutation testing sandbox), deeper in the repository.
 */
function conformancePatch(): unknown {
  for (let directory = pathToFileURL(`${process.cwd()}/`); directory.pathname !== "/"; directory = new URL("../", directory)) {
    const file = new URL("conformance/patches/sine-440.json", directory);
    if (existsSync(file)) {
      return JSON.parse(readFileSync(file, "utf8"));
    }
  }
  throw new Error("conformance corpus not found");
}

test("is the sine-440 patch of the conformance corpus, whose render has a reference hash", () => {
  expect(JSON.parse(new TextDecoder().decode(SINE_440))).toStrictEqual(conformancePatch());
});

test("is accepted by the engine, with two channels", async () => {
  const wasm = readFileSync(new URL(import.meta.resolve("@poyetik/engine-wasm/engine.wasm")));
  const loaded = await loadPatch(wasm.buffer.slice(wasm.byteOffset, wasm.byteOffset + wasm.byteLength), SINE_440, 48_000);
  expect(loaded.channels).toBe(2);
});
