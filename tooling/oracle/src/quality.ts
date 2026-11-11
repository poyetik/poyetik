/**
 * The measured quality of a channel that must hold a sine: its levels (`measure.ts`), held
 * to the bounds a sine within its derived error per sample meets (`sineErrorBound`, then
 * `sineBounds`): the thresholds of `spec/measurements.md`, "Thresholds of `osc~`", derived
 * from the expectation alone, never declared.
 */

import { MAX_FRAMES, MeasureError, hasMargin, measure, sineBounds, sineFitConditioning, sineFitRefusal, type Bounds, type Measurements } from "./measure.ts";
import { RENDER_FORMATS, type RenderFormat } from "./format.ts";
import { ideal, sineErrorBound, type Sine } from "./signal.ts";

/** The levels of a channel, in dB relative to its fundamental. */
export interface Levels {
  /** THD+N. */
  readonly thdPlusN: number;
  /** SFDR. */
  readonly sfdr: number;
  /** The harmonic fit's THD, aliasing level and SNR, or why it was refused. */
  readonly harmonics: { readonly thd: number | null; readonly aliasing: number | null; readonly snr: number } | string;
}

/** Why a channel measures nothing: its expectation has no fundamental to measure. */
export const UNMEASURED = "no fundamental strictly between DC and Nyquist with a margin above its error bound";

/** What the measurement of a channel found: its levels within their bounds, the levels out of them, or that it measures nothing ({@link UNMEASURED}). */
export type Quality = { readonly kind: "measured"; readonly levels: Levels } | { readonly kind: "failed"; readonly problems: readonly string[] } | { readonly kind: "unmeasured" };

/** `frequency` folded into [0, rate / 2]: the frequency its samples hold. */
export function fold(frequency: number, rate: number): number {
  const reduced = frequency % rate;
  return Math.min(reduced, rate - reduced);
}

/** `value` dB with one decimal. */
function db(value: number): string {
  return `${value.toFixed(1)} dB`;
}

/**
 * Measures `samples`, a render of `format` which must hold `sine` at `rate`, over at most
 * {@link MAX_FRAMES} frames from the start, and holds every level to the bound of its
 * format.
 *
 * Measures nothing when the sine has no fundamental strictly between DC and Nyquist, when
 * its ideal values over those frames all stay within its error bound of zero (silence
 * included, and a sine at 2^-126 Hz over a few thousand frames), or when its amplitude
 * leaves its fundamental no margin above that bound, even measured from its exact samples
 * (a binary32 sine just above 2^-126): no level can then tell it from silence, or be held
 * to a bound, which its distance from the ideal sine (`sineError`) already checks.
 */
export function judgeQuality(samples: readonly number[], rate: number, sine: Sine, format: RenderFormat): Quality {
  const fundamental = fold(sine.frequency, rate);
  const frames = Math.min(samples.length, MAX_FRAMES);
  const bound = sineErrorBound(sine.amplitude, frames, format);
  if (!isMeasured(sine, rate, frames, bound)) {
    return { kind: "unmeasured" };
  }
  let measured;
  let levels;
  try {
    measured = Float64Array.from(samples.slice(0, MAX_FRAMES));
    levels = measure(measured, rate, fundamental);
  } catch (error) {
    if (error instanceof MeasureError) {
      return { kind: "failed", problems: [`cannot be measured: ${error.message}`] };
    }
    throw error;
  }
  return hold(levels, sineBounds(sine.amplitude, bound, levels));
}

/**
 * Whether {@link judgeQuality} measures `sine` over its first `frames` frames at `rate`,
 * under `bound` (its error bound in the format concerned): its fundamental lies strictly
 * between DC and Nyquist, its ideal values rise above that bound somewhere, and its
 * amplitude leaves the fundamental a margin above it, for the conditioning its sine fit has
 * over those frames, from the floor of an exact render (`hasMargin`, the condition the
 * bounds of a level need), so that no expectation is measured that every render would fail.
 * A fit refused whatever the samples is measured, and fails: the expectation is refused
 * when it is read ({@link unmeasurable}).
 */
function isMeasured(sine: Sine, rate: number, frames: number, bound: number): boolean {
  const fundamental = fold(sine.frequency, rate);
  if (!(fundamental > 0 && fundamental < rate / 2)) {
    return false;
  }
  let peak = 0;
  for (let frame = 0; frame < frames; frame += 1) {
    peak = Math.max(peak, Math.abs(ideal(sine, rate, frame)));
  }
  if (!(peak > bound)) {
    return false;
  }
  const conditioning = sineFitConditioning(fundamental, rate, frames);
  return conditioning instanceof MeasureError || hasMargin(sine.amplitude, bound, conditioning);
}

