/**
 * The engine WebAssembly module, driven from JavaScript: in an `AudioWorklet`, on the
 * main thread or in Node (the package exports no Node entry point yet: it comes with the
 * SDK, #32 step 15; the tests import this module directly). See `bindings/wasm/src/ffi.rs` for the exported interface.
 */
import { ModuleError, readExports, type Exports } from "./abi.ts";

export { ABI_VERSION, ModuleError } from "./abi.ts";

/**
 * Every render format, in the order of the engine's codes (`engine_core::RenderFormat::ALL`):
 * computed and given in binary64; computed in binary64 and given in binary32; computed and
 * given in binary32.
 */
export const RENDER_FORMATS = ["binary64", "binary64-as-binary32", "binary32"] as const;

/** What a render is: the precision that computes it and the format of its samples. */
export type RenderFormat = (typeof RENDER_FORMATS)[number];

/** The render formats whose samples are binary32: what Web Audio takes. */
export type Binary32Format = Exclude<RenderFormat, "binary64">;

/** Every precision a render computes in, the default first: a host setting. */
export const PRECISIONS = ["binary64", "binary32"] as const;

/** The precision a render computes in: binary64 by default, binary32 for performance. */
export type Precision = (typeof PRECISIONS)[number];

/** Whether subnormals survive in one format: as the operands of an operation, and as its result. */
export interface SubnormalsKept {
  /** A subnormal operand is used as it is, not as zero (no denormals-are-zero). */
  readonly operands: boolean;
  /** A subnormal result is given as it is, not as zero (no flush-to-zero). */
  readonly results: boolean;
}

/**
 * What the arithmetic of a thread does with subnormal numbers, in each format, as the
 * module observed it (`engine_core::SubnormalArithmetic`). Every subnormal kept is IEEE
 * arithmetic, the engine's own; a browser may run its audio thread in a hardware flush
 * mode, which this shows when it reaches the module's arithmetic.
 */
export interface SubnormalArithmetic {
  /** In binary32. */
  readonly binary32: SubnormalsKept;
  /** In binary64. */
  readonly binary64: SubnormalsKept;
}

/** The largest code of `engine_subnormals`: its four bits set, IEEE arithmetic. */
const SUBNORMAL_CODES = 0b1111;

/**
 * Reads the code of `engine_subnormals` (`subnormals_code` in `ffi.rs`): four bits, each set
 * when subnormals are kept, 1 binary32 operands, 2 binary32 results, 4 binary64 operands,
 * 8 binary64 results; `undefined` for anything else.
 */
export function subnormalArithmetic(code: number): SubnormalArithmetic | undefined {
  if (!Number.isInteger(code) || code < 0 || code > SUBNORMAL_CODES) {
    return undefined;
  }
  const kept = (shift: number): SubnormalsKept => ({ operands: ((code >> shift) & 1) === 1, results: ((code >> (shift + 1)) & 1) === 1 });
  return { binary32: kept(0), binary64: kept(2) };
}

/**
 * The render format of a render computed in `precision` and given to a host that takes
 * binary32 (Web Audio): a binary64 render is rounded to binary32, a binary32 one given as it
 * is.
 */
export function givenAsBinary32(precision: Precision): Binary32Format {
  return GIVEN_AS_BINARY32[precision];
}

/** The render format of each precision given as binary32: a record, so that a new precision does not compile without one. */
const GIVEN_AS_BINARY32: Readonly<Record<Precision, Binary32Format>> = { binary64: "binary64-as-binary32", binary32: "binary32" };

/**
 * The precision `value` names, read at a boundary (a message, a file): `undefined` when it
 * names none. The one reader of a precision from outside the type system; each caller
 * refuses `undefined` with its own error.
 */
export function parsePrecision(value: unknown): Precision | undefined {
  return PRECISIONS.find((name) => name === value);
}

/** The sample arrays a render format fills: `Float64Array` for binary64, `Float32Array` otherwise. */
export type Samples = Float32Array | Float64Array;

/** How the samples of a render format are laid out in the module memory. */
interface Layout {
  /** Bytes per sample: 8 or 4. */
  readonly bytes: number;
  /** A view of the whole memory as samples. */
  readonly view: (buffer: ArrayBuffer) => Samples;
}

/** The layout of binary64 samples. */
const BINARY64: Layout = {
  bytes: Float64Array.BYTES_PER_ELEMENT,
  view: (buffer) => new Float64Array(buffer),
};

/** The layout of binary32 samples. */
const BINARY32: Layout = {
  bytes: Float32Array.BYTES_PER_ELEMENT,
  view: (buffer) => new Float32Array(buffer),
};

/**
 * Thrown when the engine refuses to load a patch: the patch is invalid or cannot be
 * planned, but also when it is not UTF-8 or when the engine does not support the sample
 * rate (separate errors are planned with the public API, #32 step 12). `detail` is the
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
 * One patch loaded into its own instance of the engine module, rendering in one render
 * format into arrays of type `A`: `Float64Array` for binary64, `Float32Array` for the two
 * binary32 formats.
 *
 * The engine renders fixed blocks; {@link WasmEngine.render} fills buffers of any length
 * from them, so the result never depends on how the host splits time. Rendering does
 * not allocate when the buffers are whole blocks (the 128-frame quanta of Web Audio
 * included): it can run on an audio thread without causing garbage collection, before
 * and after the JavaScript engine optimizes it. A part of a block (a buffer of another
 * length) is copied through a view of that part, which allocates the view.
 */
