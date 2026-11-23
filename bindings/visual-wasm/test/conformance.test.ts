// The visual conformance cases rendered by the real WebAssembly module, built into dist/ by
// `pnpm build:wasm`: every case of conformance/visual/cases must give the image hash of
// conformance/visual/references.json, which the native renderer (visual-cli) recorded.
// Same bits natively and in WebAssembly.
import { Buffer } from "node:buffer";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { expect, test } from "vitest";
import { VISUAL_MODULE } from "../scripts/module.ts";
import { MAX_INDEX } from "../ts/driver.ts";
import { caseHashes, type Drive, encodedCorpus, firstDifference, largestImageIn, parseVisualReferences, visualConformance } from "../ts/testing/conformance.ts";
import { displayCorpus, notAsBlessWrites } from "../ts/testing/display-corpus.ts";
import { VisualModule, VisualSetupError } from "../ts/visual.ts";
import { TestDriver } from "./test-driver.ts";

/** The visual conformance directory, found by each test that reads it, so that a failure to find it fails that test. */
function conformance(): URL {
  return visualConformance(new URL(import.meta.url));
}

/** The module, as `pnpm build:wasm` built it. */
const MODULE = new URL(`../dist/${VISUAL_MODULE.file}`, import.meta.url);

/** The module's bytes, read again by each test. */
function wasm(): Uint8Array<ArrayBuffer> {
  return new Uint8Array(readFileSync(MODULE));
}

/** Drives an instance through the tests' own driver. */
const drive: Drive = (exports, square) => new TestDriver(exports, square);

test("gives every case its reference hash, as the native renderer does", async () => {
  const { references, hashes } = await caseHashes(conformance(), wasm(), (module, square) => VisualModule.instantiate(module, square));
  expect([...hashes.keys()], "every case of the corpus").toHaveLength(readdirSync(new URL("cases/", conformance())).filter((file) => file.endsWith(".json")).length);
  expect(hashes).toStrictEqual(references);
  // Measured: 2.4 s alone (2026-11-15, the image of 2^25 pixels most of it), past vitest's
  // 5 s when Stryker's nine test processes share the machine; a limit, not a tolerance on
  // what the test checks.
}, 60_000);

/** The pairs of integers of `value`, or none when it is not a list of such pairs. */
function pairs(value: unknown): (readonly [number, number])[] | undefined {
  if (!Array.isArray(value)) {
    return undefined;
  }
  const read: (readonly [number, number])[] = [];
  for (const pair of value.values()) {
    const first: unknown = Reflect.get(Object(pair), 0);
    const second: unknown = Reflect.get(Object(pair), 1);
    if (!Array.isArray(pair) || pair.length !== 2 || typeof first !== "number" || typeof second !== "number" || !Number.isSafeInteger(first) || !Number.isSafeInteger(second)) {
      return undefined;
    }
    read.push([first, second]);
  }
  return read;
}

/**
 * The shared visual matrix (`conformance/visual/test-matrix.json`): its rates and sizes as
 * pairs, its indices read from the text, exact past 2^53.
 */
function visualMatrix(): { rates: (readonly [number, number])[]; sizes: (readonly [number, number])[]; indices: bigint[] } {
  const text = readFileSync(new URL("test-matrix.json", conformance()), "utf8");
  const json: unknown = JSON.parse(text);
  const rates = pairs(Reflect.get(Object(json), "image_rates"));
  const sizes = pairs(Reflect.get(Object(json), "image_sizes"));
  const indices = /"image_indices": \[(?<digits>[\d, ]+)\]/u.exec(text)?.groups?.["digits"]?.split(", ").map((digits) => BigInt(digits));
  if (rates === undefined || sizes === undefined || indices === undefined) {
    throw new Error("test-matrix.json is not the visual matrix");
  }
  return { rates, sizes, indices };
}

/** Subpixel units per pixel (`visual_core::SUBPIXELS`). */
const SUBPIXELS = 4096n;

/**
 * Where the square is in image `index`, in subpixel units, computed exactly from
 * `spec/visual-design.md` section 6 with integers of any size: an implementation of its own,
 * not a copy of the Rust one.
 */
