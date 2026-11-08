/** A stand-in for Playwright's `test`, whose suites and tests can be skipped the same way. */
declare const test: {
  readonly describe: { readonly fixme: (title: string, body: () => void) => void };
  readonly fixme: (title: string, body: () => void) => void;
};

test.describe.fixme("a suite left broken", () => {
  test.fixme("a test left broken", () => undefined);
});
