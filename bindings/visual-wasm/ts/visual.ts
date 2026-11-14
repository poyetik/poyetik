/**
 * The visual library's WebAssembly module, driven from JavaScript: its C interface
 * (`bindings/visual-wasm/src/ffi.rs`) checked once at the boundary, then used through typed
 * methods. Every value read from the module memory is copied before the next call, which
 * may move it.
 */

/** The version of the module's interface this code drives: "PV" and version 2. */
export const ABI_VERSION = 0x5056_0002;

/** The largest index of an image, 2⁶⁴ − 1. */
export const MAX_INDEX = (1n << 64n) - 1n;

/** The largest count of images hashed at once: the interface takes a 32-bit count. */
export const MAX_COUNT = 0xffff_ffff;

/** Thrown when a module is not the visual module this code drives, or is used outside its contract. */
export class VisualModuleError extends Error {
  /**
   * Says what is wrong.
   * @param detail What is wrong.
   */
  constructor(detail: string) {
    super(`visual module: ${detail}`);
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

/** A function the module exports, taking and returning unsigned 32-bit numbers. */
type Exported = (...args: readonly number[]) => number;

/** What a module instance exports, read at the boundary as unknown values. */
type Exports = Readonly<Record<string, unknown>>;

/** The export `name` of `exports` as a function returning an unsigned 32-bit number, or throws. */
function exported(exports: Exports, name: string): Exported {
  const value: unknown = exports[name];
  if (typeof value !== "function") {
    throw new VisualModuleError(`no function ${name}`);
  }
  return (...args) => {
    const result: unknown = Reflect.apply(value, undefined, args);
    if (typeof result !== "number") {
      throw new VisualModuleError(`${name} returned a ${typeof result}, not a number`);
    }
    return result >>> 0;
  };
}

/** `value` as an unsigned 32-bit integer, or throws naming `what`. */
function u32(value: number, what: string): number {
  if (!Number.isInteger(value) || value < 0 || value > 0xffff_ffff) {
    throw new VisualModuleError(`${what} is not an unsigned 32-bit integer`);
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

/** One instance of the visual module: one square at a time. */
export class VisualModule {
  /** The module memory, read after each call. */
  readonly #memory: WebAssembly.Memory;
  /** `visual_setup`. */
  readonly #setup: Exported;
  /** `visual_render`. */
  readonly #render: Exported;
  /** `visual_hash`. */
  readonly #hash: Exported;
  /** `visual_values_len`. */
  readonly #valuesLen: Exported;

  /**
   * The module behind `exports`, checked: a memory, every function of the interface, and
   * the interface version this code drives.
   * @param exports The exports of an instance of the module.
   * @throws {VisualModuleError} Naming what is missing or which version it has.
   */
  constructor(exports: Exports) {
    const memory: unknown = exports["memory"];
    if (!(memory instanceof WebAssembly.Memory)) {
      throw new VisualModuleError("no memory");
    }
    const version = exported(exports, "visual_abi_version")();
    if (version !== ABI_VERSION) {
      throw new VisualModuleError(`interface version 0x${version.toString(16)}, not 0x${ABI_VERSION.toString(16)}`);
    }
    this.#memory = memory;
    this.#setup = exported(exports, "visual_setup");
    this.#render = exported(exports, "visual_render");
    this.#hash = exported(exports, "visual_hash");
    this.#valuesLen = exported(exports, "visual_values_len");
  }

  /**
   * Compiles and instantiates the module `bytes`, with no imports, and checks it.
   * @param bytes The module.
   * @returns The module, checked.
   * @throws {VisualModuleError} When it is not the visual module this code drives.
   */
  static async instantiate(bytes: BufferSource): Promise<VisualModule> {
    const { instance } = await WebAssembly.instantiate(bytes, {});
    return new VisualModule(instance.exports);
  }

  /**
   * Sets the square up.
   * @param setup The images and the square.
   * @returns Whether the module accepted every value; when not, it keeps its square.
   * @throws {VisualModuleError} When a value is not an unsigned 32-bit integer.
   */
  setup(setup: SquareSetup): boolean {
    return (
      this.#setup(
        u32(setup.width, "width"),
        u32(setup.height, "height"),
        u32(setup.rate[0], "rate[0]"),
        u32(setup.rate[1], "rate[1]"),
        u32(setup.side, "side"),
        u32(setup.cycle[0], "cycle[0]"),
        u32(setup.cycle[1], "cycle[1]"),
      ) === 1
    );
  }

  /**
   * Renders image `index` of the square set up.
   * @param index The image, from 0 to 2⁶⁴ − 1.
   * @returns A copy of its values, four per pixel, row by row from the top; empty when no
   * square is set up.
   * @throws {VisualModuleError} When `index` is out of range.
   */
  render(index: bigint): Float32Array<ArrayBuffer> {
    const [low, high] = halves(index);
    const address = this.#render(low, high);
    return new Float32Array(this.#memory.buffer, address, this.#valuesLen()).slice();
  }

  /**
   * The image hash (`spec/image-hash.md`) of `count` images of the square from `first`.
   * @param first The first image, from 0 to 2⁶⁴ − 1.
   * @param count How many images, from 0 to {@link MAX_COUNT}.
   * @returns The 64 lowercase hexadecimal digits of the hash; all zeros when no square is
   * set up.
   * @throws {VisualModuleError} When an argument is out of range.
   */
  hash(first: bigint, count: number): string {
    const [low, high] = halves(first);
    const address = this.#hash(low, high, u32(count, "count"));
    return Array.from(new Uint8Array(this.#memory.buffer, address, 32), (byte) => byte.toString(16).padStart(2, "0")).join("");
  }
}
