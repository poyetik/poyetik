/**
 * The driver of the visual library's C interface (`bindings/visual-wasm/src/ffi.rs`) in an
 * instance of any module that exports it: the visual library's own, whose `VisualModule`
 * (`visual.ts`) creates the instance it drives, or a product module linking the library
 * (`@poyetik/web-page`), whose own wrapper does the same. The interface is checked once at
 * the boundary, then used through typed methods. Every address the module answers is
 * checked against its memory before it is read, and what is read is copied before the next
 * call that sets up, renders or hashes, which may move it; only an image for display is
 * handed over in place, valid until the next call.
 */

/** The version of the module's interface this code drives: "PV" and version 4. */
export const ABI_VERSION = 0x5056_0004;

/** The library's WGSL shaders, by the kind the interface numbers them with. */
export const SHADERS = { square: 0, display: 1 } as const;

/** A shader of the library: the square's, or the display encoding's. */
export type ShaderKind = keyof typeof SHADERS;

/** Where the square is in an image: its left, right, top and bottom edges, in subpixel units (4096 a pixel side). */
export type Placement = readonly [number, number, number, number];

/** The display shader's thresholds: 255 sRGB, then 255 alpha. */
export const THRESHOLDS = 510;

/** The largest index of an image, 2⁶⁴ − 1. */
export const MAX_INDEX = (1n << 64n) - 1n;

/** The largest count of images hashed at once: the interface takes a 32-bit count. */
export const MAX_COUNT = 0xffff_ffff;

/** Thrown when a module is not the visual module this code drives, or is used outside its contract. */
export class VisualModuleError extends Error {
  /**
   * Says what is wrong.
   * @param detail What is wrong.
   * @param options The error of the WebAssembly runtime it comes from, if any.
   */
  constructor(detail: string, options?: ErrorOptions) {
    super(`visual module: ${detail}`, options);
    this.name = "VisualModuleError";
  }
}

/** The square to set up: images of a size at a rate, and the square's side and cycle. */
export interface SquareSetup {
  /** Width of the images, in pixels. */
  readonly width: number;
  /** Height of the images, in pixels. */
  readonly height: number;
  /** Images per second: numerator, then denominator. */
  readonly rate: readonly [number, number];
  /** The side of the square, in 65 536ths of the height. */
  readonly side: number;
  /** The seconds one sweep lasts: numerator, then denominator. */
  readonly cycle: readonly [number, number];
}

/** Thrown when the module refuses a square: its message is the module's explanation, which names the value refused (`width and height`, `rate`, `cycle`, `side`). */
export class VisualSetupError extends Error {
  /**
   * Gives the module's explanation.
   * @param message Why the module refused the square.
   */
  constructor(message: string) {
    super(message);
    this.name = "VisualSetupError";
  }
}

/**
 * Thrown when the module traps, or a call into it fails otherwise (a host stack overflow
 * included): the instance is then dead, and every later call throws it again. An image the
 * memory cannot hold is not a trap: the module refuses it ({@link VisualSetupError}).
 */
export class VisualTrap extends Error {
  /**
   * Records the trap.
   * @param options The trap raised by the WebAssembly module, if any.
   */
  constructor(options?: ErrorOptions) {
    super("the visual module trapped and cannot be used again", options);
    this.name = "VisualTrap";
  }
}

/**
 * What a call into the module threw, whatever it was: a trap, or the host failing the call
 * (a stack overflow: a `RangeError`, an `InternalError` in Firefox). Either way the module may
 * have stopped halfway, its state unknown, so the instance is never called again.
 */
class CallFailure extends Error {
  /**
   * Records what the call threw.
   * @param cause What the call threw.
   */
  constructor(cause: unknown) {
    // Never shown: #guarded turns it into a VisualTrap with the same cause.
    super(undefined, { cause });
  }
}

/** A function the module exports, taking the unsigned 32-bit numbers `A` and returning one. */
type Exported<A extends readonly number[]> = (...args: A) => number;

/** What a module instance exports, read at the boundary as unknown values. */
export type Exports = Readonly<Record<string, unknown>>;

/**
 * The memory of every instance a driver drives: one driver per instance, so that two, each
 * with its own square and its own death, never draw or hash each other's.
 */
const DRIVEN = new WeakSet<WebAssembly.Memory>();

