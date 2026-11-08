/** The playground page: an on/off button and the oscilloscope. */

import { useCallback, useSyncExternalStore, type JSX } from "react";
import type { Player, PlayerState } from "./player.ts";
import { Oscilloscope } from "./Oscilloscope.tsx";

/** Every state, as shown in the status. */
const LABELS: Readonly<Record<PlayerState["kind"], string>> = {
  stopped: "Stopped",
  starting: "Starting…",
  playing: "Playing",
  suspended: "Paused by the browser",
  stopping: "Stopping…",
  failed: "Failed",
};

/** What the button does in each state, and says; undefined while stopping. A start can be cancelled. */
function action(player: Player, state: PlayerState): { readonly label: string; readonly run?: () => Promise<void> } {
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
  }
}

/** The id of the alert explaining a failure, which the button refers to. */
const ALERT = "player-alert";

/** The page, driving `player`. */
export function App({ player }: { readonly player: Player }): JSX.Element {
  const subscribe = useCallback((listener: () => void) => player.subscribe(listener), [player]);
  const state = useSyncExternalStore(subscribe, () => player.state());
  const signal = useCallback(() => player.latest(), [player]);
  const { label, run } = action(player, state);
  const failed = state.kind === "failed";

  return (
    <main>
      <h1>Playground</h1>
      <p>A 440 Hz sine computed by the Rust engine, compiled to WebAssembly and running in your browser.</p>
      <div className="controls">
        {/* aria-disabled, not disabled: the button keeps the keyboard focus during a transition. */}
        <button
          type="button"
          aria-disabled={run === undefined}
          aria-describedby={failed ? ALERT : undefined}
          data-action={label.toLowerCase()}
          onClick={() => {
            void run?.();
          }}
        >
          {label}
        </button>
        <span role="status">{LABELS[state.kind]}</span>
        {failed ? (
          <p role="alert" id={ALERT}>
            {state.reason}
          </p>
        ) : null}
      </div>
      <Oscilloscope signal={signal} />
    </main>
  );
}
