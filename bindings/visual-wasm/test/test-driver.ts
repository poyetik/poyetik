/**
 * A driver the tests build directly, on a fake or a real instance's exports: the driver's
 * own constructor is for the modules that create the instance they drive, so that no other
 * code holds it.
 */

import { type Exports, type SquareSetup, VisualDriver } from "../ts/driver.ts";

/** `VisualDriver`, whose constructor tests may call. */
export class TestDriver extends VisualDriver {
  /**
   * The driver of `exports`, with `square` set up (`VisualDriver`'s own constructor).
   * @param exports The exports of an instance of the module.
   * @param square The square to set up.
   */
  // A public constructor is the point: the base class's is protected.
  public constructor(exports: Exports, square: SquareSetup) {
    super(exports, square);
  }
}