/**
 * The export `name` of `exports` as a function of `arity` arguments returning an unsigned
 * 32-bit number, or throws. Every export returns an unsigned 32-bit value (addresses
 * included), which JavaScript receives as a signed `i32`: values from 2^31 on come back
 * negative and are read back as unsigned.
 */
function exported<A extends readonly number[]>(exports: Exports, name: string, arity: A["length"]): Exported<A> {
  const value: unknown = exports[name];
  if (typeof value !== "function") {
    throw new VisualModuleError(`no function ${name}`);
  }
  if (value.length !== arity) {
    throw new VisualModuleError(`${name} takes ${String(value.length)} argument(s), expected ${String(arity)}`);
  }
  return (...args) => {
    let result: unknown;
    try {
      result = Reflect.apply(value, undefined, args);
    } catch (error) {
      throw new CallFailure(error);
    }
    if (typeof result !== "number") {
      throw new VisualModuleError(`${name} did not return a number`);
    }
    return result >>> 0;
  };
}

/** `value` as an unsigned 32-bit integer, or throws naming `what`. */
function u32(value: number, what: string): number {
  if (!Number.isInteger(value) || value < 0 || value > 0xffff_ffff) {
    throw new VisualModuleError(`${what} is not an unsigned 32-bit integer: ${String(value)}`);
  }
  return value;
}

/** `index` as the two 32-bit halves the interface takes, low then high, or throws. */
function halves(index: bigint): [number, number] {
  if (index < 0n || index > MAX_INDEX) {
    throw new VisualModuleError(`image ${String(index)} is not an index from 0 to 2^64 - 1`);
  }
  return [Number(index & 0xffff_ffffn), Number(index >> 32n)];
}

/**
 * One instance of the visual module, always with a square set up: it is given one when it
 * is created, and a refused setup keeps the square it had. Its memory grows to hold the largest image
 * set up, whose values take up to 512 MiB and its bytes for display 128 MiB more (the memory a little more), and never shrinks again (WebAssembly memory cannot): only a fresh
 * instance gives it back.
 */
export class VisualDriver {
  /** The module memory, read after each call. */
  readonly #memory: WebAssembly.Memory;
  /** `visual_setup`. */
  readonly #setup: Exported<[number, number, number, number, number, number, number]>;
  /** `visual_message_ptr`. */
  readonly #messagePtr: Exported<[]>;
  /** `visual_message_len`. */
  readonly #messageLen: Exported<[]>;
  /** `visual_render`. */
  readonly #render: Exported<[number, number]>;
  /** `visual_hash`. */
  readonly #hash: Exported<[number, number, number]>;
  /** `visual_values_len`. */
  readonly #valuesLen: Exported<[]>;
  /** `visual_display`. */
  readonly #display: Exported<[number, number]>;
  /** `visual_display_len`. */
  readonly #displayLen: Exported<[]>;
  /** `visual_placement`. */
  readonly #placement: Exported<[number, number]>;
  /** `visual_shader_ptr`. */
  readonly #shaderPtr: Exported<[number]>;
  /** `visual_shader_len`. */
  readonly #shaderLen: Exported<[number]>;
  /** `visual_thresholds`. */
  readonly #thresholds: Exported<[]>;
  /** `visual_encode_values`. */
  readonly #encodeValues: Exported<[number]>;
  /** `visual_encode`. */
  readonly #encode: Exported<[]>;
  /** Whether the module trapped: its state is then unknown, and it is never called again. */
  #dead = false;
  /** The values of an image of the square set up, and its bytes for display: width × height × 4. */
  #values = 0;

