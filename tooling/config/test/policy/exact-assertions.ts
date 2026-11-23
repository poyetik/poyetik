/** A matcher of what a call throws. */
type Thrown = (expected?: unknown) => unknown;

/** A stand-in for the matchers of Vitest. */
declare const expect: (actual: unknown, message?: string) => {
  readonly toThrow: Thrown;
  readonly toBe: (expected: unknown) => unknown;
  readonly toStrictEqual: (expected: unknown) => unknown;
};

/** An error a test built. */
const expected = new RangeError("out of range");

/** An error every test expects. */
const REFUSAL = new RangeError("refused");

/** Errors by what refuses. */
const REFUSALS = { IO: REFUSAL };

// The error itself, or one held by a name of a value; the elements that fail, asserted
// empty; `every` asserted false, or compared with a value.
expect(() => undefined).toThrow(new RangeError("out of range"));
expect(() => undefined).toThrow(expected);
expect(() => undefined).toThrow(REFUSAL);
expect(() => undefined).toThrow(REFUSALS.IO);
expect(() => undefined).toThrow();
expect(() => undefined).toThrow([1].length === 1 ? expected : REFUSAL);
// A pattern of RUNTIME_MESSAGES, whose whole text another program decides.
expect(() => undefined).toThrow(/ENOENT/);
expect(() => undefined).toThrow(/^not JSON \(/u);
expect([1, 2].filter((value) => value <= 0)).toStrictEqual([]);
expect([1, 2].every((value) => value > 1)).toBe(false);
expect([[1, 2].every((value) => value > 0)]).toStrictEqual([true]);

// A module of its own, so that its `expect` is not the other fixtures' global one.
export {};
