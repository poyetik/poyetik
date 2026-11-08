/**
 * The `AudioWorklet` processors: the engine, and a tap that posts its input to the main
 * thread. Built as a module of its own, loaded with `audioWorklet.addModule`.
 *
 * The worklet scope has no `TextEncoder` or `TextDecoder` in every browser: everything
 * it receives and sends is bytes, numbers or strings built without them.
 */

import { PatchError, WasmEngine } from "../engine.ts";
import { ENGINE_PROCESSOR, MARKER_PERIOD, TAP_PROCESSOR, parseEngineOptions, type EngineMessage } from "../protocol.ts";

/** The outputs of a node without output: one shared, frozen, empty list. */
const NO_OUTPUT: readonly Float32Array[] = Object.freeze([]);

/** The message telling the main thread the patch is loaded: one shared, frozen object. */
const READY: EngineMessage = Object.freeze({ type: "ready" });

/**
 * The message describing a failure: the rejection detail of the engine, or a message.
 * The message keeps the cause of the error, when it is an error: for an `EngineTrap`, the trap of the
 * WebAssembly engine ("unreachable", "memory access out of bounds"), the only clue a
 * module built with `panic = "abort"` gives. The message protocol carries one string.
 */
function failure(error: unknown): EngineMessage {
  if (error instanceof PatchError) {
    return { type: "rejected", detail: error.detail };
  }
  const cause: unknown = Reflect.get(Object(error), "cause");
  return { type: "failed", message: cause instanceof Error ? `${String(error)} (${String(cause)})` : String(error) };
}

/**
 * Runs the engine: one output, one channel per patch channel. Tells the main thread once
 * whether the patch is loaded, or why it is not; after a failure (at load or later) it
 * outputs silence and stops.
 */
class EngineProcessor extends AudioWorkletProcessor {
  /**
   * The engine, and whether to fill the marker channel (the one after the patch
   * channels); `undefined` once it failed.
   */
  #running: { readonly engine: WasmEngine; readonly marker: boolean } | undefined;
  /** Frames rendered so far, modulo {@link MARKER_PERIOD}: what the marker channel counts. */
  #rendered = 0;

  /**
   * Loads the patch given in the node options (`processorOptions`, see `EngineOptions`).
   * @param options The node options.
   * @param options.processorOptions The patch and the engine module.
   */
  constructor(options: { readonly processorOptions?: unknown }) {
    super();
    try {
      const { wasm, patch, marker } = parseEngineOptions(options.processorOptions);
      this.#running = { engine: WasmEngine.create(new WebAssembly.Module(wasm), patch, sampleRate), marker };
      this.port.postMessage(READY);
    } catch (error) {
      this.port.postMessage(failure(error));
    }
  }

  /** Renders the next render quantum. Does not allocate, except to report a failure once. */
  process(_inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    if (this.#running === undefined) {
      return false;
    }
    try {
      const output = outputs[0] ?? NO_OUTPUT;
      this.#running.engine.render(output);
      if (this.#running.marker) {
        this.#mark(output, this.#running.engine.channels);
      }
      return true;
    } catch (error) {
      this.#running = undefined;
      this.port.postMessage(failure(error));
      return false;
    }
  }

  /**
   * Fills the marker channel, the one after the patch channels, with the count of every
   * frame of `output` (see `EngineOptions.marker`), when the host gave room for it: the
   * channel after the `channels` of the patch. Small integers only: nothing is allocated.
   */
  #mark(output: readonly Float32Array[], channels: number): void {
    const frames = output[0]?.length ?? 0;
    const marker = output[channels];
    if (marker !== undefined) {
      for (let frame = 0; frame !== frames; frame += 1) {
        marker[frame] = ((this.#rendered + frame) % MARKER_PERIOD) + 1;
      }
    }
    this.#rendered = (this.#rendered + frames) % MARKER_PERIOD;
  }
}

/**
 * Posts a copy of every render quantum of its input to the main thread, until told to
 * stop. Test and measurement only: it allocates a copy per render quantum (roadmap #12
 * replaces it with a shared ring buffer).
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

  /** Posts the input render quantum, if any. */
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
