/**
 * The product's module for the page of a browser, driven from JavaScript: the package's
 * interface. A {@link PageModule} creates the instance it drives, from a compiled module or
 * from its bytes, so that no other code holds it, and drives both interfaces the module
 * exports: the visual library's (the square, its images and their hash, as
 * `@poyetik/visual-wasm` drives it) and the coordinating layer's (which image the page shows
 * at a frame of the audio clock).
 */
import { type SquareSetup, VisualDriver } from "@poyetik/visual-wasm/driver";
import { type ImageAt, MAX_FRAME, PageModuleError, REFUSED, pageInterface } from "./interface.ts";

export { MAX_FRAME, PAGE_ABI_VERSION, PageModuleError } from "./interface.ts";
// What a page needs to name of the visual interface it drives: the square, its placement,
// its shaders and errors, from this one entry point.
export { MAX_INDEX, type Placement, SHADERS, type ShaderKind, type SquareSetup, THRESHOLDS, VisualModuleError, VisualSetupError, VisualTrap } from "@poyetik/visual-wasm/driver";

/** One instance of the page's module, which it created and alone drives, always with a square set up. */
export class PageModule extends VisualDriver {
  /** The coordinating layer's `page_image_at`, checked. */
  readonly #imageAt: ImageAt;

  /**
   * Drives the instance behind `exports`: only {@link PageModule.create} has one.
   * @param exports The exports of the instance it created.
   * @param square The square to set up.
   */
  private constructor(exports: WebAssembly.Exports, square: SquareSetup) {
    super(exports, square);
    this.#imageAt = pageInterface(exports);
  }

  /**
   * Instantiates `module`, with no imports, checks both its interfaces and sets `square` up.
   * @param module The compiled module.
   * @param square The square to set up.
   * @returns The module, checked, with its square.
   * @throws {PageModuleError} When it needs imports, or its page interface is not this one.
   * @throws {Error} As `VisualDriver` does, when its visual interface is not the one it drives or it refuses the square.
   */
  static async create(module: WebAssembly.Module, square: SquareSetup): Promise<PageModule> {
    let instance: WebAssembly.Instance;
    try {
      instance = await WebAssembly.instantiate(module, {});
    } catch (error) {
      throw new PageModuleError(`cannot be instantiated with no imports: ${String(error)}`, { cause: error });
    }
    return new PageModule(instance.exports, square);
  }

  /**
   * Compiles the module `bytes`, then {@link PageModule.create}s it.
   * @param bytes The module.
   * @param square The square to set up.
   * @returns The module, checked, with its square.
   * @throws {PageModuleError} When the bytes are not a WebAssembly module, or as {@link PageModule.create}.
   */
  static async instantiate(bytes: BufferSource, square: SquareSetup): Promise<PageModule> {
    let module: WebAssembly.Module;
    try {
      module = await WebAssembly.compile(bytes);
    } catch (error) {
      throw new PageModuleError(`cannot be compiled: ${String(error)}`, { cause: error });
    }
    return PageModule.create(module, square);
  }

  /**
   * The image the page shows at frame `frame` of an audio clock at `sampleRate` hertz, for a
   * sequence at `rate` images a second (numerator, then denominator): exact.
   * @param frame The frame, from 0 to 2⁶⁴ − 1.
   * @param sampleRate The sample rate, in hertz.
   * @param rate The image rate: numerator, then denominator.
   * @returns The image's index.
   * @throws {PageModuleError} When a value is outside the interface, or the module refuses the sample rate or the image rate.
   * @throws {VisualTrap} When the call fails, or a call into the instance has: the instance is then never called again.
   */
  imageAt(frame: bigint, sampleRate: number, rate: readonly [number, number]): bigint {
    if (frame < 0n || frame > MAX_FRAME) {
      throw new PageModuleError(`frame ${String(frame)} is not from 0 to 2^64 - 1`);
    }
    const terms = [sampleRate, ...rate];
    if (!terms.every((term) => Number.isInteger(term) && term >= 0 && term <= 0xffff_ffff)) {
      throw new PageModuleError(`${terms.join(", ")}: not unsigned 32-bit integers`);
    }
    // A 64-bit argument is passed as a signed i64: from 2^63 on, its negative twin.
    // Through the driver's guard: a failed call kills the instance, as any call of the
    // visual interface does.
    const image = this.guardedCall(() => this.#imageAt(BigInt.asIntN(64, frame), sampleRate, rate[0], rate[1]));
    if (image === REFUSED) {
      throw new PageModuleError(`sample rate ${String(sampleRate)} Hz or image rate ${String(rate[0])}/${String(rate[1])} refused`);
    }
    return image;
  }
}
