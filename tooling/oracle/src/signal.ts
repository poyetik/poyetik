/** What a channel of a WAV file contains, measured against its mathematical definition. */

import type { Wav } from "./wav.ts";

/** A sine wave: `amplitude × sin(2π × frequency × t + phase)`. */
export interface Sine {
  /** In hertz. */
  readonly frequency: number;
  readonly amplitude: number;
  /** In radians, at the first sample. */
  readonly phase: number;
}

/** The samples of one channel, as numbers. */
export function channel(wav: Wav, index: number): number[] {
  const view = new DataView(wav.data.buffer, wav.data.byteOffset, wav.data.byteLength);
  return Array.from({ length: wav.frames }, (_empty, frame) => view.getFloat32((frame * wav.channels + index) * 4, true));
}

/**
 * The ideal value of `sine` at sample `index`, in double precision.
 *
 * The phase is reduced exactly before the sine: `(frequency × index) mod rate`. Both
 * operations are exact in binary64 when the frequency is a binary32 value (24
 * significant bits, as parameters are) and `index` is below 2^29 (see
 * {@link MAX_FRAMES}): the reference loses no precision however long the case.
 */
export function ideal(sine: Sine, rate: number, index: number): number {
  return sine.amplitude * Math.sin((2 * Math.PI * ((sine.frequency * index) % rate)) / rate + sine.phase);
}

/** The longest case whose reference is exact: frequency × index stays below 2^53. */
export const MAX_FRAMES = 2 ** 29;

/**
 * The largest distance between `samples` and the ideal `sine` computed in double
 * precision; infinite when a sample is not finite.
 */
export function sineError(samples: readonly number[], rate: number, sine: Sine): number {
  return samples.reduce((worst, sample, index) => {
    return Math.max(worst, Number.isFinite(sample) ? Math.abs(sample - ideal(sine, rate, index)) : Number.POSITIVE_INFINITY);
  }, 0);
}

/**
 * The largest distance a correct sine box may have from the ideal sine, over `frames`
 * samples: what the engine promises for every sine it produces (phase accumulated in
 * double precision, output in binary32). Derived, term by term:
 *
 * - output rounding to binary32: half an ulp of the largest value, 2^(⌈log2 a⌉ − 25)
 *   for an amplitude a (2^-25 at full scale);
 * - phase accumulation in binary64: each of the `frames` additions of a phase below one
 *   cycle rounds by at most 2^-53 cycle, and the increment itself by 2^-53 of its value
 *   (below one cycle): 2^-52 cycle per sample, so 2π · frames · 2^-52 radians, times a;
 * - evaluation of the sine and of this reference (the 2π products and `Math.sin`, a few
 *   ulps of values below 2π): bounded by a · 2^-48.
 *
 * A box computing its phase in binary32, or with a gain error of 5e-7, exceeds it.
 */
export function sineErrorBound(amplitude: number, frames: number): number {
  const magnitude = Math.abs(amplitude);
  // At a zero magnitude, log2 is -Infinity and every term is 0: silence allows no error.
  return 2 ** (Math.ceil(Math.log2(magnitude)) - 25) + magnitude * (2 * Math.PI * frames * 2 ** -52 + 2 ** -48);
}
