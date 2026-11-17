/**
 * The engine in a web page: loading a patch, playing it in an `AudioContext`, rendering
 * it offline and capturing what a node outputs.
 *
 * The processors run in an `AudioWorklet`: register them once per context with
 * {@link addProcessors}, giving the URL of the compiled `worklet/processors.js`.
 */

import { ModuleError, OutputError, PatchError, RAW_OUTPUT, WasmEngine, givenAsBinary32, type LimiterSettings, type Output, type Precision, type ProtectedOutput, type RawOutput, type Replaced, type SubnormalArithmetic } from "./engine.ts";
export {
  DEFAULT_LIMITER,
  DETECTORS,
  MAX_LOOKAHEAD,
  MAX_REPLACED_COUNT,
  OUTPUT_MODES,
  PRECISIONS,
  PROTECTED_OUTPUT,
  RAW_OUTPUT,
  givenAsBinary32,
  parsePrecision,
  type Detector,
  type FirstReplaced,
  type LimitedOutput,
  type LimiterSettings,
  type LimiterStage,
  type Output,
  type OutputMode,
  type Precision,
  type ProtectedOutput,
  type RawOutput,
  type Replaced,
  type SubnormalArithmetic,
  type SubnormalsKept,
  type SwitchableOutput,
} from "./engine.ts";
export { ENGINE_BLOCKS, type EngineBlock } from "./blocks.ts";
export { REPLACED_REPORT_SECONDS } from "./protocol.ts";
import { CommandError, ENGINE_PROCESSOR, MARKER_PERIOD, TAP_PROCESSOR, parseEngineMessage, parseOutput, parseTapMessage, type EngineCommand, type EngineOptions, type TapMessage } from "./protocol.ts";

/**
 * The default render quantum of Web Audio 1.0, in frames: what an audio worklet is called
 * with unless its context runs another (`renderSizeHint`, Web Audio 1.1); the engine
 * processor renders whatever length it is called with.
 */
export const RENDER_QUANTUM = 128;

/**
 * What the engine applies in the browser (`spec/engine-design.md`, decision 1): the settings
 * of an engine node, and those the browser imposes rather than the host choosing them.
 */
export interface BrowserSettings {
  /** The sample rate, in hertz: the context's. */
  readonly rate: number;
  /**
   * The frames the engine is called for at a time: Web Audio's render quantum, assumed to
   * be its default ({@link RENDER_QUANTUM}), since it is computed before any context exists.
   */
  readonly renderQuantum: number;
  /** The engine block of the module, in frames (`engine_block_size`): the module chosen decides it. */
  readonly engineBlock: number;
  /** The settings the browser imposes: the rate the context runs at, which may differ from the one asked of it, and the render quantum, which no worklet chooses. */
  readonly imposed: readonly ("rate" | "renderQuantum")[];
}

/**
 * The output stage of a patch loaded with `O`: the one of `O`'s mode, since a load keeps a
 * copy of the stage it reads (its mode and settings), none of the caller's other fields.
 */
export type OutputOf<O extends Output> = Extract<Output, { readonly mode: O["mode"] }>;

/**
 * A patch checked by the engine, ready to run in engine nodes. Only {@link loadPatch} or
 * {@link LoadedPatch.load} makes one: holding a `LoadedPatch` proves the engine accepted this patch at this rate,
 * in this precision, for a precision of the type (a plain-JavaScript caller passing another
 * value is not checked here: #32 step 7 decides those checks for every exported function).
 * It keeps its own copies of the bytes, so changing the arrays given to `loadPatch`
 * afterwards changes nothing. Its type names the kind of its output stage (`O`), which
 * decides what its engine nodes offer ({@link createEngineNode}).
 */
export class LoadedPatch<O extends Output = Output> {
  /** Sample rate it was checked at, in hertz: the rate of the contexts it runs in. */
  readonly rate: number;
  /** Number of output channels. */
  readonly channels: number;
  /** The precision the engine computes it in (a host setting); Web Audio is given binary32. */
  readonly precision: Precision;
  /** The output stage its engines are created with (a host setting, `spec/engine-design.md` decision 12): of the mode it was given. */
  readonly output: O;
  /** The latency of that stage, in frames: an engine node's sound starts that much later. */
  readonly latency: number;
  /** What the engine applies in the browser, and what the browser imposes (decision 1). */
  readonly settings: BrowserSettings;
  /** The engine WebAssembly module, as bytes. */
  readonly #wasm: ArrayBuffer;
  /** The patch, as UTF-8 text in the patch format. */
  readonly #patch: Uint8Array;

