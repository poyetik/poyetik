/** What the main thread and the `AudioWorklet` processors agree on. */

import { DETECTORS, OUTPUT_MODES, PRECISIONS, RAW_OUTPUT, parsePrecision, type Detector, type LimiterSettings, type Output, type OutputMode, type Precision, type Replaced, type SubnormalArithmetic, type SubnormalsKept } from "./engine.ts";

/** Name of the processor running the engine. */
export const ENGINE_PROCESSOR = "poyetik-engine";

/** Name of the processor posting its input to the main thread, one render quantum at a time. */
export const TAP_PROCESSOR = "poyetik-tap";

/** Options of the engine processor (`processorOptions`). */
export interface EngineOptions {
  /** The engine WebAssembly module, as bytes: compiled again in the worklet. */
  readonly wasm: ArrayBuffer;
  /** The patch, as UTF-8 text in the patch format. */
  readonly patch: Uint8Array;
  /**
   * Whether to add one channel after the patch channels, counting the frames the
   * processor renders: 1, 2, 3… (after {@link MARKER_PERIOD}, 1 again). It shows the frame
   * where rendering really started, and that no frame was lost after it.
   */
  readonly marker: boolean;
  /** The precision the engine computes in; its render is given to Web Audio as binary32. */
  readonly precision: Precision;
  /** The output stage the engine is created with (`spec/engine-design.md`, decision 12). */
  readonly output: Output;
}

/**
 * The marker channel counts frames from 1 to this, then from 1 again: 2^24, the largest
 * count a 32-bit float holds exactly, as every count before it.
 */
export const MARKER_PERIOD = 2 ** 24;

/** Thrown when processor options do not have the expected shape. */
export class OptionsError extends Error {
  /**
   * Describes invalid options.
   * @param detail What is wrong with the options.
   */
  constructor(detail: string) {
    super(`invalid processor options: ${detail}`);
    this.name = "OptionsError";
  }
}

/**
 * Thrown when a message from a processor (an engine message, a render quantum of a tap) does not have
 * the expected shape: a bug, or a message from something else on the port.
 */
export class MessageError extends Error {
  /**
   * Describes an invalid message.
   * @param detail What is wrong with the message.
   */
  constructor(detail: string) {
    super(`invalid message from the processor: ${detail}`);
    this.name = "MessageError";
  }
}

/** The limiter settings `value`, four numbers; `fail` makes the error naming what is wrong. */
export function parseLimiter(value: unknown, fail: (detail: string) => Error): LimiterSettings {
  if (typeof value !== "object" || value === null) {
    throw fail("limiter is not an object");
  }
  const number = (name: string): number => {
    const field: unknown = Reflect.get(value, name);
    if (typeof field !== "number") {
      throw fail(`limiter ${name} is not a number`);
    }
    return field;
  };
  return { ceiling: number("ceiling"), lookahead: number("lookahead"), release: number("release"), adaptivity: number("adaptivity") };
}

/** Whether `value` is a mode of the output stage. */
function isOutputMode(value: unknown): value is OutputMode {
  return OUTPUT_MODES.some((mode) => mode === value);
}

/** Whether `value` names a detector of the limiter's true peak. */
function isDetector(value: unknown): value is Detector {
  return DETECTORS.some((detector) => detector === value);
}

/**
 * The output stage `value`, read at a boundary (the engine options, `LoadedPatch.load`): its
 * mode and, unless it is raw, its maximum lookahead, detector and limiter, each of its type,
 * in a frozen copy holding nothing else (a raw stage holds its mode only); `fail` makes the
 * error naming what is wrong. The engine checks their ranges. The one reader of an output
 * stage from outside the type system, so that what the main thread checks is what the
 * processors accept.
 */
export function parseOutput(value: unknown, fail: (detail: string) => Error): Output {
  if (typeof value !== "object" || value === null) {
    throw fail("output is not an object");
  }
  const mode: unknown = Reflect.get(value, "mode");
  if (!isOutputMode(mode)) {
    throw fail(`output mode is not one of ${OUTPUT_MODES.join(", ")}`);
  }
  if (mode === "raw") {
    return RAW_OUTPUT;
  }
  const maximumLookahead: unknown = Reflect.get(value, "maximumLookahead");
  if (typeof maximumLookahead !== "number") {
    throw fail("output maximumLookahead is not a number");
  }
  const detector: unknown = Reflect.get(value, "detector");
  if (!isDetector(detector)) {
    throw fail(`output detector is not one of ${DETECTORS.join(", ")}`);
  }
  const limiter = Object.freeze(parseLimiter(Reflect.get(value, "limiter"), (detail) => fail(`output ${detail}`)));
  return Object.freeze({ mode, maximumLookahead, detector, limiter });
}