function placed(width: number, height: number, rate: readonly [number, number], side: number, cycle: readonly [number, number], index: bigint): { left: bigint; right: bigint; top: bigint; bottom: bigint } {
  const imageWidth = BigInt(width) * SUBPIXELS;
  const imageHeight = BigInt(height) * SUBPIXELS;
  const edge = (imageHeight * BigInt(side)) / 65_536n;
  const travel = imageWidth > edge ? imageWidth - edge : 0n;
  const period = BigInt(rate[0]) * BigInt(cycle[0]);
  const within = (index * BigInt(rate[1]) * BigInt(cycle[1])) % period;
  const left = (within * travel) / period;
  const top = (imageHeight - edge) / 2n;
  return { left, right: left + edge, top, bottom: top + edge };
}

/** How much of the cell from `start` (one pixel wide) lies between `from` and `to`, in subpixel units. */
function overlap(from: bigint, to: bigint, start: bigint): bigint {
  const low = from > start ? from : start;
  const high = to < start + SUBPIXELS ? to : start + SUBPIXELS;
  return high > low ? high - low : 0n;
}

test("renders every value of the square as the spec places it, across the visual matrix, in WebAssembly", async () => {
  // Every pixel is its exact coverage, (vertical × horizontal overlap) / 2^24, white on
  // opaque black: the same values the native renderer gives at every point of the matrix,
  // its rates and image indices (exact past 2^53) included, not only the corpus.
  const side = 39_322;
  const cycle = [1001, 1000] as const;
  const { rates, sizes, indices } = visualMatrix();
  expect([rates.length, sizes.length, indices.length], "the whole matrix read").toStrictEqual([7, 6, 6]);
  expect(indices.at(-1), "exact past 2^53").toBe(18_446_744_073_709_551_615n);
  for (const rate of rates) {
    for (const [width, height] of sizes) {
      const module = await VisualModule.instantiate(readFileSync(MODULE), { width, height, rate, side, cycle });
      for (const index of indices) {
        const at = `${String(rate)} ${String(width)}x${String(height)} image ${String(index)}`;
        const square = placed(width, height, rate, side, cycle, index);
        const rows = Array.from({ length: height }, (_row, row) => overlap(square.top, square.bottom, BigInt(row) * SUBPIXELS));
        const columns = Array.from({ length: width }, (_column, column) => overlap(square.left, square.right, BigInt(column) * SUBPIXELS));
        const values = module.render(index);
        expect(values.length, at).toBe(width * height * 4);
        let wrong = "";
        for (let pixel = 0; pixel < width * height && wrong === ""; pixel += 1) {
          const coverage = Number((rows[Math.floor(pixel / width)] ?? 0n) * (columns[pixel % width] ?? 0n)) / 2 ** 24;
          const expected = [coverage, coverage, coverage, 1];
          const got = Array.from(values.subarray(pixel * 4, pixel * 4 + 4));
          if (got.some((value, channel) => !Object.is(value, expected[channel]))) {
            wrong = `pixel ${String(pixel)}: ${String(got)}, expected ${String(expected)}`;
          }
        }
        expect(wrong, at).toBe("");
      }
    }
  }
  // Measured: 0.5 s alone (2026-11-15); how much slower it runs under Stryker's parallel
  // test processes is not measured, so the limit leaves room: a limit, not a tolerance on
  // what it checks.
}, 60_000);

test("creates its own instance from a compiled module, as a worker receives one, the same images as from the bytes", async () => {
  const square = { width: 17, height: 9, rate: [60, 1], side: 39_322, cycle: [1001, 1000] } as const;
  const compiled = await WebAssembly.compile(readFileSync(MODULE));
  const fromModule = await VisualModule.create(compiled, square);
  const fromBytes = await VisualModule.instantiate(readFileSync(MODULE), square);
  expect(fromModule.hash(0n, 60)).toBe(fromBytes.hash(0n, 60));
  // Two instances of one compiled module, each its own: a setup of one leaves the other.
  const other = await VisualModule.create(compiled, square);
  other.setup({ ...square, width: 3 });
  expect(Array.from(fromModule.render(7n))).toStrictEqual(Array.from(fromBytes.render(7n)));
  expect(other.render(7n)).toHaveLength(3 * 9 * 4);
});

