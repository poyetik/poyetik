/** The playground page: an on/off button and the oscilloscope. */

import { MAX_REPLACED_COUNT, type Replaced } from "@poyetik/audio-wasm/web";
import { useCallback, useSyncExternalStore, type JSX } from "react";
import { PLAYED_OUTPUT } from "./patch.ts";
import { explain, flushes, type Player, type PlayerState } from "./player.ts";
import { Oscilloscope } from "./Oscilloscope.tsx";

/** Every state, as shown in the status. */
const LABELS: Readonly<Record<PlayerState["kind"], string>> = {
  stopped: "Stopped",
  starting: "Starting…",
  playing: "Playing",
  suspended: "Paused by the browser",
  stopping: "Stopping…",
  failed: "Failed",
  unsupported: "Not supported",
};

/** Every label of the button: the button is as wide as the widest, so that nothing moves when it changes. */
export const BUTTON_LABELS = ["Start", "Cancel", "Stop", "Resume"] as const;

/** What the button does in each state, and says; undefined while stopping. A start can be cancelled. */
function action(player: Player, state: PlayerState): { readonly label: (typeof BUTTON_LABELS)[number]; readonly run?: () => Promise<void> } {
  switch (state.kind) {
    case "stopped":
    case "failed":
      return { label: "Start", run: () => player.start() };
    case "starting":
      return { label: "Cancel", run: () => player.stop() };
    case "playing":
      return { label: "Stop", run: () => player.stop() };
    case "suspended":
      return { label: "Resume", run: () => player.resume() };
    case "stopping":
      return { label: "Stop" };
    case "unsupported":
      return { label: "Start" };
  }
}

/** The id of the alert explaining a failure or a refused resume, which the button refers to. */
const ALERT = "player-alert";

/** Shown when the browser kept the audio suspended after Resume: the click was seen, and what to try. */
export const REFUSED = "The browser kept audio paused. Try again, or check that this tab may play sound.";

/**
 * Shown once the engine has seen that this browser's audio thread flushes subnormal numbers
 * to zero: what plays here is not proven bit-identical to the engine elsewhere.
 */
export const FLUSHING =
  "This browser rounds the smallest numbers to zero while it plays, so this tone is not guaranteed to be bit-identical to the engine elsewhere. The difference is far below anything audible.";

/**
 * Announced, once per start, when the engine's output stage first replaces invalid samples,
 * with its cause (the live region holds it alone and it never changes, so that a growing
 * count is not announced again; the count is shown beside it, and continues it rather than
 * repeating it: {@link replacedNotice}).
 */
export const REPLACED = "The engine played invalid samples as silence: a defect of a box of this patch.";

/**
 * `count`, a whole number from 0, its digits grouped by three with commas, as English
 * writes them: computed from its digits alone, so that no locale, the browser's or the
 * machine's, can change it.
 */
export function grouped(count: number): string {
  return String(count).replace(/\B(?=(\d{3})+$)/gu, ",");
}

/**
 * Whether the session of each state still renders, and so may still report more replaced
 * samples: a record, so that a new state does not compile until it chooses. Once it ended
 * (stopped, failed), the reports of its last second may never have come, the processor
 * posting none when it stops.
 */
const LIVE: Readonly<Record<PlayerState["kind"], boolean>> = {
  stopped: false,
  starting: true,
  playing: true,
  suspended: true,
  stopping: true,
  failed: false,
  unsupported: false,
};

/**
 * Shown beside {@link REPLACED} once the engine's output stage has replaced invalid samples
 * (NaN or infinite) by silence (`spec/engine-design.md`, decision 12): how many so far while
 * the session is `live`, at least as many once it ended or when the count saturated, its
 * digits grouped in English whatever the browser's locale, and the channel of the first (of
 * the one sample, when it is one), counted from 1 as it says.
 */