/** Checks the options received by the engine processor. */
export function parseEngineOptions(value: unknown): EngineOptions {
  if (typeof value !== "object" || value === null) {
    throw new OptionsError("not an object");
  }
  const wasm: unknown = Reflect.get(value, "wasm");
  const patch: unknown = Reflect.get(value, "patch");
  if (!(wasm instanceof ArrayBuffer)) {
    throw new OptionsError("wasm is not an ArrayBuffer");
  }
  if (!(patch instanceof Uint8Array)) {
    throw new OptionsError("patch is not a Uint8Array");
  }
  const marker: unknown = Reflect.get(value, "marker");
  if (typeof marker !== "boolean") {
    throw new OptionsError("marker is not a boolean");
  }
  const known = parsePrecision(Reflect.get(value, "precision"));
  if (known === undefined) {
    throw new OptionsError(`precision is not one of ${PRECISIONS.join(", ")}`);
  }
  return { wasm, patch, marker, precision: known, output: parseOutput(Reflect.get(value, "output"), (detail) => new OptionsError(detail)) };
}

/**
 * What the tap processor posts, once, when its input goes from present to empty: its
 * source stopped (an engine that failed, a source that ended or was disconnected).
 */
export const SOURCE_STOPPED = "source stopped";

/** What the tap processor tells the main thread. */
export type TapMessage =
  /** A render quantum of its input: one array per channel. */
  | { readonly type: "quantum"; readonly channels: Float32Array[] }
  /** Its source stopped: no render quantum comes any more until it starts again. */
  | { readonly type: "stopped" };

/** Checks a message posted by the tap processor: a render quantum, or {@link SOURCE_STOPPED}. */
export function parseTapMessage(value: unknown): TapMessage {
  if (value === SOURCE_STOPPED) {
    return { type: "stopped" };
  }
  if (!Array.isArray(value)) {
    throw new MessageError("render quantum is not an array");
  }
  return {
    type: "quantum",
    channels: value.map((channel: unknown) => {
      if (!(channel instanceof Float32Array)) {
        throw new MessageError("tap channel is not a Float32Array");
      }
      return channel;
    }),
  };
}

/**
 * What the engine processor tells the main thread: once whether the patch is loaded
 * (`ready`) or why not, then, after `ready`, what the output stage replaced when it
 * replaced more (`replaced`, at most once per {@link REPLACED_REPORT_SECONDS}), for each
 * live command it refuses or cannot read a `refused` or `malformed` message, and at most
 * one failure while rendering.
 */
export type EngineMessage =
  /**
   * The patch is loaded: the processor renders it. `subnormals` is what the arithmetic of
   * the audio thread does with subnormals, observed by the module when it was loaded.
   */
  | { readonly type: "ready"; readonly subnormals: SubnormalArithmetic }
  /** The engine rejected the patch; `detail` is its UTF-8 explanation. */
  | { readonly type: "rejected"; readonly detail: Uint8Array }
  /** Anything else went wrong (invalid options, a trap): the processor is silent. */
  | { readonly type: "failed"; readonly message: string }
  /** The output stage replaced non-finite samples by zero: how many so far, and the first. */
  | { readonly type: "replaced"; readonly replaced: Required<Replaced> }
  /** The engine refused a command of the main thread (a setting of its output stage); `detail` is its UTF-8 explanation. It renders on. */
  | { readonly type: "refused"; readonly detail: Uint8Array }
  /** The processor could not read a command of the main thread (a {@link CommandError}, from plain JavaScript); `detail` says what is wrong with it. It renders on. */
  | { readonly type: "malformed"; readonly detail: string };

/**
 * The shortest time between two `replaced` messages of one engine processor, in seconds of
 * audio: the main thread learns of the first replacement at once, then of more at most
 * this often, so that a box emitting NaN at every frame does not flood the message port.
 */
export const REPLACED_REPORT_SECONDS = 1;

/** Whether `value` is a whole number exact as a number: `Number.isSafeInteger`, which refuses every other type, as a type guard. */
function isSafeWhole(value: unknown): value is number {
  return Number.isSafeInteger(value);
}

/** Checks a count of an engine message: a whole number from `minimum` on, exact as a number. */
function parseCount(value: unknown, name: string, minimum: number): number {
  if (!isSafeWhole(value) || value < minimum) {
    throw new MessageError(`${name} is not a whole number from ${String(minimum)}`);
  }
  return value;
}

