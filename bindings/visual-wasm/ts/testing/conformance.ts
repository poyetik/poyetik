// The visual conformance runner, shared by the tests of every module that links the visual
// library (`@poyetik/visual-wasm/testing/conformance`): the library's own module and the
// page module each give the corpus's reference hashes, encode the display corpus to the
// native bytes and refuse an image their memory cannot hold, through the same loops, so that
// a module the product ships is held to the corpus as its sibling is. Test code, never
// built into dist/ (it reads files): measured and mutated as the package's sources are.
import { Buffer } from "node:buffer";
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
  // From the file itself: a path resolved against it is resolved against its directory.
  for (let directory = from; ; directory = new URL("../", directory)) {
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
  /** `references.json`, read only as `visual-cli bless` writes it ({@link parseReferences}). */
  readonly references: Readonly<Record<string, string>>;
  /** Each case's hash, by its name. */
  readonly hashes: Readonly<Record<string, string>>;
}

/** Why {@link parseReferences} refuses a file, whatever is wrong with it. */
const NOT_AS_BLESS_WRITES = "references.json: not as visual-cli bless writes it";

/**
 * The references file's text `text`, read only exactly as `visual-cli bless` writes it
 * (`references_text` in visual-cli): a JSON object of at least one case name to its hash, a
 * string, one name a line indented by two spaces, in the order of the names' UTF-8 bytes
 * (Rust's order of strings, never JavaScript's: neither UTF-16 units nor integer keys
 * first), and a final newline; so that a file another tool wrote, a name written twice (the
 * last would win) or an emptied file, which would hold no reference, is refused, as
 * `visual-cli check` refuses it.
 * @param text The file's text.
 * @returns Each case's hash, by its name.
 * @throws {Error} When it is written otherwise.
 */
export function parseReferences(text: string): Readonly<Record<string, string>> {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch (error) {
    throw new Error(NOT_AS_BLESS_WRITES, { cause: error });
  }
  const hashes: Record<string, string> = {};
  // Any JSON value but a map of names (`null` read as none) gives no entry or entries by
  // index, which the text written back below refuses.
  for (const [name, hash] of Object.entries(parsed ?? {})) {
    if (typeof hash !== "string") {
      throw new Error(NOT_AS_BLESS_WRITES);
    }
    hashes[name] = hash;
  }
  const lines = Object.entries(hashes)
    .sort(([one], [other]) => Buffer.compare(Buffer.from(one), Buffer.from(other)))
    .map(([name, hash]) => `  ${JSON.stringify(name)}: ${JSON.stringify(hash)}`);
  if (lines.length === 0 || text !== `{\n${lines.join(",\n")}\n}\n`) {
    throw new Error(NOT_AS_BLESS_WRITES);
  }
  return hashes;
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
  // Decoded by the platform's UTF-8 decoder, as JSON is written, refusing bytes that are no
  // UTF-8 rather than reading them as U+FFFD, which would write back as the same text.
  const references = parseReferences(new TextDecoder("utf-8", { fatal: true }).decode(readFileSync(new URL("references.json", conformance))));
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
  const byte = encoded.findIndex((code, index) => code !== corpus.bytes[index]);
  if (byte === -1) {
    return "";
  }
  // The pixel's first byte: four bytes a pixel.
  const at = byte - (byte % 4);
  const got = Array.from(encoded.subarray(at, at + 4));
  const expected = Array.from(corpus.bytes.subarray(at, at + 4));
  const values = Array.from(new Uint32Array(corpus.values.buffer, at * 4, 4), (word) => word.toString(16)).join(" ");
  return `pixel ${String(at / 4)}: values ${values}: ${String(got)}, expected ${String(expected)}`;
}

