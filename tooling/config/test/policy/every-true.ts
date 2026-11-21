/** A stand-in for the matchers of Vitest. */
declare const expect: (actual: unknown, message?: string) => { readonly toBe: (expected: unknown) => unknown };

/** An array a test may not hold. */
declare const values: readonly number[] | undefined;

// `every` asserted true, with a message and without one, and on an optional chain.
expect([1, 2].every((value) => value > 0)).toBe(true);
expect([1, 2].every((value) => value > 0), "positive").toBe(true);
expect(values?.every((value) => value > 0)).toBe(true);

// A module of its own, so that its `expect` is not the other fixtures' global one.
export {};
