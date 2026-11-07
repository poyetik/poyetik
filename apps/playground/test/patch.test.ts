import { loadPatch } from "@poyetik/engine-wasm/web";
import { readFileSync } from "node:fs";
import { expect, test } from "vitest";
import { SINE_440 } from "../src/patch.ts";

test("is a 440 Hz sine on both channels", () => {
  expect(JSON.parse(new TextDecoder().decode(SINE_440))).toStrictEqual({
    format: 1,
    nodes: [
      { id: 1, box: "osc~", params: { frequency: 440 } },
      { id: 2, box: "dac~" },
    ],
    connections: [
      { from: [1, 0], to: [2, 0] },
      { from: [1, 0], to: [2, 1] },
    ],
  });
});

test("is accepted by the engine, with two channels", async () => {
  const wasm = readFileSync(new URL(import.meta.resolve("@poyetik/engine-wasm/engine.wasm")));
  const loaded = await loadPatch(wasm.buffer.slice(wasm.byteOffset, wasm.byteOffset + wasm.byteLength), SINE_440, 48_000);
  expect(loaded.channels).toBe(2);
});
