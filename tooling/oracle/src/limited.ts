/**
 * The channels of a case rendered through the output stage's true-peak limiter
 * (`spec/engine-design.md`, decision 12; `spec/measurements.md`, "The true-peak limiter"),
 * judged independently of the engine: from BS.1770's interpolator as the standard prints
 * it, the oracle derives how far the limited sine's true peak can lie from the ceiling, and
 * checks the render's latency, true peak and THD+N against it.
 *
 * Only the standard detector is judged: its coefficients are the standard's own table,
 * which the oracle holds apart from the engine; the windowed detectors are proven by the
 * engine's measurements.
 */

import type { RenderFormat } from "./format.ts";
import type { CaseOutput } from "./inputs.ts";
import { MAX_FRAMES, measure, sineBounds, type Measurements } from "./measure.ts";
import type { Sine } from "./signal.ts";

/** The interpolator of ITU-R BS.1770-4, Annex 2, as the standard prints it: twelve rows of the four phases. */
export const BS1770: readonly (readonly [number, number, number, number])[] = [
  [0.001708984375, -0.0291748046875, -0.0189208984375, -0.00830078125],
  [0.010986328125, 0.029296875, 0.0330810546875, 0.014892578125],
  [-0.0196533203125, -0.0517578125, -0.0582275390625, -0.026611328125],
  [0.033203125, 0.089111328125, 0.1015625, 0.047607421875],
  [-0.0594482421875, -0.16650390625, -0.2003173828125, -0.102294921875],
  [0.1373291015625, 0.465087890625, 0.77978515625, 0.97216796875],
  [0.97216796875, 0.77978515625, 0.465087890625, 0.1373291015625],
  [-0.102294921875, -0.2003173828125, -0.16650390625, -0.0594482421875],
  [0.047607421875, 0.1015625, 0.089111328125, 0.033203125],
  [-0.026611328125, -0.0582275390625, -0.0517578125, -0.0196533203125],
  [0.014892578125, 0.0330810546875, 0.0292968750, 0.010986328125],
  [-0.00830078125, -0.0189208984375, -0.0291748046875, 0.001708984375],
];

/** The default ceiling, in dBTP (decision 12): every protected case uses the default settings. */
export const CEILING_DBTP = -1;

/** Frames the standard detector looks behind its input: half its 12 taps. */
export const STANDARD_DELAY = 6;

/** Points of the scan of the under-read over a half period of the phase. */
export const SCAN = 4000;

/** Frames of `milliseconds` at `rate`, rounded to the nearest (halves up), at least one: the limiter's count. */
export function framesOf(milliseconds: number, rate: number): number {
  return Math.max(1, Math.round((milliseconds * rate) / 1000));
}

/** The latency of `output` at `rate`, in frames: its lookahead and the standard detector's delay. */
export function latency(output: CaseOutput, rate: number): number {
  return framesOf(output.lookahead, rate) + STANDARD_DELAY;
}

/**
 * `{ low, high }`: over a window holding a period of a sine of `omega` radians per frame, the
 * worst-case ratio of the standard detector's largest estimate to the sine's peak (`R`), and
 * the largest gain of its candidates (`G`). Each candidate is a sample of the interval
 * estimated (gain 1) or a phase of the interpolator (its response's gain and phase), moved
 * by `-omega` a frame later; `R` is scanned at {@link SCAN} points of a half period (where
 * `|sin|` repeats), then lowered by the scan's largest error, `G` times half a step.
 */
export function underRead(omega: number): { readonly low: number; readonly high: number } {
  // The four phases: the table's columns, read from its rows.
  const phases = [BS1770.map(([first]) => first), BS1770.map(([, second]) => second), BS1770.map(([, , third]) => third), BS1770.map(([, , , fourth]) => fourth)];
  const one: (readonly [number, number])[] = [
    [1, -6 * omega],
    [1, -5 * omega],
    ...phases.map((phase): readonly [number, number] => {
      const real = phase.reduce((sum, coefficient, tap) => sum + coefficient * Math.cos(omega * tap), 0);
      const imaginary = phase.reduce((sum, coefficient, tap) => sum - coefficient * Math.sin(omega * tap), 0);
      return [Math.hypot(real, imaginary), Math.atan2(imaginary, real)];
    }),
  ];
  const all = Array.from({ length: Math.floor((2 * Math.PI) / omega) + 1 }, (_frame, frame) => one.map(([gain, phase]) => [gain, phase - omega * frame] as const)).flat();
  const high = Math.max(...all.map(([gain]) => gain));
  const step = Math.PI / SCAN;
  const scanned = Array.from({ length: SCAN }, (_point, point) => {
    const at = step * point;
    return Math.max(
      ...all.map(([gain, phase]) => {
        // Stryker disable next-line ArithmeticOperator: every phase negated scans u backwards, |sin(-u - ψ)| = |sin(u + ψ)|, on a grid of a half period symmetric about it: the same minimum
        const angle = at + phase;
        return gain * Math.abs(Math.sin(angle));
      }),
    );
  });
  return { low: Math.min(...scanned) - (high * step) / 2, high };
}

