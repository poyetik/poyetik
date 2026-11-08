/** What the main thread and the `AudioWorklet` processors agree on. */

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
 * Thrown when a message from a processor (an engine message, a tap block) does not have
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
  return { wasm, patch, marker };
}

/** Checks a render quantum posted by the tap processor: one array per channel. */
export function parseTapBlock(value: unknown): Float32Array[] {
  if (!Array.isArray(value)) {
    throw new MessageError("tap block is not an array");
  }
  return value.map((channel: unknown) => {
    if (!(channel instanceof Float32Array)) {
      throw new MessageError("tap channel is not a Float32Array");
    }
    return channel;
  });
}

/** What the engine processor tells the main thread, once. */
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
