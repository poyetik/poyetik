// The visual conformance runner, shared by the tests of every module that links the visual
// library (`@poyetik/visual-wasm/testing/conformance`): the library's own module and the
// page module each give the corpus's reference hashes, encode the display corpus to the
// native bytes and refuse an image their memory cannot hold, through the same loops, so that
// a module the product ships is held to the corpus as its sibling is. Test code, never
// built into dist/ (it reads files): measured and mutated as the package's sources are.
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { parseVisualCase } from "@poyetik/oracle/visual-inputs";
import type { Exports, SquareSetup, VisualDriver } from "../driver.ts";
import { type DisplayCorpus, displayCorpus } from "./display-corpus.ts";

/**
 * The visual conformance directory, found by walking up from `from`: the tests also run
 * from a copy of a package, deeper in the repository (Stryker's sandbox).
 * @param from Where to start: a test file's URL.
 * @returns The directory, as a URL ending with `/`.
 * @throws {Error} When no directory up to the root holds it.
 */
export function visualConformance(from: URL): URL {
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

/**
 * What the runner calls of a driver: its public methods, read by their shape, so that a
 * driver from this source and one from a package's built code are both accepted.
 */
export type Driven = Pick<VisualDriver, "setup" | "render" | "display" | "placement" | "encode" | "hash">;

/** Creates a driver of a fresh instance of a module from its bytes, with a square set up: `VisualModule.instantiate`, `PageModule.instantiate`. */
export type Instantiate = (bytes: Uint8Array<ArrayBuffer>, square: SquareSetup) => Promise<Driven>;

/** Drives the instance behind `exports`, with a square set up: a test's own driver, whose constructor it may call. */
export type Drive = (exports: Exports, square: SquareSetup) => Driven;

/** The hashes a module gave the corpus's cases, and the references the native renderer recorded. */
export interface CaseHashes {
  /** `references.json`, as read. */
  readonly references: unknown;
  /** Each case's hash, by its name. */
  readonly hashes: Readonly<Record<string, string>>;
}

/**
 * Renders and hashes every case of the corpus (`cases/` of `conformance`) through a fresh
 * instance each, as the native renderer starts each case afresh.
 * @param conformance The visual conformance directory.
 * @param bytes The module.
 * @param instantiate How a driver of it is created.
 * @returns The hashes, and the references they must equal.
 */
export async function caseHashes(conformance: URL, bytes: Uint8Array<ArrayBuffer>, instantiate: Instantiate): Promise<CaseHashes> {
  const references: unknown = JSON.parse(readFileSync(new URL("references.json", conformance), "utf8"));
  const hashes: Record<string, string> = {};
  for (const file of readdirSync(new URL("cases/", conformance)).filter((entry) => entry.endsWith(".json"))) {
    const name = file.slice(0, -".json".length);
    const read = parseVisualCase(readFileSync(new URL(`cases/${file}`, conformance), "utf8"), name);
    const module = await instantiate(bytes, { width: read.width, height: read.height, rate: read.rate, side: read.square.side, cycle: read.square.cycle });
    hashes[name] = module.hash(read.first, read.images);
  }
  return { references, hashes };
}

/** The display corpus, and the bytes a module encoded its values to. */
export interface EncodedCorpus {
  /** The corpus (`display.json`), as `visual-cli bless` wrote it. */
  readonly corpus: DisplayCorpus;
  /** The module's bytes for the corpus's values. */
  readonly encoded: Uint8ClampedArray;
}

/**
 * Encodes the values of the display corpus (`display.json` of `conformance`) through a
 * module.
 * @param conformance The visual conformance directory.
 * @param bytes The module.
 * @param instantiate How a driver of it is created.
 * @returns The corpus, and what the module encoded.
 */
export async function encodedCorpus(conformance: URL, bytes: Uint8Array<ArrayBuffer>, instantiate: Instantiate): Promise<EncodedCorpus> {
  const corpus = displayCorpus(readFileSync(new URL("display.json", conformance), "utf8"));
  const module = await instantiate(bytes, { width: 1, height: 1, rate: [1, 1], side: 1, cycle: [1, 1] });
  return { corpus, encoded: module.encode(corpus.values) };
}

/**
 * The first pixel whose encoded bytes are not the corpus's, with its values' bits, or
 * nothing when every pixel is the same and as many.
 * @param read The corpus and what a module encoded.
 * @returns The difference, or an empty string.
 */
export function firstDifference(read: EncodedCorpus): string {
  const { corpus, encoded } = read;
  if (encoded.length !== corpus.bytes.length) {
    return `${String(encoded.length)} bytes encoded, ${String(corpus.bytes.length)} in the corpus`;
  }
  for (let at = 0; at < encoded.length; at += 4) {
    const got = Array.from(encoded.subarray(at, at + 4));
    const expected = Array.from(corpus.bytes.subarray(at, at + 4));
    if (got.some((byte, channel) => byte !== expected[channel])) {
      const values = Array.from(new Uint32Array(corpus.values.buffer, at * 4, 4), (word) => word.toString(16)).join(" ");
      return `pixel ${String(at / 4)}: values ${values}: ${String(got)}, expected ${String(expected)}`;
    }
  }
  return "";
}

/** What a module did with the largest image when its memory could not hold it. */
export interface MemoryRefusal {
  /** What the setup of the largest image threw (nothing when it did not throw). */
  readonly refused: unknown;
  /** The lengths of what the square it had gives at image 1 afterwards: its values, its display bytes, its placement and its hash's digits. */
  readonly kept: readonly number[];
  /** The lengths of the values and display bytes of that square, set up again. */
  readonly again: readonly number[];
}

/** The largest image's side, in pixels, as the scenarios set it up: 16 384 × 2048, 2²⁵ pixels. */
const LARGEST = { width: 16_384, height: 2048 } as const;

/**
 * Sets the largest image up in an instance of `bytes` whose memory was grown, from outside,
 * to within `free` pages (64 KiB each) of the most a 32-bit module addresses (65 536 pages,
 * 4 GiB), after a 3 × 2 square: the pages grown are never written, so the machine does not
 * hold them.
 * @param bytes The module.
 * @param drive How a driver of its instance is built.
 * @param free The pages left.
 * @returns What the setup threw, and what the square it had gives afterwards.
 * @throws {Error} When the instance exports no memory.
 */
export async function largestImageIn(bytes: Uint8Array<ArrayBuffer>, drive: Drive, free: number): Promise<MemoryRefusal> {
  const { instance } = await WebAssembly.instantiate(bytes, {});
  const memory: unknown = instance.exports["memory"];
  if (!(memory instanceof WebAssembly.Memory)) {
    throw new Error("no memory");
  }
  const square = { width: 3, height: 2, rate: [60, 1], side: 65_536, cycle: [1, 2] } as const;
  const module = drive(instance.exports, square);
  memory.grow(65_536 - free - memory.buffer.byteLength / 65_536);
  let refused: unknown;
  try {
    module.setup({ ...square, ...LARGEST });
  } catch (error) {
    refused = error;
  }
  const kept = [module.render(1n), module.display(1n), module.placement(1n), module.hash(1n, 1)].map((answer) => answer.length);
  module.setup(square);
  return { refused, kept, again: [module.render(1n).length, module.display(1n).length] };
}
