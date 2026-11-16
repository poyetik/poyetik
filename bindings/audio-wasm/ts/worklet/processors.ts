/**
 * The `AudioWorklet` processors: the engine, and a tap that posts its input to the main
 * thread. Built as a module of its own, loaded with `audioWorklet.addModule`.
 *
 * The worklet scope has no `TextEncoder` or `TextDecoder` in every browser: everything
 * it receives and sends is bytes, numbers or strings built without them.
 */

import { OutputError, PatchError, WasmEngine, ascii, givenAsBinary32 } from "../engine.ts";
import {
  ENGINE_PROCESSOR,
  MARKER_PERIOD,
  REPLACED_REPORT_SECONDS,
  SOURCE_STOPPED,
  TAP_PROCESSOR,
  parseEngineCommand,
  parseEngineOptions,
  type EngineCommand,
  type EngineMessage,
} from "../protocol.ts";

/** The outputs of a node without output: one shared, frozen, empty list. */
const NO_OUTPUT: readonly Float32Array[] = Object.freeze([]);

/**
 * The UTF-8 text `bytes`, decoded without a `TextDecoder` (see the module's doc): through
 * `decodeURIComponent`, a global of every realm; bytes that are not UTF-8 (only a module
 * breaking the interface gives them) are said to be so, in hexadecimal.
 */
function utf8(bytes: Uint8Array): string {
  const hex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0"));
  try {
    return decodeURIComponent(hex.map((digits) => `%${digits}`).join(""));
  } catch {
    return `an explanation that is not UTF-8 (${hex.join(" ")})`;
  }
}

/**
 * The message describing a failure: the rejection detail of the engine, or a message.
 * The message keeps the cause of the error, when it is an error: for an `EngineTrap`, the trap of the
 * WebAssembly engine ("unreachable", "memory access out of bounds"), the only clue a
 * module built with `panic = "abort"` gives; for an `OutputError` (an output stage the
 * engine refuses when it is created), the engine's explanation. The message protocol
 * carries one string.
 */
function failure(error: unknown): EngineMessage {
  if (error instanceof PatchError) {
    return { type: "rejected", detail: error.detail };
  }
  if (error instanceof OutputError) {
    return { type: "failed", message: `${String(error)}: ${utf8(error.detail)}` };
  }
  const cause: unknown = Reflect.get(Object(error), "cause");
  return { type: "failed", message: cause instanceof Error ? `${String(error)} (${String(cause)})` : String(error) };
}

/**
 * Runs the engine: one output, one channel per patch channel. Tells the main thread once
 * whether the patch is loaded, or why it is not; what the output stage replaced, when it
 * replaced more, at most once per {@link REPLACED_REPORT_SECONDS}; after a failure (at load
 * or later) it outputs silence and stops.
 */
class EngineProcessor extends AudioWorkletProcessor {
  /**
   * The engine, and whether to fill the marker channel (the one after the patch
   * channels); `undefined` once it failed.
   */
  #running: { readonly engine: WasmEngine<Float32Array>; readonly marker: boolean } | undefined;
  /** Frames rendered so far, modulo {@link MARKER_PERIOD}: what the marker channel counts. */
  #rendered = 0;
  /** Replaced samples already told to the main thread. */
  #told = 0;
  /** Frames rendered since the last `replaced` message, up to {@link EngineProcessor.#interval}. */
  #sinceTold = 0;
  /** The frames of {@link REPLACED_REPORT_SECONDS} at the context's rate. */
  readonly #interval = REPLACED_REPORT_SECONDS * sampleRate;

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
      const { wasm, patch, marker, precision, output } = parseEngineOptions(options.processorOptions);
      const engine = WasmEngine.create(new WebAssembly.Module(wasm), patch, sampleRate, givenAsBinary32(precision), output);
      // Observed when it was created, here on the audio thread, in the browser's mode.
      const ready: EngineMessage = { type: "ready", subnormals: engine.subnormals };
      this.#running = { engine, marker };
      this.port.onmessage = (event: MessageEvent<unknown>) => {
        this.#command(event.data);
      };
      this.port.postMessage(ready);
    } catch (error) {
      this.port.postMessage(failure(error));
    }
  }

  /**
   * Applies a command of the main thread to the running engine: a refused setting, and a
   * malformed command (the words of its `CommandError`), are told back (`refused`), the
   * engine rendering on with the settings it had; a module error or a trap stops it, as a
   * failure while rendering does. Ignored once it failed.
   */
  #command(data: unknown): void {
    const running = this.#running;
    if (running === undefined) {
      return;
    }
    let command: EngineCommand;
    try {
      command = parseEngineCommand(data);
    } catch (error) {
      // A CommandError, the one error its reader throws: its message, which is ASCII.
      const refused: EngineMessage = { type: "refused", detail: ascii(String(Reflect.get(Object(error), "message"))) };
      this.port.postMessage(refused);
      return;
    }
    try {
      if (command.type === "limiter") {
        running.engine.setLimiter(command.settings);
      } else {
        running.engine.setProtected(command.protect);
      }
    } catch (error) {
      if (error instanceof OutputError) {
        const refused: EngineMessage = { type: "refused", detail: error.detail };
        this.port.postMessage(refused);
        return;
      }
      this.#running = undefined;
      this.port.postMessage(failure(error));
    }
  }

  /**
   * Renders the next render quantum. Does not allocate when the quantum is a whole number of
   * engine blocks (the module's engine block divides it), but to report a failure once, or
   * replaced samples (never while none is replaced); with a module whose engine block is
   * larger than the quantum, every quantum is a part of a block, copied through a view that
   * is allocated (see `WasmEngine`).
   */
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
      this.#report(this.#running.engine, output[0]?.length ?? 0);
      return true;
    } catch (error) {
      this.#running = undefined;
      this.port.postMessage(failure(error));
      return false;
    }
  }

  /**
   * Tells the main thread what the output stage replaced, when it replaced more than told:
   * the first time at once, then once {@link EngineProcessor.#interval} frames passed since
   * the last message. Reading the count allocates nothing. Nothing is posted when the
   * processor stops: replacements of its last interval after the last message go untold.
   * Once the count saturates (`MAX_REPLACED_COUNT`), it never grows, and nothing is posted
   * again.
   */
  #report(engine: WasmEngine<Float32Array>, frames: number): void {
    this.#sinceTold = Math.min(this.#sinceTold + frames, this.#interval);
    const count = engine.replacedCount;
    if (count > this.#told && (this.#told === 0 || this.#sinceTold === this.#interval)) {
      this.#told = count;
      this.#sinceTold = 0;
      const message: EngineMessage = { type: "replaced", replaced: { count, first: engine.replacedFirst() } };
      this.port.postMessage(message);
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
