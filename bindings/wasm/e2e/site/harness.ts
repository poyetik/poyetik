// Test bench page: drives the built package (dist/) and shows results as text, for the
// end-to-end tests and for Claude in Chrome. This list is the one source of what it takes
// and offers (the package's README points here):
// - `?case=<name>`: the conformance case it renders, `sine-440` when absent;
// - `?precision=<binary64|binary32>`: the precision the engine computes in, binary64 when
//   absent; the render is given as binary32, as Web Audio takes it;
// - `?runs=<n>`: how many times "Time construction" constructs the engine processor, 1 when
//   absent;
// - the buttons "Render offline" (shown in "Offline hash"), "Capture live" ("Live hash"),
//   "Load an invalid patch" (its refusal shown in "Error") and "Time construction"
//   ("Construction times");
// - "Error": why the page could not load, or why the last action failed;
// - `window.timeRendering(patch, rate, precision, blocks, runs)`: the times of rendering a
//   patch (JSON text) on the page's own thread, which the per-box end-to-end benchmarks
//   call (see `timeRendering` below).
// While a button's action runs, it is disabled and its outputs and the error are cleared
// (page.ts).
import { WasmEngine } from "../../dist/engine.js";
import { interleave, renderHash, STRICT } from "../../dist/hash.js";
import { addProcessors, captureEngine, createEngineNode, givenAsBinary32, loadPatch, parsePrecision, renderOffline, type LoadedPatch, type Precision } from "../../dist/web.js";
import { fetchOk } from "./fetch.ts";
import { Action, showFailure } from "./page.ts";
import { PAGE_CLOCK, renderBlocks, runsOf, timeEach, timeRuns, timed } from "./timing.ts";

/** The built `AudioWorklet` module that registers the engine and tap processors. */
const PROCESSORS = new URL("../../dist/worklet/processors.js", import.meta.url);

/** The element with this id, of this type. */
function element<T extends HTMLElement>(id: string, type: new () => T): T {
  const found = document.getElementById(id);
  if (!(found instanceof type)) {
    throw new Error(`no ${type.name} #${id}`);
  }
  return found;
}

/** A conformance case: sample rate, duration and the bytes of its patch file. */
interface Case {
  /** Sample rate of the render, in hertz: the rate the patch is checked at and the context runs at. */
  readonly rate: number;
  /** Duration of the render, in frames. */
  readonly frames: number;
  /** The bytes of the patch file, unchanged. */
  readonly patch: Uint8Array;
}

/** Fetches a conformance case and its patch file, kept byte for byte. */
async function fetchCase(name: string): Promise<Case> {
  const json: unknown = await (await fetchOk(`/conformance/cases/${name}.json`)).json();
  const patch = new Uint8Array(await (await fetchOk(`/conformance/patches/${name}.json`)).arrayBuffer());
  if (typeof json !== "object" || json === null) {
    throw new Error("invalid case");
  }
  const rate: unknown = Reflect.get(json, "sample_rate");
  const frames: unknown = Reflect.get(json, "frames");
  if (typeof rate !== "number" || typeof frames !== "number") {
    throw new Error("invalid case");
  }
  return { rate, frames, patch };
}

/** What the page loads before its buttons work. */
interface Loaded {
  /** The bytes of the built engine module, shared by every button. */
  readonly wasm: ArrayBuffer;
  /** The case named by `?case=`. */
  readonly testCase: Case;
  /** The precision named by `?precision=`. */
  readonly precision: Precision;
  /** The case's patch, already validated by the engine at the case's sample rate. */
  readonly loaded: LoadedPatch;
}

/**
 * Loads what the page needs: the module, the case, its precision and its patch. A missing
 * file, an unknown precision or a rejected patch fails the page load.
 */
async function load(): Promise<Loaded> {
  const module = await (await fetchOk(new URL("../../dist/engine.wasm", import.meta.url))).arrayBuffer();
  const asked = await fetchCase(new URLSearchParams(location.search).get("case") ?? "sine-440");
  const chosen = parsePrecision(new URLSearchParams(location.search).get("precision") ?? "binary64");
  if (chosen === undefined) {
    throw new Error(`unknown precision: ${String(new URLSearchParams(location.search).get("precision"))}`);
  }
  return { wasm: module, testCase: asked, precision: chosen, loaded: await loadPatch(module, asked.patch, asked.rate, chosen) };
}

