/**
 * The engine WebAssembly module, driven from JavaScript: in an `AudioWorklet`, on the
 * main thread or in Node. See `bindings/wasm/src/ffi.rs` for the exported interface.
 */

/** A function exported by the module, without arguments. */
type Exported0 = () => number;

/** A function exported by the module, with one argument. */
type Exported1 = (value: number) => number;

/** The exports of the engine module. */
interface Exports {
  readonly memory: WebAssembly.Memory;
  readonly input: Exported1;
  readonly load: Exported1;
  readonly messagePtr: Exported0;
  readonly messageLen: Exported0;
  readonly channels: Exported0;
  readonly blockSize: Exported0;
  readonly process: Exported0;
}

/** The version of the exported interface this code drives (`ABI_VERSION` in `ffi.rs`). */
export const ABI_VERSION = 1;

/** Thrown when the module does not have the expected interface. */
export class ModuleError extends Error {
  /**
   * Describes a module that is not an engine module.
   * @param detail What is wrong with the module.
   * @param options The underlying error, if any.
   */
  constructor(detail: string, options?: ErrorOptions) {
    super(`not an engine module: ${detail}`, options);
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

/**
 * Thrown when the engine module traps (a bug, or memory exhausted). The instance is then
 * in an unknown state: the engine refuses to render again, and the patch must be loaded
 * into a new engine.
 */
export class EngineTrap extends Error {
  /**
   * Records the trap.
   * @param options The trap raised by the WebAssembly engine, if any.
   */
  constructor(options?: ErrorOptions) {
    super("the engine module trapped and cannot be used again", options);
    this.name = "EngineTrap";
  }
}

/** The arguments of a function called without any: one shared, frozen, empty list. */
// Stryker disable next-line ArrayDeclaration: the functions called with it take no argument, and WebAssembly ignores extra ones
const NO_ARGUMENTS: readonly number[] = Object.freeze([]);

/**
 * Reads the result of an exported function. Every export returns an unsigned 32-bit
 * value (addresses included), which JavaScript receives as a signed `i32`: values from
 * 2^31 on come back negative and are read back as unsigned.
 */
function unsigned(name: string, result: unknown): number {
  if (typeof result !== "number") {
    throw new ModuleError(`${name} did not return a number`);
  }
  return result >>> 0;
}

/** Reads one exported function taking `arity` arguments: a caller, given the arguments. */
function exportedFunction(exports: WebAssembly.Exports, name: string, arity: number): (args: readonly number[]) => number {
  const value = exports[name];
  if (typeof value !== "function") {
    throw new ModuleError(`no function ${name}`);
  }
  if (value.length !== arity) {
    throw new ModuleError(`${name} takes ${String(value.length)} argument(s), expected ${String(arity)}`);
  }
  return (args) => unsigned(name, Reflect.apply(value, undefined, args));
}

/** Reads one exported function without arguments. Calling it does not allocate. */
function exported0(exports: WebAssembly.Exports, name: string): Exported0 {
  const call = exportedFunction(exports, name, 0);
  return () => call(NO_ARGUMENTS);
}

/** Reads one exported function with one argument. */
function exported1(exports: WebAssembly.Exports, name: string): Exported1 {
  const call = exportedFunction(exports, name, 1);
  return (argument) => call([argument]);
}

/** Reads and checks the exports of an engine module instance, version first. */
function readExports(exports: WebAssembly.Exports): Exports {
  const memory = exports["memory"];
  if (!(memory instanceof WebAssembly.Memory)) {
    throw new ModuleError("no memory");
  }
  const version = exported0(exports, "engine_abi_version")();
  if (version !== ABI_VERSION) {
    throw new ModuleError(`interface version ${String(version)}, expected ${String(ABI_VERSION)}`);
  }
  return {
    memory,
    input: exported1(exports, "engine_input"),
    load: exported1(exports, "engine_load"),
    messagePtr: exported0(exports, "engine_message_ptr"),
    messageLen: exported0(exports, "engine_message_len"),
    channels: exported0(exports, "engine_channels"),
    blockSize: exported0(exports, "engine_block_size"),
    process: exported0(exports, "engine_process"),
  };
}

/** Instantiates an engine module, explaining why it cannot be. */
function instantiate(module: WebAssembly.Module): WebAssembly.Exports {
  try {
    return new WebAssembly.Instance(module, {}).exports;
  } catch (error) {
    throw new ModuleError(`cannot be instantiated: ${String(error)}`, { cause: error });
  }
}

/** Runs `action` on the engine module, turning a trap into an {@link EngineTrap}. */
function guarded<T>(action: () => T): T {
  try {
    return action();
  } catch (error) {
    throw error instanceof WebAssembly.RuntimeError ? new EngineTrap({ cause: error }) : error;
  }
}

/**
 * One patch loaded into its own instance of the engine module.
 *
 * The engine renders fixed blocks; {@link WasmEngine.render} fills buffers of any length
 * from them, so the result never depends on how the host splits time. Rendering does
 * not allocate: it can run on an audio thread without causing garbage collection.
 */
export class WasmEngine {
  /** Number of output channels of the patch. */
  readonly channels: number;
  /** The module instance. */
  readonly #exports: Exports;
  /** Frames per block. */
  readonly #blockSize: number;
  /** A view of the module memory, renewed when the memory grows. */
  #memory: Float32Array;
  /** Index in {@link WasmEngine.#memory} of the current block, planar. */
  #base = 0;
  /** Next frame of the current block to output; `#blockSize` when it is used up. */
  #position: number;
  /** Number of blocks rendered so far. */
  #blocks = 0;
  /** Whether the module trapped: it is never called again. */
  #dead = false;

  /**
   * Wraps an instance with a patch loaded.
   * @param exports The instance, with a patch loaded.
   */
  private constructor(exports: Exports) {
    this.#exports = exports;
    this.channels = exports.channels();
    this.#blockSize = exports.blockSize();
    this.#memory = new Float32Array(exports.memory.buffer);
    this.#position = this.#blockSize;
  }

  /**
   * Instantiates `module` and loads `patch` (UTF-8 text in the patch format) at `rate` hertz.
   * @throws {RangeError} when `rate` is not a whole number of hertz that fits 32 bits.
   * @throws {ModuleError} when `module` is not an engine module of this version.
   * @throws {PatchError} when the engine rejects the patch.
   * @throws {EngineTrap} when the module traps.
   */
  static create(module: WebAssembly.Module, patch: Uint8Array, rate: number): WasmEngine {
    if (!Number.isInteger(rate) || rate < 0 || rate > 0xff_ff_ff_ff) {
      throw new RangeError(`the sample rate must be a whole number of hertz, not ${String(rate)}`);
    }
    const exports = readExports(instantiate(module));
    return guarded(() => {
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
    });
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
   * the arrays are dropped. Does not allocate.
   * @throws {RangeError} when the arrays differ in length (nothing is rendered).
   * @throws {EngineTrap} when the module traps, or trapped before.
   */
  render(outputs: readonly Float32Array[]): void {
    if (this.#dead) {
      throw new EngineTrap();
    }
    const frames = outputs[0]?.length ?? 0;
    for (const output of outputs) {
      if (output.length !== frames) {
        throw new RangeError(`every output must have the same length: ${String(output.length)} instead of ${String(frames)}`);
      }
    }
    let written = 0;
    while (written < frames) {
      if (this.#position === this.#blockSize) {
        this.#next();
      }
      const count = Math.min(frames - written, this.#blockSize - this.#position);
      let channel = 0;
      for (const output of outputs) {
        if (channel >= this.channels) {
          output.fill(0, written, written + count);
        } else {
          const start = this.#base + channel * this.#blockSize + this.#position;
          // Stryker disable next-line EqualityOperator: one frame more is overwritten by the next pass, or past the end of the output, where writes are ignored
          for (let frame = 0; frame < count; frame += 1) {
            output[written + frame] = this.#memory[start + frame] ?? 0;
          }
        }
        channel += 1;
      }
      written += count;
      this.#position += count;
    }
  }

  /** Renders the next block, in the module memory. */
  #next(): void {
    let pointer: number;
    try {
      pointer = this.#exports.process();
    } catch (error) {
      this.#dead = true;
      throw error instanceof WebAssembly.RuntimeError ? new EngineTrap({ cause: error }) : error;
    }
    // Stryker disable next-line ConditionalExpression: renewing the view at every block gives the same samples but allocates, which test:allocation (run without instrumentation, so outside Stryker) rejects
    if (this.#memory.buffer !== this.#exports.memory.buffer) {
      this.#memory = new Float32Array(this.#exports.memory.buffer);
    }
    this.#base = pointer / Float32Array.BYTES_PER_ELEMENT;
    this.#position = 0;
    this.#blocks += 1;
  }
}
