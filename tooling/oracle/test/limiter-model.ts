/**
 * A model of the engine's true-peak limiter at its default settings, in binary64, written
 * from its definition (decision 12), apart from the oracle's derivation: the renders the
 * oracle's tests judge, as the engine would give them.
 */

/** The phases of a detector's interpolator, tap `k` multiplying the sample `k` frames before the newest. */
export type Phases = readonly (readonly number[])[];

/**
 * `frames` frames of `input` (zero before frame 0) at `rate` through the limiter with
 * `phases`, a lookahead of `lookahead` milliseconds (its maximum too), a ceiling of -1 dBTP,
 * a release of 100 ms and an adaptivity of 0.5: each frame's estimate (the interval's two
 * ends and each phase), its need, the largest need over the lookahead and 2 frames, the fast
 * stage (55 ms) and the slow one (100 ms), their larger smoothed by a Hann window as long
 * as the lookahead, applied to the input delayed by the lookahead and half the taps.
 */
export function limit(input: (frame: number) => number, frames: number, rate: number, lookahead: number, phases: Phases): number[] {
  const signal = (frame: number): number => (frame < 0 ? 0 : input(frame));
  const taps = phases[0]?.length ?? 0;
  const length = Math.max(1, Math.round((lookahead * rate) / 1000));
  const delay = length + taps / 2;
  const ceiling = 10 ** (-1 / 20);
  const fast = Math.exp(-1000 / (55 * rate));
  const slow = Math.exp(-1000 / (100 * rate));
  const needs: number[] = [];
  const released: number[] = [];
  let fastStage = 0;
  let slowStage = 0;
  return Array.from({ length: frames }, (_value, frame) => {
    const ends = [signal(frame - taps / 2), signal(frame - taps / 2 + 1)];
    const interpolated = phases.map((phase) => phase.reduce((sum, coefficient, tap) => sum + coefficient * signal(frame - tap), 0));
    const peak = Math.max(...[...ends, ...interpolated].map(Math.abs));
    needs.push(Math.max(0, (peak - ceiling) / Math.max(peak, ceiling)));
    const held = Math.max(...needs.slice(-(length + 2)));
    fastStage = Math.max(held, fastStage * fast);
    slowStage += (1 - slow) * (held - slowStage);
    released.push(Math.max(fastStage, slowStage));
    let smoothed = 0;
    for (let back = 0; back < length; back++) {
      smoothed += ((released[frame - back] ?? 0) * (1 - Math.cos((2 * Math.PI * (back + 1)) / (length + 1)))) / (length + 1);
    }
    return signal(frame - delay) * (1 - smoothed);
  });
}
