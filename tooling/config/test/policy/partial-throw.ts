/** A matcher of what a call throws. */
type Thrown = (expected?: unknown) => unknown;

/** A stand-in for the matchers of Vitest. */
declare const expect: ((actual: unknown) => { readonly toThrow: Thrown; readonly toThrowError: Thrown }) & {
  readonly objectContaining: (fields: object) => unknown;
  readonly stringContaining: (text: string) => unknown;
  readonly stringMatching: (pattern: RegExp) => unknown;
  readonly anything: () => unknown;
};

/** Another case. */
declare const early: boolean;

/** An error a test may hold, or not. */
declare const held: Error | undefined;

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
// What the sixth deep review of visual block 2 found beyond the parts a rule listed: a
// matcher of some fields or of a part, anything, a fallback, a choice inside a choice, a
// side of a choice that is a matcher of expect.
expect(() => undefined).toThrow(expect.objectContaining({ kind: "range" }));
expect(() => undefined).toThrow(expect.objectContaining({ message: expect.stringContaining("out") }));
expect(() => undefined).toThrow(expect.stringContaining("out of range"));
expect(() => undefined).toThrowError(expect.stringMatching(/out/u));
expect(() => undefined).toThrow(expect.anything());
expect(() => undefined).toThrow(held ?? new RangeError("late"));
expect(() => undefined).toThrow(late ? new RangeError("late") : early ? held : held);
expect(() => undefined).toThrow(late ? expect.anything : new RangeError("late"));
// What the seventh found: a pattern or a text built by `RegExp` or `String`, called or
// constructed, alone or on a side of a choice.
expect(() => undefined).toThrow(new RegExp("out of range", "u"));
expect(() => undefined).toThrow(RegExp("out of range", "u"));
expect(() => undefined).toThrowError(String(late));
expect(() => undefined).toThrow(late ? new RegExp("out", "u") : new RangeError("late"));

// A module of its own, so that its `expect` is not the other fixtures' global one.
export {};
