/** A stand-in for the matchers of Vitest and Playwright. */
declare const expect: (actual: unknown) => {
  readonly toMatch: (expected: RegExp | string) => void;
  readonly toHaveText: (expected: RegExp | string) => Promise<void>;
};

/** An alternation kept apart from any matcher: what a test expects among its errors, say. */
export const EXPECTED = /\.wasm|status of 404/u;

// A regular expression without an alternation, an escaped bar, and a plain string.
expect("cannot load the engine from engine.wasm (HTTP 404)").toMatch(/\(HTTP 404\)$/u);
await expect("a | b").toHaveText(/^a \| b$/u);
await expect("either | or").toHaveText("either | or");
