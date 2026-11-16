/**
 * The channels of a case rendered through the output stage's true-peak limiter
 * (`spec/engine-design.md`, decision 12; `spec/measurements.md`, "The true-peak limiter"),
 * judged independently of the engine: from the detector's interpolator and the limiter's
 * stages, the oracle derives how far the limited sine's amplitude can lie from the ceiling
 * and from which frame it has settled there ({@link limitedPlan}), and checks the render's
 * latency, its fundamental and THD+N against it. What it measures is the fundamental of a
 * sine fit, the amplitude of the limited sine, not a true peak read between the samples of
 * the output.
 *
 * Every detector is judged, each from its own interpolator held apart from the engine: the
 * standard's table as it prints it, and the windowed ones computed from their definition
 * (decision 12: a Kaiser-windowed sinc, each phase at a unit gain at DC).
 */

import { RENDER_FORMATS, type RenderFormat } from "./format.ts";
import type { CaseOutput, DETECTORS } from "./inputs.ts";
import { MAX_FRAMES, MeasureError, hasMargin, measure, sineBounds, sineFitConditioning, type Measurements } from "./measure.ts";
import { fold } from "./quality.ts";
import { ideal, sineErrorBound, type Sine } from "./signal.ts";

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

/** A peak detector of the limiter. */
export type Detector = (typeof DETECTORS)[number];

/**
 * The interpolator of a detector: its phases, each holding its taps, tap `k` multiplying the
 * sample `k` frames before the newest. It estimates the interval between the samples
 * `taps / 2` and `taps / 2 - 1` frames before the newest ({@link delayOf}).
 */
export interface Interpolator {
  /** Its phases, each of {@link Interpolator.taps} coefficients. */
  readonly phases: readonly (readonly number[])[];
  /** Its taps per phase. */
  readonly taps: number;
}

/**
 * The modified Bessel function of the first kind and order zero, `I0(x)`, by its series
 * `Σ ((x/2)^k / k!)^2`, summed until a term no longer changes the sum.
 */
export function besselI0(x: number): number {
  const half = x / 2;
  let term = 1;
  let sum = 0;
  for (let order = 1; ; order++) {
    const next = sum + term;
    if (next === sum) {
      return sum;
    }
    sum = next;
    term *= (half / order) ** 2;
  }
}

/**
 * The windowed interpolator of decision 12 for oversampling by `phases`: a sinc cut at half
 * the input rate, `sin(π m / phases) / (π m / phases)` at the point `m` frames of the output
 * rate from the centre of its `phases × taps` points, under the Kaiser window of `beta`
 * spanning them, `I0(β √(1 − r²))` (`r` from −1 at the first point to 1 at the last; its
 * scale, `1 / I0(β)`, cancels below), each phase then scaled to a unit gain at DC. Phase `p`
 * takes the points `k × phases + p`, tap `k` its `k`-th.
 *
 * Each coefficient lies within `(taps + 14) · 2^-53` of its exact value, the engine's as the
 * oracle's: each is at most 1, the phase's gain sums `taps` points (`taps` roundings of
 * relative 2^-53 moving every coefficient by at most `taps · 2^-53`), and a point takes a dozen
 * roundings of 2^-53 at most (π, the angle, its sine and quotient, the ratio, its square,
 * the root, the series of `I0` and the products), 14 counting the division by the gain.
 * Measured against 60-digit decimal values: at most 2.4 · 2^-53 (precise) and 9.9 · 2^-53
 * (mastering), on 2026-11-16.
 */
export function kaiser(phases: number, taps: number, beta: number): Interpolator {
  const centre = (phases * taps - 1) / 2;
  const point = (index: number): number => {
    const offset = index - centre;
    const angle = (Math.PI * offset) / phases;
    const ratio = offset / centre;
    return (Math.sin(angle) / angle) * besselI0(beta * Math.sqrt(1 - ratio * ratio));
  };
  return {
    taps,
    phases: Array.from({ length: phases }, (_phase, phase) => {
      const raw = Array.from({ length: taps }, (_tap, tap) => point(tap * phases + phase));
      const gain = raw.reduce((sum, coefficient) => sum + coefficient, 0);
      return raw.map((coefficient) => coefficient / gain);
    }),
  };
}

