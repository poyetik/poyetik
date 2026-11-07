/** Entry point: mounts the playground. `?muted` silences the speakers (end-to-end tests). */

import wasm from "@poyetik/engine-wasm/engine.wasm?url";
import worklet from "@poyetik/engine-wasm/worklet?worker&url";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App.tsx";
import { SINE_440 } from "./patch.ts";
import { EnginePlayer } from "./player.ts";
import { GAIN, HISTORY } from "./settings.ts";

const root = document.getElementById("root");
if (root === null) {
  throw new Error("no #root element");
}
const muted = new URLSearchParams(location.search).has("muted");
const player = new EnginePlayer({ wasm, worklet, patch: SINE_440, gain: muted ? 0 : GAIN, history: HISTORY, onError: reportError });
createRoot(root).render(
  <StrictMode>
    <App player={player} />
  </StrictMode>,
);
