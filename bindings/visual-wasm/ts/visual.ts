/**
 * The visual library's WebAssembly module, driven from JavaScript: its C interface
 * (`bindings/visual-wasm/src/ffi.rs`) checked once at the boundary, then used through typed
 * methods. Every address the module answers is checked against its memory before it is
 * read, and what is read is copied before the next call that sets up, renders or hashes,
 * which may move it.
 */

/** The version of the module's interface this code drives: "PV" and version 3. */
export const ABI_VERSION = 0x5056_0003;

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
 * included), out of memory among them: the instance is then dead, and every later call throws
 * it again.
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

/** The memories of the instances a {@link VisualModule} drives: each is driven by one only. */
const DRIVEN = new WeakSet<WebAssembly.Memory>();

/** A function the module exports, taking the unsigned 32-bit numbers `A` and returning one. */
type Exported<A extends readonly number[]> = (...args: A) => number;

/** What a module instance exports, read at the boundary as unknown values. */
type Exports = Readonly<Record<string, unknown>>;

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
 * is created, and a refused setup keeps the square it had. It drives its instance alone: a
 * second wrapper of the same instance is refused. Its memory grows to hold the largest image
 * set up, whose values take up to 512 MiB (the memory a little more), and never shrinks again (WebAssembly memory cannot): only a fresh
 * instance gives it back.
 */
export class VisualModule {
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
  /** Whether the module trapped: its state is then unknown, and it is never called again. */
  #dead = false;
  /** The values of an image of the square set up: width × height × 4. */
  #values = 0;

  /**
   * The module behind `exports`, checked (a memory, every function of the interface with
   * its arity, and the interface version this code drives), with `square` set up.
   * @param exports The exports of an instance of the module.
   * @param square The square to set up.
   * @throws {VisualModuleError} Naming what is missing or which version it has, when the
   * instance is already driven by another wrapper, when a value of `square` is not an
   * unsigned 32-bit integer, or when the module answers outside its contract.
   * @throws {VisualSetupError} With the module's explanation, when it refuses the square.
   * @throws {VisualTrap} When a call into the module fails: a trap (out of memory among
   * them), or the host's own failure (a stack overflow).
   */
  constructor(exports: Exports, square: SquareSetup) {
    const memory: unknown = exports["memory"];
    if (!(memory instanceof WebAssembly.Memory)) {
      throw new VisualModuleError("no memory");
    }
    // Two wrappers on one instance would each keep their own square and their own death,
    // while the instance holds only one: the second would read the first's images.
    if (DRIVEN.has(memory)) {
      throw new VisualModuleError("this instance is already driven by a VisualModule, or a call into it failed");
    }
    // Driven from its first call: an instance a call failed into is never called again,
    // whichever wrapper tries; one refused before any failed call is freed for another.
    DRIVEN.add(memory);
    try {
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
      this.setup(square);
    } catch (error) {
      if (!(error instanceof VisualTrap)) {
        DRIVEN.delete(memory);
      }
      throw error;
    }
  }

  /**
   * Compiles and instantiates the module `bytes`, with no imports, checks it and sets
   * `square` up.
   * @param bytes The module.
   * @param square The square to set up.
   * @returns The module, checked, with its square.
   * @throws {VisualModuleError} When it is not the visual module this code drives: bytes
   * that are not a WebAssembly module, a module that needs imports, or another module; or
   * when a value of `square` is not an unsigned 32-bit integer.
   * @throws {VisualSetupError} With the module's explanation, when it refuses the square.
   * @throws {VisualTrap} When a call into the module fails.
   */
  static async instantiate(bytes: BufferSource, square: SquareSetup): Promise<VisualModule> {
    let module: WebAssembly.Module;
    try {
      module = await WebAssembly.compile(bytes);
    } catch (error) {
      throw new VisualModuleError(`cannot be compiled: ${String(error)}`, { cause: error });
    }
    let instance: WebAssembly.Instance;
    try {
      instance = await WebAssembly.instantiate(module, {});
    } catch (error) {
      throw new VisualModuleError(`cannot be instantiated with no imports: ${String(error)}`, { cause: error });
    }
    return new VisualModule(instance.exports, square);
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
   * The image hash (`spec/image-hash.md`) of `count` images of the square from `first`.
   * @param first The first image, from 0 to 2⁶⁴ − 1.
   * @param count How many images, from 0 to {@link MAX_COUNT}, the last at most image
   * 2⁶⁴ − 1.
   * @returns The 64 lowercase hexadecimal digits of the hash. Every image is rendered and
   * hashed before it returns, `count` × width × height pixels, on the calling thread: tens
   * of nanoseconds each in WebAssembly, the hash costing far more than the render (a 1080p
   * image about a tenth of a second, measured once in Node on the maintainer's Mac,
   * 2026-11-15; no benchmark holds it yet, #34 step 5), so a page bounds what it asks, as
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
