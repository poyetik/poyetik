import { expect, test } from "vitest";
import { RENDER_FORMATS, precisionBits, sampleBits } from "../src/format.ts";

test("names the three render formats, in the order of the references", () => {
  expect(RENDER_FORMATS).toStrictEqual(["binary64", "binary64-as-binary32", "binary32"]);
});

test("gives the precision and the sample width of each format", () => {
  expect(RENDER_FORMATS.map((format) => [precisionBits(format), sampleBits(format)])).toStrictEqual([
    [64, 64],
    [64, 32],
    [32, 32],
  ]);
});
