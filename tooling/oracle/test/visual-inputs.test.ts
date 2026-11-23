import { existsSync, readdirSync, readFileSync } from "node:fs";
import { describe, expect, test } from "vitest";
import { readBounded } from "../src/files.ts";
import { InputError } from "../src/inputs.ts";
import { FULL_SIDE, MAX_IMAGE_PIXELS, MAX_IMAGE_SIDE, MAX_IMAGES, MAX_RENDERED_PIXELS, MAX_IMAGES_PER_SECOND, MAX_VISUAL_CASE_BYTES, parseVisualCase } from "../src/visual-inputs.ts";
import { splitMix } from "./split-mix.ts";

/** A valid case, the base of every malformed one below. */
const BASE = `{"rate": [24000, 1001], "width": 17, "height": 9, "first": 3, "images": 5, "square": {"side": 40000, "cycle": [1001, 1000]}}`;

/** `BASE` with `from` replaced by `to`, which must occur once. */
function replaced(from: string, to: string): string {
  expect(BASE.split(from)).toHaveLength(2);
  return BASE.replace(from, to);
}

/**
 * The visual conformance directory, found by walking up from `from`: the tests also run
 * from a copy of the package, deeper in the repository.
 * @throws {Error} When no directory up to the root holds it.
 */
function visualConformance(from: URL = new URL(import.meta.url)): string {
  for (let directory = new URL("./", from); ; directory = new URL("../", directory)) {
    const candidate = new URL("conformance/visual/", directory);
    if (existsSync(candidate)) {
      return candidate.pathname;
    }
    // The root, where going up changes nothing: `/` here, `/C:/` on Windows.
    if (new URL("../", directory).href === directory.href) {
      throw new Error("conformance/visual not found");
    }
  }
}

/** The text of the file at `path`, read as `visual-cli` reads it: at most a case's bytes, UTF-8 only. */
function caseText(path: string): string {
  return new TextDecoder("utf-8", { fatal: true }).decode(readBounded(path, MAX_VISUAL_CASE_BYTES));
}

/** The message `parseVisualCase` refuses `text` with. */
function refusal(text: string): string {
  try {
    parseVisualCase(text, "c");
  } catch (error) {
    expect(error).toBeInstanceOf(InputError);
    return error instanceof Error ? error.message : "";
  }
  throw new Error("accepted");
}

describe("the limits are the visual library's", () => {
  test("written as the Rust constants are", () => {
    expect([MAX_VISUAL_CASE_BYTES, MAX_IMAGES, MAX_IMAGES_PER_SECOND, MAX_IMAGE_SIDE, MAX_IMAGE_PIXELS, MAX_RENDERED_PIXELS, FULL_SIDE]).toStrictEqual([65_536, 10_000, 1000, 16_384, 33_554_432, 268_435_456, 65_536]);
  });
});