/** The range a limited sine's amplitude, its true peak, lies in: `c/G` to `c/R`. */
export interface Range {
  /** `c/G`, the lowest. */
  readonly lowest: number;
  /** `c/R`, the highest. */
  readonly highest: number;
}

/** The range of a sine of `omega` radians per frame limited at the default ceiling, `c = 10^(CEILING_DBTP/20)`. */
export function range(omega: number): Range {
  const ceiling = 10 ** (CEILING_DBTP / 20);
  const { low, high } = underRead(omega);
  return { lowest: ceiling / high, highest: ceiling / low };
}

/** The relative rounding of a sample given in `format`: two units in the last place of binary64 (the gain and its product), or half a binary32 one and those. */
export function rounding(format: RenderFormat): number {
  return format === "binary64" ? 2 * Number.EPSILON : 2 ** -23;
}

/** The error per sample of a limited sine against the sine of the middle gain: half the range, and the rounding of the format at its top. */
export function limitedError(within: Range, format: RenderFormat): number {
  return (within.highest - within.lowest) / 2 + rounding(format) * within.highest;
}

/** The verdict on one limited channel: its problems, or its true peak and THD+N. */
export type Limited = { readonly problems: readonly string[] } | { readonly peak: number; readonly thdPlusN: number };

/**
 * Holds the measurements of a limited channel to `within`: its amplitude, the true peak,
 * between the range's ends within the fit's floor, and its THD+N at most that of a sine
 * within {@link limitedError} of the sine of the middle gain.
 */
export function holdLimited(levels: Measurements, within: Range, format: RenderFormat): Limited {
  const problems: string[] = [];
  const peak = levels.fundamental;
  if (!(peak <= within.highest + levels.sine.floor && peak >= within.lowest - levels.sine.floor)) {
    problems.push(`true peak ${String(peak)}, not between ${String(within.lowest)} and ${String(within.highest)}`);
  }
  const bounds = sineBounds((within.highest + within.lowest) / 2, limitedError(within, format), levels);
  if (bounds === null) {
    problems.push("its range leaves the fundamental no margin to be measured");
  } else if (!(levels.thdPlusN <= bounds.thdPlusN)) {
    problems.push(`THD+N ${String(levels.thdPlusN)} dB, above ${String(bounds.thdPlusN)} dB`);
  }
  return problems.length > 0 ? { problems } : { peak, thdPlusN: levels.thdPlusN };
}

/**
 * Judges one channel of a render through the protected `output`, expected to hold `sine`
 * limited at the default ceiling:
 * - its first {@link latency} frames are silence, exactly;
 * - once a period of the sine fits the window the limiter holds its needs over (its
 *   lookahead and 2 frames), and that window and the Hann smoothing have passed (two
 *   windows after the latency), its measurements hold to the sine's range
 *   ({@link holdLimited}).
 */
export function judgeLimited(samples: readonly number[], rate: number, sine: Sine, output: CaseOutput, format: RenderFormat): Limited {
  if (output.detector !== "standard") {
    return { problems: [`the oracle judges limited channels with the standard detector only, not ${output.detector}`] };
  }
  const window = framesOf(output.lookahead, rate) + 2;
  if (rate / sine.frequency > window) {
    return { problems: [`a period of the sine (${String(rate / sine.frequency)} frames) does not fit the limiter's window (${String(window)} frames)`] };
  }
  const delay = latency(output, rate);
  if (!samples.slice(0, delay).every((sample) => Object.is(sample, 0))) {
    return { problems: [`not silent over its latency, ${String(delay)} frames`] };
  }
  const settled = delay + 2 * window;
  let levels;
  try {
    levels = measure(Float64Array.from(samples.slice(settled, settled + MAX_FRAMES)), rate, sine.frequency);
  } catch (error) {
    // A MeasureError, or any other: a channel that cannot be measured fails its case.
    return { problems: [`cannot be measured: ${String(error)}`] };
  }
  return holdLimited(levels, range((2 * Math.PI * sine.frequency) / rate), format);
}