/** What a module did with the largest image when its memory could not hold it. */
export interface MemoryRefusal {
  /** What the setup of the largest image threw (nothing when it did not throw). */
  readonly refused: unknown;
  /** The lengths of what the square it had gives at image 1 before the refused setup: its values, its display bytes, its placement and its hash's digits. */
  readonly held: readonly number[];
  /** Which of those answers ({@link ANSWERS}) the refused setup changed, by their bits: none when the square it had is kept. */
  readonly changed: readonly string[];
  /** The lengths of the values and display bytes of another image ({@link AGAIN}), set up afterwards. */
  readonly again: readonly number[];
  /** The pages the memory had left once grown, before the setup: `free`. */
  readonly left: number;
}

/** What the square gives at an image, by name, in the order {@link answersAt} gives them. */
const ANSWERS = ["values", "display bytes", "placement", "hash"] as const;

/**
 * What `module`'s square gives at image 1, each answer written out exactly (the values by
 * their bits, so that a NaN or a -0 compares as itself), in the order of {@link ANSWERS}.
 * @param module The driver.
 * @returns The answers, and their lengths.
 */
function answersAt(module: Driven): { readonly written: readonly string[]; readonly lengths: readonly number[] } {
  const values = module.render(1n);
  const display = module.display(1n);
  const placement = module.placement(1n);
  const hash = module.hash(1n, 1);
  return {
    written: [new Uint32Array(values.buffer, values.byteOffset, values.length).join(), display.join(), placement.join(), hash],
    lengths: [values.length, display.length, placement.length, hash.length],
  };
}

/** The image set up after the refusal, of another size than the square's, so that its lengths show it was set up: 5 × 1. */
const AGAIN = { width: 5, height: 1 } as const;

/**
 * The setup refused, the largest image (16 384 × 2048, 2²⁵ pixels), every other term valid
 * and differing from the square's, so that a setup refused after taking any of them shows:
 * half the side, a third of a second, 30 images a second.
 */
const REFUSED = { width: 16_384, height: 2048, rate: [30, 1], side: 32_768, cycle: [1, 3] } as const;

/** The bytes of a page of WebAssembly memory, 64 KiB. */
const PAGE_BYTES = 65_536;

/** The most pages a 32-bit module addresses: 65 536 pages, 4 GiB. */
const MOST_PAGES = 65_536;

/** A side as tall as the image, in 65 536ths of its height. */
const FULL_SIDE = 65_536;

/**
 * Sets the largest image up in an instance of `bytes` whose memory was grown, from outside,
 * to within `free` pages ({@link PAGE_BYTES}) of the most a 32-bit module addresses
 * ({@link MOST_PAGES}), after a 3 × 2 square: the pages grown are never written, so the
 * machine does not hold them.
 * @param bytes The module.
 * @param drive How a driver of its instance is built.
 * @param free The pages left.
 * @returns What the setup threw, what the square it had gives before it and whether it
 *   gives the same after, what another image set up then gives, and the pages left.
 * @throws {Error} When the instance exports no memory.
 */
export async function largestImageIn(bytes: Uint8Array<ArrayBuffer>, drive: Drive, free: number): Promise<MemoryRefusal> {
  const { instance } = await WebAssembly.instantiate(bytes, {});
  const memory: unknown = instance.exports["memory"];
  if (!(memory instanceof WebAssembly.Memory)) {
    throw new Error("no memory");
  }
  const square = { width: 3, height: 2, rate: [60, 1], side: FULL_SIDE, cycle: [1, 2] } as const;
  const module = drive(instance.exports, square);
  memory.grow(MOST_PAGES - free - memory.buffer.byteLength / PAGE_BYTES);
  const left = MOST_PAGES - memory.buffer.byteLength / PAGE_BYTES;
  const before = answersAt(module);
  let refused: unknown;
  try {
    module.setup(REFUSED);
  } catch (error) {
    refused = error;
  }
  const after = answersAt(module).written;
  const changed = ANSWERS.filter((_name, at) => after[at] !== before.written[at]);
  module.setup({ ...square, ...AGAIN });
  return { refused, held: before.lengths, changed, again: [module.render(1n).length, module.display(1n).length], left };
}