/** The interpolator of each detector (decision 12): BS.1770's table, read phase by phase from its columns, and the two windowed ones. */
export const INTERPOLATORS: Readonly<Record<Detector, Interpolator>> = {
  standard: {
    taps: BS1770.length,
    phases: [BS1770.map(([first]) => first), BS1770.map(([, second]) => second), BS1770.map(([, , third]) => third), BS1770.map(([, , , fourth]) => fourth)],
  },
  precise: kaiser(4, 32, 2.5),
  mastering: kaiser(8, 128, 5),
};

/** Frames a detector looks behind its input: half its taps, the interval it estimates lying between the samples that many frames and one fewer before the newest. */
export function delayOf(interpolator: Interpolator): number {
  return interpolator.taps / 2;
}

/** Points of the scan of the under-read over a half period of the phase. */
export const SCAN = 4000;

/** Frames of `milliseconds` at `rate`, rounded to the nearest (halves up), at least one: the limiter's count. */
export function framesOf(milliseconds: number, rate: number): number {
  return Math.max(1, Math.round((milliseconds * rate) / 1000));
}

/** The latency of `output` at `rate`, in frames: its lookahead and its detector's delay. */
export function latency(output: CaseOutput, rate: number): number {
  return framesOf(output.lookahead, rate) + delayOf(INTERPOLATORS[output.detector]);
}

/**
 * `{ low, high }`: over a window holding a period of a sine of `omega` radians per frame, the
 * worst-case ratio of the largest estimate of the detector of `interpolator` to the sine's peak (`R`), and
 * the largest gain of its candidates (`G`). Each candidate is a sample of the interval
 * estimated (gain 1) or a phase of the interpolator (its response's gain and phase), moved
 * by `-omega` a frame later; `R` is scanned at {@link SCAN} points of a half period (where
 * `|sin|` repeats), then lowered by the scan's largest error, `G` times half a step.
 */
export function underRead(omega: number, interpolator: Interpolator): { readonly low: number; readonly high: number } {
  const delay = delayOf(interpolator);
  const one: (readonly [number, number])[] = [
    // The interval's ends, `delay` and `delay - 1` frames before the newest sample.
    [1, -delay * omega],
    [1, -(delay - 1) * omega],
    ...interpolator.phases.map((phase): readonly [number, number] => {
      const real = phase.reduce((sum, coefficient, tap) => sum + coefficient * Math.cos(omega * tap), 0);
      const imaginary = phase.reduce((sum, coefficient, tap) => sum - coefficient * Math.sin(omega * tap), 0);
      return [Math.hypot(real, imaginary), Math.atan2(imaginary, real)];
    }),
  ];
  // The phase each frame of one period adds, `ω·frame`: the candidates of a frame are the
  // interval's ends and the phases shifted by it, scanned as they are made, never stored
  // together, so that the memory holds one period's shifts whatever `ω` is.
  const shifts = Array.from({ length: Math.floor((2 * Math.PI) / omega) + 1 }, (_frame, frame) => omega * frame);
  const high = Math.max(...one.map(([gain]) => gain));
  const step = Math.PI / SCAN;
  const scanned = Array.from({ length: SCAN }, (_point, point) => {
    const at = step * point;
    return shifts.reduce(
      (largest, shift) =>
        one.reduce((frame, [gain, phase]) => {
          const shifted = phase - shift;
          // Stryker disable next-line ArithmeticOperator: every phase negated scans u backwards, |sin(-u - ψ)| = |sin(u + ψ)|, on a grid of a half period symmetric about it: the same minimum
          const angle = at + shifted;
          return Math.max(frame, gain * Math.abs(Math.sin(angle)));
        }, largest),
      0,
    );
  });
  return { low: Math.min(...scanned) - (high * step) / 2, high };
}

