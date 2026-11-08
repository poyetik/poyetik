import { readFileSync } from "node:fs";
import { expect, test } from "vitest";
import { TESTS } from "../vitest.config.ts";

// Vitest allows `.only` unless the `CI` variable is set: a focused test would pass a
// local check and fail in CI. Every configuration refuses it explicitly.
test("refuses a focused test, locally as in CI", () => {
  expect(TESTS.allowOnly).toBe(false);
});

// Astro's command-line tool sends telemetry and writes in the user's home unless told
// not to: a check must neither reach the network nor write outside the repository.
test("runs every Astro command with its telemetry disabled", () => {
  const sources = ["../package.json", "../playwright.config.ts"].map((path) => readFileSync(new URL(path, import.meta.url), "utf8"));
  const commands = /\bastro (?:add|build|check|dev|preview|sync|telemetry)\b/g;
  const disabled = /ASTRO_TELEMETRY_DISABLED=1 astro (?:add|build|check|dev|preview|sync|telemetry)\b/g;
  // The package runs check, dev, build and preview, and Playwright preview again: each
  // with the variable before it.
  expect(sources.map((text) => text.match(commands)?.length)).toStrictEqual([4, 1]);
  expect(sources.map((text) => text.match(disabled)?.length)).toStrictEqual([4, 1]);
});
