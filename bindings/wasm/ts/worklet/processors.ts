/**
 * The `AudioWorklet` processors: the engine, and a tap that posts its input to the main
 * thread. Built as a module of its own, loaded with `audioWorklet.addModule`.
 *
 * The worklet scope has no `TextEncoder` or `TextDecoder` in every browser: everything
 * it receives and sends is already bytes or numbers.
 */

import { WasmEngine } from "../engine.ts";
import { ENGINE_PROCESSOR, TAP_PROCESSOR, parseEngineOptions } from "../protocol.ts";

/** Runs the engine: one output, one channel per patch channel. */
class EngineProcessor extends AudioWorkletProcessor {
  /** The engine, with the patch loaded. */
  readonly #engine: WasmEngine;

  /**
   * Loads the patch given in the node options (`processorOptions`, see `EngineOptions`).
   * @param options The node options.
   * @param options.processorOptions The patch and the engine module.
   */
  constructor(options: { readonly processorOptions?: unknown }) {
    super();
    const { wasm, patch } = parseEngineOptions(options.processorOptions);
    this.#engine = WasmEngine.create(new WebAssembly.Module(wasm), patch, sampleRate);
  }

  /** Renders the next render quantum. */
  process(_inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    this.#engine.render(outputs[0] ?? []);
    return true;
  }
}

/** Posts a copy of every block of its input to the main thread, until told to stop. */
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
