/**
 * The `AudioWorklet` processors: the engine, and a tap that posts its input to the main
 * thread. Built as a module of its own, loaded with `audioWorklet.addModule`.
 *
 * The worklet scope has no `TextEncoder` or `TextDecoder` in every browser: everything
 * it receives and sends is bytes, numbers or strings built without them.
 */

import { PatchError, WasmEngine, givenAsBinary32 } from "../engine.ts";
import { ENGINE_PROCESSOR, MARKER_PERIOD, SOURCE_STOPPED, TAP_PROCESSOR, parseEngineOptions, type EngineMessage } from "../protocol.ts";

/** The outputs of a node without output: one shared, frozen, empty list. */
const NO_OUTPUT: readonly Float32Array[] = Object.freeze([]);

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
  #running: { readonly engine: WasmEngine<Float32Array>; readonly marker: boolean } | undefined;
  /** Frames rendered so far, modulo {@link MARKER_PERIOD}: what the marker channel counts. */
  #rendered = 0;

  /**
   * Loads the patch given in the node options (`processorOptions`, see `EngineOptions`), to
   * render in their precision, given to Web Audio as binary32, and tells the main thread
   * what the audio thread's arithmetic does with subnormals.
   * @param options The node options.
   * @param options.processorOptions The patch, the engine module and the precision.
   */
  constructor(options: { readonly processorOptions?: unknown }) {
    super();
    try {
      const { wasm, patch, marker, precision } = parseEngineOptions(options.processorOptions);
      const engine = WasmEngine.create(new WebAssembly.Module(wasm), patch, sampleRate, givenAsBinary32(precision));
      // Observed when it was created, here on the audio thread, in the browser's mode.
      const ready: EngineMessage = { type: "ready", subnormals: engine.subnormals };
      this.#running = { engine, marker };
      this.port.postMessage(ready);
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
 * stop. It allocates a copy per render quantum, on the audio thread: a known deviation,
 * run by capture and by the playground's oscilloscope until roadmap #12 step 21 replaces
 * it, with a ring in shared memory for capture and an `AnalyserNode` for the oscilloscope.
 */
class TapProcessor extends AudioWorkletProcessor {
  /** Whether to keep posting. */
  #running = true;
  /** Whether the last render quantum had input: its source was running. */
  #fed = false;

  /** Stops at the first message received. */
  constructor() {
    super();
    this.port.onmessage = () => {
      this.#running = false;
    };
  }

  /**
   * Posts the input render quantum, if any; when the input goes from present to empty
   * (its source stopped), posts {@link SOURCE_STOPPED} once, so that a capture fails
   * instead of waiting for quanta that never come.
   */
  process(inputs: Float32Array[][]): boolean {
    const input = inputs[0] ?? [];
    if (this.#running) {
      if (input.length > 0) {
        this.#fed = true;
        this.port.postMessage(input.map((channel) => channel.slice()));
      } else if (this.#fed) {
        this.#fed = false;
        this.port.postMessage(SOURCE_STOPPED);
      }
    }
    return this.#running;
  }
}

registerProcessor(ENGINE_PROCESSOR, EngineProcessor);
registerProcessor(TAP_PROCESSOR, TapProcessor);
