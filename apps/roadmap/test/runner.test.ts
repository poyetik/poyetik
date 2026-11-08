import { expect, test } from "vitest";
import { TESTS } from "../vitest.config.ts";

// Vitest allows `.only` unless the `CI` variable is set: a focused test would pass a
// local check and fail in CI. Every configuration refuses it explicitly.
test("refuses a focused test, locally as in CI", () => {
  expect(TESTS.allowOnly).toBe(false);
});