  /**
   * Records a checked patch.
   * @param wasm A copy of the module bytes.
   * @param patch A copy of the patch bytes.
   * @param rate The rate it was checked at.
   * @param precision The precision it was checked in.
   * @param output The output stage it was checked with.
   * @param engine Its number of output channels and its latency.
   * @param engine.channels Its number of output channels.
   * @param engine.latency The latency of its output stage, in frames.
   * @param engine.blockSize The engine block of its module, in frames.
   */
  private constructor(wasm: ArrayBuffer, patch: Uint8Array, rate: number, precision: Precision, output: O, engine: { readonly channels: number; readonly latency: number; readonly blockSize: number }) {
    this.#wasm = wasm;
    this.#patch = patch;
    this.rate = rate;
    this.precision = precision;
    this.output = output;
    this.channels = engine.channels;
    this.latency = engine.latency;
    this.settings = { rate, renderQuantum: RENDER_QUANTUM, engineBlock: engine.blockSize, imposed: ["rate", "renderQuantum"] };
  }

  /**
   * Checks `patch` with the engine at `rate` hertz in `precision` (binary64 by default, a
   * host setting; the render is given to Web Audio as binary32) through the output stage
   * `output` (raw by default), on this thread: errors are thrown here
   * with the explanation of the engine, rather than in the audio thread. What it rejects
   * with, told apart by `name` (the error classes are not exported yet, roadmap #32 step 12):
   * @throws {RangeError} when `rate` is not a whole number of hertz that fits 32 bits.
   * @throws {TypeError} when a field of `output` is not of its type (a plain-JavaScript
   * caller), as its processors would refuse it, before the module is compiled.
   * @throws {Error} named `ModuleError` ("not an engine module: …") when `wasm` is not an
   * engine module of this version.
   * @throws {Error} with the explanation of the engine as message, and a `PatchError` as
   * `cause`, when it rejects the patch or the rate (an unsupported rate, a patch that is
   * not UTF-8, invalid or that cannot be planned).
   * @throws {Error} with the explanation of the engine as message, and an `OutputError` as
   * `cause`, when it refuses the output stage (a setting out of range, a lookahead above
   * the maximum).
   * @throws {Error} named `EngineTrap`, with the WebAssembly `RuntimeError` as `cause`, when
   * the module traps while checking the patch.
   */
  static load(wasm: ArrayBuffer, patch: Uint8Array, rate: number, precision?: Precision): Promise<LoadedPatch<RawOutput>>;
  /**
   * Checks `patch` through the output stage `output`: see the overload above. Its type is
   * inferred from `output`, never named without it, so that a patch typed as limited is one
   * loaded limited, and is the stage of its mode ({@link OutputOf}), which a load copies.
   */
  static load<O extends Output>(wasm: ArrayBuffer, patch: Uint8Array, rate: number, precision: Precision | undefined, output: O): Promise<LoadedPatch<OutputOf<O>>>;
  /** Checks `patch`: see the overloads above, typed by the mode of its output stage. */
  static async load(wasm: ArrayBuffer, patch: Uint8Array, rate: number, precision: Precision = "binary64", output: Output = RAW_OUTPUT): Promise<LoadedPatch> {
    // Its fields read once, by the processors' own reader, into a frozen copy: the engine
    // checks what the processors get, and a field of the wrong type is refused here.
    const stage = parseOutput(output, (detail) => new TypeError(detail));
    const bytes = wasm.slice(0);
    const module = await WebAssembly.compile(bytes).catch((error: unknown) => {
      throw new ModuleError(`cannot be compiled: ${String(error)}`, { cause: error });
    });
    const copy = patch.slice();
    try {
      const engine = WasmEngine.create(module, copy, rate, givenAsBinary32(precision), stage);
      return new LoadedPatch(bytes, copy, rate, precision, stage, engine);
    } catch (error) {
      throw error instanceof PatchError || error instanceof OutputError ? new Error(new TextDecoder().decode(error.detail), { cause: error }) : error;
    }
  }

