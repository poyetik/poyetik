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
  return Array.from({ length: wav.frames }, (_, frame) => view.getFloat32((frame * wav.channels + index) * 4, true));
}

/**
 * The largest distance between `samples` and the ideal `sine` computed in double
 * precision; infinite when a sample is not finite.
 */
export function sineError(samples: readonly number[], rate: number, sine: Sine): number {
  return samples.reduce((worst, sample, index) => {
    const ideal = sine.amplitude * Math.sin((2 * Math.PI * sine.frequency * index) / rate + sine.phase);
    return Math.max(worst, Number.isFinite(sample) ? Math.abs(sample - ideal) : Number.POSITIVE_INFINITY);
  }, 0);
}
