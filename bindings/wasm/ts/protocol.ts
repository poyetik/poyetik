/** What the main thread and the `AudioWorklet` processors agree on. */

/** Name of the processor running the engine. */
export const ENGINE_PROCESSOR = "poyetik-engine";

/** Name of the processor posting its input to the main thread, one block at a time. */
export const TAP_PROCESSOR = "poyetik-tap";

/** Options of the engine processor (`processorOptions`). */
export interface EngineOptions {
  /** The engine WebAssembly module, as bytes: compiled again in the worklet. */
  readonly wasm: ArrayBuffer;
  /** The patch, as UTF-8 text in the patch format. */
  readonly patch: Uint8Array;
}

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
  return { wasm, patch };
}

/** Checks a block posted by the tap processor: one array per channel. */
export function parseTapBlock(value: unknown): Float32Array[] {
  if (!Array.isArray(value)) {
    throw new OptionsError("tap block is not an array");
  }
  return value.map((channel: unknown) => {
    if (!(channel instanceof Float32Array)) {
      throw new OptionsError("tap channel is not a Float32Array");
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
    throw new OptionsError("engine message is not an object");
  }
  const type: unknown = Reflect.get(value, "type");
  if (type === "ready") {
    return { type };
  }
  if (type === "rejected") {
    const detail: unknown = Reflect.get(value, "detail");
    if (!(detail instanceof Uint8Array)) {
      throw new OptionsError("rejection detail is not a Uint8Array");
    }
    return { type, detail };
  }
  if (type === "failed") {
    const message: unknown = Reflect.get(value, "message");
    if (typeof message !== "string") {
      throw new OptionsError("failure message is not a string");
    }
    return { type, message };
  }
  throw new OptionsError("unknown engine message");
}
