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

/** `page_image_at`: the image at a frame, from the frame, the sample rate and the image rate's terms. */
export type ImageAt = (frame: bigint, sampleRate: number, numerator: number, denominator: number) => bigint;

/**
 * The export `name` of `exports`, a function of `arity` arguments, or throws naming it.
 * @throws {PageModuleError} When it is missing or takes another number of arguments.
 */
function exported(exports: Readonly<Record<string, unknown>>, name: string, arity: number): (...args: readonly unknown[]) => unknown {
  const value: unknown = exports[name];
  if (typeof value !== "function") {
    throw new PageModuleError(`no function ${name}`);
  }
  if (value.length !== arity) {
    throw new PageModuleError(`${name} takes ${String(value.length)} argument(s), expected ${String(arity)}`);
  }
  return (...args) => {
    const answer: unknown = Reflect.apply(value, undefined, args);
    return answer;
  };
}

/**
 * The page's interface in `exports`, checked: its version first, then `page_image_at`.
 * @param exports The exports of an instance of the page module.
 * @returns `page_image_at`, typed, its answer checked to be a 64-bit integer.
 * @throws {PageModuleError} When a function is missing or of another arity, or the version
 * is another.
 */
export function pageInterface(exports: Readonly<Record<string, unknown>>): ImageAt {
  const version = exported(exports, "page_abi_version", 0)();
  if (version !== PAGE_ABI_VERSION) {
    // In hexadecimal, as the visual and audio modules' drivers say theirs.
    const found = typeof version === "number" ? `0x${version.toString(16)}` : String(version);
    throw new PageModuleError(`interface ${found}, expected 0x${PAGE_ABI_VERSION.toString(16)}: build the module again from this source (pnpm build:wasm)`);
  }
  const imageAt = exported(exports, "page_image_at", 4);
  return (frame, sampleRate, numerator, denominator) => {
    const image = imageAt(frame, sampleRate, numerator, denominator);
    if (typeof image !== "bigint") {
      throw new PageModuleError("page_image_at did not answer a 64-bit integer");
    }
    // A signed i64 to JavaScript: read back as unsigned.
    return BigInt.asUintN(64, image);
  };
}
