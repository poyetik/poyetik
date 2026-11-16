/**
 * A model of the engine's true-peak limiter at its default settings, written from its
 * definition (decision 12), apart from the oracle's derivation: the renders the oracle's
 * tests judge, in each render format. It follows the engine's steps where they set the
 * precision of a result: the estimate in the computing precision (its coefficients rounded
 * to binary32 in a binary32 render, its sums in the reduction order of decision 28), the
 * ceiling rounded toward zero in the format given, the margin on every need above it, the
 * gain rounded toward zero in the computing precision, and a binary64 render given as
 * binary32 rounded at the end. Its release stages and its Hann mean, which run in binary64
 * in the engine, are computed directly (the engine's running sums apart), and its input is
 * the ideal sine rounded to the computing precision, not the oscillator's: it is no
 * substitute for the engine's renders, which the corpus's cases judge, only a source of
 * renders the oracle's allowance must hold.
 */

import type { RenderFormat } from "../src/format.ts";

/** The phases of a detector's interpolator, tap `k` multiplying the sample `k` frames before the newest. */
export type Phases = readonly (readonly number[])[];

/**
 * How the estimate of a binary32 render rounds each product and sum: to the nearest, as
 * the engine does, or `outward`, each toward the sign of the phase's sum (its magnitude read
 * as high as rounding can), a worst case the oracle's allowance must hold too.
 */
export type Rounding = "nearest" | "outward";

/** The bits of a binary32 value, to step it by one unit in the last place. */
const BITS = new Uint32Array(1);

/** `BITS` read as a binary32. */
const VALUE = new Float32Array(BITS.buffer);

/** The binary32 next to `value` (itself a binary32, finite and nonzero) away from zero (`away`) or toward it. */
function stepped(value: number, away: boolean): number {
  VALUE[0] = value;
  BITS[0] = (BITS[0] ?? 0) + (away ? 1 : -1);
  return VALUE[0];
}

/** `value` rounded to binary32 toward zero. */
export function towardZero32(value: number): number {
  const nearest = Math.fround(value);
  return Math.abs(nearest) > Math.abs(value) ? stepped(nearest, false) : nearest;
}

/** `value` rounded to binary32 toward +∞ when `up`, toward −∞ otherwise. */
function directed32(value: number, up: boolean): number {
  const nearest = Math.fround(value);
  if (up ? nearest >= value : nearest <= value) {
    return nearest;
  }
  if (nearest === 0) {
    // Below the smallest subnormal: the smallest of `up`'s side.
    return up ? 2 ** -149 : -(2 ** -149);
  }
  // The nearest lies on the wrong side: one step toward `up`'s side, away from zero when it
  // has that side's sign, toward zero otherwise. From a sum in binary64, rounded once more:
  // a model's approximation of the directed rounding of the exact sum.
  return stepped(nearest, (nearest > 0) === up);
}

/**
 * The sum of `terms` in the reduction order of decision 28, each addition rounded by
 * `round`: term `k` into lane `k mod 16`, each lane in increasing `k`, then the lanes 8, 4,
 * 2 and 1 apart added pairwise, over the lanes holding a term.
 */
function ordered(terms: readonly number[], round: (sum: number) => number): number {
  const lanes: number[] = [];
  terms.forEach((term, index) => {
    const lane = index % 16;
    const held = lanes[lane];
    lanes[lane] = held === undefined ? term : round(held + term);
  });
  for (const half of [8, 4, 2, 1]) {
    for (let lane = 0; lane < half; lane++) {
      const other = lanes[lane + half];
      if (other !== undefined) {
        lanes[lane] = round((lanes[lane] ?? 0) + other);
      }
    }
  }
  return lanes[0] ?? 0;
}

/**
 * `frames` frames of `input` (zero before frame 0) at `rate` through the limiter with
 * `phases`, a lookahead of `lookahead` milliseconds (its maximum too), a ceiling of -1 dBTP,
 * a release of 100 ms and an adaptivity of 0.5, rendered in `format`, the estimate of a
 * binary32 render rounded as `rounding` says: each frame's estimate (the interval's two
 * ends and each phase), its need plus the margin `64·M·2^-53` when above the ceiling, the
 * largest need over the lookahead and 2 frames, the fast stage (55 ms) and the slow one
 * (100 ms), their larger smoothed by a Hann window as long as the lookahead, its gain
 * applied to the input delayed by the lookahead and half the taps.
 */
export function limit(input: (frame: number) => number, frames: number, rate: number, lookahead: number, phases: Phases, format: RenderFormat = "binary64", rounding: Rounding = "nearest"): number[] {
  const narrow = format === "binary32";
  const received = (frame: number): number => (frame < 0 ? 0 : narrow ? Math.fround(input(frame)) : input(frame));
  const coefficients = narrow ? phases.map((phase) => phase.map(Math.fround)) : phases;
  const taps = phases[0]?.length ?? 0;
  const length = Math.max(1, Math.round((lookahead * rate) / 1000));
  const delay = length + taps / 2;
  const exact = 10 ** (-1 / 20);
  const ceiling = format === "binary64" ? exact : towardZero32(exact);
  const margin = length * 2 ** -47;
  const fast = Math.exp(-1000 / (55 * rate));
  const slow = Math.exp(-1000 / (100 * rate));
  const needs: number[] = [];
  const released: number[] = [];
  let fastStage = 0;
  let slowStage = 0;
  /** The estimate of one phase at `frame`. */
  const phaseAt = (phase: readonly number[], frame: number): number => {
    const products = phase.map((coefficient, tap) => coefficient * received(frame - tap));
    if (!narrow) {
      return ordered(products, (sum) => sum);
    }
    if (rounding === "nearest") {
      return ordered(products.map(Math.fround), Math.fround);
    }
    // Every step rounded toward the side of the sum's own sign: its magnitude read high.
    const up = ordered(products, (sum) => sum) >= 0;
    return ordered(
      products.map((product) => directed32(product, up)),
      (sum) => directed32(sum, up),
    );
  };
  return Array.from({ length: frames }, (_value, frame) => {
    const ends = [received(frame - taps / 2), received(frame - taps / 2 + 1)];
    const peak = Math.max(...[...ends, ...coefficients.map((phase) => phaseAt(phase, frame))].map(Math.abs));
    const share = (peak - ceiling) / Math.max(peak, ceiling);
    needs.push(share > 0 ? share + margin : 0);
    const held = Math.max(...needs.slice(-(length + 2)));
    fastStage = Math.max(held, fastStage * fast);
    slowStage += (1 - slow) * (held - slowStage);
    released.push(Math.max(fastStage, slowStage));
    let smoothed = 0;
    for (let back = 0; back < length; back++) {
      smoothed += ((released[frame - back] ?? 0) * (1 - Math.cos((2 * Math.PI * (back + 1)) / (length + 1)))) / (length + 1);
    }
    const sample = received(frame - delay);
    if (smoothed === 0) {
      return format === "binary64-as-binary32" ? Math.fround(sample) : sample;
    }
    const gain = narrow ? towardZero32(1 - smoothed) : 1 - smoothed;
    const out = sample * gain;
    return format === "binary64" ? out : Math.fround(out);
  });
}