test("explains a refused square and keeps the one it had", async () => {
  const square = { rate: [60, 1], side: 65_536, cycle: [1, 2] } as const;
  await expect(VisualModule.instantiate(readFileSync(MODULE), { ...square, width: 0, height: 2 })).rejects.toThrow(new VisualSetupError("width and height: invalid image size 0x2: each side must be from 1 to 16384 pixels"));
  const module = await VisualModule.instantiate(readFileSync(MODULE), { ...square, width: 3, height: 2 });
  const before = module.render(1n);
  expect(before).toHaveLength(3 * 2 * 4);
  for (const [refused, reason] of [
    [{ ...square, width: 3, height: 2, side: 65_537 }, "side: invalid square side 65537: at most 65536, a square as tall as the image (a side is in 65536ths of the image height)"],
    [{ ...square, width: 3, height: 2, rate: [2, 4] }, "rate: invalid image rate 2/4: a reduced fraction from 1 to 1000 images per second is required"],
    [{ ...square, width: 3, height: 2, cycle: [0, 1] }, "cycle: invalid duration 0/1 s: a reduced, nonzero fraction is required"],
  ] as const) {
    expect(() => { module.setup(refused); }, reason).toThrow(new VisualSetupError(reason));
    expect(module.render(1n), reason).toStrictEqual(before);
  }
});

test("sets up the largest image, refuses one pixel row more, and releases the image it replaces before allocating the next", async () => {
  // One instance of 640 MiB (the image and its display bytes) for the whole test: the tests
  // share the machine with Stryker's processes, and each such image costs as much again in
  // its copy.
  const { instance } = await WebAssembly.instantiate(readFileSync(MODULE), {});
  const memory: unknown = instance.exports["memory"];
  if (!(memory instanceof WebAssembly.Memory)) {
    throw new Error("no memory");
  }
  const square = { rate: [60, 1], side: 65_536, cycle: [1, 2] } as const;
  const module = new TestDriver(instance.exports, { ...square, width: 1, height: 1 });
  expect(() => { module.setup({ ...square, width: 16_384, height: 2049 }); }).toThrow(new VisualSetupError("width and height: invalid image size 16384x2049: an image holds at most 33554432 pixels"));
  module.setup({ ...square, width: 16_384, height: 2048 });
  expect(module.render(0n)).toHaveLength(16_384 * 2048 * 4);
  expect(module.display(0n)).toHaveLength(16_384 * 2048 * 4);
  // Its values, 16 bytes a pixel, and its display bytes, 4.
  const image = 16_384 * 2048 * 20;
  module.setup({ ...square, width: 2048, height: 16_384 });
  expect(memory.buffer.byteLength, "one image of 640 MiB at a time, not two").toBeLessThan(image + image / 2);
  // A refusal between a smaller image and the largest again allocates nothing that could
  // keep the largest from reusing the space (it grew to two images when it did).
  module.setup({ ...square, width: 4096, height: 4096 });
  expect(() => { module.setup({ ...square, width: 0, height: 1 }); }).toThrow(new VisualSetupError("width and height: invalid image size 0x1: each side must be from 1 to 16384 pixels"));
  module.setup({ ...square, width: 16_384, height: 2048 });
  expect(memory.buffer.byteLength, "still one image at a time after a refusal").toBeLessThan(image + image / 2);
  expect(module.hash(1n, 0)).toMatch(/^[0-9a-f]{64}$/u);
  // Measured: 0.3 s alone (2026-11-15); how much slower it runs under Stryker's parallel
  // test processes is not measured, so the limit leaves room: a limit, not a tolerance on
  // what it checks.
}, 60_000);

test("refuses an image the memory cannot hold, naming its size, never a trap, and keeps working", async () => {
  // The memory grown to within 8000 pages (500 MiB) of the most a 32-bit module addresses:
  // the largest image, 512 MiB, cannot fit. The square it had is kept, every call working on
  // it before any other setup.
  expect(await largestImageIn(wasm(), drive, 8000)).toStrictEqual({
    refused: new VisualSetupError("width and height: image 16384x2048 does not fit in this machine's memory: 536870912 bytes are needed"),
    held: [3 * 2 * 4, 3 * 2 * 4, 4, 64],
    changed: [],
    again: [5 * 1 * 4, 5 * 1 * 4],
    left: 8000,
  });
});

