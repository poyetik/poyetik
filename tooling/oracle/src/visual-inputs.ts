/**
 * The visual conformance cases (`conformance/visual/cases`), parsed at the boundary as
 * `visual-cli` parses them: a case arrives as text and leaves typed, or is rejected.
 */

import { InputError, field, onlyKeys } from "./inputs.ts";

/** The largest case file read, in bytes (`visual_cli::MAX_CASE_BYTES`). */
export const MAX_VISUAL_CASE_BYTES = 64 * 1024;

/** The most images a case renders (`visual_cli::MAX_IMAGES`). */
export const MAX_IMAGES = 10_000;

/** The most images per second (`visual_core::ImageRate::MAX_PER_SECOND`). */
export const MAX_IMAGES_PER_SECOND = 1000;

/** The longest side of an image, in pixels (`visual_core::ImageSize::MAX_SIDE`). */
export const MAX_IMAGE_SIDE = 16_384;

/** The most pixels an image holds (`visual_core::ImageSize::MAX_PIXELS`). */
export const MAX_IMAGE_PIXELS = 2 ** 25;

/** The most pixels a case renders in all, its images times the pixels of each (`visual_cli::MAX_RENDERED_PIXELS`). */
export const MAX_RENDERED_PIXELS = 2 ** 28;

/** The side of a square as tall as the image (`visual_boxes::Square::FULL_SIDE`). */
export const FULL_SIDE = 65_536;

/** The largest unsigned 32-bit integer, the range of every field but `first` and `images`. */
const MAX_U32 = 0xffff_ffff;

/** The largest unsigned 64-bit integer, the range of `first`. */
const MAX_U64 = (1n << 64n) - 1n;

/** A fraction: numerator, then denominator. */
export type Fraction = readonly [number, number];

/** What a visual case declares. */
export interface VisualCase {
  /** Images per second, a reduced fraction from 1 to {@link MAX_IMAGES_PER_SECOND}. */
  readonly rate: Fraction;
  /** Width of the images, in pixels, from 1 to {@link MAX_IMAGE_SIDE}. */
  readonly width: number;
  /** Height of the images, in pixels, from 1 to {@link MAX_IMAGE_SIDE}. */
  readonly height: number;
  /** The index of the first image rendered, exact up to 2⁶⁴ − 1. */
  readonly first: bigint;
  /** How many images are rendered, from 1 to {@link MAX_IMAGES}. */
  readonly images: number;
  /** The square: its side in 65 536ths of the height, and the seconds one sweep lasts. */
  readonly square: { readonly side: number; readonly cycle: Fraction };
}

/** The greatest common divisor of two whole numbers. */
function gcd(left: number, right: number): number {
  return right === 0 ? left : gcd(right, left % right);
}

/** Reads an unsigned 32-bit integer, or rejects `where`. */
function u32(value: unknown, where: string): number {
  // Every test after the type is false for a value that is not a number, so the type is what refuses one.
  if (typeof value !== "number" || !(value >= 0 && value <= MAX_U32 && value % 1 === 0)) {
    throw new InputError(`${where} is not an unsigned 32-bit integer`);
  }
  return value;
}

/** Reads a pair of unsigned 32-bit integers, or rejects `where`. */
function pair(value: unknown, where: string): Fraction {
  if (!Array.isArray(value) || value.length !== 2) {
    throw new InputError(`${where} is not a pair of numbers`);
  }
  const numerator: unknown = Reflect.get(value, 0);
  const denominator: unknown = Reflect.get(value, 1);
  return [u32(numerator, `${where}[0]`), u32(denominator, `${where}[1]`)];
}

/** Reads an image rate: a reduced fraction of images per second from 1 to {@link MAX_IMAGES_PER_SECOND}. */
function rate(value: unknown, where: string): Fraction {
  const [numerator, denominator] = pair(value, where);
  // A zero denominator is refused by the upper bound, or by the divisor when the numerator is zero too.
  if (numerator < denominator || numerator > denominator * MAX_IMAGES_PER_SECOND || gcd(numerator, denominator) !== 1) {
    throw new InputError(`${where} is not a reduced rate from 1 to ${String(MAX_IMAGES_PER_SECOND)} images per second`);
  }
  return [numerator, denominator];
}

/** Reads a duration: a reduced fraction of seconds, neither part zero. */
function cycle(value: unknown, where: string): Fraction {
  const [numerator, denominator] = pair(value, where);
  if (numerator === 0 || denominator === 0 || gcd(numerator, denominator) !== 1) {
    throw new InputError(`${where} is not a reduced, nonzero duration`);
  }
  return [numerator, denominator];
}

/** Reads a side of an image, from 1 to {@link MAX_IMAGE_SIDE} pixels. */
function side(value: unknown, where: string): number {
  const read = u32(value, where);
  if (read < 1 || read > MAX_IMAGE_SIDE) {
    throw new InputError(`${where} is not from 1 to ${String(MAX_IMAGE_SIDE)} pixels`);
  }
  return read;
}

/** The fields of a case and of its square, every one required, none other allowed. */
const CASE_KEYS = ["rate", "width", "height", "first", "images", "square"] as const;

/** The fields of the square of a case. */
const SQUARE_KEYS = ["side", "cycle"] as const;