/** Checks what a `replaced` message says the output stage replaced: at least one sample. */
function parseReplaced(value: unknown): Required<Replaced> {
  if (typeof value !== "object" || value === null) {
    throw new MessageError("replaced samples are not an object");
  }
  const first: unknown = Reflect.get(value, "first");
  if (typeof first !== "object" || first === null) {
    throw new MessageError("the first replaced sample is not an object");
  }
  return {
    count: parseCount(Reflect.get(value, "count"), "the replaced count", 1),
    first: {
      frame: parseCount(Reflect.get(first, "frame"), "the first replaced frame", 0),
      channel: parseCount(Reflect.get(first, "channel"), "the first replaced channel", 0),
    },
  };
}

/** Checks what subnormals a format keeps, in an engine message; `format` names it in the error. */
function parseSubnormalsKept(value: unknown, format: string): SubnormalsKept {
  if (typeof value !== "object" || value === null) {
    throw new MessageError(`${format} subnormals are not an object`);
  }
  const operands: unknown = Reflect.get(value, "operands");
  const results: unknown = Reflect.get(value, "results");
  if (typeof operands !== "boolean" || typeof results !== "boolean") {
    throw new MessageError(`${format} subnormals are not two booleans`);
  }
  return { operands, results };
}

/** Checks the subnormal arithmetic of a `ready` message. */
function parseSubnormalArithmetic(value: unknown): SubnormalArithmetic {
  if (typeof value !== "object" || value === null) {
    throw new MessageError("subnormal arithmetic is not an object");
  }
  return {
    binary32: parseSubnormalsKept(Reflect.get(value, "binary32"), "binary32"),
    binary64: parseSubnormalsKept(Reflect.get(value, "binary64"), "binary64"),
  };
}

/** Checks a message posted by the engine processor. */
export function parseEngineMessage(value: unknown): EngineMessage {
  if (typeof value !== "object" || value === null) {
    throw new MessageError("engine message is not an object");
  }
  const type: unknown = Reflect.get(value, "type");
  if (type === "ready") {
    return { type, subnormals: parseSubnormalArithmetic(Reflect.get(value, "subnormals")) };
  }
  if (type === "rejected" || type === "refused") {
    const detail: unknown = Reflect.get(value, "detail");
    if (!(detail instanceof Uint8Array)) {
      throw new MessageError(`${type === "rejected" ? "rejection" : "refusal"} detail is not a Uint8Array`);
    }
    return { type, detail };
  }
  if (type === "failed") {
    const message: unknown = Reflect.get(value, "message");
    if (typeof message !== "string") {
      throw new MessageError("failure message is not a string");
    }
    return { type, message };
  }
  if (type === "replaced") {
    return { type, replaced: parseReplaced(Reflect.get(value, "replaced")) };
  }
  if (type === "malformed") {
    const detail: unknown = Reflect.get(value, "detail");
    if (typeof detail !== "string") {
      throw new MessageError("malformed command detail is not a string");
    }
    return { type, detail };
  }
  throw new MessageError("unknown engine message");
}

/**
 * Thrown when a command posted to the engine processor does not have the expected shape; the
 * processor tells it back as a `malformed` message, and the main thread builds it again as
 * the cause of the refusal it hands over.
 */
export class CommandError extends Error {
  /** What is wrong with the command. */
  readonly detail: string;

  /**
   * Describes an invalid command.
   * @param detail What is wrong with the command.
   */
  constructor(detail: string) {
    super(`invalid command to the engine processor: ${detail}`);
    this.name = "CommandError";
    this.detail = detail;
  }
}

/** What the main thread asks of a running engine processor: the output stage's settings, live. */
export type EngineCommand =
  /** New settings of the limiter. */
  | { readonly type: "limiter"; readonly settings: LimiterSettings }
  /** A crossfade of the limiter toward protected (`true`) or raw (`false`). */
  | { readonly type: "protect"; readonly protect: boolean };

/** Checks a command posted to the engine processor. */
export function parseEngineCommand(value: unknown): EngineCommand {
  if (typeof value !== "object" || value === null) {
    throw new CommandError("not an object");
  }
  const type: unknown = Reflect.get(value, "type");
  if (type === "limiter") {
    return { type, settings: parseLimiter(Reflect.get(value, "settings"), (detail) => new CommandError(detail)) };
  }
  if (type === "protect") {
    const protect: unknown = Reflect.get(value, "protect");
    if (typeof protect !== "boolean") {
      throw new CommandError("protect is not a boolean");
    }
    return { type, protect };
  }
  throw new CommandError("unknown command");
}
