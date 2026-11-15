import { existsSync, readdirSync } from "node:fs";
import { describe, expect, test } from "vitest";
import { readBounded } from "../src/files.ts";
import { InputError } from "../src/inputs.ts";
import { FULL_SIDE, MAX_IMAGE_PIXELS, MAX_IMAGE_SIDE, MAX_IMAGES, MAX_IMAGES_PER_SECOND, MAX_VISUAL_CASE_BYTES, parseVisualCase } from "../src/visual-inputs.ts";

/** A valid case, the base of every malformed one below. */
const BASE = `{"rate": [24000, 1001], "width": 17, "height": 9, "first": 3, "images": 5, "square": {"side": 40000, "cycle": [1001, 1000]}}`;

/** `BASE` with `from` replaced by `to`, which must occur once. */
function replaced(from: string, to: string): string {
  expect(BASE.split(from)).toHaveLength(2);
  return BASE.replace(from, to);
}

/** The visual conformance directory, found by walking up: the tests also run from a copy of the package, deeper in the repository. */
function visualConformance(): string {
  for (let directory = new URL("./", import.meta.url); directory.pathname !== "/"; directory = new URL("../", directory)) {
    const candidate = new URL("conformance/visual/", directory);
    if (existsSync(candidate)) {
      return candidate.pathname;
    }
  }
  throw new Error("conformance/visual not found");
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
    expect([MAX_VISUAL_CASE_BYTES, MAX_IMAGES, MAX_IMAGES_PER_SECOND, MAX_IMAGE_SIDE, MAX_IMAGE_PIXELS, FULL_SIDE]).toStrictEqual([65_536, 10_000, 1000, 16_384, 33_554_432, 65_536]);
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
      [`"width": 17, "height": 9`, `"width": 16384, "height": 2048`],
      [`"width": 17, "height": 9`, `"width": 2048, "height": 16384`],
      [`"side": 40000`, `"side": 0`],
      [`"side": 40000`, `"side": 65536`],
      [`[1001, 1000]`, `[4294967295, 1]`],
      [`[1001, 1000]`, `[1, 4294967295]`],
      [`"first": 3`, `"first": -0`],
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
      [`"side": 40000`, `"side": 4e4`, "side is not a whole number written without fraction or exponent"],
    ] as const) {
      expect(refusal(replaced(from, to)), to).toBe(`${prefix}${reason}`);
    }
    expect(refusal("[]")).toBe("invalid conformance input: c is not an object");
    expect(refusal(`${BASE.slice(0, BASE.indexOf(`"square"`))}"square": []}`)).toBe(`${prefix}square is not an object`);
    expect(refusal("{")).toMatch(/^invalid conformance input: c: not JSON \(SyntaxError: /);
    expect(refusal(BASE.replace(`"height": 9, `, ""))).toBe(`${prefix}height is not an unsigned 32-bit integer`);
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

describe("the repository's visual corpus", () => {
  test("every case reads", () => {
    const dir = `${visualConformance()}cases/`;
    const names = readdirSync(dir).filter((name) => name.endsWith(".json"));
    expect(names.length).toBeGreaterThanOrEqual(4);
    for (const name of names) {
      expect(() => parseVisualCase(caseText(`${dir}${name}`), name), name).not.toThrow();
    }
    expect(parseVisualCase(caseText(`${dir}square-far-in-time-at-2997.json`), "far").first).toBe(9_007_199_254_740_000n);
  });

  test("every case of the shared invalid corpus is refused, read as visual-cli reads it", () => {
    const dir = `${visualConformance()}invalid/cases/`;
    const names = readdirSync(dir).filter((name) => name.endsWith(".json"));
    expect(names.length).toBeGreaterThanOrEqual(24);
    for (const name of names) {
      expect(() => parseVisualCase(caseText(`${dir}${name}`), name), name).toThrow(name === "case-above-64-kib.json" ? /larger than 65536 bytes/ : InputError);
    }
  });
});