export class WasmEngine<A extends Samples> {
  /** Number of output channels of the patch. */
  readonly channels: number;
  /**
   * What the arithmetic of the thread that created the engine does with subnormals, as the
   * module observed it then (`engine_subnormals`): on an audio thread, whether the
   * browser's flush mode reaches the engine.
   */
  readonly subnormals: SubnormalArithmetic;
  /** The module instance. */
  readonly #exports: Exports;
  /** Frames per block. */
  readonly #blockSize: number;
  /** How its samples are laid out in the module memory. */
  readonly #layout: Layout;
  /** A view of the module memory as samples, renewed when the memory grows. */
  #memory: Samples;
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
  readonly #views: Samples[] = [];
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
  readonly #checkLength = (output: A): void => {
    if (output.length !== this.#frames) {
      throw new RangeError(`every output must have the same length: ${String(output.length)} instead of ${String(this.#frames)}`);
    }
  };

  /**
   * Copies the current chunk into one output (see {@link WasmEngine.#checkLength} for
   * why it is made once): silence beyond the patch channels; a whole block in one copy,
   * which reads no sample as a number and so allocates nothing; a part of a block in one
   * copy through a view of that part, bit for bit too.
   */
  readonly #copyChunk = (output: A, channel: number): void => {
    const view = this.#views[channel];
    if (view === undefined) {
      output.fill(0, this.#written, this.#written + this.#count);
    } else if (this.#count === this.#blockSize) {
      output.set(view, this.#written);
    } else {
      // A part of the block, through a view of that part: copied bit for bit, as `set`
      // copies between arrays of one type (a NaN payload included), where reading samples
      // as numbers could change a NaN.
      output.set(view.subarray(this.#position, this.#position + this.#count), this.#written);
    }
  };

  /**
   * Wraps an instance with a patch loaded.
   * @param exports The instance, with a patch loaded.
   * @param layout How its samples are laid out.
   * @throws {ModuleError} when its blocks have no frame: rendering would never end; or when
   * its observation of subnormals is not a code of `engine_subnormals`.
   */
  private constructor(exports: Exports, layout: Layout) {
    this.#exports = exports;
    this.#layout = layout;
    this.channels = exports.channels();
    const code = exports.subnormals();
    const subnormals = subnormalArithmetic(code);
    if (subnormals === undefined) {
      throw new ModuleError(`engine_subnormals returned ${String(code)}, not a code from 0 to ${String(SUBNORMAL_CODES)}`);
    }
    this.subnormals = subnormals;
    this.#blockSize = exports.blockSize();
    if (this.#blockSize === 0) {
      throw new ModuleError("engine_block_size returned 0: a block has at least one frame");
    }
    this.#memory = layout.view(exports.memory.buffer);
    this.#position = this.#blockSize;
  }

  /**
   * Instantiates `module` and loads `patch` (UTF-8 text in the patch format) at `rate`
   * hertz, rendering in `format`: binary64 renders into `Float64Array`s, the two binary32
   * formats into `Float32Array`s.
   * @throws {RangeError} when `rate` is not a whole number of hertz that fits 32 bits.
   * @throws {ModuleError} when `module` is not an engine module of this version, gives
   * blocks of 0 frames, or observes subnormals in a code that is not one.
   * @throws {PatchError} when the engine rejects the patch, its bytes or the rate; a patch
   * longer than the engine reads is refused before it is copied into the module, whose
   * memory never shrinks again, with the words of the engine.
   * @throws {EngineTrap} when the module traps.
   */
  static create(module: WebAssembly.Module, patch: Uint8Array, rate: number, format: "binary64"): WasmEngine<Float64Array>;
  static create(module: WebAssembly.Module, patch: Uint8Array, rate: number, format: Binary32Format): WasmEngine<Float32Array>;
  static create(module: WebAssembly.Module, patch: Uint8Array, rate: number, format: RenderFormat): WasmEngine<Float64Array> | WasmEngine<Float32Array> {
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
      if (exports.load(rate, RENDER_FORMATS.indexOf(format)) !== 1) {
        const pointer = exports.messagePtr();
        const length = exports.messageLen();
        throw new PatchError(new Uint8Array(exports.memory.buffer, pointer, length).slice());
      }
      return format === "binary64" ? new WasmEngine<Float64Array>(exports, BINARY64) : new WasmEngine<Float32Array>(exports, BINARY32);
    });
  }

  /** Number of frames of every block the module renders (`engine_block_size`), read once when the engine is created. */
  get blockSize(): number {
    return this.#blockSize;
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
  render(outputs: readonly A[]): void {
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
    const bytes = this.#layout.bytes;
    const base = pointer / bytes;
    if (!Number.isInteger(base) || base + this.channels * this.#blockSize > buffer.byteLength / bytes) {
      this.#dead = true;
      throw new ModuleError(`engine_process returned the block address ${String(pointer)}, unaligned or outside the memory`);
    }
    if (this.#memory.buffer !== buffer || base !== this.#base) {
      if (this.#memory.buffer !== buffer) {
        this.#memory = this.#layout.view(buffer);
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
