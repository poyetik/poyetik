import { describe, it, test } from "vitest";

describe.skip("a skipped suite", () => {
  it.skip("a skipped test", () => undefined);
  it.todo("a test left for later");
  test.skipIf(true)("a test skipped on a condition", () => undefined);
  test.runIf(false)("a test run only on a condition", () => undefined);
});
