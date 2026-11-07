/**
 * The engine WebAssembly module, driven from JavaScript: in an `AudioWorklet`, on the
 * main thread or in Node. See `bindings/wasm/src/ffi.rs` for the exported interface.
 */

/** A function exported by the module: numbers in, a number out. */
type Exported = (...args: number[]) => number;

/** The exports of the engine module. */
interface Exports {
  readonly memory: WebAssembly.Memory;
  readonly input: Exported;
  readonly load: Exported;
  readonly messagePtr: Exported;
  readonly messageLen: Exported;
  readonly channels: Exported;
  readonly blockSize: Exported;
  readonly process: Exported;
}

/** Thrown when the module does not have the expected interface. */
export class ModuleError extends Error {
  /**
   * Describes a module that is not an engine module.
   * @param detail What is wrong with the module.
   */
  constructor(detail: string) {
    super(`not an engine module: ${detail}`);
    this.name = "ModuleError";
  }
}

/**
 * Thrown when the engine rejects a patch. `detail` is the UTF-8 explanation: decode it
 * where a `TextDecoder` is available (an `AudioWorklet` may not have one).
 */
export class PatchError extends Error {
  /** The UTF-8 explanation given by the engine. */
  readonly detail: Uint8Array;

  /**
   * Records the explanation of the engine.
   * @param detail The UTF-8 explanation given by the engine.
   */
  constructor(detail: Uint8Array) {
    super("the engine rejected the patch");
    this.name = "PatchError";
    this.detail = detail;
  }
}

/** Reads one exported function, checking that it returns numbers. */
function exported(exports: WebAssembly.Exports, name: string): Exported {
  const value = exports[name];
  if (typeof value !== "function") {
    throw new ModuleError(`no function ${name}`);
  }
  return (...args) => {
    const result: unknown = Reflect.apply(value, undefined, args);
    if (typeof result !== "number") {
      throw new ModuleError(`${name} did not return a number`);
    }
    return result;
  };
}

/** Reads and checks the exports of an engine module instance. */
function readExports(exports: WebAssembly.Exports): Exports {
  const memory = exports["memory"];
  if (!(memory instanceof WebAssembly.Memory)) {
    throw new ModuleError("no memory");
  }
  return {
    memory,
    input: exported(exports, "engine_input"),
    load: exported(exports, "engine_load"),
    messagePtr: exported(exports, "engine_message_ptr"),
    messageLen: exported(exports, "engine_message_len"),
    channels: exported(exports, "engine_channels"),
    blockSize: exported(exports, "engine_block_size"),
    process: exported(exports, "engine_process"),
  };
}

/**
 * One patch loaded into its own instance of the engine module.
 *
 * The engine renders fixed blocks; {@link WasmEngine.render} fills buffers of any length
 * from them, so the result never depends on how the host splits time.
 */
export class WasmEngine {
  /** Number of output channels of the patch. */
  readonly channels: number;
  /** The module instance. */
  readonly #exports: Exports;
  /** Frames per block. */
  readonly #blockSize: number;
  /** A copy of the current block, planar: channel after channel. */
  readonly #block: Float32Array;
  /** Next frame of the current block to output; `#blockSize` when it is used up. */
  #position: number;
  /** Number of blocks rendered so far. */
  #blocks = 0;

  /**
   * Wraps an instance with a patch loaded.
   * @param exports The instance, with a patch loaded.
   */
  private constructor(exports: Exports) {
    this.#exports = exports;
    this.channels = exports.channels();
    this.#blockSize = exports.blockSize();
    this.#block = new Float32Array(this.channels * this.#blockSize);
    this.#position = this.#blockSize;
  }

  /**
   * Instantiates `module` and loads `patch` (UTF-8 text in the patch format) at `rate` hertz.
   * @throws {ModuleError} when `module` is not an engine module.
   * @throws {PatchError} when the engine rejects the patch.
   */
  static create(module: WebAssembly.Module, patch: Uint8Array, rate: number): WasmEngine {
    const exports = readExports(new WebAssembly.Instance(module, {}).exports);
    // Every call may grow the memory, which detaches its previous buffer: call first,
    // then read `memory.buffer`.
    const input = exports.input(patch.length);
    new Uint8Array(exports.memory.buffer, input, patch.length).set(patch);
    if (exports.load(rate) !== 1) {
      const pointer = exports.messagePtr();
      const length = exports.messageLen();
      throw new PatchError(new Uint8Array(exports.memory.buffer, pointer, length).slice());
    }
    return new WasmEngine(exports);
  }

  /**
   * Number of blocks rendered so far. The engine never renders ahead: a block is rendered
   * only when its first frame is output, so that it never depends on how the host splits
   * time.
   */
  get blocks(): number {
    return this.#blocks;
  }

  /**
   * Renders the next frames into `outputs`, one array per channel, all of the same
   * length. Arrays beyond the patch channels receive silence; patch channels beyond
   * the arrays are dropped.
   */
  render(outputs: readonly Float32Array[]): void {
    const frames = outputs[0]?.length ?? 0;
    let written = 0;
    // Exactly one pass for what is left of the current block, then one per new block:
    // the loop is bounded by construction, never infinite.
    const head = Math.min(frames, this.#blockSize - this.#position);
    const passes = (head > 0 ? 1 : 0) + Math.ceil((frames - head) / this.#blockSize);
    for (let pass = 0; pass < passes; pass += 1) {
      if (this.#position === this.#blockSize) {
        this.#next();
      }
      const count = Math.min(frames - written, this.#blockSize - this.#position);
      outputs.forEach((output, channel) => {
        if (channel < this.channels) {
          const start = channel * this.#blockSize + this.#position;
          output.set(this.#block.subarray(start, start + count), written);
        } else {
          output.fill(0, written, written + count);
        }
      });
      written += count;
      this.#position += count;
    }
  }

  /** Renders the next block into `#block`. */
  #next(): void {
    const pointer = this.#exports.process();
    this.#block.set(new Float32Array(this.#exports.memory.buffer, pointer, this.#block.length));
    this.#position = 0;
    this.#blocks += 1;
  }
}
