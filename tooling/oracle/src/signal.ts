/** What a channel of a WAV file contains, measured against its mathematical definition. */

import type { RenderFormat } from "./format.ts";
import type { Wav } from "./wav.ts";

/** A sine wave: `amplitude × sin(2π × frequency × t + phase)`. */
export interface Sine {
  /** In hertz. */
  readonly frequency: number;
  /** Peak value, in sample units (1 is full scale). */
  readonly amplitude: number;
  /** In radians, at the first sample. */
  readonly phase: number;
}

/** The samples of one channel, as numbers (binary32 samples exactly, as binary64 ones). */
export function channel(wav: Wav, index: number): number[] {
  const view = new DataView(wav.data.buffer, wav.data.byteOffset, wav.data.byteLength);
  const at = (frame: number): number => frame * wav.channels + index;
  return Array.from({ length: wav.frames }, (_empty, frame) => (wav.bits === 64 ? view.getFloat64(at(frame) * 8, true) : view.getFloat32(at(frame) * 4, true)));
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
 * samples of a render of `format`: what the engine promises for every sine it produces
 * (`spec/measurements.md`, "Thresholds of `osc~`", and the `sine_error_bounds` of
 * `spec/measurement-vectors.json`). Derived, term by term, for an amplitude a, with
 * e = ⌈log2 |a|⌉:
 *
 * - computing and giving the sine, by format:
 *   - binary64: the sine computed in binary64, half an ulp below |a|'s power of two,
 *     2^(e − 54); values below 2^-1022 are flushed to zero, an error up to 2^-1022, which
 *     the exponent's floor of −968 keeps;
 *   - binary64 given as binary32: the binary64 sine rounded to binary32, half an ulp,
 *     2^(e − 25), and never below 2^-150: under 2^-125 the samples are binary32 subnormals,
 *     spaced 2^-149 however small a is;
 *   - binary32: the angle 2π · phase, below 8, rounded to binary32 within 2^-22, which moves
 *     the sine by as much; `sinf` within 2^-33 of the sine of that angle before its result
 *     is rounded to binary32, half an ulp, 2^(e − 25); values below 2^-126 are flushed to
 *     zero, an error up to 2^-126, which the exponent's floor of −101 keeps; the first two
 *     terms scale with |a|;
 * - phase accumulation in binary64, in every format, at c = frequency / rate cycles per
 *   sample: the increment is c rounded, less its whole cycles (an exact subtraction), and
 *   the phase below one cycle adds it once per sample. By regime of c:
 *   - below 1: c rounds by at most 2^-54 (half an ulp below 1), and each addition, of a
 *     sum below 2, by at most 2^-53;
 *   - from 1 to 4: c rounds by at most half its ulp, 2^-52 from 2 on, but the increment
 *     and every phase are then multiples of that ulp, which sums below 2 hold exactly:
 *     the additions are exact;
 *   so at most 2^-52 cycle per sample, 2π · frames · 2^-52 radians, times |a|. From 4
 *   cycles on, c rounds by up to 2^-51 and this term no longer holds: `parseExpectations`
 *   requires a frequency in [0, 4 × rate);
 * - evaluation of the sine and of this reference (the 2π products and `Math.sin`, a few
 *   ulps of values below 4π: a reduced phase plus a start phase in [0, 2π), which
 *   `parseExpectations` requires): bounded by |a| · 2^-48.
 *
 * Given as binary32, a box computing its phase in binary32 exceeds it (measured over one
 * second at 48 kHz). So does a gain error of 5e-7, over cases up to about 3e8 frames (1.7
 * hours at 48 kHz): the accumulation term grows with the case, and the bound reaches 5e-7
 * at about 3.37e8 frames, 7.8e-7 at {@link MAX_FRAMES}.
 */
export function sineErrorBound(amplitude: number, frames: number, format: RenderFormat): number {
  const magnitude = Math.abs(amplitude);
  if (magnitude === 0) {
    // Silence is exact: it allows no error.
    return 0;
  }
  const exponent = Math.ceil(Math.log2(magnitude));
  const accumulated = 2 * Math.PI * frames * 2 ** -52 + 2 ** -48;
  switch (format) {
    case "binary64":
      return 2 ** (Math.max(exponent, -968) - 54) + magnitude * accumulated;
    case "binary64-as-binary32":
      return 2 ** (Math.max(exponent, -125) - 25) + magnitude * accumulated;
    case "binary32":
      return 2 ** (Math.max(exponent, -101) - 25) + magnitude * (2 ** -22 + 2 ** -33 + accumulated);
  }
}