/** Where why the page could not load, or why the last action failed, is shown. */
const error = element("error", HTMLOutputElement);
/** What the page loaded; a failed load is shown in {@link error}, then fails the page load. */
const { wasm, testCase, precision, loaded } = await showFailure(load, error);
/** The render format of what the page captures: the precision, given as binary32. */
const format = givenAsBinary32(precision);

/** Runs `action` when `button` is clicked, as an {@link Action} showing its results in the `outputs` named. */
function on(button: string, outputs: readonly string[], action: () => Promise<void>): void {
  const target = element(button, HTMLButtonElement);
  const run = new Action(
    target,
    outputs.map((id) => element(id, HTMLOutputElement)),
    error,
  );
  target.addEventListener("click", () => {
    void run.run(action);
  });
  target.disabled = false;
}

on("offline", ["offline-hash"], async () => {
  const channels = await renderOffline(loaded, testCase.frames, PROCESSORS);
  element("offline-hash", HTMLOutputElement).value = await renderHash(loaded.rate, loaded.channels, interleave(channels), format, STRICT);
});

on("live", ["live-hash"], async () => {
  const context = new AudioContext({ sampleRate: loaded.rate });
  let channels: Float32Array[];
  // Closed however the capture ends: a failed one leaves no context running.
  try {
    await context.suspend();
    await addProcessors(context, PROCESSORS);
    const engine = createEngineNode(
      context,
      loaded,
      (failure) => {
        error.value = String(failure);
      },
      true,
    );
    const { node, ready } = engine;
    const captured = captureEngine(context, engine, testCase.frames);
    // Silent on the speakers: the graph runs for real, the capture reads the engine output
    // before this gain.
    const mute = new GainNode(context, { gain: 0 });
    node.connect(mute).connect(context.destination);
    await context.resume();
    await ready;
    channels = await captured;
  } finally {
    await context.close();
  }
  element("live-hash", HTMLOutputElement).value = await renderHash(loaded.rate, loaded.channels, interleave(channels), format, STRICT);
});

on("invalid", [], async () => {
  await loadPatch(wasm, new TextEncoder().encode('{"format":1,"nodes":[{"id":1,"box":"nope~"}],"connections":[]}'), loaded.rate, precision);
});

/**
 * Times the construction of the engine processor `?runs=` times (once by default), one
 * engine node after the other in one running context: from creating the engine node (copying
 * the module and the patch into its options included) to the processor's `ready` (the
 * module compiled, instantiated, the patch parsed and its plan built, on the audio
 * thread). Shows each time in milliseconds, separated by spaces.
 */
on("construction", ["construction-ms"], async () => {
  const runs = runsOf(location.search);
  const context = new AudioContext({ sampleRate: loaded.rate });
  try {
    await addProcessors(context, PROCESSORS);
    await context.resume();
    const times = await timeEach(
      runs,
      PAGE_CLOCK,
      async () => {
        const { node, ready } = createEngineNode(context, loaded, (failure) => {
          error.value = String(failure);
        });
        await ready;
        return node;
      },
      (node) => {
        node.disconnect();
      },
    );
    element("construction-ms", HTMLOutputElement).value = times.map((time) => time.toFixed(3)).join(" ");
  } finally {
    await context.close();
  }
});

/**
 * Times rendering `blocks` blocks of `patch` (JSON text) at `rate` hertz, computed in the
 * precision `asked` and given as binary32 as to Web Audio, in render quanta of 128 frames
 * (blocks of the engine's block size), on the page's own thread: a fresh
 * engine each time, one warm-up render first (compiling the code), then `runs` times in
 * milliseconds. What the end-to-end per-box benchmarks call through `page.evaluate`: the
 * same module, compiled by the browser engine, as in its `AudioWorklet`.
 */
async function timeRendering(patch: string, rate: number, asked: string, blocks: number, runs: number): Promise<number[]> {
  const chosen = parsePrecision(asked);
  if (chosen === undefined) {
    throw new Error(`unknown precision: ${asked}`);
  }
  const module = await WebAssembly.compile(wasm);
  const bytes = new TextEncoder().encode(patch);
  const once = (): number => {
    const engine = WasmEngine.create(module, bytes, rate, givenAsBinary32(chosen));
    return timed(PAGE_CLOCK, () => {
      renderBlocks(engine, blocks);
    });
  };
  return timeRuns(runs, once);
}

Reflect.set(window, "timeRendering", timeRendering);
