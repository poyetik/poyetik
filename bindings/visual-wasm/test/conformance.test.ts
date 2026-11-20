// The visual conformance cases rendered by the real WebAssembly module, built into dist/ by
// `pnpm build:wasm`: every case of conformance/visual/cases must give the image hash of
// conformance/visual/references.json, which the native renderer (visual-cli) recorded.
// Same bits natively and in WebAssembly.
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { parseVisualCase } from "@poyetik/oracle/visual-inputs";
import { expect, test } from "vitest";
import { VISUAL_MODULE } from "../scripts/module.ts";
import { MAX_INDEX } from "../ts/driver.ts";
import { VisualModule, VisualSetupError } from "../ts/visual.ts";
import { TestDriver } from "./test-driver.ts";

/**
 * The visual conformance directory, found by walking up from `from`: the tests also run
 * from a copy of the package, deeper in the repository (Stryker's sandbox).
 * @throws {Error} When no directory up to the root holds it.
 */
function visualConformance(from: URL = new URL(import.meta.url)): URL {
  for (let directory = new URL("./", from); ; directory = new URL("../", directory)) {
    const candidate = new URL("conformance/visual/", directory);
    if (existsSync(candidate)) {
      return candidate;
    }
    // The root, where going up changes nothing: `/` here, `/C:/` on Windows.
    if (new URL("../", directory).href === directory.href) {
      throw new Error("conformance/visual not found");
    }
  }
}

/** The visual conformance directory. */
const CONFORMANCE = visualConformance();

/** The module, as `pnpm build:wasm` built it. */
const MODULE = new URL(`../dist/${VISUAL_MODULE.file}`, import.meta.url);

test("gives every case its reference hash, as the native renderer does", async () => {
  const references: unknown = JSON.parse(readFileSync(new URL("references.json", CONFORMANCE), "utf8"));
  const names = readdirSync(new URL("cases/", CONFORMANCE)).filter((name) => name.endsWith(".json"));
  expect(names, "the eleven cases of the corpus").toHaveLength(11);
  for (const file of names) {
    const name = file.slice(0, -".json".length);
    const read = parseVisualCase(readFileSync(new URL(`cases/${file}`, CONFORMANCE), "utf8"), name);
    // A fresh instance per case, as the native renderer starts each case afresh.
    const module = await VisualModule.instantiate(readFileSync(MODULE), { width: read.width, height: read.height, rate: read.rate, side: read.square.side, cycle: read.square.cycle });
    expect(module.hash(read.first, read.images), name).toBe(Reflect.get(Object(references), name));
  }
  expect(typeof references === "object" && references !== null ? Object.keys(references).sort() : []).toStrictEqual(names.map((file) => file.slice(0, -".json".length)).sort());
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
  const text = readFileSync(new URL("test-matrix.json", CONFORMANCE), "utf8");
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
  expect(() => { module.setup({ ...square, width: 0, height: 1 }); }).toThrow(VisualSetupError);
  module.setup({ ...square, width: 16_384, height: 2048 });
  expect(memory.buffer.byteLength, "still one image at a time after a refusal").toBeLessThan(image + image / 2);
  expect(module.hash(1n, 0)).toMatch(/^[0-9a-f]{64}$/u);
  // Measured: 0.3 s alone (2026-11-15); how much slower it runs under Stryker's parallel
  // test processes is not measured, so the limit leaves room: a limit, not a tolerance on
  // what it checks.
}, 60_000);

test("refuses an image the memory cannot hold, naming its size, never a trap, and keeps working", async () => {
  // The memory grown, from outside, to within 8000 pages (500 MiB) of the most a 32-bit
  // module addresses (65 536 pages, 4 GiB): the largest image, 512 MiB, cannot fit. The pages
  // grown are never written, so the machine does not hold them.
  const { instance } = await WebAssembly.instantiate(readFileSync(MODULE), {});
  const memory: unknown = instance.exports["memory"];
  if (!(memory instanceof WebAssembly.Memory)) {
    throw new Error("no memory");
  }
  const square = { rate: [60, 1], side: 65_536, cycle: [1, 2] } as const;
  const module = new TestDriver(instance.exports, { ...square, width: 3, height: 2 });
  memory.grow(65_536 - 8000 - memory.buffer.byteLength / 65_536);
  expect(() => { module.setup({ ...square, width: 16_384, height: 2048 }); }).toThrow(new VisualSetupError("width and height: image 16384x2048 does not fit in this machine's memory: 536870912 bytes are needed"));
  // The square it had, kept: every call works on it, before any other setup.
  expect([module.render(1n), module.display(1n), module.placement(1n), module.hash(1n, 1)].map((answer) => answer.length)).toStrictEqual([3 * 2 * 4, 3 * 2 * 4, 4, 64]);
  module.setup({ ...square, width: 3, height: 2 });
  expect(module.render(1n)).toHaveLength(3 * 2 * 4);
});

