/** What the main thread and the `AudioWorklet` processors agree on. */

import { PRECISIONS, parsePrecision, type Precision } from "./engine.ts";

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
  return { wasm, patch, marker, precision: known };
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
 * (`ready`) or why not, then, after `ready`, at most one failure while rendering.
 */
export type EngineMessage =
  /** The patch is loaded: the processor renders it. */
  | { readonly type: "ready" }
  /** The engine rejected the patch; `detail` is its UTF-8 explanation. */
  | { readonly type: "rejected"; readonly detail: Uint8Array }
  /** Anything else went wrong (invalid options, a trap): the processor is silent. */
  | { readonly type: "failed"; readonly message: string };

/** Checks a message posted by the engine processor. */
export function parseEngineMessage(value: unknown): EngineMessage {
  if (typeof value !== "object" || value === null) {
    throw new MessageError("engine message is not an object");
  }
  const type: unknown = Reflect.get(value, "type");
  if (type === "ready") {
    return { type };
  }
  if (type === "rejected") {
    const detail: unknown = Reflect.get(value, "detail");
    if (!(detail instanceof Uint8Array)) {
      throw new MessageError("rejection detail is not a Uint8Array");
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
  throw new MessageError("unknown engine message");
}
