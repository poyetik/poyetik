import { expect, test } from "vitest";
import { FADE, GAIN, HISTORY } from "../src/settings.ts";

test("plays at about -12 dB and keeps 4096 samples", () => {
  expect(GAIN).toBe(0.25);
  expect(HISTORY).toBe(4096);
});

test("fades in and out over 20 ms", () => {
  expect(FADE).toBe(0.02);
});
