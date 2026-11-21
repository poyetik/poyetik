/**
 * The visual library's WebAssembly module, driven from JavaScript: the package's interface.
 * A {@link VisualModule} creates the instance it drives from a compiled module (as a worker
 * receives one) or from its bytes, so that no other code holds that instance: two drivers of
 * one instance, each with its own square and its own death, cannot be written.
 */
import { VisualDriver, VisualModuleError, type SquareSetup } from "./driver.ts";

export { ABI_VERSION, MAX_COUNT, MAX_INDEX, type Placement, SHADERS, type ShaderKind, type SquareSetup, THRESHOLDS, VisualModuleError, VisualSetupError, VisualTrap } from "./driver.ts";

/** One instance of the visual module, which it created and alone drives, with a square set up (see {@link VisualDriver}). */
export class VisualModule extends VisualDriver {
  /**
   * Drives the instance behind `exports`: only {@link VisualModule.create} has one.
   * @param exports The exports of the instance it created.
   * @param square The square to set up.
   */
  private constructor(exports: WebAssembly.Exports, square: SquareSetup) {
    super(exports, square);
  }

  /**
   * Instantiates `module`, with no imports, checks it and sets `square` up.
   * @param module The compiled module: the page's, or one a worker received.
   * @param square The square to set up.
   * @returns The module, checked, with its square.
   * @throws {VisualModuleError} When it is not the visual module this code drives: a module
   * that needs imports, or another module; or when a value of `square` is not an unsigned
   * 32-bit integer, or the module answers outside its contract.
   * @throws {VisualSetupError} With the module's explanation, when it refuses the square.
   * @throws {VisualTrap} When a call into the module fails: a trap, or the host's own
   * failure (a stack overflow).
   */
  static async create(module: WebAssembly.Module, square: SquareSetup): Promise<VisualModule> {
    return new VisualModule(await VisualDriver.exportsOf(module, VisualModuleError), square);
  }

  /**
   * Compiles the module `bytes`, then {@link VisualModule.create}s it.
   * @param bytes The module.
   * @param square The square to set up.
   * @returns The module, checked, with its square.
   * @throws {VisualModuleError} When the bytes are not a WebAssembly module, or as
   * {@link VisualModule.create} does.
   * @throws {VisualSetupError} With the module's explanation, when it refuses the square.
   * @throws {VisualTrap} When a call into the module fails.
   */
  static async instantiate(bytes: BufferSource, square: SquareSetup): Promise<VisualModule> {
    return VisualModule.create(await VisualDriver.compiled(bytes, VisualModuleError), square);
  }
}
