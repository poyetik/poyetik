// Test bench page: drives the built package (dist/) and shows results as text, for the
// end-to-end tests and for Claude in Chrome. The case comes from `?case=<name>`.
import { interleave, renderHash } from "../../dist/hash.js";
import { addProcessors, capture, createEngineNode, loadPatch, renderOffline, type LoadedPatch } from "../../dist/web.js";

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
  readonly rate: number;
  readonly frames: number;
  readonly patch: Uint8Array;
}

/** Fetches a conformance case and its patch file, kept byte for byte. */
async function fetchCase(name: string): Promise<Case> {
  const json: unknown = await (await fetch(`/conformance/cases/${name}.json`)).json();
  const patch = new Uint8Array(await (await fetch(`/conformance/patches/${name}.json`)).arrayBuffer());
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

const wasm = await (await fetch(new URL("../../dist/engine.wasm", import.meta.url))).arrayBuffer();
const testCase = await fetchCase(new URLSearchParams(location.search).get("case") ?? "sine-440");
const loaded: LoadedPatch = await loadPatch(wasm, testCase.patch, testCase.rate);
const error = element("error", HTMLOutputElement);

/** Runs `action` when `button` is clicked, showing any error. */
function on(button: string, action: () => Promise<void>): void {
  const target = element(button, HTMLButtonElement);
  target.addEventListener("click", () => {
    action().catch((reason: unknown) => {
      error.value = String(reason);
    });
  });
  target.disabled = false;
}

on("offline", async () => {
  const channels = await renderOffline(loaded, testCase.frames, PROCESSORS);
  element("offline-hash", HTMLOutputElement).value = await renderHash(loaded.rate, loaded.channels, interleave(channels));
});

on("live", async () => {
  const context = new AudioContext({ sampleRate: loaded.rate });
  await context.suspend();
  await addProcessors(context, PROCESSORS);
  const node = createEngineNode(context, loaded);
  node.addEventListener("processorerror", () => {
    error.value = "processor error";
  });
  const captured = capture(context, node, loaded.channels, testCase.frames);
  // Silent on the speakers: the graph runs for real, the capture reads the engine output
  // before this gain.
  const mute = new GainNode(context, { gain: 0 });
  node.connect(mute).connect(context.destination);
  await context.resume();
  const channels = await captured;
  await context.close();
  element("live-hash", HTMLOutputElement).value = await renderHash(loaded.rate, loaded.channels, interleave(channels));
});

on("invalid", async () => {
  await loadPatch(wasm, new TextEncoder().encode('{"format":1,"nodes":[{"id":1,"box":"nope~"}],"connections":[]}'), loaded.rate);
});
