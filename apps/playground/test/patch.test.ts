import { loadPatch } from "@poyetik/audio-wasm/web";
import { readFileSync } from "node:fs";
import { expect, test } from "vitest";
import { SINE_440 } from "../src/patch.ts";
import { findUp } from "./repository.ts";

/** The conformance patch file of sine-440, parsed. */
function conformancePatch(): unknown {
  return JSON.parse(readFileSync(findUp("conformance/patches/sine-440.json"), "utf8"));
}

test("is the sine-440 patch of the conformance corpus, whose render has a reference hash", () => {
  expect(JSON.parse(new TextDecoder().decode(SINE_440))).toStrictEqual(conformancePatch());
});

test("is accepted by the engine, with two channels", async () => {
  const wasm = readFileSync(new URL(import.meta.resolve("@poyetik/audio-wasm/engine.wasm")));
  const loaded = await loadPatch(wasm.buffer.slice(wasm.byteOffset, wasm.byteOffset + wasm.byteLength), SINE_440, 48_000);
  expect(loaded.channels).toBe(2);
});