test("refuses an image whose display bytes the memory cannot hold, naming the bytes they need", async () => {
  // The memory grown to within 9000 pages (562.5 MiB) of the most a 32-bit module
  // addresses: the largest image's values, 512 MiB, fit, its display bytes, 128 MiB more,
  // do not.
  expect(await largestImageIn(wasm(), drive, 9000)).toStrictEqual({
    refused: new VisualSetupError("width and height: the display encoding of image 16384x2048 does not fit in this machine's memory: 134217728 bytes are needed"),
    held: [3 * 2 * 4, 3 * 2 * 4, 4, 64],
    changed: [],
    again: [5 * 1 * 4, 5 * 1 * 4],
    left: 9000,
  });
});

test("names every answer of the square it had that a refused setup changed", async () => {
  // A driver that takes the refused setup's side, cycle and rate on its square's size, then
  // refuses: the runner must see the square it had change, by its values, display bytes,
  // placement and hash, its lengths alike.
  const partial: Drive = (exports, square) => {
    const driver = new TestDriver(exports, square);
    return {
      render: (index) => driver.render(index),
      display: (index) => driver.display(index),
      placement: (index) => driver.placement(index),
      encode: (values) => driver.encode(values),
      hash: (first, count) => driver.hash(first, count),
      setup: (setup) => {
        if (setup.width === 16_384) {
          driver.setup({ ...setup, width: square.width, height: square.height });
          throw new VisualSetupError("refused after taking its terms");
        }
        driver.setup(setup);
      },
    };
  };
  expect(await largestImageIn(wasm(), partial, 8000)).toStrictEqual({
    refused: new VisualSetupError("refused after taking its terms"),
    held: [3 * 2 * 4, 3 * 2 * 4, 4, 64],
    changed: ["values", "display bytes", "placement", "hash"],
    again: [5 * 1 * 4, 5 * 1 * 4],
    left: 8000,
  });
});

test("refuses to grow the memory of an instance that has none", async () => {
  // The empty module: a valid module exporting nothing.
  await expect(largestImageIn(Uint8Array.of(0x00, 0x61, 0x73, 0x6d, 1, 0, 0, 0), drive, 8000)).rejects.toThrow(new Error("no memory"));
});

/** The sRGB transfer function of IEC 61966-2-1 in binary64, scaled to 255: where a linear value falls among the 8-bit codes. */
function srgbScaled(linear: number): number {
  return 255 * (linear <= 0.003_130_8 ? 12.92 * linear : 1.055 * linear ** (1 / 2.4) - 0.055);
}

test("displays each image as its values encode: opaque, the color in sRGB at the nearest code", async () => {
  // The square is white over opaque black: every pixel opaque, its three colors equal, so
  // that its display is that color's sRGB code three times, then 255. The exact codes are
  // proven natively, threshold by threshold (visual-core); here, a code is the nearest to
  // the binary64 transfer function: within half a step, and 10⁻⁹ more for the binary64
  // function's own rounding (its error is below 10⁻¹³ of a step), so that a value at a
  // tie may take either side.
  const module = await VisualModule.instantiate(readFileSync(MODULE), { width: 17, height: 9, rate: [60, 1], side: 39_322, cycle: [1001, 1000] });
  const seen = new Set<number>();
  for (const index of [0n, 7n, 59n, MAX_INDEX]) {
    const values = module.render(index);
    const bytes = module.display(index).slice();
    expect(bytes, `image ${String(index)}`).toHaveLength(values.length);
    for (let at = 0; at < values.length; at += 4) {
      const [red = Number.NaN, green, blue, alpha] = values.subarray(at, at + 4);
      const [code = -1, codeGreen, codeBlue, opacity] = bytes.subarray(at, at + 4);
      const where = `image ${String(index)}, value ${String(at)}`;
      expect([green, blue, alpha, codeGreen, codeBlue, opacity], where).toStrictEqual([red, red, 1, code, code, 255]);
      expect(Math.abs(code - srgbScaled(red)), where).toBeLessThanOrEqual(0.5 + 1e-9);
      seen.add(code);
    }
  }
  expect([seen.has(0), seen.has(255), seen.size > 2], "black, white and edges between").toStrictEqual([true, true, true]);
});

