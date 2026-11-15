// A seeded generator for the tests: the same values on every run and every machine.

/** SplitMix64 (Steele, Lea and Flood, 2014), as the specification gives it: uniform values in [0, 1) of 53 bits. */
export function splitMix(seed: bigint): () => number {
  let state = seed;
  const mask = (1n << 64n) - 1n;
  return () => {
    state = (state + 0x9e3779b97f4a7c15n) & mask;
    let mixed = state;
    mixed = ((mixed ^ (mixed >> 30n)) * 0xbf58476d1ce4e5b9n) & mask;
    mixed = ((mixed ^ (mixed >> 27n)) * 0x94d049bb133111ebn) & mask;
    mixed ^= mixed >> 31n;
    return Number(mixed >> 11n) * 2 ** -53;
  };
}
