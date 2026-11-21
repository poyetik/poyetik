/** A stand-in for Playwright's test, whose own timeout is no sleep. */
declare const test: { readonly setTimeout: (milliseconds: number) => void };

/** Sleeps in test code: the timers bare, then through each global object; not a test's timeout, nor a timer named in a string. */
export async function sleeps(): Promise<readonly string[]> {
  await new Promise((resolve) => {
    setTimeout(resolve, 10);
  });
  setInterval(() => undefined, 10);
  globalThis.setTimeout(() => undefined, 1);
  window.setInterval(() => undefined, 1);
  self.setTimeout(() => undefined, 1);
  test.setTimeout(180_000);
  return ["setTimeout", "clearTimeout"];
}
