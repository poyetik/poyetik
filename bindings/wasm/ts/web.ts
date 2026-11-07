/**
 * The engine in a web page: loading a patch, playing it in an `AudioContext`, rendering
 * it offline and capturing what a node outputs.
 *
 * The processors run in an `AudioWorklet`: register them once per context with
 * {@link addProcessors}, giving the URL of the compiled `worklet/processors.js`.
 */

import { PatchError, WasmEngine } from "./engine.ts";
import { ENGINE_PROCESSOR, TAP_PROCESSOR, parseTapBlock, type EngineOptions } from "./protocol.ts";

/** A patch checked by the engine, ready to run in an engine node. */
export interface LoadedPatch {
  /** The engine WebAssembly module, as bytes. */
  readonly wasm: ArrayBuffer;
  /** The patch, as UTF-8 text in the patch format. */
  readonly patch: Uint8Array;
  /** Sample rate it was checked at, in hertz: the rate of the contexts it runs in. */
  readonly rate: number;
  /** Number of output channels. */
  readonly channels: number;
}

/** Registers the engine and tap processors in `context`. */
export async function addProcessors(context: BaseAudioContext, url: string | URL): Promise<void> {
  await context.audioWorklet.addModule(url);
}

/**
 * Checks `patch` with the engine at `rate` hertz, on this thread: errors are thrown here
 * with the explanation of the engine, rather than in the audio thread.
 * @throws {Error} with the explanation of the engine, when it rejects the patch.
 */
export async function loadPatch(wasm: ArrayBuffer, patch: Uint8Array, rate: number): Promise<LoadedPatch> {
  const module = await WebAssembly.compile(wasm);
  try {
    const { channels } = WasmEngine.create(module, patch, rate);
    return { wasm, patch, rate, channels };
  } catch (error) {
    if (error instanceof PatchError) {
      throw new Error(new TextDecoder().decode(error.detail), { cause: error });
    }
    throw error;
  }
}

/**
 * Creates a node running `loaded`: no input, one output with one channel per patch
 * channel (one silent channel when the patch has none).
 * @throws {Error} when the context does not run at the rate the patch was checked at.
 */
export function createEngineNode(context: BaseAudioContext, loaded: LoadedPatch): AudioWorkletNode {
  if (context.sampleRate !== loaded.rate) {
    throw new Error(`the context runs at ${String(context.sampleRate)} Hz, the patch was loaded at ${String(loaded.rate)} Hz`);
  }
  const processorOptions: EngineOptions = { wasm: loaded.wasm, patch: loaded.patch };
  return new AudioWorkletNode(context, ENGINE_PROCESSOR, {
    numberOfInputs: 0,
    numberOfOutputs: 1,
    outputChannelCount: [Math.max(1, loaded.channels)],
    processorOptions,
  });
}

/** A running tap, posting every block of its source to the main thread. */
export interface Tap {
  /** Disconnects the tap and stops its processor. Calling it again does nothing. */
  readonly stop: () => void;
}

/** Turns anything thrown into an `Error` of this realm (the worklet is another realm). */
function toError(thrown: unknown): Error {
  return thrown instanceof Error ? thrown : new Error(String(thrown));
}

/**
 * Taps `source`: `onBlock` receives every block it outputs (`channels` channels, exactly
 * as the audio thread computes them), until `stop` is called. `onError` receives a
 * malformed block, which is then dropped.
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

/** Renders `frames` frames of `loaded` offline, one array per channel. */
export async function renderOffline(loaded: LoadedPatch, frames: number, processors: string | URL): Promise<Float32Array[]> {
  const context = new OfflineAudioContext(Math.max(1, loaded.channels), Math.max(1, frames), loaded.rate);
  await addProcessors(context, processors);
  createEngineNode(context, loaded).connect(context.destination);
  const buffer = await context.startRendering();
  return Array.from({ length: loaded.channels }, (_, channel) => buffer.getChannelData(channel).slice(0, frames));
}
