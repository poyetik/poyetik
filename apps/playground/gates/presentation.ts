// The law of the square's presentation: the canvas's name and its description agree.
import type { Presentation } from "../src/Square.tsx";

/**
 * The failed presentations among `told` whose line does not say the state their name gives
 * (the name's words after "The square, ", a "which" dropped), so that the canvas's name and
 * its description never contradict each other.
 */
export function contradictions(told: readonly Presentation[]): readonly string[] {
  return told.flatMap(({ name, line }) => {
    const state = name.replace(/^The square, (?:which )?/u, "");
    return line.includes(state) ? [] : [`${name}: "${line}"`];
  });
}