  /**
   * The options of an engine node running this patch, with fresh copies of the bytes.
   * @param marker Whether the node adds a marker channel (see `EngineOptions`).
   */
  options(marker: boolean): EngineOptions {
    return { wasm: this.#wasm.slice(0), patch: this.#patch.slice(), marker, precision: this.precision, output: this.output };
  }
}

/** Registers the engine and tap processors in `context`, naming the URL on failure. */
export async function addProcessors(context: BaseAudioContext, url: string | URL): Promise<void> {
  await context.audioWorklet.addModule(url).catch((error: unknown) => {
    throw new Error(`cannot load the processors from ${String(url)}: ${String(error)}`, { cause: error });
  });
}

/**
 * Checks `patch` with the engine at `rate` hertz, to compute in `precision` through the
 * output stage `output` (by default, those of {@link LoadedPatch.load}): see {@link LoadedPatch.load}, which lists what
 * it rejects with.
 * @throws {Error} as {@link LoadedPatch.load}.
 */
export function loadPatch(wasm: ArrayBuffer, patch: Uint8Array, rate: number, precision?: Precision): Promise<LoadedPatch<RawOutput>>;
/** Checks `patch` through the output stage `output`, whose type it infers, typed by its mode ({@link OutputOf}): see {@link LoadedPatch.load}. */
export function loadPatch<O extends Output>(wasm: ArrayBuffer, patch: Uint8Array, rate: number, precision: Precision | undefined, output: O): Promise<LoadedPatch<OutputOf<O>>>;
/** Checks `patch`: see the overloads above. */
export function loadPatch(wasm: ArrayBuffer, patch: Uint8Array, rate: number, precision?: Precision, output: Output = RAW_OUTPUT): Promise<LoadedPatch> {
  return LoadedPatch.load(wasm, patch, rate, precision, output);
}

/** What every engine node offers: the node, and when it is ready. */
export interface EngineNodeBase {
  /** The node: no input, one output with one channel per patch channel. */
  readonly node: AudioWorkletNode;
  /**
   * Resolves when the processor has loaded the patch, with what the arithmetic of the
   * audio thread does with subnormals (IEEE when every one is kept); rejects if it could
   * not, or with a {@link ContextClosed} if the context closed first (then it never will).
   */
  readonly ready: Promise<SubnormalArithmetic>;
}

/** The engine node of a patch loaded with a raw output stage: nothing of it is set live. */
export interface RawEngineNode extends EngineNodeBase {
  /** The mode of its output stage. */
  readonly mode: "raw";
}

/** The engine node of a patch loaded with a protected output stage: its limiter set live, never switched to raw. */
export interface ProtectedEngineNode extends EngineNodeBase {
  /** The mode of its output stage. */
  readonly mode: "protected";
  /**
   * Changes the settings of its output stage's limiter, live, from the processor's next
   * render quantum; a refusal (a value out of range, or a command the processor cannot
   * read, from plain JavaScript) reaches `onRefused`, the engine rendering on with the
   * settings it had.
   */
  readonly setLimiter: (settings: LimiterSettings) => void;
}

/** The engine node of a patch loaded with a switchable output stage: its limiter set and switched live. */
export interface SwitchableEngineNode extends EngineNodeBase {
  /** The mode of its output stage. */
  readonly mode: "switchable";
  /** Changes the settings of its limiter, live: see {@link ProtectedEngineNode.setLimiter}. */
  readonly setLimiter: (settings: LimiterSettings) => void;
  /**
   * Starts the crossfade of its limiter toward protected (`true`) or raw (`false`); a
   * refusal reaches `onRefused`, the engine rendering on.
   */
  readonly setProtected: (protect: boolean) => void;
}

/** An engine node, of any mode: told apart by `mode`. */
export type EngineNode = RawEngineNode | ProtectedEngineNode | SwitchableEngineNode;

/** The engine node of a patch loaded with the output stage `O`: what it offers follows its mode. */
export type EngineNodeOf<O extends Output> = O extends RawOutput ? RawEngineNode : O extends ProtectedOutput ? ProtectedEngineNode : SwitchableEngineNode;

/** What an engine node of a raw output stage tells its creator, beside `ready`: it sets nothing live, so nothing is refused. */
export interface RawHandlers {
  /**
   * Receives the first failure of the processor, at load or later (the node is then
   * silent); not a closing context, which is no failure of the engine.
   */
  readonly onFailure: (error: Error) => void;
  /**
   * Receives what the output stage replaced (non-finite samples, each played as silence,
   * `spec/engine-design.md` decision 12): the count so far and the first sample, at once
   * the first time, then when it grew, at most once per `REPLACED_REPORT_SECONDS` of audio.
   * The processor posts no final count when the node stops: samples replaced in the last
   * interval after the previous report are never told. Once the count reaches
   * `MAX_REPLACED_COUNT`, where it saturates, no report follows.
   */
  readonly onReplaced?: (replaced: Required<Replaced>) => void;
}

/** What an engine node of a limited output stage tells its creator: its refusals too, which it must hear. */
export interface LimitedHandlers extends RawHandlers {
  /**
   * Receives the refusal of a setting set live ({@link ProtectedEngineNode.setLimiter},
   * {@link SwitchableEngineNode.setProtected}), its explanation as the message, the engine
   * rendering on with the settings it had: by the engine, with an `OutputError` as `cause`;
   * by the processor, for a command it cannot read (from plain JavaScript), with a
   * `CommandError` as `cause` (told apart by `name`: the error classes are not exported yet,
   * roadmap #32 step 12).
   */
  readonly onRefused: (error: Error) => void;
}

/**
 * The handlers an engine node of a patch loaded with the output stage `O` takes: a limited
 * stage's, which hear refusals, unless the stage is known to be raw.
 */
export type HandlersOf<O extends Output> = [O] extends [RawOutput] ? RawHandlers : LimitedHandlers;

/** What the implementation receives: either kind of handlers. */
type AnyHandlers = RawHandlers & { readonly onRefused?: (error: Error) => void };

/** Creates the engine node of {@link createEngineNode}, with or without the marker channel. */
function engineNode(context: BaseAudioContext, loaded: LoadedPatch, handlers: AnyHandlers, marker: boolean): EngineNode {
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
  const ready = new Promise<SubnormalArithmetic>((resolve, reject) => {
    let failed = false;
    // A processor never answers once its context is closed: the wait ends there too,
    // which is no failure of the engine (`onFailure` is not told).
    const follow = (): void => {
      if (context.state === "closed") {
        context.removeEventListener("statechange", follow);
        reject(new ContextClosed("the context closed before the engine processor was ready"));
      }
    };
    const fail = (error: Error): void => {
      if (!failed) {
        failed = true;
        context.removeEventListener("statechange", follow);
        reject(error);
        handlers.onFailure(error);
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
          context.removeEventListener("statechange", follow);
          resolve(message.subnormals);
          break;
        case "rejected":
          fail(new Error(new TextDecoder().decode(message.detail), { cause: new PatchError(message.detail) }));
          break;
        case "failed":
          fail(new Error(`the engine failed: ${message.message}`));
          break;
        case "replaced":
          handlers.onReplaced?.(message.replaced);
          break;
        case "refused":
          handlers.onRefused?.(new Error(new TextDecoder().decode(message.detail), { cause: new OutputError(message.detail) }));
          break;
        case "malformed": {
          const cause = new CommandError(message.detail);
          handlers.onRefused?.(new Error(cause.message, { cause }));
          break;
        }
      }
    };
    node.onprocessorerror = () => {
      fail(new Error("the engine processor failed"));
    };
    context.addEventListener("statechange", follow);
    follow();
  });
  // Rejections also reach onFailure: `ready` is only awaited by those who need it.
  ready.catch(() => undefined);
  const command = (message: EngineCommand): void => {
    node.port.postMessage(message);
  };
  /** Posts new settings of the limiter. */
  const setLimiter = (settings: LimiterSettings): void => {
    command({ type: "limiter", settings });
  };
  // Only what the stage of the loaded patch can set live.
  switch (loaded.output.mode) {
    case "raw":
      return { mode: "raw", node, ready };
    case "protected":
      return { mode: "protected", node, ready, setLimiter };
    case "switchable":
      return {
        mode: "switchable",
        node,
        ready,
        setLimiter,
        setProtected: (protect) => {
          command({ type: "protect", protect });
        },
      };
  }
}

/**
 * An engine node created with the marker channel, after its `channels` patch channels
 * (see `EngineOptions.marker`): only `createEngineNode(…, true)` makes one, and only such
 * a node can be captured with {@link captureEngine}. `engine` is the node as its mode
 * offers it (`N`).
 */
export class MarkedEngineNode<N extends EngineNode = EngineNode> {
  /** The node, as its output stage's mode offers it: its controls set live. */
  readonly engine: N;
  /** The node: no input, one output with the patch channels then the marker channel. */
  readonly node: AudioWorkletNode;
  /** Resolves when the processor has loaded the patch, with the audio thread's subnormal arithmetic; rejects if it could not, or with a {@link ContextClosed} if the context closed first. */
  readonly ready: Promise<SubnormalArithmetic>;
  /** Number of patch channels, before the marker channel. */
  readonly #channels: number;

