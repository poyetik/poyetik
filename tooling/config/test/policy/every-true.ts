/** A stand-in for the matchers of Vitest. */
interface Matchers {
  /** Passes when the value is `expected`. */
  readonly toBe: (expected: unknown) => unknown;
  /** Passes when the value equals `expected`. */
  readonly toEqual: (expected: unknown) => unknown;
  /** Passes when the value strictly equals `expected`. */
  readonly toStrictEqual: (expected: unknown) => unknown;
  /** Passes when the value is truthy. */
  readonly toBeTruthy: () => unknown;
}

/** A stand-in for Vitest's `expect`, its soft and its polling forms included. */
declare const expect: ((actual: unknown, message?: string) => Matchers) & {
  /** An expect that lets the test go on when it fails. */
  readonly soft: (actual: unknown, message?: string) => Matchers;
  /** An expect that calls `actual` until the matcher passes. */
  readonly poll: (actual: () => unknown, message?: string) => Matchers;
};

/** An array a test may not hold. */
declare const values: readonly number[] | undefined;

// `every` asserted true, with a message and without one, and on an optional chain.
expect([1, 2].every((value) => value > 0)).toBe(true);
expect([1, 2].every((value) => value > 0), "positive").toBe(true);
expect(values?.every((value) => value > 0)).toBe(true);
// Two joined, then under each other matcher of true.
expect([1].every((value) => value > 0) && [2].every((value) => value > 0)).toBe(true);
expect([1, 2].every((value) => value > 0)).toEqual(true);
expect([1, 2].every((value) => value > 0)).toStrictEqual(true);
expect([1, 2].every((value) => value > 0)).toBeTruthy();
// Finding PC-6 of the eighth deep review of visual block 2 (#34): in a soft expect, and in
// what a polling expect calls.
expect.soft([1, 2].every((value) => value > 0)).toBe(true);
expect.poll(() => [1, 2].every((value) => value > 0)).toBe(true);

// A module of its own, so that its `expect` is not the other fixtures' global one.
export {};
