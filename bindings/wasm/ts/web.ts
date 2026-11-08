/**
 * The engine in a web page: loading a patch, playing it in an `AudioContext`, rendering
 * it offline and capturing what a node outputs.
 *
 * The processors run in an `AudioWorklet`: register them once per context with
 * {@link addProcessors}, giving the URL of the compiled `worklet/processors.js`.
 */

import { ModuleError, PatchError, WasmEngine } from "./engine.ts";
import { ENGINE_PROCESSOR, TAP_PROCESSOR, parseEngineMessage, parseTapBlock, type EngineOptions } from "./protocol.ts";

/**
 * A patch checked by the engine, ready to run in engine nodes. Only {@link loadPatch}
 * makes one: holding a `LoadedPatch` proves the engine accepted this patch at this rate.
 * It keeps its own copies of the bytes, so changing the arrays given to `loadPatch`
 * afterwards changes nothing.
 */
export class LoadedPatch {
  /** Sample rate it was checked at, in hertz: the rate of the contexts it runs in. */
  readonly rate: number;
  /** Number of output channels. */
  readonly channels: number;
  /** The engine WebAssembly module, as bytes. */
  readonly #wasm: ArrayBuffer;
  /** The patch, as UTF-8 text in the patch format. */
  readonly #patch: Uint8Array;

  /**
   * Records a checked patch.
   * @param wasm A copy of the module bytes.
   * @param patch A copy of the patch bytes.
   * @param rate The rate it was checked at.
   * @param channels Its number of output channels.
   */
  private constructor(wasm: ArrayBuffer, patch: Uint8Array, rate: number, channels: number) {
    this.#wasm = wasm;
    this.#patch = patch;
    this.rate = rate;
    this.channels = channels;
  }

  /**
   * Checks `patch` with the engine at `rate` hertz, on this thread: errors are thrown here
   * with the explanation of the engine, rather than in the audio thread. What it rejects
   * with, told apart by `name` (the error classes are not exported yet, roadmap #5):
   * @throws {RangeError} when `rate` is not a whole number of hertz that fits 32 bits.
   * @throws {Error} named `ModuleError` ("not an engine module: …") when `wasm` is not an
   * engine module of this version.
   * @throws {Error} with the explanation of the engine as message, and a `PatchError` as
   * `cause`, when it rejects the patch or the rate (an unsupported rate, a patch that is
   * not UTF-8, invalid or that cannot be planned).
   * @throws {Error} named `EngineTrap`, with the WebAssembly `RuntimeError` as `cause`, when
   * the module traps while checking the patch.
   */
  static async load(wasm: ArrayBuffer, patch: Uint8Array, rate: number): Promise<LoadedPatch> {
    const bytes = wasm.slice(0);
    const module = await WebAssembly.compile(bytes).catch((error: unknown) => {
      throw new ModuleError(`cannot be compiled: ${String(error)}`, { cause: error });
    });
    const copy = patch.slice();
    try {
      const { channels } = WasmEngine.create(module, copy, rate);
      return new LoadedPatch(bytes, copy, rate, channels);
    } catch (error) {
      throw error instanceof PatchError ? new Error(new TextDecoder().decode(error.detail), { cause: error }) : error;
    }
  }

