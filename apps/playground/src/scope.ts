/**
 * The oscilloscope as data: what to draw and what to measure, computed from samples. Pure
 * functions, with no drawing: the renderer (`draw.ts`) only executes the scene.
 */

/** A point on the canvas, in pixels. */
export interface Point {
  readonly x: number;
  readonly y: number;
}

/** What the oscilloscope draws. */
export interface ScopeScene {
  readonly width: number;
  readonly height: number;
  /** Horizontal lines of the grid, in pixels from the top: the middle (zero) and ±1. */
  readonly grid: readonly number[];
  /** The signal, left to right. */
  readonly trace: readonly Point[];
}

/** Fraction of the half height used by a full-scale signal (±1), leaving a margin. */
const SCALE = 0.9;

/**
 * Positions where the signal crosses zero going up, in fractional samples: between
 * `index - 1` (negative or zero) and `index` (positive), linearly interpolated.
 */
export function risingCrossings(samples: Float32Array): number[] {
  const crossings: number[] = [];
  // No sample before the first one: +Infinity is never "negative or zero".
  let before = Number.POSITIVE_INFINITY;
  samples.forEach((after, index) => {
    if (before <= 0 && after > 0) {
      crossings.push(index - 1 + -before / (after - before));
    }
    before = after;
  });
  return crossings;
}

/** The frequency of a periodic signal, in hertz, from its rising zero crossings; undefined with fewer than two. */
export function frequency(samples: Float32Array, rate: number): number | undefined {
  const crossings = risingCrossings(samples);
  // Crossings strictly increase: the span is positive exactly when there are two or more.
  const span = (crossings.at(-1) ?? 0) - (crossings[0] ?? 0);
  return span > 0 ? ((crossings.length - 1) * rate) / span : undefined;
}

/** The largest absolute sample value; 0 for no samples. */
export function peak(samples: Float32Array): number {
  return samples.reduce((largest, sample) => Math.max(largest, Math.abs(sample)), 0);
}

/**
 * The scene for `samples`: `windowSamples` samples across `width` pixels, starting at a
 * rising zero crossing so that a periodic signal stands still (the latest one that leaves
 * a full window, else the most recent window).
 */
export function scopeScene(samples: Float32Array, windowSamples: number, width: number, height: number): ScopeScene {
  const middle = height / 2;
  const amplitude = middle * SCALE;
  const latestStart = samples.length - windowSamples;
  const trigger = risingCrossings(samples)
    .map(Math.ceil)
    .filter((index) => index <= latestStart)
    .pop();
  const start = trigger ?? Math.max(0, latestStart);
  const window = samples.subarray(start, start + windowSamples);
  const step = windowSamples > 1 ? width / (windowSamples - 1) : 0;
  return {
    width,
    height,
    grid: [middle - amplitude, middle, middle + amplitude],
    trace: Array.from(window, (sample, index) => ({ x: index * step, y: middle - sample * amplitude })),
  };
}
