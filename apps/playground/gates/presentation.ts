// The law of the square's presentation: the canvas's name and its description agree in every
// state, whatever the module, the reduced motion and the player are.
import { NAMES, type Presentation } from "../src/Square.tsx";
import { type Words, misfits } from "./remedies.ts";

/** What the line of each state the canvas's name gives must say, and must never say: the state in words the name and the line share. */
const STATE_WORDS: Readonly<Record<keyof typeof NAMES, Words>> = {
  loading: { says: ["loading"], never: ["switching"] },
  switching: { says: ["switching"], never: ["loading"] },
  failed: { says: ["cannot be shown"], never: ["stopped"] },
  stopped: { says: ["stopped on an error"], never: ["cannot be shown"] },
  still: { says: ["still"], never: ["loading"] },
  sweeping: { says: ["sweep"], never: ["still"] },
};

/** Every state, as {@link NAMES} keys it. */
const STATES = ["loading", "switching", "failed", "stopped", "still", "sweeping"] as const satisfies readonly (keyof typeof NAMES)[];

/**
 * The presentations among `told` whose line does not say the state their name gives, or says
 * another, so that the canvas's name and its description never contradict each other; a name
 * that gives no state is refused.
 */
export function contradictions(told: readonly Presentation[]): readonly string[] {
  return told.flatMap(({ name, line }) => {
    const rows = STATES.filter((state) => NAMES[state] === name);
    return rows.length === 0 ? [`${name}: the name of no state`] : rows.flatMap((state) => misfits(`${state}, "${line}"`, STATE_WORDS[state], line));
  });
}
