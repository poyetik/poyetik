// Compile-time equality of two unions, for the tables of states the tests keep: a table keyed
// by a union written by hand can miss a state its type has (the third deep review of visual block
// 2 found the square's table without the GPU's lost and failed endings), which `tsc` proves it
// does not.

/** `true` when `A` and `B` are the same union, `false` otherwise. */
export type KeysEqual<A, B> = [A] extends [B] ? ([B] extends [A] ? true : false) : false;

/**
 * `proof` given back: only `true` compiles when `A` and `B` are the same union, so that a
 * table whose keys are not exactly its type's states does not compile.
 */
export function keysEqual<A, B>(proof: KeysEqual<A, B>): KeysEqual<A, B> {
  return proof;
}