export function replacedNotice(replaced: Required<Replaced>, live: boolean): string {
  // One sample is no first of several: the noun and where it was agree with the count together.
  const [noun, where] = replaced.count === 1 ? ["sample", "on"] : ["samples", "the first on"];
  const samples = `${grouped(replaced.count)} invalid ${noun} (not a number, or infinite), ${where} channel ${String(replaced.first.channel + 1)} (channels counted from 1)`;
  // A saturated count is a lower bound, and the engine reports no more after it; so is the
  // count of an ended session, whose last reports may never have come.
  if (replaced.count === MAX_REPLACED_COUNT) {
    return `At least ${samples}; the engine counts no further.`;
  }
  return live ? `So far ${samples}.` : `At least ${samples}.`;
}

/**
 * What the page explains next to the button, or null: why the player cannot start or
 * failed, and what to do; or why a resume did nothing.
 */
function notice(state: PlayerState): string | null {
  switch (state.kind) {
    case "unsupported":
      return explain(state.lacking);
    case "failed":
      return explain(state.cause);
    case "suspended":
      return state.refused ? REFUSED : null;
    case "stopped":
    case "starting":
    case "playing":
    case "stopping":
      return null;
  }
}

/** The page, driving `player`. */
export function App({ player }: { readonly player: Player }): JSX.Element {
  const subscribe = useCallback((listener: () => void) => player.subscribe(listener), [player]);
  const state = useSyncExternalStore(subscribe, () => player.state());
  const signal = useCallback(() => player.latest(), [player]);
  const { label, run } = action(player, state);
  const explanation = notice(state);
  const arithmetic = player.arithmetic();
  const replaced = useSyncExternalStore(subscribe, () => player.replaced());

  return (
    <main>
      <h1>Playground</h1>
      <p>{`A 440 Hz sine computed by the Rust engine, compiled to WebAssembly and running in your browser, played through the engine's true-peak limiter, at a ceiling of ${String(PLAYED_OUTPUT.limiter.ceiling)} dB true peak (dBTP).`}</p>
      <p>Plays a continuous tone: lower your volume first.</p>
      <div className="controls">
        {/* aria-disabled, not disabled: the button keeps the keyboard focus during a transition. */}
        <button
          type="button"
          aria-disabled={run === undefined}
          aria-describedby={explanation === null ? undefined : ALERT}
          data-action={label.toLowerCase()}
          onClick={(event) => {
            // Only the first click of a series (a keyboard activation has detail 0): the
            // second click of a double-click would undo the first, a tone starting again
            // right after Stop, or a start cancelled at once.
            if (event.detail <= 1) {
              void run?.();
            }
          }}
          onKeyDown={(event) => {
            // A held Enter or Space repeats: it must not toggle the tone on and off.
            if (event.repeat && (event.key === "Enter" || event.key === " ")) {
              event.preventDefault();
            }
          }}
        >
          <span>{label}</span>
          {/* Invisible copies of every label, in the same grid cell: they size the button. */}
          {BUTTON_LABELS.map((each) => (
            <span key={each} className="sizer" aria-hidden="true">
              {each}
            </span>
          ))}
        </button>
        <span role="status">{LABELS[state.kind]}</span>
        {explanation === null ? null : (
          <p role="alert" id={ALERT}>
            {explanation}
          </p>
        )}
        {/* What exactly failed, for those who debug: shown on demand, never announced. */}
        {state.kind === "failed" ? (
          <details>
            <summary>Details</summary>
            <code>{state.detail}</code>
          </details>
        ) : null}
      </div>
      {arithmetic !== undefined && flushes(arithmetic) ? <p>{FLUSHING}</p> : null}
      {/* A live region announcing the first replacement once, without moving the focus; the
          count beside it grows while playing without being announced again. */}
      <div aria-live="polite" data-testid="replaced">
        {replaced === undefined ? null : <p>{REPLACED}</p>}
      </div>
      {replaced === undefined ? null : <p>{replacedNotice(replaced, LIVE[state.kind])}</p>}
      <Oscilloscope active={state.kind === "playing"} signal={signal} />
    </main>
  );
}
