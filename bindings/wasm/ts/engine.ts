/**
 * The engine WebAssembly module, driven from JavaScript: in an `AudioWorklet`, on the
 * main thread or in Node (the package exports no Node entry point yet: it comes with the
 * SDK, roadmap #32; the tests import this module directly). See `bindings/wasm/src/ffi.rs` for the exported interface.
 */

/** A function exported by the module, without arguments. */
type Exported0 = () => number;

/** A function exported by the module, with one argument. */
type Exported1 = (value: number) => number;

/** The exports of the engine module. */
interface Exports {
  /** The module memory: every address the functions return points into it, and it may grow at any call. */
  readonly memory: WebAssembly.Memory;
  /** `engine_input(len)`: makes the input buffer `len` bytes long and returns its address. */
  readonly input: Exported1;
  /** `engine_load(rate)`: loads the patch in the input buffer at `rate` hertz; 1 on success, 0 on failure. */
  readonly load: Exported1;
  /** `engine_message_ptr()`: the address of the UTF-8 explanation of the last failed load. */
  readonly messagePtr: Exported0;
  /** `engine_message_len()`: the length in bytes of that explanation; 0 after a success. */
  readonly messageLen: Exported0;
  /** `engine_channels()`: the number of output channels of the loaded patch. */
  readonly channels: Exported0;
  /** `engine_block_size()`: the number of frames of every block the engine renders. */
  readonly blockSize: Exported0;
  /** `engine_process()`: renders the next block and returns the address of its planar samples. */
  readonly process: Exported0;
  /** `engine_max_patch_bytes()`: the longest patch the engine reads, in bytes; a longer one is refused before it is copied in. */
  readonly maxPatchBytes: Exported0;
}

/**
 * The identity and version of the exported interface this code drives (`ABI_VERSION` in
 * `ffi.rs`): "PY" in the high 16 bits, version 2 in the low 16 bits.
 */
export const ABI_VERSION = 0x5059_0002;

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
 * Thrown when the engine refuses to load a patch: the patch is invalid or cannot be
 * planned, but also when it is not UTF-8 or when the engine does not support the sample
 * rate (separate errors are planned with the public API, roadmap #32). `detail` is the
 * UTF-8 explanation: decode it where a `TextDecoder` is available (an `AudioWorklet` may
 * not have one).
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
    throw new ModuleError(`interface 0x${version.toString(16)}, expected 0x${ABI_VERSION.toString(16)}`);
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
    maxPatchBytes: exported0(exports, "engine_max_patch_bytes"),
  };
}

/**
 * Encodes `text`, which is ASCII, as the UTF-8 bytes of a {@link PatchError}: an
 * `AudioWorklet` may have no `TextEncoder`.
 */
