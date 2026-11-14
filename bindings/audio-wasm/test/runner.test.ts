import { expect, test } from "vitest";
import allocation from "../vitest.allocation.config.ts";
import config from "../vitest.config.ts";

// Vitest allows `.only` unless the `CI` variable is set: a focused test would pass a
// local check and fail in CI. Every configuration refuses it explicitly.
test("refuses a focused test, locally as in CI", () => {
  expect(config.test?.allowOnly).toBe(false);
  expect(allocation.test?.allowOnly).toBe(false);
});
