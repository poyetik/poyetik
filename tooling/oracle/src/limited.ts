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
import { MAX_FRAMES, measure, sineBounds } from "./measure.ts";
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
 * by `-omega` a frame later; `R` is scanned at {@link SCAN} points, then lowered by the
 * scan's largest error, `G` times half a step.
 */
export function underRead(omega: number): { readonly low: number; readonly high: number } {
  const one: [number, number][] = [
    [1, -6 * omega],
    [1, -5 * omega],
  ];
  // The four phases: the table's columns, read from its rows.
  const phases = [BS1770.map(([first]) => first), BS1770.map(([, second]) => second), BS1770.map(([, , third]) => third), BS1770.map(([, , , fourth]) => fourth)];
  for (const phase of phases) {
    let real = 0;
    let imaginary = 0;
    phase.forEach((coefficient, tap) => {
      real += coefficient * Math.cos(omega * tap);
      imaginary -= coefficient * Math.sin(omega * tap);
    });
    one.push([Math.hypot(real, imaginary), Math.atan2(imaginary, real)]);
  }
  const frames = Math.floor((2 * Math.PI) / omega) + 1;
  const all: [number, number][] = [];
  for (let frame = 0; frame < frames; frame += 1) {
    for (const [gain, phase] of one) {
      all.push([gain, phase - omega * frame]);
    }
  }
  const high = Math.max(...all.map(([gain]) => gain));
  const step = Math.PI / SCAN;
  let low = Infinity;
  for (let point = 0; point < SCAN; point += 1) {
    let best = 0;
    for (const [gain, phase] of all) {
      best = Math.max(best, gain * Math.abs(Math.sin(step * point + phase)));
    }
    low = Math.min(low, best);
  }
  return { low: low - (high * step) / 2, high };
}

/** The verdict on one limited channel: its problems, or its true peak and THD+N. */
export type Limited = { readonly problems: readonly string[] } | { readonly peak: number; readonly thdPlusN: number };

/**
 * Judges one channel of a render through the protected `output`, expected to hold `sine`
 * limited at the default ceiling:
 * - its first {@link latency} frames are silence, exactly;
 * - once a period of the sine fits the window the limiter holds its needs over (its
 *   lookahead and 2 frames) and that window and the Hann smoothing have passed, its
 *   amplitude, the limited sine's true peak, lies between `c/G` and `c/R`, `c` the
 *   ceiling's amplitude, within the fit's floor;
 * - each sample is then within `(c/R - c/G)/2` of the sine of the middle gain, plus the
 *   rounding of the format, which bounds its THD+N.
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
  const silent = samples.slice(0, delay).every((sample) => Object.is(sample, 0));
  const settled = delay + 2 * window;
  const tail = Float64Array.from(samples.slice(settled, settled + MAX_FRAMES));
  if (!silent) {
    return { problems: [`not silent over its latency, ${String(delay)} frames`] };
  }
  const ceiling = 10 ** (CEILING_DBTP / 20);
  const { low, high } = underRead((2 * Math.PI * sine.frequency) / rate);
  const [lowest, highest] = [ceiling / high, ceiling / low];
  let levels;
  try {
    levels = measure(tail, rate, sine.frequency);
  } catch (error) {
    // A MeasureError, or any other: a channel that cannot be measured fails its case.
    return { problems: [`cannot be measured: ${String(error)}`] };
  }
  const problems: string[] = [];
  if (!(levels.fundamental <= highest + levels.sine.floor && levels.fundamental >= lowest - levels.sine.floor)) {
    problems.push(`true peak ${String(levels.fundamental)}, not between ${String(lowest)} and ${String(highest)}`);
  }
  const rounding = format === "binary64" ? 2 * Number.EPSILON : 2 ** -23;
  const bounds = sineBounds((highest + lowest) / 2, (highest - lowest) / 2 + rounding * highest, levels);
  if (bounds !== null && !(levels.thdPlusN <= bounds.thdPlusN)) {
    problems.push(`THD+N ${String(levels.thdPlusN)} dB, above ${String(bounds.thdPlusN)} dB`);
  }
  return problems.length > 0 ? { problems } : { peak: levels.fundamental, thdPlusN: levels.thdPlusN };
}