/**
 * Why a channel expected to hold `sine` over `frames` frames at `rate` would fail as a
 * render no correct engine can pass: {@link judgeQuality} measures it in some render format,
 * but its sine fit over the frames measured is refused whatever the samples (too few
 * frames, or a fundamental too close to DC or Nyquist to be told apart from it). Null when
 * it can be measured, or is not measured at all.
 */
export function unmeasurable(sine: Sine, rate: number, frames: number): string | null {
  const counted = Math.min(frames, MAX_FRAMES);
  if (!RENDER_FORMATS.some((format) => isMeasured(sine, rate, counted, sineErrorBound(sine.amplitude, counted, format)))) {
    return null;
  }
  return sineFitRefusal(fold(sine.frequency, rate), rate, counted)?.message ?? null;
}

/**
 * Holds the levels `measured` to `bounds`: every level within its bound, or the problems;
 * no bounds (no margin for the fundamental) is a problem, and so are no harmonic bounds for
 * a harmonic fit that was not refused.
 */
export function hold(measured: Measurements, bounds: Bounds | null): Quality {
  if (bounds === null) {
    return { kind: "failed", problems: ["its error bound leaves the fundamental no margin to be measured"] };
  }
  const problems: string[] = [];
  const check = (what: string, level: number, bound: number, rising: boolean): void => {
    if (rising ? !(level >= bound) : !(level <= bound)) {
      problems.push(`${what} ${db(level)}, ${rising ? "below" : "above"} its bound ${db(bound)}`);
    }
  };
  check("THD+N", measured.thdPlusN, bounds.thdPlusN, false);
  check("SFDR", measured.sfdr, bounds.sfdr, true);
  const harmonics = measured.harmonics instanceof MeasureError ? measured.harmonics.message : measured.harmonics;
  if (typeof harmonics !== "string") {
    if (bounds.harmonics === null) {
      problems.push("its error bound leaves the harmonic fit no margin to be measured");
    } else {
      check("SNR", harmonics.snr, bounds.harmonics.snr, true);
      if (harmonics.thd !== null) {
        check("THD", harmonics.thd, bounds.harmonics.thd, false);
      }
      if (harmonics.aliasing !== null) {
        check("aliasing", harmonics.aliasing, bounds.harmonics.thd, false);
      }
    }
  }
  if (problems.length > 0) {
    return { kind: "failed", problems };
  }
  return {
    kind: "measured",
    levels: {
      thdPlusN: measured.thdPlusN,
      sfdr: measured.sfdr,
      harmonics: typeof harmonics === "string" ? harmonics : { thd: harmonics.thd, aliasing: harmonics.aliasing, snr: harmonics.snr },
    },
  };
}

/** One line of the levels of a channel: `THD+N … dB, SFDR … dB, THD … dB, aliasing … dB, SNR … dB`, or the harmonic fit's refusal. */
export function describe(levels: Levels): string {
  const harmonics =
    typeof levels.harmonics === "string"
      ? `harmonics not measured: ${levels.harmonics}`
      : `THD ${levels.harmonics.thd === null ? "none" : db(levels.harmonics.thd)}, aliasing ${levels.harmonics.aliasing === null ? "none" : db(levels.harmonics.aliasing)}, SNR ${db(levels.harmonics.snr)}`;
  return `THD+N ${db(levels.thdPlusN)}, SFDR ${db(levels.sfdr)}, ${harmonics}`;
}

/** The larger of two levels where null means none. */
function larger(left: number | null, right: number | null): number | null {
  return left === null ? right : right === null ? left : Math.max(left, right);
}

/** The worst of the levels of several channels: the highest THD+N, THD and aliasing, the lowest SFDR and SNR; a harmonic fit refused anywhere is shown. Null for no channel. */
export function worst(channels: readonly Levels[]): Levels | null {
  return channels.reduce<Levels | null>(
    (worse, levels) =>
      worse === null
        ? levels
        : {
            thdPlusN: Math.max(worse.thdPlusN, levels.thdPlusN),
            sfdr: Math.min(worse.sfdr, levels.sfdr),
            harmonics:
              typeof worse.harmonics === "string"
                ? worse.harmonics
                : typeof levels.harmonics === "string"
                  ? levels.harmonics
                  : {
                      thd: larger(worse.harmonics.thd, levels.harmonics.thd),
                      aliasing: larger(worse.harmonics.aliasing, levels.harmonics.aliasing),
                      snr: Math.min(worse.harmonics.snr, levels.harmonics.snr),
                    },
          },
    null,
  );
}