function ascii(text: string): Uint8Array {
  return Uint8Array.from(text, (character) => character.charCodeAt(0));
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
 * not allocate when the buffers are whole blocks (the 128-frame quanta of Web Audio
 * included): it can run on an audio thread without causing garbage collection, before
 * and after the JavaScript engine optimizes it. Buffers of other lengths are copied
 * sample by sample, which may allocate until the code is optimized.
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
  /** The module memory read sample by sample (little-endian, as WebAssembly), renewed with {@link WasmEngine.#memory}. */
  #data: DataView;
  /**
   * Index in {@link WasmEngine.#memory} of the current block, planar; -1 before the first,
   * so that it builds the views. Always a small integer: a field once holding a fraction
   * (`NaN`) stores numbers boxed, and reading it allocates until the code is optimized.
   */
  #base = -1;
  /**
   * One view per patch channel over the current block, renewed only when the block moves
   * or the memory grows: copying a whole block through them allocates nothing.
   */
  readonly #views: Float32Array[] = [];
  /** Next frame of the current block to output; `#blockSize` when it is used up. */
  #position: number;
  /** Number of blocks rendered so far. */
  #blocks = 0;
  /** Whether the module failed (a trap, a wrong result): it is never called again. */
  #dead = false;
  /** Length of every output of the current render, in frames. */
  #frames = 0;
  /** Frames of the current render already written. */
  #written = 0;
  /** Frames of the current chunk: copied from the current block, from {@link WasmEngine.#position}. */
  #count = 0;

  /**
   * Checks the length of one output of the current render. A method made once, for
   * `forEach`: an indexed loop would read the outputs as possibly missing, and an
   * iterator allocates until the code is optimized.
   * @throws {RangeError} when it differs from the length of the first output.
   */
  readonly #checkLength = (output: Float32Array): void => {
    if (output.length !== this.#frames) {
      throw new RangeError(`every output must have the same length: ${String(output.length)} instead of ${String(this.#frames)}`);
    }
  };

  /**
   * Copies the current chunk into one output (see {@link WasmEngine.#checkLength} for
   * why it is made once): silence beyond the patch channels; a whole block in one copy,
   * which reads no sample as a number and so allocates nothing; a part of a block sample
   * by sample.
   */
  readonly #copyChunk = (output: Float32Array, channel: number): void => {
    const view = this.#views[channel];
    if (view === undefined) {
      output.fill(0, this.#written, this.#written + this.#count);
    } else if (this.#count === this.#blockSize) {
      output.set(view, this.#written);
    } else {
      const offset = view.byteOffset + this.#position * Float32Array.BYTES_PER_ELEMENT;
      for (let frame = 0; frame !== this.#count; frame += 1) {
        const at = this.#written + frame;
        output.fill(this.#data.getFloat32(offset + frame * Float32Array.BYTES_PER_ELEMENT, true), at, at + 1);
      }
    }
  };

  /**
   * Wraps an instance with a patch loaded.
   * @param exports The instance, with a patch loaded.
   * @throws {ModuleError} when its blocks have no frame: rendering would never end.
   */
  private constructor(exports: Exports) {
    this.#exports = exports;
    this.channels = exports.channels();
    this.#blockSize = exports.blockSize();
    if (this.#blockSize === 0) {
      throw new ModuleError("engine_block_size returned 0: a block has at least one frame");
    }
    this.#memory = new Float32Array(exports.memory.buffer);
    this.#data = new DataView(exports.memory.buffer);
    this.#position = this.#blockSize;
  }

  /**
   * Instantiates `module` and loads `patch` (UTF-8 text in the patch format) at `rate` hertz.
   * @throws {RangeError} when `rate` is not a whole number of hertz that fits 32 bits.
   * @throws {ModuleError} when `module` is not an engine module of this version, or
   * gives blocks of 0 frames.
   * @throws {PatchError} when the engine rejects the patch, its bytes or the rate; a patch
   * longer than the engine reads is refused before it is copied into the module, whose
   * memory never shrinks again, with the words of the engine.
   * @throws {EngineTrap} when the module traps.
   */
  static create(module: WebAssembly.Module, patch: Uint8Array, rate: number): WasmEngine {
    if (!Number.isInteger(rate) || rate < 0 || rate > 0xff_ff_ff_ff) {
      throw new RangeError(`the sample rate must be a whole number of hertz, not ${String(rate)}`);
    }
    const exports = readExports(instantiate(module));
    return guarded(() => {
      const max = exports.maxPatchBytes();
      if (patch.length > max) {
        throw new PatchError(ascii(`patch too large: ${String(patch.length)} bytes (at most ${String(max)})`));
      }
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
   * the arrays are dropped. Allocates nothing when every chunk is a whole block (see
   * {@link WasmEngine}).
   * @throws {RangeError} when the arrays differ in length (nothing is rendered).
   * @throws {ModuleError} when the module places a block outside its memory; the engine
   * is then dead.
   * @throws {EngineTrap} when the module traps, or failed before.
   */
  render(outputs: readonly Float32Array[]): void {
    if (this.#dead) {
      throw new EngineTrap();
    }
    this.#frames = outputs[0]?.length ?? 0;
    outputs.forEach(this.#checkLength);
    this.#written = 0;
    while (this.#written !== this.#frames) {
      if (this.#position === this.#blockSize) {
        this.#next();
      }
      this.#count = Math.min(this.#frames - this.#written, this.#blockSize - this.#position);
      outputs.forEach(this.#copyChunk);
      this.#written += this.#count;
      this.#position += this.#count;
    }
  }

  /**
   * Renders the next block, in the module memory, and checks where the module put it: a
   * block the views cannot cover entirely is a module error, never read as silence.
   */
  #next(): void {
    let pointer: number;
    try {
      pointer = this.#exports.process();
    } catch (error) {
      this.#dead = true;
      throw error instanceof WebAssembly.RuntimeError ? new EngineTrap({ cause: error }) : error;
    }
    const buffer = this.#exports.memory.buffer;
    const base = pointer / Float32Array.BYTES_PER_ELEMENT;
    if (!Number.isInteger(base) || base + this.channels * this.#blockSize > buffer.byteLength / Float32Array.BYTES_PER_ELEMENT) {
      this.#dead = true;
      throw new ModuleError(`engine_process returned the block address ${String(pointer)}, unaligned or outside the memory`);
    }
    if (this.#memory.buffer !== buffer || base !== this.#base) {
      if (this.#memory.buffer !== buffer) {
        this.#memory = new Float32Array(buffer);
        this.#data = new DataView(buffer);
      }
      for (let channel = 0; channel !== this.channels; channel += 1) {
        const start = base + channel * this.#blockSize;
        this.#views[channel] = this.#memory.subarray(start, start + this.#blockSize);
      }
      this.#base = base;
    }
    this.#position = 0;
    this.#blocks += 1;
  }
}