  /**
   * The module behind `exports`, checked (a memory, every function of the interface with
   * its arity, and the interface version this code drives), with `square` set up.
   * @param exports The exports of an instance of the module.
   * @param square The square to set up.
   * @throws {VisualModuleError} Naming what is missing or which version it has, when the
   * instance already has its driver (protected, so that only a module that creates the
   * instance it drives builds one), when a value of `square` is not an unsigned 32-bit
   * integer, or when the module answers outside its contract.
   * @throws {VisualSetupError} With the module's explanation, when it refuses the square.
   * @throws {VisualTrap} When a call into the module fails: a trap, or the host's own
   * failure (a stack overflow).
   */
  protected constructor(exports: Exports, square: SquareSetup) {
    const memory: unknown = exports["memory"];
    if (!(memory instanceof WebAssembly.Memory)) {
      throw new VisualModuleError("no memory");
    }
    if (DRIVEN.has(memory)) {
      throw new VisualModuleError("this instance already has its driver: two drivers of one instance would draw and hash each other's square");
    }
    const abiVersion = exported<[]>(exports, "visual_abi_version", 0);
    const version = this.#guarded(abiVersion);
    if (version !== ABI_VERSION) {
      throw new VisualModuleError(`interface 0x${version.toString(16)}, expected 0x${ABI_VERSION.toString(16)}: build the module again from this source (pnpm build:wasm)`);
    }
    this.#memory = memory;
    this.#setup = exported(exports, "visual_setup", 7);
    this.#messagePtr = exported(exports, "visual_message_ptr", 0);
    this.#messageLen = exported(exports, "visual_message_len", 0);
    this.#render = exported(exports, "visual_render", 2);
    this.#hash = exported(exports, "visual_hash", 3);
    this.#valuesLen = exported(exports, "visual_values_len", 0);
    this.#display = exported(exports, "visual_display", 2);
    this.#displayLen = exported(exports, "visual_display_len", 0);
    this.#placement = exported(exports, "visual_placement", 2);
    this.#shaderPtr = exported(exports, "visual_shader_ptr", 1);
    this.#shaderLen = exported(exports, "visual_shader_len", 1);
    this.#thresholds = exported(exports, "visual_thresholds", 0);
    this.#encodeValues = exported(exports, "visual_encode_values", 1);
    this.#encode = exported(exports, "visual_encode", 0);
    this.setup(square);
    DRIVEN.add(memory);
  }