test("finds the repository's conformance directory from a test, up the tree", () => {
  // From a copy of the package too (Stryker's sandbox, deeper in the repository): the
  // repository holding the test, its conformance/visual/, with the references in it.
  const found = conformance();
  expect([found.href.endsWith("/conformance/visual/"), import.meta.url.startsWith(new URL("../../", found).href), existsSync(new URL("references.json", found))]).toStrictEqual([true, true, true]);
});

test("hashes only the corpus's cases, its .json files", async () => {
  // A conformance directory of the test's own: one case of the corpus, and a file beside it
  // that is no case.
  const root = mkdtempSync(join(tmpdir(), "visual-conformance-"));
  try {
    mkdirSync(join(root, "cases"));
    copyFileSync(new URL("cases/square-without-side-at-25.json", conformance()), join(root, "cases", "square-without-side-at-25.json"));
    writeFileSync(join(root, "cases", "notes.txt"), "no case\n");
    writeFileSync(join(root, "references.json"), '{\n  "square-without-side-at-25": "a"\n}\n');
    const { references, hashes } = await caseHashes(pathToFileURL(`${root}/`), wasm(), (module, square) => VisualModule.instantiate(module, square));
    expect([references, [...hashes.keys()]]).toStrictEqual([new Map([["square-without-side-at-25", "a"]]), ["square-without-side-at-25"]]);
  } finally {
    rmSync(root, { recursive: true });
  }
});

/** The error the JSON parser refuses `text` with, its words read from the parser. */
function refusedByParser(text: string): unknown {
  try {
    JSON.parse(text);
  } catch (error) {
    return error;
  }
  return undefined;
}

test("reads the references only as visual-cli bless writes them, in the order of their names' UTF-8 bytes", () => {
  // As serde_json writes a map of names in Rust's string order: "10" before "9", whatever
  // JavaScript's order of integer keys, and U+FFFF before U+10000, whose first UTF-16 unit
  // is smaller.
  expect(parseVisualReferences('{\n  "10": "a",\n  "9": "b",\n  "\uffff": "c",\n  "\u{10000}": "d"\n}\n')).toStrictEqual(
    new Map([["10", "a"], ["9", "b"], ["\uffff", "c"], ["\u{10000}", "d"]]),
  );
  // A name a record would set as its prototype, and lose.
  expect(parseVisualReferences('{\n  "__proto__": "a"\n}\n')).toStrictEqual(new Map([["__proto__", "a"]]));
  for (const [text, why] of [
    ['{\n  "9": "b",\n  "10": "a"\n}\n', "in another order"],
    ['{\n  "\u{10000}": "d",\n  "\uffff": "c"\n}\n', "in UTF-16 order"],
    ['{\n  "a": "0",\n  "a": "1"\n}\n', "a name written twice"],
    ['{ "a": "1" }\n', "in another layout"],
    ['{\n  "a": "1"\n}', "without its last newline"],
    ['{\n  "a": 1\n}\n', "a hash that is no string"],
    ['{}\n', "emptied of its names"],
    ['{\n\n}\n', "emptied, in the layout of its lines"],
    ['"a"\n', "a string"],
    ["5\n", "a number"],
    ["", "empty"],
    ['[\n  "a"\n]\n', "an array"],
    ["null\n", "null"],
  ] as const) {
    expect(() => parseVisualReferences(text), why).toThrow(new Error(`references.json: ${notAsBlessWrites("references.json")}`));
  }
  // Text that is no JSON keeps the parser's error as its cause, read from the parser.
  expect(() => parseVisualReferences("{"), "no JSON").toThrow(new Error(`references.json: ${notAsBlessWrites("references.json")}`, { cause: refusedByParser("{") }));
});