/** The range of a limited sine's amplitude, the fundamental of its sine fit: `min(A, c/G)` to `min(A, c/R)`. */
export interface Range {
  /** `min(A, c/G)`, the lowest. */
  readonly lowest: number;
  /** `min(A, c/R)`, the highest. */
  readonly highest: number;
}

/** The ceiling of the default settings as an amplitude, `c = 10^(CEILING_DBTP/20)`. */
export function ceiling(): number {
  return 10 ** (CEILING_DBTP / 20);
}

/**
 * The range of a sine of amplitude `amplitude` limited at the default ceiling through a
 * detector that under-reads it by `read` ({@link underRead}): the steady gain lies between
 * `c/(G·A)` and `c/(R·A)` (`spec/measurements.md`, "The true-peak limiter"), and never above
 * 1, the limiter changing no bit below the ceiling.
 */
export function range(read: { readonly low: number; readonly high: number }, amplitude: number): Range {
  const magnitude = Math.abs(amplitude);
  return { lowest: Math.min(magnitude, ceiling() / read.high), highest: Math.min(magnitude, ceiling() / read.low) };
}

/** The release of the default settings, in milliseconds (decision 12, `audio_core::LimiterSettings::DEFAULT`): every protected case uses it. */
export const RELEASE_MS = 100;

/** The adaptivity of the release of the default settings, from 0 to 1. */
export const ADAPTIVITY = 0.5;

/** How much faster the fast release stage is than the release, at an adaptivity of 1 (`audio_core`'s `FAST_RELEASE_DIVISOR`). */
export const FAST_RELEASE_DIVISOR = 10;

/** The margin per frame of the maximum lookahead, 64 units of 2^-53: `2^-47` (`spec/measurements.md`, "The true-peak limiter"). */
const MARGIN_PER_FRAME = 2 ** -47;

/**
 * The margin the engine adds to every need above the ceiling, absolute, so that the
 * rounding of its control path never leaves a reduction below the one needed: `E = 64·M·2^-53`,
 * `M` the maximum lookahead in frames, which a case's lookahead is (`spec/measurements.md`,
 * "The true-peak limiter"), exact (a count times a power of two). It lowers the steady gain
 * by as much, the amplitude by `A` times it ({@link allowance}); the corpus's plans take it
 * (`join` in `oracle.ts`).
 */
export function needMargin(output: CaseOutput, rate: number): number {
  return framesOf(output.lookahead, rate) * MARGIN_PER_FRAME;
}

/** The unit roundoff of binary64, 2^-53: the relative rounding of one operation of the limiter's control path, which runs in binary64. */
const UNIT = 2 ** -53;

/** The fast release time of the default settings, in milliseconds: the release and its {@link FAST_RELEASE_DIVISOR}-th mixed by the adaptivity. */
export function fastRelease(): number {
  return RELEASE_MS * (1 - ADAPTIVITY) + (RELEASE_MS / FAST_RELEASE_DIVISOR) * ADAPTIVITY;
}

/** The share a one-pole of `milliseconds` keeps per frame at `rate`: `exp(-1 / frames)`. */
export function keep(milliseconds: number, rate: number): number {
  return Math.exp(-1000 / (milliseconds * rate));
}

/** The fast stage's keep at `rate`, raised by `4u` for the engine's own (its exponential and the product's rounding): the slowest it can release. */
export function fastKeep(rate: number): number {
  return keep(fastRelease(), rate) + 4 * UNIT;
}

/** The slow stage's keep at `rate`, lowered by `4u`: the fastest it can rise. */
export function slowKeep(rate: number): number {
  return keep(RELEASE_MS, rate) - 4 * UNIT;
}

