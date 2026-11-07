import { expect, test } from "vitest";
import { GAIN, HISTORY } from "../src/settings.ts";

test("plays at about -12 dB and keeps 4096 samples", () => {
  expect(GAIN).toBe(0.25);
  expect(HISTORY).toBe(4096);
});