test("refuses references whose bytes are no UTF-8, never reading them as a replacement character", async () => {
  const root = mkdtempSync(join(tmpdir(), "visual-conformance-"));
  try {
    mkdirSync(join(root, "cases"));
    writeFileSync(join(root, "references.json"), Buffer.concat([Buffer.from('{\n  "a'), Uint8Array.of(0xff), Buffer.from('": "a"\n}\n')]));
    // The file named, with its remedy (finding H13-7 of the fifth deep review of visual
    // block 2 (#34)), the decoder's error its cause.
    await expect(caseHashes(pathToFileURL(`${root}/`), wasm(), (module, square) => VisualModule.instantiate(module, square))).rejects.toMatchObject({
      message: `references.json: ${notAsBlessWrites("references.json")}`,
      cause: { name: "TypeError", code: "ERR_ENCODING_INVALID_ENCODED_DATA", message: "The encoded data was not valid for encoding utf-8" },
    });
  } finally {
    rmSync(root, { recursive: true });
  }
});

test("refuses references after a byte order mark, which visual-cli bless never writes and a decoder drops", async () => {
  const root = mkdtempSync(join(tmpdir(), "visual-conformance-"));
  try {
    mkdirSync(join(root, "cases"));
    writeFileSync(join(root, "references.json"), Buffer.concat([Uint8Array.of(0xef, 0xbb, 0xbf), Buffer.from('{\n  "a": "a"\n}\n')]));
    await expect(caseHashes(pathToFileURL(`${root}/`), wasm(), (module, square) => VisualModule.instantiate(module, square))).rejects.toThrow(
      new Error(`references.json: ${notAsBlessWrites("references.json")}`, { cause: refusedByParser('\ufeff{\n  "a": "a"\n}\n') }),
    );
  } finally {
    rmSync(root, { recursive: true });
  }
});

test("stops looking for the conformance directory at the root, a Windows drive's included", () => {
  for (const root of ["file:///", "file:///C:/", "file:///C:/Users/nobody/"]) {
    expect(() => visualConformance(new URL(root)), root).toThrow(new Error("conformance/visual not found"));
  }
});

test("gives a GPU the library's shaders, its thresholds and the square's exact placement", async () => {
  const module = await VisualModule.instantiate(readFileSync(MODULE), { width: 17, height: 9, rate: [60, 1], side: 65_536, cycle: [1001, 1000] });
  // The library's crates, beside the conformance directory.
  const library = new URL("../../crates/", conformance());
  expect(module.shader("square")).toBe(readFileSync(new URL("visual-boxes/src/square.wgsl", library), "utf8"));
  expect(module.shader("display")).toBe(readFileSync(new URL("visual-core/src/display.wgsl", library), "utf8"));
  const thresholds = module.thresholds();
  expect([thresholds.length, thresholds[0], thresholds[255], thresholds[509]], "the first sRGB threshold, the first and the last alpha ones").toStrictEqual([510, 0x391f_22b4, 0x3b00_8081, 0x3f7f_7f80]);
  // A square as tall as the image: from its top to its bottom, as wide as it is tall.
  for (const index of [0n, 7n, MAX_INDEX]) {
    const [left, right, top, bottom] = module.placement(index);
    expect([top, bottom, right - left], `image ${String(index)}`).toStrictEqual([0, 9 * 4096, 9 * 4096]);
  }
});

test("encodes the display corpus to the bytes of the native encoding, every pixel", async () => {
  const read = await encodedCorpus(conformance(), wasm(), (module, square) => VisualModule.instantiate(module, square));
  expect(read.corpus.values.length, "the corpus's pixels, as visual-cli writes them").toBe(3504 * 4);
  expect(firstDifference(read)).toBe("");
});