/**
 * The frames after `settled - 1` from which the fast stage releases to at most
 * `steady + error`, its largest need in the onset `onset` (steps 4 and 5 of
 * {@link limitedPlan}): 0 when the onset needs no more than the steady sine; null when the
 * slow stage's return is not derived (the steady sine needs nothing while its onset does, or
 * the slow stage rises above `steady + error` by frame `settled`).
 */
export function released(onset: number, steady: number, error: number, rate: number, settled: number): number | null {
  if (!(onset > steady)) {
    return 0;
  }
  const slow = (onset + error) * (1 - slowKeep(rate) ** settled);
  if (steady === 0 || slow > steady + error) {
    return null;
  }
  return Math.ceil(Math.log((onset + error) / (steady + error)) / -Math.log(fastKeep(rate)));
}

/**
 * The detector's estimate at frame `frame` of `signal` (`signal(n)`, zero before frame 0):
 * the largest magnitude of the interval's two ends and of each phase of `interpolator`.
 */
export function estimate(interpolator: Interpolator, signal: (frame: number) => number, frame: number): number {
  const delay = delayOf(interpolator);
  const phases = interpolator.phases.map((phase) =>
    Math.abs(
      phase.reduce((sum, coefficient, tap) => {
        const product = coefficient * signal(frame - tap);
        // Stryker disable next-line ArithmeticOperator: every product subtracted from 0 gives the sum's negation, whose magnitude is the same
        return sum + product;
      }, 0),
    ),
  );
  return Math.max(Math.abs(signal(frame - delay)), Math.abs(signal(frame - delay + 1)), ...phases);
}

/** The reduction an estimate `peak` needs to stay at the ceiling: `max(0, (peak - c) / max(peak, c))`. */
export function need(peak: number): number {
  return Math.max(0, (peak - ceiling()) / Math.max(peak, ceiling()));
}

/** The largest sum of the magnitudes of a phase's coefficients, at least 1 (the interval's ends): how much an error per sample can move an estimate. */
export function spread(interpolator: Interpolator): number {
  return Math.max(1, ...interpolator.phases.map((phase) => phase.reduce((sum, coefficient) => sum + Math.abs(coefficient), 0)));
}

/** What a limited channel's samples may lie off the exact limited sine, per sample, below and above, absolute. */
export interface Allowance {
  /** Toward a lower amplitude. */
  readonly below: number;
  /** Toward a higher amplitude. */
  readonly above: number;
}

/**
 * The plan of a limited channel: the range its amplitude settles in, the frame from which it
 * has settled, and the allowance of each render format; a fact of the case and its
 * expectation alone ({@link limitedPlan}).
 */
export interface LimitedPlan {
  /** The sample rate, in hertz. */
  readonly rate: number;
  /** The sine's frequency folded into [0, rate / 2], in hertz: the one its samples hold. */
  readonly frequency: number;
  /** Silence, exactly, over these first frames: the limiter's latency. */
  readonly latency: number;
  /** The first frame measured: the limiter has settled from it on. */
  readonly start: number;
  /** The range of the amplitude once settled. */
  readonly within: Range;
  /** The largest need of the onset's estimates (frames 0 to `taps - 2`). */
  readonly onset: number;
  /** The largest need of the steady sine, `need(A·G)`. */
  readonly steady: number;
  /** What each format's samples may lie off the exact limited sine. */
  readonly allowances: Readonly<Record<RenderFormat, Allowance>>;
}

