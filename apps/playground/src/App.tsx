/** The playground page: an on/off button and the oscilloscope. */

import { useCallback, useSyncExternalStore, type JSX } from "react";
import { explain, type Player, type PlayerState } from "./player.ts";
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

  return (
    <main>
      <h1>Playground</h1>
      <p>A 440 Hz sine computed by the Rust engine, compiled to WebAssembly and running in your browser.</p>
      <p>Plays a continuous tone: lower your volume first.</p>
      <div className="controls">
        {/* aria-disabled, not disabled: the button keeps the keyboard focus during a transition. */}
        <button
          type="button"
          aria-disabled={run === undefined}
          aria-describedby={explanation === null ? undefined : ALERT}
          data-action={label.toLowerCase()}
          onClick={() => {
            void run?.();
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
      <Oscilloscope active={state.kind === "playing"} signal={signal} />
    </main>
  );
}