  /**
   * Records a marked node.
   * @param engine The node, created with the marker channel.
   * @param channels Its number of patch channels.
   */
  private constructor(engine: N, channels: number) {
    this.engine = engine;
    this.node = engine.node;
    this.ready = engine.ready;
    this.#channels = channels;
  }

  /** Number of patch channels, before the marker channel. */
  get channels(): number {
    return this.#channels;
  }

  /** Creates the marked node of `createEngineNode(…, true)`. */
  static create(context: BaseAudioContext, loaded: LoadedPatch, handlers: AnyHandlers): MarkedEngineNode {
    return new MarkedEngineNode(engineNode(context, loaded, handlers, true), loaded.channels);
  }
}

/**
 * Creates a node running `loaded`: no input, one output with one channel per patch
 * channel (one silent channel when the patch has none), plus the marker channel when
 * `marker` is set (then a {@link MarkedEngineNode}). What the node offers follows the mode
 * of the patch's output stage (`spec/engine-design.md`, decision 12): a raw node sets
 * nothing live; a protected one sets its limiter ({@link ProtectedEngineNode}); a
 * switchable one also switches it ({@link SwitchableEngineNode}). `handlers.onFailure`
 * receives the first failure of the processor, at load or later (the node is then silent);
 * `ready` also rejects with it when it comes before the patch is loaded, and rejects with a
 * {@link ContextClosed} when the context closes first (which `onFailure` is not told: the
 * engine did not fail). `handlers.onReplaced` receives what the output stage replaced;
 * a limited node's `handlers.onRefused`, required, the refusals of what it sets live (see
 * {@link LimitedHandlers}).
 * @throws {Error} when the context does not run at the rate the patch was checked at.
 */
export function createEngineNode<O extends Output>(context: BaseAudioContext, loaded: LoadedPatch<O>, handlers: HandlersOf<O>, marker?: false): EngineNodeOf<O>;
/** Creates a node running `loaded` with the marker channel: see the overload above. */
export function createEngineNode<O extends Output>(context: BaseAudioContext, loaded: LoadedPatch<O>, handlers: HandlersOf<O>, marker: true): MarkedEngineNode<EngineNodeOf<O>>;
/** Creates a node running `loaded`: see the overloads above. */
export function createEngineNode(context: BaseAudioContext, loaded: LoadedPatch, handlers: AnyHandlers, marker = false): EngineNode | MarkedEngineNode {
  return marker ? MarkedEngineNode.create(context, loaded, handlers) : engineNode(context, loaded, handlers, false);
}

/** A running tap, posting every render quantum of its source to the main thread. */
export interface Tap {
  /** Disconnects the tap and stops its processor. Calling it again does nothing. */
  readonly stop: () => void;
}

/**
 * Thrown when the audio context closed before the work that needed it ended: the end of
 * the context (the browser closed the device, or its owner closed it), no failure of the
 * engine; told apart from the engine's errors by its name.
 */
export class ContextClosed extends Error {
  /**
   * Records what the context's end interrupted.
   * @param message What could not end.
   */
  constructor(message: string) {
    super(message);
    this.name = "ContextClosed";
  }
}

/** The failure of a capture whose context closed before its end. */
function closed(): ContextClosed {
  return new ContextClosed("the context closed before the end of the capture");
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
 * @throws {Error} when the source stops before the end of the capture.
 * @throws {ContextClosed} when the context closes before the end of the capture.
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
 * marker channel), never a silent beginning or a silent end. It returns the samples only:
 * what the output stage replaced (each non-finite sample rendered as zero) is not
 * reported.
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
  // A marked node whatever the mode of its stage: nothing is set live while it renders, so
  // nothing can be refused.
  const engine = MarkedEngineNode.create(context, loaded, {
    onFailure: (error) => {
      failure = error;
    },
  });
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
 * @throws {ContextClosed} when the context closes before the end of the capture.
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
