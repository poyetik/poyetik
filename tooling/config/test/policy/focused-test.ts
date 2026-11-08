import { describe, test } from "vitest";

describe.only("a focused suite", () => {
  test.only("a focused test", () => undefined);
});
