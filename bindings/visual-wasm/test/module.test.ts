import { expect, test } from "vitest";
import { VISUAL_MODULE } from "../scripts/module.ts";

test("names the visual module: its crate, what messages call it and its file in dist/", () => {
  expect(VISUAL_MODULE).toStrictEqual({ package: "visual-wasm", label: "visual module", file: "visual.wasm" });
});
