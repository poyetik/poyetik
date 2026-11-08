/**
 * The `AudioWorklet` processors: the engine, and a tap that posts its input to the main
 * thread. Built as a module of its own, loaded with `audioWorklet.addModule`.
 *
 * The worklet scope has no `TextEncoder` or `TextDecoder` in every browser: everything
 * it receives and sends is bytes, numbers or strings built without them.
 */

import { PatchError, WasmEngine } from "../engine.ts";
import { ENGINE_PROCESSOR, TAP_PROCESSOR, parseEngineOptions, type EngineMessage } from "../protocol.ts";

/** The outputs of a node without output: one shared, frozen, empty list. */
const NO_OUTPUT: readonly Float32Array[] = Object.freeze([]);

/** The message telling the main thread the patch is loaded: one shared, frozen object. */
const READY: EngineMessage = Object.freeze({ type: "ready" });

/** The message describing a failure: the rejection detail of the engine, or a message. */
function failure(error: unknown): EngineMessage {
  return error instanceof PatchError ? { type: "rejected", detail: error.detail } : { type: "failed", message: String(error) };
}

/**
 * Runs the engine: one output, one channel per patch channel. Tells the main thread once
 * whether the patch is loaded, or why it is not; after a failure (at load or later) it
 * outputs silence and stops.
 */
class EngineProcessor extends AudioWorkletProcessor {
  /** The engine; `undefined` once it failed. */
  #engine: WasmEngine | undefined;

  /**
   * Loads the patch given in the node options (`processorOptions`, see `EngineOptions`).
   * @param options The node options.
   * @param options.processorOptions The patch and the engine module.
   */
  constructor(options: { readonly processorOptions?: unknown }) {
    super();
    try {
      const { wasm, patch } = parseEngineOptions(options.processorOptions);
      this.#engine = WasmEngine.create(new WebAssembly.Module(wasm), patch, sampleRate);
      this.port.postMessage(READY);
    } catch (error) {
      this.port.postMessage(failure(error));
    }
  }

  /** Renders the next render quantum. Does not allocate, except to report a failure once. */
  process(_inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    if (this.#engine === undefined) {
      return false;
    }
    try {
      this.#engine.render(outputs[0] ?? NO_OUTPUT);
      return true;
    } catch (error) {
      this.#engine = undefined;
      this.port.postMessage(failure(error));
      return false;
    }
  }
}

/**
 * Posts a copy of every block of its input to the main thread, until told to stop. Test
 * and measurement only: it allocates a copy per block (roadmap #12 replaces it with a
 * shared ring buffer).
 */
class TapProcessor extends AudioWorkletProcessor {
  /** Whether to keep posting. */
  #running = true;

  /** Stops at the first message received. */
  constructor() {
    super();
    this.port.onmessage = () => {
      this.#running = false;
    };
  }

  /** Posts the input block, if any. */
  process(inputs: Float32Array[][]): boolean {
    const input = inputs[0] ?? [];
    if (this.#running && input.length > 0) {
      this.port.postMessage(input.map((channel) => channel.slice()));
    }
    return this.#running;
  }
}

registerProcessor(ENGINE_PROCESSOR, EngineProcessor);
registerProcessor(TAP_PROCESSOR, TapProcessor);
