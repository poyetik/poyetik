/**
 * The render formats of a conformance case (`spec/render-hash.md`): the precision that
 * computed a render and the width of its samples. Every case has one render, one WAV file
 * and one reference hash per format.
 */

/** Every render format, in the order of `conformance/references.json`. */
export const RENDER_FORMATS = ["binary64", "binary64-as-binary32", "binary32"] as const;

/** A render format: computed and given in binary64, computed in binary64 and given in binary32, or computed and given in binary32. */
export type RenderFormat = (typeof RENDER_FORMATS)[number];

/** The width, in bits, of the precision that computes a render of `format`. */
export function precisionBits(format: RenderFormat): 32 | 64 {
  return format === "binary32" ? 32 : 64;
}

/** The width, in bits, of the samples a render of `format` gives. */
export function sampleBits(format: RenderFormat): 32 | 64 {
  return format === "binary64" ? 64 : 32;
}
