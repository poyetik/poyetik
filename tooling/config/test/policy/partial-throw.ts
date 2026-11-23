/** A matcher of what a call throws. */
type Thrown = (expected?: unknown) => unknown;

/** A stand-in for the matchers of Vitest. */
declare const expect: (actual: unknown) => { readonly toThrow: Thrown; readonly toThrowError: Thrown };

/** A case a text depends on. */
declare const late: boolean;

// A part of an error's text: a string, a template, a regular expression, either side of a
// choice; an unreviewed pattern beside a reviewed one.
expect(() => undefined).toThrow("out of range");
expect(() => undefined).toThrow(`out of ${String(late)}`);
expect(() => undefined).toThrowError(/out of range/u);
expect(() => undefined).toThrow(late ? "out of range" : new RangeError("late"));
expect(() => undefined).toThrow(late ? new RangeError("late") : `out of ${String(late)}`);
expect(() => undefined).toThrow(/EN?OENT/u);

// A module of its own, so that its `expect` is not the other fixtures' global one.
export {};
