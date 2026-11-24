// The law of the square's presentation: every text the page tells of the square (the canvas's
// name, its description, what draws it) gives one state, in every state, whatever the module,
// the reduced motion and the player are.
import { NAMES, type Presentation } from "../src/Square.tsx";
import { type Words, misfits } from "./remedies.ts";

/** A text told beside the name: each field of {@link Presentation} but the name, so that a text added to it does not compile without its words in every state. */
type Output = Exclude<keyof Presentation, "name">;

/** What each text of each state the canvas's name gives must say, and must never say (the state in words the name and the text share), or that it is absent. */
const STATE_WORDS: Readonly<Record<keyof typeof NAMES, Readonly<Record<Output, Words | "absent">>>> = {
  loading: { line: { says: ["loading"], never: ["switching"] }, drawnBy: "absent" },
  switching: { line: { says: ["switching"], never: ["loading"] }, drawnBy: { says: ["switching"], never: ["drawn by"] } },
  failed: { line: { says: ["cannot be shown"], never: ["stopped"] }, drawnBy: "absent" },
  stopped: { line: { says: ["stopped on an error"], never: ["cannot be shown"] }, drawnBy: "absent" },
  still: { line: { says: ["still"], never: ["loading"] }, drawnBy: { says: ["drawn by"], never: ["switching"] } },
  sweeping: { line: { says: ["sweep"], never: ["still"] }, drawnBy: { says: ["drawn by"], never: ["switching"] } },
};

/** Every state, as {@link NAMES} keys it. */
const STATES = ["loading", "switching", "failed", "stopped", "still", "sweeping"] as const satisfies readonly (keyof typeof NAMES)[];

/** What `text`, of a state (`words`), lacks or says that it must not; a text given where none may be is refused, one missing lacks every word. */
function faults(state: keyof typeof NAMES, words: Words | "absent", text: string | undefined): readonly string[] {
  if (words === "absent") {
    return text === undefined ? [] : [`${state}, "${text}": a text where the state has none`];
  }
  const said = text ?? "";
  return misfits(`${state}, "${said}"`, words, said);
}

/**
 * The presentations among `told` one of whose texts does not say the state their name gives,
 * or says another, so that the canvas's name, its description and what draws it never
 * contradict one another; a name that gives no state is refused.
 */
export function contradictions(told: readonly Presentation[]): readonly string[] {
  return told.flatMap(({ name, ...texts }) => {
    const rows = STATES.filter((state) => NAMES[state] === name);
    const given = new Map(Object.entries(texts));
    return rows.length === 0 ? [`${name}: the name of no state`] : rows.flatMap((state) => Object.entries(STATE_WORDS[state]).flatMap(([output, words]) => faults(state, words, given.get(output))));
  });
}
