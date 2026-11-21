/** A matcher of what a call throws. */
type Thrown = (expected?: unknown) => unknown;

/** A stand-in for the matchers of Vitest. */
declare const expect: (actual: unknown) => { readonly toThrow: Thrown; readonly toThrowError: Thrown; readonly rejects: { readonly toThrow: Thrown } };

// An error class alone, given to each matcher of what a call throws.
expect(() => undefined).toThrow(RangeError);
expect(() => undefined).toThrowError(TypeError);
expect(Promise.resolve()).rejects.toThrow(Error);

// A module of its own, so that its `expect` is not the other fixtures' global one.
export {};
