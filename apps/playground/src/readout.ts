/** Measurements shown as text: readable by people, end-to-end tests and Claude in Chrome. */

import { frequency, peak } from "./scope.ts";
import type { Signal } from "./player.ts";

/** Shown when there is nothing to measure. */
export const NONE = "—";

/**
 * The frequency of the signal, to a tenth of a hertz (its accuracy, 0.05 Hz, within the
 * domain of `frequency`); "—" outside that domain.
 */
export function formatFrequency(signal: Signal | undefined): string {
  const value = signal === undefined ? undefined : frequency(signal.samples, signal.rate);
  return value === undefined ? NONE : `${value.toFixed(1)} Hz`;
}

/** The peak level of the signal, full scale being 1. */
export function formatPeak(signal: Signal | undefined): string {
  return signal === undefined || signal.samples.length === 0 ? NONE : peak(signal.samples).toFixed(2);
}

/** The sample rate of the signal. */
export function formatRate(signal: Signal | undefined): string {
  return signal === undefined ? NONE : `${String(signal.rate)} Hz`;
}