  /**
   * The `count` values of `size` bytes each that `name` answered at `address`, checked to
   * lie, aligned, in the module memory.
   * @throws {VisualModuleError} When they do not.
   */
  #span(name: string, address: number, count: number, size: number): readonly [number, number] {
    if (address % size !== 0 || address + count * size > this.#memory.buffer.byteLength) {
      throw new VisualModuleError(`${name} answered ${String(count)} values of ${String(size)} bytes at ${String(address)}, unaligned or outside the module memory`);
    }
    return [address, count];
  }

  /**
   * Runs `action` on the module, turning a call into it that failed in any way into a
   * {@link VisualTrap} that kills this instance; a refusal of what the module answered
   * (a {@link VisualModuleError}) leaves it alive.
   */
  #guarded<T>(action: () => T): T {
    if (this.#dead) {
      throw new VisualTrap();
    }
    try {
      return action();
    } catch (error) {
      if (error instanceof CallFailure) {
        this.#dead = true;
        throw new VisualTrap({ cause: error.cause });
      }
      throw error;
    }
  }

  /**
   * Runs `action`, a call into the module through an interface of a product module's own (a
   * module linking the visual library with more, as the page's), as this driver's calls
   * run: never once the instance has failed, and any failure of it a trap that kills the
   * instance.
   * @param action The call.
   * @returns What it returned.
   * @throws {VisualTrap} When the call fails, or one has.
   */
  protected guardedCall<T>(action: () => T): T {
    return this.#guarded(() => {
      try {
        return action();
      } catch (error) {
        throw new CallFailure(error);
      }
    });
  }

  /**
   * Sets another square up.
   * @param square The images and the square.
   * @throws {VisualModuleError} When a value is not an unsigned 32-bit integer, or the
   * module answers outside its contract.
   * @throws {VisualSetupError} With the module's explanation, naming the value refused, when
   * it refuses the square: it then keeps the one it had.
   * @throws {VisualTrap} When a call into the module fails, or has.
   */
  setup(square: SquareSetup): void {
    const args = [
      u32(square.width, "width"),
      u32(square.height, "height"),
      u32(square.rate[0], "rate[0]"),
      u32(square.rate[1], "rate[1]"),
      u32(square.side, "side"),
      u32(square.cycle[0], "cycle[0]"),
      u32(square.cycle[1], "cycle[1]"),
    ] as const;
    const refused = this.#guarded(() => {
      const result = this.#setup(...args);
      if (result === 1) {
        this.#values = args[0] * args[1] * 4;
        return undefined;
      }
      if (result !== 0) {
        throw new VisualModuleError(`visual_setup returned ${String(result)}, not 0 or 1`);
      }
      // Where the explanation is, asked before the memory is read: a call may grow the
      // memory, which replaces its buffer.
      const span = this.#span("visual_message_ptr", this.#messagePtr(), this.#messageLen(), 1);
      const bytes = new Uint8Array(this.#memory.buffer, ...span);
      try {
        return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
      } catch {
        throw new VisualModuleError("visual_setup explained its refusal in bytes that are not UTF-8");
      }
    });
    if (refused !== undefined) {
      throw new VisualSetupError(refused);
    }
  }

  /**
   * Renders image `index` of the square.
   * @param index The image, from 0 to 2⁶⁴ − 1.
   * @returns A copy of its values, four per pixel, row by row from the top.
   * @throws {VisualModuleError} When `index` is out of range, or the module answers outside
   * its contract (a count of values not of the square set up, an address unaligned or
   * outside its memory).
   * @throws {VisualTrap} When a call into the module fails, or has.
   */
  render(index: bigint): Float32Array<ArrayBuffer> {
    const [low, high] = halves(index);
    return this.#guarded(() => {
      const address = this.#render(low, high);
      const count = this.#valuesLen();
      if (count !== this.#values) {
        throw new VisualModuleError(`visual_values_len answered ${String(count)}, not the ${String(this.#values)} values of the square set up`);
      }
      return new Float32Array(this.#memory.buffer, ...this.#span("visual_render", address, count, 4)).slice();
    });
  }

  /**
   * Renders image `index` of the square encoded for display (`spec/visual-design.md`,
   * "Display encoding"), what a canvas's `ImageData` holds.
   * @param index The image, from 0 to 2⁶⁴ − 1.
   * @returns Its bytes, four per pixel (8-bit sRGB red, green and blue, then straight
   * alpha), row by row from the top: a view of the module memory, not a copy, so that
   * showing an image copies it once only, into the canvas. It is valid until the next call
   * to this driver, which may overwrite it or move the memory (a view of a memory that has
   * grown is empty): read or draw it before then.
   * @throws {VisualModuleError} When `index` is out of range, or the module answers outside
   * its contract (a count of bytes not of the square set up, an address outside its
   * memory).
   * @throws {VisualTrap} When a call into the module fails, or has.
   */
  display(index: bigint): Uint8ClampedArray<ArrayBuffer> {
    const [low, high] = halves(index);
    return this.#guarded(() => {
      const address = this.#display(low, high);
      const count = this.#displayLen();
      if (count !== this.#values) {
        throw new VisualModuleError(`visual_display_len answered ${String(count)}, not the ${String(this.#values)} bytes of the square set up`);
      }
      return new Uint8ClampedArray(this.#memory.buffer, ...this.#span("visual_display", address, count, 1));
    });
  }

  /**
   * Where the square is in image `index`, exactly as the CPU renders it: what a GPU drawing
   * the square itself needs (the library's square shader).
   * @param index The image, from 0 to 2⁶⁴ − 1.
   * @returns Its left, right, top and bottom edges, in subpixel units.
   * @throws {VisualModuleError} When `index` is out of range, or the module answers outside
   * its contract (no placement for the square set up, an address outside its memory).
   * @throws {VisualTrap} When a call into the module fails, or has.
   */
  placement(index: bigint): Placement {
    const [low, high] = halves(index);
    return this.#guarded(() => {
      const address = this.#placement(low, high);
      if (address === 0) {
        throw new VisualModuleError("visual_placement answered no placement for the square set up");
      }
      const [left = 0, right = 0, top = 0, bottom = 0] = new Uint32Array(this.#memory.buffer, ...this.#span("visual_placement", address, 4, 4));
      return [left, right, top, bottom];
    });
  }

  /**
   * The WGSL source of one of the library's shaders, as the module holds it: the one source
   * every GPU interface runs (`spec/visual-design.md`).
   * @param kind The shader.
   * @returns Its source.
   * @throws {VisualModuleError} When the module holds none of that kind, its address is
   * outside its memory, or its bytes are not UTF-8.
   * @throws {VisualTrap} When a call into the module fails, or has.
   */
  shader(kind: ShaderKind): string {
    return this.#guarded(() => {
      const span = this.#span("visual_shader_ptr", this.#shaderPtr(SHADERS[kind]), this.#shaderLen(SHADERS[kind]), 1);
      if (span[1] === 0) {
        throw new VisualModuleError(`visual_shader_len answered no ${kind} shader`);
      }
      try {
        return new TextDecoder("utf-8", { fatal: true }).decode(new Uint8Array(this.#memory.buffer, ...span));
      } catch {
        throw new VisualModuleError(`the ${kind} shader is not UTF-8`);
      }
    });
  }

  /**
   * Encodes any `values` for display as the module does (`spec/visual-design.md`, "Display
   * encoding"), apart from the square: what compares the module's display encoding with the
   * native one on values a box does not write yet (the display corpus,
   * `conformance/visual/display.json`).
   * @param values Four values a pixel (red, green, blue, alpha, premultiplied linear light),
   * any bits, at least one pixel and at most the pixels of the largest image, which the
   * module checks.
   * @returns A copy of their bytes, four a pixel (8-bit sRGB, straight alpha).
   * @throws {VisualModuleError} When `values` is not whole pixels or more than an unsigned
   * 32-bit count of them, the module refuses the room (no pixel, more than the largest
   * image, its memory full) or answers outside its contract (an address unaligned or
   * outside its memory).
   * @throws {VisualTrap} When a call into the module fails, or has.
   */
  encode(values: Float32Array): Uint8Array<ArrayBuffer> {
    if (values.length % 4 !== 0) {
      throw new VisualModuleError(`${String(values.length)} values are not whole pixels of 4`);
    }
    const pixels = u32(values.length / 4, "the pixels to encode");
    return this.#guarded(() => {
      const room = this.#encodeValues(pixels);
      if (room === 0) {
        throw new VisualModuleError(`the module has no room for ${String(pixels)} pixels to encode`);
      }
      new Float32Array(this.#memory.buffer, ...this.#span("visual_encode_values", room, values.length, 4)).set(values);
      return new Uint8Array(this.#memory.buffer, ...this.#span("visual_encode", this.#encode(), values.length, 1)).slice();
    });
  }

  /**
   * The display shader's thresholds, as the module holds them: each threshold's bits.
   * @returns A copy of the 510 thresholds: 255 sRGB, then 255 alpha.
   * @throws {VisualModuleError} When their address is unaligned or outside the module memory.
   * @throws {VisualTrap} When a call into the module fails, or has.
   */
  thresholds(): Uint32Array<ArrayBuffer> {
    return this.#guarded(() => new Uint32Array(this.#memory.buffer, ...this.#span("visual_thresholds", this.#thresholds(), THRESHOLDS, 4)).slice());
  }

  /**
   * The image hash (`spec/image-hash.md`) of `count` images of the square from `first`.
   * @param first The first image, from 0 to 2⁶⁴ − 1.
   * @param count How many images, from 0 to {@link MAX_COUNT}, the last at most image
   * 2⁶⁴ − 1.
   * @returns The 64 lowercase hexadecimal digits of the hash. Every image is rendered and
   * hashed before it returns, `count` × width × height pixels, on the calling thread: tens
   * of nanoseconds each in WebAssembly, the hash costing far more than the render (a 1080p
   * image about a tenth of a second, measured once in Node on the maintainer's Mac,
   * 2026-11-15; the visual benchmarks hold its count of instructions, render and hash,
   * `spec/benchmarks.md`), so a page bounds what it asks, as
   * `visual-cli` bounds a case to 2²⁸ pixels.
   * @throws {VisualModuleError} When an argument is out of range, the last image is past
   * the largest index, or the module refuses the images or answers outside its contract.
   * @throws {VisualTrap} When a call into the module fails, or has.
   */
  hash(first: bigint, count: number): string {
    const [low, high] = halves(first);
    const images = u32(count, "count");
    if (first + BigInt(images) - 1n > MAX_INDEX) {
      throw new VisualModuleError(`${String(images)} images from image ${String(first)} pass the largest index, 2^64 - 1`);
    }
    return this.#guarded(() => {
      const address = this.#hash(low, high, images);
      if (address === 0) {
        throw new VisualModuleError("visual_hash refused the images");
      }
      return Array.from(new Uint8Array(this.#memory.buffer, ...this.#span("visual_hash", address, 32, 1)), (byte) => byte.toString(16).padStart(2, "0")).join("");
    });
  }
}