test("names the first pixel encoded otherwise than the corpus, or a count of bytes not its own", () => {
  const corpus = { values: new Float32Array(Uint32Array.of(0x3f80_0000, 0, 0, 0x3f80_0000, 0x3f00_0000, 1, 2, 0x3f80_0000).buffer), bytes: Uint8Array.of(255, 0, 0, 255, 188, 0, 0, 255) };
  expect(firstDifference({ corpus, encoded: Uint8ClampedArray.of(255, 0, 0, 255, 188, 0, 0, 255) })).toBe("");
  expect(firstDifference({ corpus, encoded: Uint8ClampedArray.of(255, 0, 0, 255, 188, 0, 1, 255) })).toBe("pixel 1: values 3f000000 1 2 3f800000: 188,0,1,255, expected 188,0,0,255");
  expect(firstDifference({ corpus, encoded: Uint8ClampedArray.of(254, 0, 0, 255, 188, 0, 1, 255) }), "the first only").toBe("pixel 0: values 3f800000 0 0 3f800000: 254,0,0,255, expected 255,0,0,255");
  expect(firstDifference({ corpus, encoded: Uint8ClampedArray.of(255, 0, 0, 255, 188, 0, 0, 254) }), "the alpha byte").toBe("pixel 1: values 3f000000 1 2 3f800000: 188,0,0,254, expected 188,0,0,255");
  expect(firstDifference({ corpus, encoded: Uint8ClampedArray.of(255, 0, 0, 255) })).toBe("4 bytes encoded, 8 in the corpus");
});

test("reads the display corpus only as visual-cli writes it", () => {
  const line = '    ["3f800000", "3f000000", "00000000", "3f800000", "ffbc00ff"]';
  expect(displayCorpus(`{\n  "pixels": [\n${line}\n  ]\n}\n`).bytes).toStrictEqual(Uint8Array.of(0xff, 0xbc, 0, 0xff));
  const two = displayCorpus(`{\n  "pixels": [\n${line},\n${line}\n  ]\n}\n`);
  expect([two.values.length, two.bytes.length], "two pixels, a comma after the first").toStrictEqual([8, 8]);
  expect(displayCorpus(`{\n  "pixels": [\n  ]\n}\n`).values, "no pixel").toStrictEqual(new Float32Array(0));
  // One spelling of the remedy (finding H7-9 of the fifth deep review of visual block 2
  // (#34)), its restore runnable from any directory of the repository.
  expect(notAsBlessWrites("display.json")).toBe(
    "not as bless writes it: restore it (see: git checkout HEAD -- :/conformance/visual/display.json), or rewrite it only when the renders are meant to change (see: cargo run -p visual-cli -- bless conformance/visual)",
  );
  for (const [text, message] of [
    [`[\n  "pixels": [\n${line}\n  ]\n}\n`, `display.json: ${notAsBlessWrites("display.json")}`],
    [`{\n  "pixels": [\n${line}\n  ]\n}`, `display.json: ${notAsBlessWrites("display.json")}`],
    [`{\n  "pixels": [\n${line.replace("3f000000", "3F000000")}\n  ]\n}\n`, `display.json, pixel 0, ${JSON.stringify(line.replace("3f000000", "3F000000"))}: ${notAsBlessWrites("display.json")}`],
    [`{\n  "pixels": [\n ${line}\n  ]\n}\n`, `display.json, pixel 0, ${JSON.stringify(` ${line}`)}: ${notAsBlessWrites("display.json")}`],
    [`{\r\n  "pixels": [\n${line}\n  ]\n}\n`, `display.json: ${notAsBlessWrites("display.json")}`],
    [`{\n  "pixels": [\n${line}\r\n  ]\n}\n`, `display.json, pixel 0, ${JSON.stringify(`${line}\r`)}: ${notAsBlessWrites("display.json")}`],
    [`{\n  "pixels": [\n${line},\n  ]\n}\n`, `display.json, pixel 0, ${JSON.stringify(`${line},`)}: ${notAsBlessWrites("display.json")}`],
    [`{\n  "pixels": [\n${line}\n${line}\n  ]\n}\n`, `display.json, pixel 0, ${JSON.stringify(line)}: ${notAsBlessWrites("display.json")}`],
    [`{\n  "pixels": [\n${line.replace("]", ', "00000000"]')}\n  ]\n}\n`, `display.json, pixel 0, ${JSON.stringify(line.replace("]", ', "00000000"]'))}: ${notAsBlessWrites("display.json")}`],
  ] as const) {
    expect(() => displayCorpus(text), message).toThrow(new Error(message));
  }
});