test("refuses an image whose display bytes the memory cannot hold, naming the bytes they need", async () => {
  // The memory grown to within 9000 pages (562.5 MiB) of the most a 32-bit module
  // addresses: the largest image's values, 512 MiB, fit, its display bytes, 128 MiB more,
  // do not.
  const { instance } = await WebAssembly.instantiate(readFileSync(MODULE), {});
  const memory: unknown = instance.exports["memory"];
  if (!(memory instanceof WebAssembly.Memory)) {
    throw new Error("no memory");
  }
  const square = { rate: [60, 1], side: 65_536, cycle: [1, 2] } as const;
  const module = new TestDriver(instance.exports, { ...square, width: 3, height: 2 });
  memory.grow(65_536 - 9000 - memory.buffer.byteLength / 65_536);
  expect(() => { module.setup({ ...square, width: 16_384, height: 2048 }); }).toThrow(new VisualSetupError("width and height: image 16384x2048 does not fit in this machine's memory: 134217728 bytes are needed"));
  expect(module.display(1n), "the square it had, kept").toHaveLength(3 * 2 * 4);
  module.setup({ ...square, width: 3, height: 2 });
  expect(module.display(1n)).toHaveLength(3 * 2 * 4);
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

test("stops looking for the conformance directory at the root, a Windows drive's included", () => {
  for (const root of ["file:///", "file:///C:/", "file:///C:/Users/nobody/"]) {
    expect(() => visualConformance(new URL(root)), root).toThrow(new Error("conformance/visual not found"));
  }
});

test("gives a GPU the library's shaders, its thresholds and the square's exact placement", async () => {
  const module = await VisualModule.instantiate(readFileSync(MODULE), { width: 17, height: 9, rate: [60, 1], side: 65_536, cycle: [1001, 1000] });
  // The library's crates, beside the conformance directory.
  const library = new URL("../../crates/", CONFORMANCE);
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

/**
 * The pixels of the display corpus file `text`, each its four values and the four bytes the
 * native encoding gives them, read only in the form `visual-cli bless` writes.
 * @throws {Error} Naming the first line in another form.
 */
function displayCorpus(text: string): { readonly values: Float32Array; readonly bytes: Uint8Array } {
  const lines = text.split("\n");
  const pixels = lines.slice(2, -3);
  if (lines.slice(0, 2).join("\n") !== '{\n  "pixels": [' || lines.slice(-3).join("\n") !== "  ]\n}\n") {
    throw new Error("display.json: not as visual-cli bless writes it");
  }
  const bits = new Uint32Array(pixels.length * 4);
  const bytes = new Uint8Array(pixels.length * 4);
  for (const [at, line] of pixels.entries()) {
    const words = /^ {4}\["([0-9a-f]{8})", "([0-9a-f]{8})", "([0-9a-f]{8})", "([0-9a-f]{8})", "([0-9a-f]{8})"\],?$/u.exec(line);
    if (words === null) {
      throw new Error(`display.json, pixel ${String(at)}: not as visual-cli bless writes it: ${line}`);
    }
    const [, ...hex] = words;
    const [red = "", green = "", blue = "", alpha = "", encoded = ""] = hex;
    bits.set([red, green, blue, alpha].map((word) => Number.parseInt(word, 16)), at * 4);
    new DataView(bytes.buffer).setUint32(at * 4, Number.parseInt(encoded, 16));
  }
  return { values: new Float32Array(bits.buffer), bytes };
}

test("encodes the display corpus to the bytes of the native encoding, every pixel", async () => {
  const corpus = displayCorpus(readFileSync(new URL("display.json", CONFORMANCE), "utf8"));
  expect(corpus.values.length, "the corpus's pixels, as visual-cli writes them").toBe(3504 * 4);
  const module = await VisualModule.instantiate(readFileSync(MODULE), { width: 1, height: 1, rate: [1, 1], side: 1, cycle: [1, 1] });
  const encoded = module.encode(corpus.values);
  for (let pixel = 0; pixel < corpus.bytes.length / 4; pixel += 1) {
    const at = pixel * 4;
    expect(Array.from(encoded.subarray(at, at + 4)), `pixel ${String(pixel)}: values ${Array.from(new Uint32Array(corpus.values.buffer, at * 4, 4), (word) => word.toString(16)).join(" ")}`).toStrictEqual(Array.from(corpus.bytes.subarray(at, at + 4)));
  }
});

test("reads the display corpus only as visual-cli writes it", () => {
  const line = '    ["3f800000", "3f000000", "00000000", "3f800000", "ffbc00ff"]';
  expect(displayCorpus(`{\n  "pixels": [\n${line}\n  ]\n}\n`).bytes).toStrictEqual(Uint8Array.of(0xff, 0xbc, 0, 0xff));
  for (const [text, message] of [
    [`[\n  "pixels": [\n${line}\n  ]\n}\n`, "display.json: not as visual-cli bless writes it"],
    [`{\n  "pixels": [\n${line}\n  ]\n}`, "display.json: not as visual-cli bless writes it"],
    [`{\n  "pixels": [\n${line.replace("3f000000", "3F000000")}\n  ]\n}\n`, `display.json, pixel 0: not as visual-cli bless writes it: ${line.replace("3f000000", "3F000000")}`],
    [`{\n  "pixels": [\n ${line}\n  ]\n}\n`, `display.json, pixel 0: not as visual-cli bless writes it:  ${line}`],
  ] as const) {
    expect(() => displayCorpus(text), message).toThrow(new Error(message));
  }
});