describe("parseVisualCase", () => {
  test("reads every field of a valid case", () => {
    expect(parseVisualCase(BASE, "c")).toStrictEqual({
      rate: [24_000, 1001],
      width: 17,
      height: 9,
      first: 3n,
      images: 5,
      square: { side: 40_000, cycle: [1001, 1000] },
    });
  });

  test("reads the first image exactly, past 2^53", () => {
    expect(parseVisualCase(replaced(`"first": 3`, `"first": 9007199254740993`), "c").first).toBe(9_007_199_254_740_993n);
    expect(parseVisualCase(replaced(`"first": 3, "images": 5`, `"first": 18446744073709551615, "images": 1`), "c").first).toBe(18_446_744_073_709_551_615n);
    expect(parseVisualCase(replaced(`"first": 3, "images": 5`, `"first": 18446744073709551611, "images": 5`), "c").first).toBe(18_446_744_073_709_551_611n);
  });

  test("accepts each limit and refuses one past it", () => {
    for (const [from, to] of [
      [`"images": 5`, `"images": 1`],
      [`"images": 5`, `"images": 10000`],
      [`[24000, 1001]`, `[1, 1]`],
      [`[24000, 1001]`, `[1000, 1]`],
      [`[24000, 1001]`, `[4294967291, 4294968]`],
      [`"width": 17`, `"width": 1`],
      [`"height": 9`, `"height": 16384`],
      [`"width": 17, "height": 9, "first": 3, "images": 5`, `"width": 16384, "height": 2048, "first": 3, "images": 8`],
      [`"width": 17, "height": 9`, `"width": 2048, "height": 16384`],
      [`"side": 40000`, `"side": 0`],
      [`"side": 40000`, `"side": 65536`],
      [`[1001, 1000]`, `[4294967295, 1]`],
      [`[1001, 1000]`, `[1, 4294967295]`],
    ] as const) {
      expect(() => parseVisualCase(replaced(from, to), "c"), to).not.toThrow();
    }
  });

  test("refuses each fault with a reason naming the case and the field", () => {
    const prefix = "invalid conformance input: c: ";
    for (const [from, to, reason] of [
      [`"images": 5`, `"images": 0`, "images is not from 1 to 10000"],
      [`"images": 5`, `"images": 10001`, "images is not from 1 to 10000"],
      [`"images": 5`, `"images": "5"`, "images is not from 1 to 10000"],
      [`"images": 5`, `"images": 1.5`, "images is not from 1 to 10000"],
      [`"first": 3`, `"first": 18446744073709551616`, "first is not an unsigned 64-bit integer"],
      [`"first": 3`, `"first": -1`, "first is not an unsigned 64-bit integer"],
      [`"first": 3`, `"first": "3"`, "first is not a whole number"],
      [`"first": 3`, `"first": 18446744073709551612`, "the last image is past the largest index"],
      [`[24000, 1001]`, `[120, 2]`, "rate is not a reduced rate from 1 to 1000 images per second"],
      [`[24000, 1001]`, `[1, 2]`, "rate is not a reduced rate from 1 to 1000 images per second"],
      [`[24000, 1001]`, `[1001, 1]`, "rate is not a reduced rate from 1 to 1000 images per second"],
      [`[24000, 1001]`, `[60, 0]`, "rate is not a reduced rate from 1 to 1000 images per second"],
      [`[24000, 1001]`, `[60]`, "rate is not a pair of numbers"],
      [`[24000, 1001]`, `[60, 1, 1]`, "rate is not a pair of numbers"],
      [`[24000, 1001]`, `{"0": 60, "1": 1}`, "rate is not a pair of numbers"],
      [`[24000, 1001]`, `[4294967296, 1]`, "rate[0] is not an unsigned 32-bit integer"],
      [`[24000, 1001]`, `[60, -1]`, "rate[1] is not an unsigned 32-bit integer"],
      [`"width": 17`, `"width": 0`, "width is not from 1 to 16384 pixels"],
      [`"height": 9`, `"height": 16385`, "height is not from 1 to 16384 pixels"],
      [`"width": 17, "height": 9`, `"width": 16384, "height": 2049`, "the image has more than 33554432 pixels"],
      [`"width": 17, "height": 9, "first": 3, "images": 5`, `"width": 16384, "height": 2048, "first": 3, "images": 9`, "a case renders at most 268435456 pixels in all, this one 301989888"],
      [`"width": 17, "height": 9`, `"width": 5793, "height": 5793`, "the image has more than 33554432 pixels"],
      [`"width": 17`, `"width": 4.5`, "width is not an unsigned 32-bit integer"],
      [`"width": 17`, `"width": "17"`, "width is not an unsigned 32-bit integer"],
      [`"side": 40000`, `"side": 65537`, "square.side is above 65536, a square taller than the image"],
      [`"side": 40000`, `"side": 4294967296`, "square.side is not an unsigned 32-bit integer"],
      [`[1001, 1000]`, `[0, 1]`, "square.cycle is not a reduced, nonzero duration"],
      [`[1001, 1000]`, `[1, 0]`, "square.cycle is not a reduced, nonzero duration"],
      [`[1001, 1000]`, `[2, 4]`, "square.cycle is not a reduced, nonzero duration"],
      [`"images": 5`, `"images": 5, "speed": 1`, "unknown field speed"],
      [`"side": 40000`, `"side": 40000, "color": 1`, "square: unknown field color"],
      [`"images": 5`, `"images": 5.0`, "images is not a whole number written without fraction or exponent"],
      [`"width": 17`, `"width": 1.7e1`, "width is not a whole number written without fraction or exponent"],
      [`"side": 40000`, `"side": 4e4`, "square.side is not a whole number written without fraction or exponent"],
      [`[24000, 1001]`, `[24e3, 1001]`, "rate[0] is not a whole number written without fraction or exponent"],
      [`[1001, 1000]`, `[1001, 1000.0]`, "square.cycle[1] is not a whole number written without fraction or exponent"],
      [`"first": 3`, `"first": -0`, "first is negative zero, which an unsigned integer refuses"],
      [`"side": 40000`, `"side": -0`, "square.side is negative zero, which an unsigned integer refuses"],
      [`"first": 3`, `"first": 3, "first": 4`, "a field is written twice"],
      [`"side": 40000`, `"side": 40000, "side": 1`, "a field is written twice"],
      [`"first": 3`, `"first": 3, "fir\\u0073t": 4`, "a field is written twice"],
    ] as const) {
      expect(refusal(replaced(from, to)), to).toBe(`${prefix}${reason}`);
    }
    expect(refusal("[]")).toBe("invalid conformance input: c is not an object");
    expect(refusal(`${BASE.slice(0, BASE.indexOf(`"square"`))}"square": []}`)).toBe(`${prefix}square is not an object`);
    expect(refusal("{")).toMatch(/^invalid conformance input: c: not JSON \(SyntaxError: /);
    expect(refusal(BASE.replace(`"height": 9, `, ""))).toBe(`${prefix}missing field height`);
    expect(refusal(BASE.replace(`"side": 40000, `, ""))).toBe(`${prefix}square: missing field side`);
  });

  test("refuses a text longer than a case file holds, counted in bytes", () => {
    const padded = `${BASE}${" ".repeat(MAX_VISUAL_CASE_BYTES - BASE.length)}`;
    expect(() => parseVisualCase(padded, "c")).not.toThrow();
    expect(refusal(`${padded} `)).toBe(`invalid conformance input: c: a case file holds at most 65536 bytes, this one 65537`);
    const wide = `${BASE}${" ".repeat(MAX_VISUAL_CASE_BYTES - BASE.length - 1)}é`;
    expect(new TextEncoder().encode(wide)).toHaveLength(MAX_VISUAL_CASE_BYTES + 1);
    expect(refusal(wide)).toMatch(/^invalid conformance input: c: a case file holds at most 65536 bytes, this one 65537$/);
  });
});

/** Every file of the shared invalid corpus and why the oracle refuses it (`<name>` stands for the case's name, in the order of the file names), so that no file is refused for another fault than the one it holds. */
const INVALID_CORPUS: readonly (readonly [string, string])[] = [
  ["case-above-64-kib", "larger than 65536 bytes"],
  ["case-as-an-array", "<name> is not an object"],
  ["cycle-0", "<name>: square.cycle is not a reduced, nonzero duration"],
  ["cycle-not-reduced", "<name>: square.cycle is not a reduced, nonzero duration"],
  ["field-written-twice", "<name>: a field is written twice"],
  ["first-above-u64", "<name>: first is not an unsigned 64-bit integer"],
  ["first-negative-zero", "<name>: first is negative zero, which an unsigned integer refuses"],
  ["first-negative", "<name>: first is not an unsigned 64-bit integer"],
  ["height-0", "<name>: height is not from 1 to 16384 pixels"],
  ["height-above-16384", "<name>: height is not from 1 to 16384 pixels"],
  ["image-above-the-pixel-limit", "<name>: the image has more than 33554432 pixels"],
  ["images-0", "<name>: images is not from 1 to 10000"],
  ["images-above-the-limit", "<name>: images is not from 1 to 10000"],
  ["last-image-past-the-largest-index", "<name>: the last image is past the largest index"],
  ["missing-field", "<name>: missing field height"],
  ["not-an-object", "<name> is not an object"],
  ["number-as-string", "<name>: width is not an unsigned 32-bit integer"],
  ["number-with-exponent", "<name>: width is not a whole number written without fraction or exponent"],
  ["number-with-fraction", "<name>: images is not a whole number written without fraction or exponent"],
  ["rate-above-1000-per-second", "<name>: rate is not a reduced rate from 1 to 1000 images per second"],
  ["rate-below-one-per-second", "<name>: rate is not a reduced rate from 1 to 1000 images per second"],
  ["rate-denominator-0", "<name>: rate is not a reduced rate from 1 to 1000 images per second"],
  ["rate-not-a-pair", "<name>: rate is not a pair of numbers"],
  ["rate-not-reduced", "<name>: rate is not a reduced rate from 1 to 1000 images per second"],
  ["side-above-a-full-square", "<name>: square.side is above 65536, a square taller than the image"],
  ["side-negative-zero", "<name>: square.side is negative zero, which an unsigned integer refuses"],
  ["square-as-an-array", "<name>: square is not an object"],
  ["square-field-written-twice", "<name>: a field is written twice"],
  ["too-many-pixels-rendered", "<name>: a case renders at most 268435456 pixels in all, this one 301989888"],
  ["unknown-field", "<name>: unknown field speed"],
  ["unknown-square-field", "<name>: square: unknown field color"],
  ["value-above-u32", "<name>: square.side is not an unsigned 32-bit integer"],
  ["width-0", "<name>: width is not from 1 to 16384 pixels"],
  ["width-16385", "<name>: width is not from 1 to 16384 pixels"],
];

describe("the repository's visual corpus", () => {
  test("every case reads", () => {
    const dir = `${visualConformance()}cases/`;
    const names = readdirSync(dir).filter((name) => name.endsWith(".json"));
    const references: unknown = JSON.parse(readFileSync(`${visualConformance()}references.json`, "utf8"));
    expect(names.map((name) => name.slice(0, -".json".length)).sort(), "every case of the corpus has its reference, and every reference its case").toStrictEqual(typeof references === "object" && references !== null ? Object.keys(references).sort() : []);
    for (const name of names) {
      expect(() => parseVisualCase(caseText(`${dir}${name}`), name), name).not.toThrow();
    }
    // Past 2^53: a reader through a 64-bit float would round it.
    expect(parseVisualCase(caseText(`${dir}square-last-images-at-extreme-terms.json`), "extreme").first).toBe(18_446_744_073_709_551_568n);
  });

  test("every case of the shared invalid corpus is refused, read as visual-cli reads it, for its own fault", () => {
    const dir = `${visualConformance()}invalid/cases/`;
    const names = readdirSync(dir)
      .filter((file) => file.endsWith(".json"))
      .sort()
      .map((file) => file.slice(0, -".json".length));
    expect(names, "every file has its reason, and every reason its file").toStrictEqual(INVALID_CORPUS.map(([name]) => name));
    for (const [name, reason] of INVALID_CORPUS) {
      const expected = reason.replace("<name>", name);
      const path = `${dir}${name}.json`;
      expect(() => parseVisualCase(caseText(path), name), name).toThrow(name === "case-above-64-kib" ? new Error(`cannot read ${path}: ${expected}`) : new InputError(expected));
    }
  });

});

test("reads any text to a case within its limits or refuses it as an input error, never anything else", () => {
  // From a fixed seed, 4096 texts: the valid case with a slice replaced by characters of
  // JSON (digits, signs, brackets, quotes, letters of its fields), or a number replaced by
  // an edge of its range. Every one is a case within every limit, or an InputError: never
  // another exception, which would mean a value reached code that does not expect it.
  const random = splitMix(0x5eed_0a11n);
  const below = (bound: number): number => Math.floor(random() * bound);
  const alphabet = `0123456789-+.eE[]{}:," \\nfalsetruenullratewidthheightfirstimagessquaresidecycle\u00e9`;
  const edges = ["0", "1", "-0", "4294967295", "4294967296", "18446744073709551615", "18446744073709551616", "1e3", "1.5", "65536", "65537", "16384", "16385", "10000", "10001"];
  let accepted = 0;
  for (let draw = 0; draw < 4096; draw += 1) {
    let text: string;
    if (below(2) === 0) {
      const start = below(BASE.length);
      const end = Math.min(BASE.length, start + below(8));
      const inserted = Array.from({ length: below(6) }, () => alphabet.charAt(below(alphabet.length))).join("");
      text = `${BASE.slice(0, start)}${inserted}${BASE.slice(end)}`;
    } else {
      const numbers = [...BASE.matchAll(/\d+/gu)];
      const at = numbers[below(numbers.length)];
      const edge = edges[below(edges.length)] ?? "0";
      text = at === undefined ? BASE : `${BASE.slice(0, at.index)}${edge}${BASE.slice(at.index + at[0].length)}`;
    }
    let read;
    try {
      read = parseVisualCase(text, "random");
    } catch (error) {
      expect(error, text).toBeInstanceOf(InputError);
      continue;
    }
    accepted += 1;
    const pixels = read.width * read.height;
    const limits: readonly (readonly [string, boolean])[] = [
      ["width", read.width >= 1 && read.width <= MAX_IMAGE_SIDE],
      ["height", read.height >= 1 && read.height <= MAX_IMAGE_SIDE],
      ["pixels", pixels <= MAX_IMAGE_PIXELS],
      ["images", read.images >= 1 && read.images <= MAX_IMAGES],
      ["pixels rendered", read.images * pixels <= MAX_RENDERED_PIXELS],
      ["rate", read.rate[1] >= 1 && read.rate[0] >= read.rate[1] && read.rate[0] <= MAX_IMAGES_PER_SECOND * read.rate[1]],
      ["cycle", read.square.cycle[0] >= 1 && read.square.cycle[1] >= 1],
      ["side", read.square.side >= 0 && read.square.side <= FULL_SIDE],
      ["first", read.first >= 0n],
      ["last index", read.first + BigInt(read.images) - 1n <= (1n << 64n) - 1n],
    ];
    expect(limits.filter(([, kept]) => !kept).map(([limit]) => limit), `accepted within every limit: ${text}`).toStrictEqual([]);
  }
  // Measured with this seed: 761 of the 4096 texts are valid cases.
  expect(accepted, "enough valid cases among them to check the limits").toBeGreaterThan(400);
});

test("stops looking for the conformance directory at the root, a Windows drive's included", () => {
  for (const root of ["file:///", "file:///C:/", "file:///C:/Users/nobody/"]) {
    expect(() => visualConformance(new URL(root)), root).toThrow(new Error("conformance/visual not found"));
  }
});