/**
 * The error of the need the engine computes, absolute, for a sine of `amplitude` given in
 * `format` over `frames` frames, through `interpolator` with a lookahead of `lookahead`
 * frames at `rate`, term by term (the control path in binary64, `u = 2^-53`):
 * - the estimate, which moves the need by at most its own error over `c` (the need is
 *   `1/c`-Lipschitz): the input sine off its ideal by at most `ε = sineErrorBound` in its
 *   format, moving an estimate by `H·ε` (`H`, {@link spread}); the engine's sum of `T`
 *   products, `(T + 1)·u·H·A`; the oracle's own response of each phase, as much; each
 *   coefficient within `(T + 14)·u` of exact, the engine's as the oracle's ({@link kaiser}),
 *   `2·T·(T + 14)·u·A` at most: in all `H·ε + (2(T + 1)·H + 2T(T + 14))·u·A`, over `c`;
 * - the need's difference and quotient, `2u`;
 * - the slow stage's one-pole, two roundings a frame contracted by its keep, `2u` over its
 *   step, `2u·(τ + 1)` with `τ` the release in frames;
 * - the Hann smoothing's running sums over `L` frames, refreshed exactly once per window:
 *   the plain sum and the turning sum each grow by at most a few roundings of a sum of `L`
 *   needs (at most 1) per frame for at most `L` frames, the phasor's turns within a few
 *   roundings, and the final difference and scale: `16·(L + 1)·u` covers them.
 */
export function controlError(interpolator: Interpolator, amplitude: number, frames: number, format: RenderFormat, lookahead: number, rate: number): number {
  const taps = interpolator.taps;
  const spreading = spread(interpolator);
  const magnitude = Math.abs(amplitude);
  const estimated = spreading * sineErrorBound(magnitude, frames, format) + (2 * (taps + 1) * spreading + 2 * taps * (taps + 14)) * UNIT * magnitude;
  return estimated / ceiling() + 2 * UNIT + 2 * UNIT * ((RELEASE_MS * rate) / 1000 + 1) + 16 * (lookahead + 1) * UNIT;
}

/**
 * The {@link Allowance} of a limited sine of `amplitude` in `within` given in `format`, whose
 * need errs by at most `error` ({@link controlError}), term by term, `u_S` the unit roundoff
 * of the precision that limits it (`2^-24` for a binary32 render, `2^-53` otherwise):
 * - the input sine, off its ideal by `ε = sineErrorBound` in its format, passed through a
 *   gain of at most 1: `ε` both ways;
 * - the need's error and the engine's margin on every need (`margin`, {@link needMargin}),
 *   scaled by `A`: the reduction lies within `error` of the exact one, plus the margin, so
 *   `A·(error + margin)` below, `A·error` above;
 * - the ceiling: the oracle's within `2u`, the engine's within `3u` of `10^(-1/20)`, and
 *   rounded toward zero in the sample precision, `2·u_S`: `(5u + 2·u_S)` of the lowest below,
 *   `5u` of the highest above;
 * - the gain `1 - r` given in the sample precision, rounded toward zero (`2·u_S` below) or to
 *   the nearest (`u_S` above), and its product with the sample, `u_S` either way: `3·u_S`
 *   and `2·u_S` of the highest;
 * - a binary64 render given as binary32, rounded once more, `2^-24` of the highest either way;
 * - a sample flushed to zero below the smallest normal number, `2^-1022` given in binary64,
 *   `2^-126` in binary32, either way.
 */
export function allowance(within: Range, amplitude: number, error: number, margin: number, frames: number, format: RenderFormat): Allowance {
  const sample = format === "binary32" ? 2 ** -24 : UNIT;
  const delivered = format === "binary64-as-binary32" ? 2 ** -24 : 0;
  const flushed = format === "binary64" ? 2 ** -1022 : 2 ** -126;
  const magnitude = Math.abs(amplitude);
  const input = sineErrorBound(magnitude, frames, format);
  return {
    below: input + magnitude * (error + margin) + within.lowest * (5 * UNIT + 2 * sample) + within.highest * (3 * sample + delivered) + flushed,
    above: input + magnitude * error + within.highest * (5 * UNIT + 2 * sample + delivered) + flushed,
  };
}

/** The error per sample of a limited sine against the sine of the middle gain: half the range, and the larger side of its allowance. */
export function limitedError(within: Range, allowed: Allowance): number {
  return (within.highest - within.lowest) / 2 + Math.max(allowed.below, allowed.above);
}

