// The law of the square's presentation: every text the page tells of the square (the canvas's
// name, its description, what draws it) gives one state, in every state, whatever the module,
// the reduced motion and the player are.
import { NAMES, type Presentation } from "../src/Square.tsx";
import { misfits } from "./remedies.ts";

/** A text told beside the name: each field of {@link Presentation} but the name, so that a text added to it does not compile without its words in every state. */
type Output = Exclude<keyof Presentation, "name">;

/** Every text told beside the name: a record, so that a text added to {@link Presentation} does not compile without it. */
const OUTPUTS: readonly Output[] = Object.values({ line: "line", drawnBy: "drawnBy" } as const satisfies Readonly<Record<Output, Output>>);

/**
 * What each text of each state the canvas's name gives must say (the state in words the name
 * and the text share), or that it is absent. It must never say the words of every other state
 * for the same text: a stopped square says "stopped" alone too, so that no other state says it.
 */
const STATE_WORDS: Readonly<Record<keyof typeof NAMES, Readonly<Record<Output, readonly string[] | "absent">>>> = {
  loading: { line: ["loading"], drawnBy: "absent" },
  switching: { line: ["switching"], drawnBy: ["switching"] },
  failed: { line: ["cannot be shown"], drawnBy: "absent" },
  stopped: { line: ["stopped on an error", "stopped"], drawnBy: "absent" },
  still: { line: ["still"], drawnBy: ["drawn by"] },
  sweeping: { line: ["sweep"], drawnBy: ["drawn by"] },
};

/** The words of another state a state's texts may say: a still square says the sound would sweep it (`UNPLAYABLE`, `SWEEPING` before `WAITING`). */
const MAY_SAY: Readonly<Partial<Record<keyof typeof NAMES, readonly string[]>>> = { still: ["sweep"] };

/** Every state, as {@link NAMES} keys it. */
const STATES = ["loading", "switching", "failed", "stopped", "still", "sweeping"] as const satisfies readonly (keyof typeof NAMES)[];

/** The words `state`, which says `own` in `output`, must never say there: those every other state says there, but those it may say ({@link MAY_SAY}). */
function never(state: keyof typeof NAMES, output: Output, own: readonly string[]): readonly string[] {
  const others = STATES.map((other) => STATE_WORDS[other][output]).filter((words) => words !== "absent").flat();
  return [...new Set(others)].filter((word) => !own.includes(word) && MAY_SAY[state]?.includes(word) !== true);
}

/** What `text`, the `output` of `state`, lacks or says that it must not; a text given where none may be is refused, one missing lacks every word. */
function faults(state: keyof typeof NAMES, output: Output, text: string | undefined): readonly string[] {
  const says = STATE_WORDS[state][output];
  if (says === "absent") {
    return text === undefined ? [] : [`${state}, "${text}": a text where the state has none`];
  }
  const said = text ?? "";
  return misfits(`${state}, "${said}"`, { says, never: never(state, output, says) }, said);
}

/**
 * The presentations among `told` one of whose texts does not say the state their name gives,
 * or says another, so that the canvas's name, its description and what draws it never
 * contradict one another; a name that gives no state is refused.
 */
export function contradictions(told: readonly Presentation[]): readonly string[] {
  return told.flatMap(({ name, ...texts }) => {
    const rows = STATES.filter((state) => NAMES[state] === name);
    return rows.length === 0 ? [`${name}: the name of no state`] : rows.flatMap((state) => OUTPUTS.flatMap((output) => faults(state, output, texts[output])));
  });
}
