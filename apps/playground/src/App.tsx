/** The playground page: an on/off button and the oscilloscope. */

import { useCallback, useState, type JSX } from "react";
import type { Player } from "./player.ts";
import { Oscilloscope } from "./Oscilloscope.tsx";

/** Where the player is. */
type State = "stopped" | "starting" | "playing" | "stopping";

/** The state, as shown. */
const LABELS: Readonly<Record<State, string>> = {
  stopped: "Stopped",
  starting: "Starting…",
  playing: "Playing",
  stopping: "Stopping…",
};

/** The page, driving `player`. */
export function App({ player }: { readonly player: Player }): JSX.Element {
  const [state, setState] = useState<State>("stopped");
  const [error, setError] = useState<string | undefined>(undefined);
  const signal = useCallback(() => player.latest(), [player]);

  /** Runs a transition of the player, then settles on `after`, or on "stopped" with the error. */
  const run = (during: State, action: () => Promise<void>, after: State): void => {
    setState(during);
    setError(undefined);
    action().then(
      () => {
        setState(after);
      },
      (reason: unknown) => {
        setError(String(reason));
        setState("stopped");
      },
    );
  };

  const toggle =
    state === "stopped"
      ? () => {
          run("starting", () => player.start(), "playing");
        }
      : state === "playing"
        ? () => {
            run("stopping", () => player.stop(), "stopped");
          }
        : undefined;

  return (
    <main>
      <h1>Playground</h1>
      <p>A 440 Hz sine computed by the Rust engine, compiled to WebAssembly and running in your browser.</p>
      <div className="controls">
        <button type="button" disabled={toggle === undefined} onClick={toggle}>
          {state === "playing" || state === "stopping" ? "Stop" : "Start"}
        </button>
        <span role="status">{LABELS[state]}</span>
      </div>
      <Oscilloscope signal={signal} />
      {error === undefined ? null : <p role="alert">{error}</p>}
    </main>
  );
}
