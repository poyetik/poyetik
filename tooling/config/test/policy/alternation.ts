/** A matcher of text, given a regular expression. */
type Matcher = (expected: RegExp) => unknown;

/** A stand-in for the matchers of text of Vitest and Playwright. */
declare const expect: (actual: unknown) => {
  readonly toMatch: Matcher;
  readonly toHaveText: Matcher;
  readonly toContainText: Matcher;
  readonly toHaveAccessibleName: Matcher;
  readonly toHaveAccessibleDescription: Matcher;
  readonly toHaveValue: Matcher;
  readonly toHaveTitle: Matcher;
  readonly toHaveURL: Matcher;
  readonly toHaveAttribute: (name: string, expected: RegExp) => unknown;
  readonly toThrow: Matcher;
  readonly toThrowError: Matcher;
};

// An alternation, at the start, in a group or at the top level, in each matcher of text.
expect("no adapter").toMatch(/^no (WebGPU|adapter)$/u);
expect("Drawn by the GPU").toHaveText(/GPU|CPU/u);
expect("Drawn by the GPU").toContainText(/|GPU/u);
expect("Start").toHaveAccessibleName(/^(Start|Stop)$/u);
expect("Moving").toHaveAccessibleDescription(/Moving|Still/u);
expect("1").toHaveValue(/1|2/u);
expect("Playground").toHaveTitle(/Playground|Poyetik/u);
expect("./").toHaveURL(/muted|loud/u);
expect("button").toHaveAttribute("aria-disabled", /true|false/u);
expect(() => undefined).toThrow(/^(first|second)$/u);
expect(() => undefined).toThrowError(/first|second/u);