/** `value` as an object with exactly the fields `keys`, none missing and none other, or rejects `where`. */
function exactly(value: unknown, keys: readonly string[], where: string): object {
  const checked = onlyKeys(value, keys, where);
  const missing = keys.find((key) => !Object.hasOwn(checked, key));
  if (missing !== undefined) {
    throw new InputError(`${where}: missing field ${missing}`);
  }
  return checked;
}

/**
 * Parses `text` with a reviver that refuses any number not written as a whole number,
 * without fraction or exponent, and negative zero, as `visual-cli`'s reader refuses them
 * for an integer field; names the number refused by its path (`square.cycle[0]`); and
 * returns the exact value of `first` from its text (a number above 2⁵³ has lost it).
 *
 * The reviver visits every value before the one holding it: the path of a number refused
 * is built on the way up, from the values holding it, and thrown at the root.
 */
function exactFirst(text: string, name: string): bigint {
  let first = -1n;
  // The number refused: its path so far, why, and the value holding it, whose own visit adds its key.
  let refused: { path: string; reason: string; holder: unknown } | undefined;
  JSON.parse(text, function (this: unknown, key, value: unknown, ...context: unknown[]) {
    if (refused !== undefined) {
      if (value === refused.holder) {
        if (key === "") {
          throw new InputError(`${name}: ${refused.path} ${refused.reason}`);
        }
        refused = { ...refused, path: refused.path.startsWith("[") ? `${key}${refused.path}` : `${key}.${refused.path}`, holder: this };
      }
    } else if (typeof value === "number") {
      const source = String(Reflect.get(Object(context[0]), "source"));
      const path = Array.isArray(this) ? `[${key}]` : key;
      if (source === "-0") {
        refused = { path, reason: "is negative zero, which an unsigned integer refuses", holder: this };
      } else if (!/^-?\d+$/.test(source)) {
        refused = { path, reason: "is not a whole number written without fraction or exponent", holder: this };
      } else if (key === "first") {
        first = BigInt(source);
      }
    }
    return value;
  });
  return first;
}

/**
 * Whether every field of `text` is written once: its shape is already checked, so that
 * its only strings are its keys (the values are numbers and arrays of numbers), and a
 * field written twice, which `JSON.parse` silently keeps the last of, adds a string.
 */
function eachFieldOnce(text: string): boolean {
  return text.split(/"(?:[^"\\]|\\.)*"/).length - 1 === CASE_KEYS.length + SQUARE_KEYS.length;
}

/**
 * Reads the text of a visual case file, as `visual-cli` reads it: at most
 * {@link MAX_VISUAL_CASE_BYTES} bytes of a JSON object with exactly the fields `rate`,
 * `width`, `height`, `first`, `images` and `square` (itself exactly `side` and `cycle`),
 * each written once, every number whole, written without fraction or exponent and not
 * negative zero, each within its range.
 *
 * The text is parsed twice, as the audio cases are: plainly first, to check the shape and
 * the values; then, the shape known to be shallow, with the reviver that checks how every
 * number is written and reads `first` exactly.
 * @throws {InputError} Naming the case `name`, when the text is not such a case.
 */
export function parseVisualCase(text: string, name: string): VisualCase {
  const bytes = new TextEncoder().encode(text).length;
  if (bytes > MAX_VISUAL_CASE_BYTES) {
    throw new InputError(`${name}: a case file holds at most ${String(MAX_VISUAL_CASE_BYTES)} bytes, this one ${String(bytes)}`);
  }
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch (error) {
    throw new InputError(`${name}: not JSON (${String(error)})`);
  }
  const checked = exactly(value, CASE_KEYS, name);
  const square = exactly(field(checked, "square"), SQUARE_KEYS, `${name}: square`);
  const read = {
    rate: rate(field(checked, "rate"), `${name}: rate`),
    width: side(field(checked, "width"), `${name}: width`),
    height: side(field(checked, "height"), `${name}: height`),
    images: field(checked, "images"),
    square: {
      side: u32(field(square, "side"), `${name}: square.side`),
      cycle: cycle(field(square, "cycle"), `${name}: square.cycle`),
    },
  };
  if (read.width * read.height > MAX_IMAGE_PIXELS) {
    throw new InputError(`${name}: the image has more than ${String(MAX_IMAGE_PIXELS)} pixels`);
  }
  if (typeof read.images !== "number" || !(read.images >= 1 && read.images <= MAX_IMAGES && read.images % 1 === 0)) {
    throw new InputError(`${name}: images is not from 1 to ${String(MAX_IMAGES)}`);
  }
  if (read.images * read.width * read.height > MAX_RENDERED_PIXELS) {
    throw new InputError(`${name}: a case renders at most ${String(MAX_RENDERED_PIXELS)} pixels in all, this one ${String(read.images * read.width * read.height)}`);
  }
  if (read.square.side > FULL_SIDE) {
    throw new InputError(`${name}: square.side is above ${String(FULL_SIDE)}, a square taller than the image`);
  }
  if (typeof field(checked, "first") !== "number") {
    throw new InputError(`${name}: first is not a whole number`);
  }
  if (!eachFieldOnce(text)) {
    throw new InputError(`${name}: a field is written twice`);
  }
  const first = exactFirst(text, name);
  if (first < 0n || first > MAX_U64) {
    throw new InputError(`${name}: first is not an unsigned 64-bit integer`);
  }
  // The last image is `first + images - 1`: it must be an index too.
  if (first + BigInt(read.images) - 1n > MAX_U64) {
    throw new InputError(`${name}: the last image is past the largest index`);
  }
  return { ...read, images: read.images, first };
}