  /**
   * The options of an engine node running this patch, with fresh copies of the bytes.
   * @param marker Whether the node adds a marker channel (see `EngineOptions`).
   */
  options(marker: boolean): EngineOptions {
    return { wasm: this.#wasm.slice(0), patch: this.#patch.slice(), marker };
  }
}

/** Registers the engine and tap processors in `context`, naming the URL on failure. */
export async function addProcessors(context: BaseAudioContext, url: string | URL): Promise<void> {
  await context.audioWorklet.addModule(url).catch((error: unknown) => {
    throw new Error(`cannot load the processors from ${String(url)}: ${String(error)}`, { cause: error });
  });
}

/**
 * Checks `patch` with the engine at `rate` hertz: see {@link LoadedPatch.load}, which
 * lists what it rejects with.
 * @throws {Error} as {@link LoadedPatch.load}.
 */
export function loadPatch(wasm: ArrayBuffer, patch: Uint8Array, rate: number): Promise<LoadedPatch> {
  return LoadedPatch.load(wasm, patch, rate);
}

/** An engine node, and when it is ready. */
export interface EngineNode {
  /** The node: no input, one output with one channel per patch channel. */
  readonly node: AudioWorkletNode;
  /** Resolves when the processor has loaded the patch; rejects if it could not. */
  readonly ready: Promise<void>;
}

/**
 * Creates a node running `loaded`: no input, one output with one channel per patch
 * channel (one silent channel when the patch has none), plus the marker channel when
 * `marker` is set. `onFailure` receives the first
 * failure of the processor, at load or later (the node is then silent); `ready` also
 * rejects with it when it comes before the patch is loaded.
 * @throws {Error} when the context does not run at the rate the patch was checked at.
 */
export function createEngineNode(context: BaseAudioContext, loaded: LoadedPatch, onFailure: (error: Error) => void, marker = false): EngineNode {
  if (context.sampleRate !== loaded.rate) {
    throw new Error(`the context runs at ${String(context.sampleRate)} Hz, the patch was loaded at ${String(loaded.rate)} Hz`);
  }
  const node = new AudioWorkletNode(context, ENGINE_PROCESSOR, {
    numberOfInputs: 0,
    numberOfOutputs: 1,
    outputChannelCount: [marker ? loaded.channels + 1 : Math.max(1, loaded.channels)],
    processorOptions: loaded.options(marker),
  });
  const ready = Promise.withResolvers<undefined>();
  // Rejections also reach onFailure: `ready` is only awaited by those who need it.
  ready.promise.catch(() => undefined);
  let failed = false;
  const fail = (error: Error): void => {
    if (!failed) {
      failed = true;
      ready.reject(error);
      onFailure(error);
    }
  };
  node.port.onmessage = (event: MessageEvent<unknown>) => {
    let message;
    try {
      message = parseEngineMessage(event.data);
    } catch (error) {
      fail(toError(error));
      return;
    }
    switch (message.type) {
      case "ready":
        ready.resolve(undefined);
        break;
      case "rejected":
        fail(new Error(new TextDecoder().decode(message.detail), { cause: new PatchError(message.detail) }));
        break;
      case "failed":
        fail(new Error(`the engine failed: ${message.message}`));
        break;
    }
  };
  node.onprocessorerror = () => {
    fail(new Error("the engine processor failed"));
  };
  return { node, ready: ready.promise };
}

/** A running tap, posting every render quantum of its source to the main thread. */
export interface Tap {
  /** Disconnects the tap and stops its processor. Calling it again does nothing. */
  readonly stop: () => void;
}

/** Turns anything thrown into an `Error` of this realm (the worklet is another realm). */
function toError(thrown: unknown): Error {
  return thrown instanceof Error ? thrown : new Error(String(thrown));
}

/**
 * Taps `source`: `onBlock` receives every render quantum it outputs (`channels`
 * channels, exactly as the audio thread computes them), until `stop` is called. `onError`
 * receives a malformed one, which is then dropped. (`onBlock` keeps its name until the
 * public API takes its final names: a render quantum is not an engine block.)
 */
export function tap(
  context: BaseAudioContext,
  source: AudioNode,
  channels: number,
  onBlock: (block: Float32Array[]) => void,
  onError: (error: Error) => void,
): Tap {
  const node = new AudioWorkletNode(context, TAP_PROCESSOR, {
    numberOfInputs: 1,
    numberOfOutputs: 0,
    channelCount: channels,
    channelCountMode: "explicit",
    channelInterpretation: "discrete",
  });
  let stopped = false;
  node.port.onmessage = (event: MessageEvent<unknown>) => {
    let block: Float32Array[];
    try {
      block = parseTapBlock(event.data);
    } catch (error) {
      onError(toError(error));
      return;
    }
    onBlock(block);
  };
  source.connect(node);
  return {
    stop: () => {
      if (!stopped) {
        stopped = true;
        node.port.onmessage = null;
        node.port.postMessage("stop");
        source.disconnect(node);
      }
    },
  };
}

/**
 * Captures the next `frames` frames output by `source`, `channels` channels, exactly as
 * the audio thread computes them.
 *
 * Connect `source` and start capturing before the context starts running to capture from
 * its very first frame.
 */
export function capture(context: BaseAudioContext, source: AudioNode, channels: number, frames: number): Promise<Float32Array[]> {
  if (!Number.isInteger(channels) || channels < 1) {
    return Promise.reject(new RangeError(`a capture needs a whole number of channels from 1, not ${String(channels)}`));
  }
  if (!Number.isInteger(frames) || frames < 0) {
    return Promise.reject(new RangeError(`a capture needs a whole number of frames from 0, not ${String(frames)}`));
  }
  const out = Array.from({ length: channels }, () => new Float32Array(frames));
  if (frames === 0) {
    return Promise.resolve(out);
  }
  return new Promise((resolve, reject) => {
    let filled = 0;
    const running = tap(
      context,
      source,
      channels,
      (block) => {
        const count = Math.min(frames - filled, block[0]?.length ?? 0);
        out.forEach((channel, index) => {
          channel.set(block[index]?.subarray(0, count) ?? new Float32Array(count), filled);
        });
        filled += count;
        if (filled === frames) {
          running.stop();
          resolve(out);
        }
      },
      (error) => {
        running.stop();
        reject(error);
      },
    );
  });
}

/**
 * Frames rendered or captured beyond those asked for, in which the engine processor may
 * start: a browser may render the first quanta before the processor exists (seen in
 * WebKit under load, starting at frame 128). 16 384 frames: 128 render quanta.
 */
export const START_MARGIN = 16_384;

/**
 * The `frames` frames of `channels` patch channels from where the engine started, found
 * by the marker channel that follows them (see `EngineOptions.marker`).
 * @throws {Error} when the engine did not start within {@link START_MARGIN}.
 */
function fromStart(all: readonly Float32Array[], channels: number, frames: number, where: string): Float32Array[] {
  // The marker comes after the patch channels: its first 1.0 is the first frame rendered.
  const marker = all.slice(channels).flatMap((channel) => Array.from(channel));
  const start = marker.indexOf(1);
  if (start < 0) {
    throw new Error(`the engine started too late in the ${where}: not within the ${String(START_MARGIN)} frames of margin`);
  }
  if (start > START_MARGIN) {
    throw new Error(`the engine started too late in the ${where}: at frame ${String(start)}, beyond the ${String(START_MARGIN)} frames of margin`);
  }
  return all.slice(0, channels).map((channel) => channel.slice(start, start + frames));
}

/**
 * Renders `frames` frames of `loaded` offline, one array per channel: exactly the first
 * frames the engine renders, wherever the browser started the processor (located by the
 * marker channel), never a silent beginning.
 * @throws {Error} when the engine processor fails, or starts too late in the render.
 */
export async function renderOffline(loaded: LoadedPatch, frames: number, processors: string | URL): Promise<Float32Array[]> {
  const context = new OfflineAudioContext(loaded.channels + 1, frames + START_MARGIN, loaded.rate);
  await addProcessors(context, processors);
  let failure: Error | undefined;
  const engine = createEngineNode(
    context,
    loaded,
    (error) => {
      failure = error;
    },
    true,
  );
  engine.node.connect(context.destination);
  const [buffer] = await Promise.all([context.startRendering(), engine.ready]);
  if (failure !== undefined) {
    throw failure;
  }
  const all = Array.from({ length: loaded.channels + 1 }, (_empty, channel) => buffer.getChannelData(channel));
  return fromStart(all, loaded.channels, frames, "offline render");
}

/**
 * Captures the first `frames` frames the engine of `engine` renders live, `channels`
 * patch channels, wherever the browser started its processor: `engine` must have been
 * created with the marker channel. Start it, as `capture`, before the context runs.
 * @throws {Error} when the engine did not start within {@link START_MARGIN} frames.
 */
export function captureEngine(context: BaseAudioContext, engine: EngineNode, channels: number, frames: number): Promise<Float32Array[]> {
  return capture(context, engine.node, channels + 1, frames + START_MARGIN).then((all) => fromStart(all, channels, frames, "live capture"));
}
