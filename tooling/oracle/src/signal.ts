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

/**
 * The longest case whose reference is exact: a frequency (binary32, 24 significant bits)
 * times an index below 2^29 (29 bits) fits the 53 significant bits of binary64, so the
 * product is exact. Its size is never the limit: one bit more of index makes products
 * round, while they stay far below 2^53.
 */
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
 *   for an amplitude a (2^-25 at full scale), and never below 2^-150: under 2^-125 the
 *   samples are subnormal, spaced 2^-149 however small a is;
 * - phase accumulation in binary64: each of the `frames` additions of a phase below one
 *   cycle rounds by at most 2^-53 cycle, and the increment itself by 2^-53 of its value
 *   (below one cycle): 2^-52 cycle per sample, so 2π · frames · 2^-52 radians, times a;
 * - evaluation of the sine and of this reference (the 2π products and `Math.sin`, a few
 *   ulps of values below 4π: a reduced phase plus a start phase in [0, 2π), which
 *   `parseExpectations` requires): bounded by a · 2^-48.
 *
 * A box computing its phase in binary32 exceeds it (measured over one second at 48 kHz).
 * So does a gain error of 5e-7, over cases up to about 3e8 frames (1.7 hours at 48 kHz):
 * the accumulation term grows with the case, and the bound reaches 5e-7 at about 3.37e8
 * frames, 7.8e-7 at {@link MAX_FRAMES}.
 */
export function sineErrorBound(amplitude: number, frames: number): number {
  const magnitude = Math.abs(amplitude);
  if (magnitude === 0) {
    // Silence is exact: it allows no error.
    return 0;
  }
  const exponent = Math.max(Math.ceil(Math.log2(magnitude)), -125);
  return 2 ** (exponent - 25) + magnitude * (2 * Math.PI * frames * 2 ** -52 + 2 ** -48);
}
