/**
 * The oscilloscope as data: what to draw and what to measure, computed from samples. Pure
 * functions, with no drawing: the renderer (`draw.ts`) only executes the scene.
 */

/** A point on the canvas, in pixels. */
export interface Point {
  /** Horizontal position, in pixels from the left. */
  readonly x: number;
  /** Vertical position, in pixels from the top. */
  readonly y: number;
}

/** What the oscilloscope draws. */
export interface ScopeScene {
  /** Width of the scene, in pixels. */
  readonly width: number;
  /** Height of the scene, in pixels. */
  readonly height: number;
  /** Horizontal lines of the grid, in pixels from the top: the middle (zero) and ±1. */
  readonly grid: readonly number[];
  /** The signal, left to right. */
  readonly trace: readonly Point[];
}

/** Fraction of the half height used by a full-scale signal (±1), leaving a margin. */
const SCALE = 0.9;

/**
 * Calls `visit` with each position where the signal crosses zero going up, in order, in
 * fractional samples: between `index - 1` (negative or zero) and `index` (positive),
 * linearly interpolated.
 */
function eachRisingCrossing(samples: Float32Array, visit: (position: number) => void): void {
  // No sample before the first one: +Infinity is never "negative or zero".
  let before = Number.POSITIVE_INFINITY;
  samples.forEach((after, index) => {
    if (before <= 0 && after > 0) {
      visit(index - 1 + -before / (after - before));
    }
    before = after;
  });
}

/** Positions where the signal crosses zero going up, in fractional samples, in increasing order. */
export function risingCrossings(samples: Float32Array): number[] {
  const crossings: number[] = [];
  eachRisingCrossing(samples, (position) => {
    crossings.push(position);
  });
  return crossings;
}

/**
 * The whole periods between the first and the last rising crossings (-1 when there is
 * none), and the distance between those crossings, in samples (0 for fewer than two). In
 * one pass, with nothing kept but three numbers: any length of signal is measured.
 */
export function crossingSpan(samples: Float32Array): { readonly periods: number; readonly span: number } {
  let count = 0;
  let first = 0;
  let last = 0;
  eachRisingCrossing(samples, (position) => {
    if (count === 0) {
      first = position;
    }
    last = position;
    count += 1;
  });
  return { periods: count - 1, span: last - first };
}

/** Fewest whole periods a frequency is measured over: fewer give no measurement. */
export const MIN_PERIODS = 3;

/** Highest frequency measured, as a fraction of the sample rate: above, no measurement. */
export const MAX_FREQUENCY_RATIO = 1 / 20;

/**
 * The frequency of a periodic signal, in hertz, from its rising zero crossings (linearly
 * interpolated). Measured only within its domain: at least {@link MIN_PERIODS} periods,
 * and at most {@link MAX_FREQUENCY_RATIO} of the rate, where it is within 0.05 Hz of the
 * truth for a sine at every rate of the test matrix (tested). Above that ratio the linear
 * interpolation of the crossings is biased (1.3 Hz off at 19 kHz and 48 kHz): undefined.
 */
export function frequency(samples: Float32Array, rate: number): number | undefined {
  const { periods, span } = crossingSpan(samples);
  if (periods < MIN_PERIODS) {
    return undefined;
  }
  // Crossings strictly increase: with two or more, the span is positive.
  const measured = (periods * rate) / span;
  return measured <= rate * MAX_FREQUENCY_RATIO ? measured : undefined;
}

/** The largest absolute sample value; 0 for no samples. */
export function peak(samples: Float32Array): number {
  return samples.reduce((largest, sample) => Math.max(largest, Math.abs(sample)), 0);
}

/**
 * The scene for `samples`: `windowSamples` samples across `width` pixels, starting at a
 * rising zero crossing so that a periodic signal stands still (the latest one that leaves
 * a full window, else the most recent window). The crossing falls between two samples:
 * the trace is shifted by that fraction of a sample, so that the crossing is exactly at
 * the left edge and the trace never jitters by up to a sample from frame to frame.
 */
export function scopeScene(samples: Float32Array, windowSamples: number, width: number, height: number): ScopeScene {
  const middle = height / 2;
  const amplitude = middle * SCALE;
  const latestStart = samples.length - windowSamples;
  const crossing = risingCrossings(samples)
    .filter((position) => Math.ceil(position) <= latestStart)
    .pop();
  const start = crossing === undefined ? Math.max(0, latestStart) : Math.ceil(crossing);
  const shift = crossing === undefined ? 0 : start - crossing;
  const window = samples.subarray(start, start + windowSamples);
  const step = windowSamples > 1 ? width / (windowSamples - 1) : 0;
  return {
    width,
    height,
    grid: [middle - amplitude, middle, middle + amplitude],
    trace: Array.from(window, (sample, index) => ({ x: (index + shift) * step, y: middle - sample * amplitude })),
  };
}
