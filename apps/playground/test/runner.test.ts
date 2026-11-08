import { expect, test } from "vitest";
import config from "../vite.config.ts";

// Vitest allows `.only` unless the `CI` variable is set: a focused test would pass a
// local check and fail in CI. Every configuration refuses it explicitly.
test("refuses a focused test, locally as in CI", () => {
  expect(config.test?.allowOnly).toBe(false);
});
