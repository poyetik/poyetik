/** Entry point: mounts the playground. `?muted` silences the speakers (end-to-end tests). */

import wasm from "@poyetik/audio-wasm/engine.wasm?url";
import worklet from "@poyetik/audio-wasm/worklet?worker&url";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App.tsx";
import { SINE_440 } from "./patch.ts";
import { EnginePlayer } from "./player.ts";
import { FADE, GAIN, HISTORY_SECONDS } from "./settings.ts";

/** The element the playground mounts into; the page is broken without it, so its absence throws. */
const root = document.getElementById("root");
if (root === null) {
  throw new Error("no #root element");
}
/** Whether the page was opened with `?muted`: then the gain is 0 and nothing reaches the speakers. */
const muted = new URLSearchParams(location.search).has("muted");
/** The single player of the page, built from the settings; nothing loads before the first start. */
const player = new EnginePlayer({ wasm, worklet, patch: SINE_440, gain: muted ? 0 : GAIN, fade: FADE, historySeconds: HISTORY_SECONDS, environment: globalThis });
createRoot(root).render(
  <StrictMode>
    <App player={player} />
  </StrictMode>,
);
