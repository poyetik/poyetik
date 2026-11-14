import { expect, test } from "vitest";
import { ENGINE_MODULE } from "../scripts/module.ts";

test("names the engine module: its crate, what messages call it and its file in dist/", () => {
  expect(ENGINE_MODULE).toStrictEqual({ package: "audio-wasm", label: "engine module", file: "engine.wasm" });
});