/**
 * The plan of a channel of a case of `frames` frames at `rate` through the protected
 * `output`, expected to hold `sine` limited at the default ceiling, or why the oracle
 * cannot judge it (a fault of the corpus, not of a render), the engine adding `margin` to
 * every need ({@link needMargin}). With `L` the lookahead in frames, `W = L + 2` the window
 * the limiter holds its needs over, `T` the taps, `c` the ceiling, `ε` the largest error of
 * the need ({@link controlError} over the formats) plus the margin:
 * 1. its frequency is folded into [0, rate / 2], strictly between DC and Nyquist, and a
 *    period of it fits `W`;
 * 2. from frame `T - 1` on, each estimate reads the steady sine, so its need is at most
 *    `steady = need(A·G)`; the onset's needs (frames 0 to `T - 2`, whose taps reach before
 *    the sine) are computed exactly, their largest `onset`;
 * 3. from `T1 = T - 2 + W` on, the window holds no onset need: the held need lies between
 *    `need(A·R)` and `steady`;
 * 4. the fast stage keeps the largest of the held need and its own decay, `k_f` per frame:
 *    it is at most `steady + ε` from `T1 - 1 + j` on, `j` the fewest frames with
 *    `(onset + ε)·k_f^j ≤ steady + ε` (0 when the onset needs no more than the steady sine),
 *    `k_f` raised by `4u` for its own rounding;
 * 5. the slow stage, a one-pole of the release toward the held need, reaches at most
 *    `(onset + ε)·(1 - k_s^T1)` by `T1` (`k_s` lowered by `4u`), and never rises above the
 *    held need after it: when that exceeds `steady + ε`, or the steady sine needs nothing
 *    while its onset does, its slow return is not derived and the plan is refused;
 * 6. the Hann smoothing averages the last `L` released reductions: the gain is within its
 *    range from `T1 - 1 + j + L - 1` on, and the output holds the sine from the latency on:
 *    the measurement starts at the later of both;
 * 7. the frames left must be measurable, and the range must leave the fundamental a margin
 *    above its error in every format (`hasMargin`).
 */
export function limitedPlan(output: CaseOutput, rate: number, frames: number, sine: Sine, margin: number): LimitedPlan | string {
  const interpolator = INTERPOLATORS[output.detector];
  const frequency = fold(sine.frequency, rate);
  const lookahead = framesOf(output.lookahead, rate);
  const window = lookahead + 2;
  if (!(frequency > 0 && frequency < rate / 2)) {
    return `the sine, folded to ${String(frequency)} Hz, is not strictly between DC and Nyquist`;
  }
  if (rate / frequency > window) {
    return `a period of the sine (${String(rate / frequency)} frames) does not fit the limiter's window (${String(window)} frames)`;
  }
  const read = underRead((2 * Math.PI * frequency) / rate, interpolator);
  const within = range(read, sine.amplitude);
  const signal = (frame: number): number => (frame < 0 ? 0 : ideal(sine, rate, frame));
  const taps = interpolator.taps;
  const onset = Math.max(...Array.from({ length: taps - 1 }, (_frame, frame) => need(estimate(interpolator, signal, frame))));
  const steady = need(Math.abs(sine.amplitude) * read.high);
  const errorOf = (format: RenderFormat): number => controlError(interpolator, sine.amplitude, frames, format, lookahead, rate);
  const settled = taps - 2 + window;
  const decayed = released(onset, steady, Math.max(...RENDER_FORMATS.map(errorOf)) + margin, rate, settled);
  if (decayed === null) {
    return `its onset needs a reduction of ${String(onset)}, the steady sine ${String(steady)}: the slow stage's return is not derived`;
  }
  const delay = lookahead + delayOf(interpolator);
  const start = Math.max(delay, settled - 1 + decayed + lookahead - 1);
  const measured = Math.min(frames - start, MAX_FRAMES);
  const conditioning = sineFitConditioning(frequency, rate, Math.max(measured, 0));
  if (conditioning instanceof MeasureError) {
    return `it cannot be measured from frame ${String(start)}, where the limiter has settled: ${conditioning.message}`;
  }
  const allowed = (format: RenderFormat): Allowance => allowance(within, sine.amplitude, errorOf(format), margin, frames, format);
  const allowances = { binary64: allowed("binary64"), "binary64-as-binary32": allowed("binary64-as-binary32"), binary32: allowed("binary32") };
  if (!RENDER_FORMATS.every((format) => hasMargin((within.lowest + within.highest) / 2, limitedError(within, allowances[format]), conditioning))) {
    return "its range leaves the fundamental no margin to be measured";
  }
  return { rate, frequency, latency: delay, start, within, onset, steady, allowances };
}

