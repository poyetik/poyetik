import { expect, test } from "vitest";
import { FADE, GAIN, HISTORY_SECONDS } from "../src/settings.ts";

test("plays at about -12 dB and keeps 85 ms", () => {
  expect(GAIN).toBe(0.25);
  expect(HISTORY_SECONDS).toBe(0.085);
});

test("fades in and out over 20 ms", () => {
  expect(FADE).toBe(0.02);
});
