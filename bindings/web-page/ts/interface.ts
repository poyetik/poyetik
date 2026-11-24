/**
 * The coordinating layer's C interface in the page module (`bindings/web-page/src/ffi.rs`),
 * checked once at the boundary, internal to the package: `page.ts` drives it.
 */

/** The version of the page's interface this code drives: "PP" and version 1. */
export const PAGE_ABI_VERSION = 0x5050_0001;

/** What `page_image_at` answers for a rate it refuses: 2⁶⁴ − 1, never an image. */
export const REFUSED = (1n << 64n) - 1n;

/** The largest frame of the audio clock, 2⁶⁴ − 1. */
export const MAX_FRAME = (1n << 64n) - 1n;

/** Thrown when the page module is not the one this code drives, or is asked outside its interface. */
export class PageModuleError extends Error {
  /**
   * Says what is wrong.
   * @param detail What is wrong.
   * @param options What it comes from, if anything.
   */
  constructor(detail: string, options?: ErrorOptions) {
    super(`page module: ${detail}`, options);
    this.name = "PageModuleError";
  }
}

/**
 * `page_image_at`: the image at a frame, from the frame, the sample rate and the image rate's
 * terms; its answer as the module gave it, read by {@link imageOf}.
 */
export type ImageAt = (frame: bigint, sampleRate: number, numerator: number, denominator: number) => unknown;

/**
 * Runs a call into the module as the driver runs its own (`VisualDriver`'s `guardedCall`):
 * never once the instance has failed, and any failure of it a `VisualTrap` that kills the
 * instance.
 */
export type Guard = <T>(call: () => T) => T;

/**
 * The export `name` of `exports`, a function of `arity` arguments, or throws naming it: each
 * call of it runs through `guard`, so that this helper, the one place this code calls the
 * module (the shared lint policy's `MODULE_DRIVERS` rule), hands out no unguarded call.
 * @throws {PageModuleError} When it is missing or takes another number of arguments.
 */
function exported(exports: Readonly<Record<string, unknown>>, name: string, arity: number, guard: Guard): (...args: readonly unknown[]) => unknown {
  const value: unknown = exports[name];
  if (typeof value !== "function") {
    throw new PageModuleError(`no function ${name}`);
  }
  if (value.length !== arity) {
    throw new PageModuleError(`${name} takes ${String(value.length)} argument(s), expected ${String(arity)}`);
  }
  return (...args) => guard(() => {
    const answer: unknown = Reflect.apply(value, undefined, args);
    return answer;
  });
}

/**
 * The answer of `page_image_at` as an image index: the signed i64 the module answers, read
 * back as unsigned.
 * @param answer What `page_image_at` answered.
 * @returns The image, from 0 to 2⁶⁴ − 1 ({@link REFUSED} for a refused rate).
 * @throws {PageModuleError} When the answer is not a 64-bit integer.
 */
export function imageOf(answer: unknown): bigint {
  if (typeof answer !== "bigint") {
    throw new PageModuleError("page_image_at did not answer a 64-bit integer");
  }
  return BigInt.asUintN(64, answer);
}

/**
 * The page's interface in `exports`, checked: its version first, then `page_image_at`. Each
 * call into the module, the version read included, runs through `guard` (every function
 * `exported` reads is guarded), so that this interface hands out no call the guard does not
 * hold; what is checked of an export or an
 * answer (its presence, its arity, the version) is checked outside it, so that it is a
 * {@link PageModuleError}, never a trap.
 * @param exports The exports of an instance of the page module.
 * @param guard The driver's guard.
 * @returns `page_image_at`, typed and guarded, its answer as the module gave it
 * ({@link imageOf} reads it).
 * @throws {PageModuleError} When a function is missing or of another arity, or the version
 * is another.
 * @throws {Error} What `guard` throws when the version read fails: a `VisualTrap`.
 */
export function pageInterface(exports: Readonly<Record<string, unknown>>, guard: Guard): ImageAt {
  const version = exported(exports, "page_abi_version", 0, guard)();
  if (version !== PAGE_ABI_VERSION) {
    // In hexadecimal, as the visual and audio modules' drivers say theirs.
    const found = typeof version === "number" ? `0x${version.toString(16)}` : String(version);
    throw new PageModuleError(`interface ${found}, expected 0x${PAGE_ABI_VERSION.toString(16)}: build the module again from this source (see: pnpm --filter @poyetik/web-page build:wasm)`);
  }
  return exported(exports, "page_image_at", 4, guard);
}