/** The verdict on one limited channel: its problems, or the fundamental of its sine fit and its THD+N. */
export type Limited = { readonly problems: readonly string[] } | { readonly fundamental: number; readonly thdPlusN: number };

/**
 * Holds the measurements of a limited channel to `within` and its allowance `allowed`: the
 * fundamental of its sine fit, between the range's ends moved by the allowance as the sine fit carries it
 * (`√(2(1 + ρ))/(1 - ρ)` times an error per sample, `ρ` the fit's conditioning, "Bounds for
 * a sine") and by the fit's floor, and its THD+N at most that of a sine within
 * {@link limitedError} of the sine of the middle gain.
 */
export function holdLimited(levels: Measurements, within: Range, allowed: Allowance): Limited {
  const problems: string[] = [];
  const fundamental = levels.fundamental;
  const rho = levels.sine.conditioning;
  const carried = Math.sqrt(2 * (1 + rho)) / (1 - rho);
  const lowest = within.lowest - carried * allowed.below - levels.sine.floor;
  const highest = within.highest + carried * allowed.above + levels.sine.floor;
  if (!(fundamental <= highest && fundamental >= lowest)) {
    problems.push(`fundamental ${String(fundamental)}, not between ${String(lowest)} and ${String(highest)}`);
  }
  const bounds = sineBounds((within.highest + within.lowest) / 2, limitedError(within, allowed), levels);
  if (bounds === null) {
    problems.push("its range leaves the fundamental no margin to be measured");
  } else if (!(levels.thdPlusN <= bounds.thdPlusN)) {
    problems.push(`THD+N ${String(levels.thdPlusN)} dB, above ${String(bounds.thdPlusN)} dB`);
  }
  return problems.length > 0 ? { problems } : { fundamental, thdPlusN: levels.thdPlusN };
}

/**
 * Why `samples` are not silence, exactly (+0), over their first `frames` frames, the
 * latency of a protected output stage, whose delay starts empty; null when they are.
 */
export function unsilenced(samples: readonly number[], frames: number): string | null {
  return samples.slice(0, frames).every((sample) => Object.is(sample, 0)) ? null : `not silent over its latency, ${String(frames)} frames`;
}

/**
 * Judges one channel of a render of `format` by its `plan` ({@link limitedPlan}): its first
 * `plan.latency` frames are silence, exactly ({@link unsilenced}); from `plan.start` on, its
 * measurements hold to the plan's range and allowance ({@link holdLimited}).
 */
export function judgeLimited(samples: readonly number[], plan: LimitedPlan, format: RenderFormat): Limited {
  const noise = unsilenced(samples, plan.latency);
  if (noise !== null) {
    return { problems: [noise] };
  }
  let levels;
  try {
    levels = measure(Float64Array.from(samples.slice(plan.start, plan.start + MAX_FRAMES)), plan.rate, plan.frequency);
  } catch (error) {
    // A MeasureError, or any other: a channel that cannot be measured fails its case.
    return { problems: [`cannot be measured: ${String(error)}`] };
  }
  return holdLimited(levels, plan.within, plan.allowances[format]);
}
