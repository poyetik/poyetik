/**
 * The engine in a web page: loading a patch, playing it in an `AudioContext`, rendering
 * it offline and capturing what a node outputs.
 *
 * The processors run in an `AudioWorklet`: register them once per context with
 * {@link addProcessors}, giving the URL of the compiled `worklet/processors.js`.
 */

import { ModuleError, PatchError, WasmEngine } from "./engine.ts";
import { ENGINE_PROCESSOR, MARKER_PERIOD, TAP_PROCESSOR, parseEngineMessage, parseTapMessage, type EngineOptions, type TapMessage } from "./protocol.ts";

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

/** Creates the engine node of {@link createEngineNode}, with or without the marker channel. */
function engineNode(context: BaseAudioContext, loaded: LoadedPatch, onFailure: (error: Error) => void, marker: boolean): EngineNode {
  if (context.sampleRate !== loaded.rate) {
    throw new Error(`the context runs at ${String(context.sampleRate)} Hz, the patch was loaded at ${String(loaded.rate)} Hz`);
  }
  const node = new AudioWorkletNode(context, ENGINE_PROCESSOR, {
    numberOfInputs: 0,
    numberOfOutputs: 1,
    outputChannelCount: [marker ? loaded.channels + 1 : Math.max(1, loaded.channels)],
    processorOptions: loaded.options(marker),
  });
  // The handlers are installed by the executor, which runs at once: they settle `ready`.
  const ready = new Promise<undefined>((resolve, reject) => {
    let failed = false;
    const fail = (error: Error): void => {
      if (!failed) {
        failed = true;
        reject(error);
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
          resolve(undefined);
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
  });
  // Rejections also reach onFailure: `ready` is only awaited by those who need it.
  ready.catch(() => undefined);
  return { node, ready };
}

/**
 * An engine node created with the marker channel, after its `channels` patch channels
 * (see `EngineOptions.marker`): only `createEngineNode(…, true)` makes one, and only such
 * a node can be captured with {@link captureEngine}.
 */
export class MarkedEngineNode implements EngineNode {
  /** The node: no input, one output with the patch channels then the marker channel. */
  readonly node: AudioWorkletNode;
  /** Resolves when the processor has loaded the patch; rejects if it could not. */
  readonly ready: Promise<void>;
  /** Number of patch channels, before the marker channel. */
  readonly #channels: number;

  /**
   * Records a marked node.
   * @param engine The node, created with the marker channel.
   * @param channels Its number of patch channels.
   */
  private constructor(engine: EngineNode, channels: number) {
    this.node = engine.node;
    this.ready = engine.ready;
    this.#channels = channels;
  }

  /** Number of patch channels, before the marker channel. */
  get channels(): number {
    return this.#channels;
  }

  /** Creates the marked node of `createEngineNode(…, true)`. */
  static create(context: BaseAudioContext, loaded: LoadedPatch, onFailure: (error: Error) => void): MarkedEngineNode {
    return new MarkedEngineNode(engineNode(context, loaded, onFailure, true), loaded.channels);
  }
}

/**
 * Creates a node running `loaded`: no input, one output with one channel per patch
 * channel (one silent channel when the patch has none), plus the marker channel when
 * `marker` is set (then a {@link MarkedEngineNode}). `onFailure` receives the first
 * failure of the processor, at load or later (the node is then silent); `ready` also
 * rejects with it when it comes before the patch is loaded.
 * @throws {Error} when the context does not run at the rate the patch was checked at.
 */
export function createEngineNode(context: BaseAudioContext, loaded: LoadedPatch, onFailure: (error: Error) => void, marker?: false): EngineNode;
/** Creates a node running `loaded` with the marker channel: see the overload above. */
export function createEngineNode(context: BaseAudioContext, loaded: LoadedPatch, onFailure: (error: Error) => void, marker: true): MarkedEngineNode;
/** Creates a node running `loaded`: see the overloads above. */
export function createEngineNode(context: BaseAudioContext, loaded: LoadedPatch, onFailure: (error: Error) => void, marker = false): EngineNode {
  return marker ? MarkedEngineNode.create(context, loaded, onFailure) : engineNode(context, loaded, onFailure, false);
}

/** A running tap, posting every render quantum of its source to the main thread. */
export interface Tap {
  /** Disconnects the tap and stops its processor. Calling it again does nothing. */
  readonly stop: () => void;
}

/** The failure of a capture whose context closed before its end. */
function closed(): Error {
  return new Error("the context closed before the end of the capture");
}

/** Turns anything thrown into an `Error` of this realm (the worklet is another realm). */
function toError(thrown: unknown): Error {
  return thrown instanceof Error ? thrown : new Error(String(thrown));
}

/**
 * Taps `source`: `onBlock` receives every render quantum it outputs (`channels`
 * channels, exactly as the audio thread computes them), until `stop` is called. `onError`
 * receives a malformed one, which is then dropped. The source stopping is not reported:
 * it may start again. (`onBlock` keeps its name until the public API takes its final
 * names: a render quantum is not an engine block.)
 */
export function tap(
  context: BaseAudioContext,
  source: AudioNode,
  channels: number,
  onBlock: (block: Float32Array[]) => void,
  onError: (error: Error) => void,
): Tap {
  return listen(
    context,
    source,
    channels,
    (message) => {
      if (message.type === "quantum") {
        onBlock(message.channels);
      }
    },
    onError,
  );
}

/**
 * Taps `source` as {@link tap} does, giving every message of the tap processor to
 * `onMessage`: the render quanta, and its source stopping.
 */
function listen(context: BaseAudioContext, source: AudioNode, channels: number, onMessage: (message: TapMessage) => void, onError: (error: Error) => void): Tap {
  const node = new AudioWorkletNode(context, TAP_PROCESSOR, {
    numberOfInputs: 1,
    numberOfOutputs: 0,
    channelCount: channels,
    channelCountMode: "explicit",
    channelInterpretation: "discrete",
  });
  let stopped = false;
  node.port.onmessage = (event: MessageEvent<unknown>) => {
    let message: TapMessage;
    try {
      message = parseTapMessage(event.data);
    } catch (error) {
      onError(toError(error));
      return;
    }
    onMessage(message);
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
 * @throws {Error} when the source stops, or the context closes, before the end of the
 * capture.
 */
export function capture(context: BaseAudioContext, source: AudioNode, channels: number, frames: number): Promise<Float32Array[]> {
  if (!Number.isInteger(channels) || channels < 1) {
    return Promise.reject(new RangeError(`a capture needs a whole number of channels from 1, not ${String(channels)}`));
  }
  const invalid = frameCount("a capture", frames);
  if (invalid !== undefined) {
    return Promise.reject(invalid);
  }
  const out = Array.from({ length: channels }, () => new Float32Array(frames));
  return fill(context, source, out, frames).then(() => out);
}

/**
 * Fills `out` (one array per channel, at least one, all of `frames` frames) with the next
 * frames output by `source`: see {@link capture}.
 */
function fill(context: BaseAudioContext, source: AudioNode, out: readonly Float32Array[], frames: number): Promise<void> {
  if (frames === 0) {
    return Promise.resolve();
  }
  if (context.state === "closed") {
    return Promise.reject(closed());
  }
  return new Promise((resolve, reject) => {
    let filled = 0;
    // A capture starts before its context runs: only a closed context ends it.
    const follow = (): void => {
      if (context.state === "closed") {
        end(closed());
      }
    };
    /** Stops the tap and the following of the context, then settles: failed with `error`, if given. */
    const end = (error?: Error): void => {
      running.stop();
      context.removeEventListener("statechange", follow);
      if (error === undefined) {
        resolve();
      } else {
        reject(error);
      }
    };
    const running = listen(
      context,
      source,
      out.length,
      (message) => {
        if (message.type === "stopped") {
          end(new Error("the source stopped before the end of the capture"));
          return;
        }
        const block = message.channels;
        const count = Math.min(frames - filled, block[0]?.length ?? 0);
        out.forEach((channel, index) => {
          channel.set(block[index]?.subarray(0, count) ?? new Float32Array(count), filled);
        });
        filled += count;
        if (filled === frames) {
          end();
        }
      },
      end,
    );
    context.addEventListener("statechange", follow);
  });
}

/**
 * Frames rendered or captured beyond those asked for, in which the engine processor may
 * start: a browser may render the first quanta before the processor exists (seen in
 * WebKit under load, starting at frame 128). 16 384 frames: 128 render quanta.
 */
export const START_MARGIN = 16_384;

/**
 * The most frames {@link renderOffline} and {@link captureEngine} return: with the margin,
 * the marker counts them all exactly, without starting again from 1.
 */
export const MAX_MARKED_FRAMES = MARKER_PERIOD - START_MARGIN;

/**
 * Rejects a number of frames that is not a whole number from 0, asked for by `what` (an
 * entry point: "a capture", "a render").
 */
function frameCount(what: string, frames: number): RangeError | undefined {
  return Number.isInteger(frames) && frames >= 0 ? undefined : new RangeError(`${what} needs a whole number of frames from 0, not ${String(frames)}`);
}

/** Rejects, as {@link frameCount} does, a number of frames, and one the marker channel cannot count. */
function markedFrames(what: string, frames: number): RangeError | undefined {
  return (
    frameCount(what, frames) ??
    (frames > MAX_MARKED_FRAMES ? new RangeError(`at most ${String(MAX_MARKED_FRAMES)} frames can be rendered from the start of the engine, not ${String(frames)}`) : undefined)
  );
}

/**
 * The `frames` frames of the patch `channels` from where the engine started, found by
 * the `marker` channel that follows them (see `EngineOptions.marker`), checked to count
 * every frame returned: never shifted, never cut short.
 * @throws {Error} when the engine did not start within {@link START_MARGIN}, when the
 * capture started after the engine, when the marker breaks (the engine stopped), or when
 * `marker` holds no marker.
 */
function fromStart(channels: readonly Float32Array[], marker: Float32Array, frames: number, where: string): Float32Array[] {
  // Read in place: never copied into a JavaScript array, whatever its length.
  const start = marker.findIndex((value) => value !== 0);
  const first = marker[start];
  if (first === undefined) {
    throw new Error(`the engine started too late in the ${where}: not within the ${String(START_MARGIN)} frames of margin`);
  }
  if (!Number.isInteger(first) || first < 1) {
    throw new Error(`the ${where} has no marker after the patch channels: ${String(first)} at frame ${String(start)}`);
  }
  if (first !== 1) {
    throw new Error(`the ${where} started ${String(first - 1)} frames after the engine`);
  }
  if (start > START_MARGIN) {
    throw new Error(`the engine started too late in the ${where}: at frame ${String(start)}, beyond the ${String(START_MARGIN)} frames of margin`);
  }
  for (let frame = 0; frame < frames; frame += 1) {
    if (marker[start + frame] !== frame + 1) {
      throw new Error(`the engine output of the ${where} breaks at frame ${String(frame)}: the engine stopped`);
    }
  }
  return channels.map((channel) => channel.slice(start, start + frames));
}

/**
 * Renders `frames` frames of `loaded` offline, one array per channel: exactly the first
 * frames the engine renders, wherever the browser started the processor (located by the
 * marker channel), never a silent beginning or a silent end.
 * @throws {RangeError} when `frames` is not a whole number from 0 to {@link MAX_MARKED_FRAMES}.
 * @throws {Error} when the engine processor fails, starts too late in the render, or
 * stops before its end.
 */
export async function renderOffline(loaded: LoadedPatch, frames: number, processors: string | URL): Promise<Float32Array[]> {
  const invalid = markedFrames("a render", frames);
  if (invalid !== undefined) {
    throw invalid;
  }
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
  // Not `engine.ready`: a processor the browser never constructs during the render never
  // answers; the marker channel then shows that the engine started too late.
  const buffer = await context.startRendering();
  if (failure !== undefined) {
    throw failure;
  }
  const channels = Array.from({ length: loaded.channels }, (_empty, channel) => buffer.getChannelData(channel));
  return fromStart(channels, buffer.getChannelData(loaded.channels), frames, "offline render");
}

/**
 * Captures the first `frames` frames the engine of `engine` renders live, wherever the
 * browser started its processor. Start it, as `capture`, before the context runs.
 * @throws {RangeError} when `frames` is not a whole number from 0 to {@link MAX_MARKED_FRAMES}.
 * @throws {Error} when the engine did not start within {@link START_MARGIN} frames, when
 * the capture started after it, or when it stopped before the end of the capture.
 */
export function captureEngine(context: BaseAudioContext, engine: MarkedEngineNode, frames: number): Promise<Float32Array[]> {
  const invalid = markedFrames("a capture", frames);
  if (invalid !== undefined) {
    return Promise.reject(invalid);
  }
  const length = frames + START_MARGIN;
  const channels = Array.from({ length: engine.channels }, () => new Float32Array(length));
  const marker = new Float32Array(length);
  return fill(context, engine.node, [...channels, marker], length).then(() => fromStart(channels, marker, frames, "live capture"));
}
