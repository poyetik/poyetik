// Shared test data: the built engine module and the sine-440 conformance case.
import { existsSync, readdirSync, readFileSync } from "node:fs";

/**
 * The conformance corpus, found by walking up from this file: the tests also run from
 * a copy of the package (mutation testing sandbox), deeper in the repository.
 */
function conformance(): URL {
  for (let directory = new URL("../", import.meta.url); directory.pathname !== "/"; directory = new URL("../", directory)) {
    const candidate = new URL("conformance/", directory);
    if (existsSync(new URL("references.json", candidate))) {
      return candidate;
    }
  }
  throw new Error("conformance corpus not found");
}

const CONFORMANCE = conformance();

/** The built engine module (`pnpm build` writes it). */
export function wasmBytes(): ArrayBuffer {
  const file = readFileSync(new URL("../dist/engine.wasm", import.meta.url));
  return file.buffer.slice(file.byteOffset, file.byteOffset + file.byteLength);
}

/** Reads a field of a JSON object. */
function field(json: unknown, name: string): unknown {
  return typeof json === "object" && json !== null ? Reflect.get(json, name) : undefined;
}

const sineCase: unknown = JSON.parse(readFileSync(new URL("cases/sine-440.json", CONFORMANCE), "utf8"));
const references: unknown = JSON.parse(readFileSync(new URL("references.json", CONFORMANCE), "utf8"));

/** The patch file of the sine-440 case, byte for byte: never parsed and written again. */
export const SINE_PATCH = new Uint8Array(readFileSync(new URL("patches/sine-440.json", CONFORMANCE)));

/** Duration of the sine-440 case, in frames. */
export const SINE_FRAMES = Number(field(sineCase, "frames"));

/** The native reference hash of the sine-440 case. */
export const SINE_HASH = String(field(references, "sine-440"));

/** A case of the conformance corpus, read from its files. */
export interface CorpusCase {
  readonly name: string;
  readonly rate: number;
  readonly frames: number;
  /** The bytes of its patch file, unchanged. */
  readonly patch: Uint8Array;
}

/** The names of the `.json` files of a corpus directory, sorted. */
function jsonNames(directory: string): string[] {
  return readdirSync(new URL(directory, CONFORMANCE))
    .filter((file) => file.endsWith(".json"))
    .map((file) => file.slice(0, -".json".length))
    .sort();
}

/** Every case of the corpus, sorted by name. */
export const CORPUS: readonly CorpusCase[] = jsonNames("cases/").map((name) => {
  const json: unknown = JSON.parse(readFileSync(new URL(`cases/${name}.json`, CONFORMANCE), "utf8"));
  return {
    name,
    rate: Number(field(json, "sample_rate")),
    frames: Number(field(json, "frames")),
    patch: new Uint8Array(readFileSync(new URL(`patches/${name}.json`, CONFORMANCE))),
  };
});

/** The names of the patch files of the corpus, sorted. */
export const PATCH_NAMES: readonly string[] = jsonNames("patches/");

/** The native reference hashes of the corpus, by case name. */
export const REFERENCES: ReadonlyMap<string, string> = new Map(
  typeof references === "object" && references !== null ? Object.entries(references).map(([name, hash]) => [name, String(hash)]) : [],
);

/** A patch the engine rejects: an unknown box. */
export const UNKNOWN_BOX = new TextEncoder().encode('{"format":1,"nodes":[{"id":1,"box":"nope~"}],"connections":[]}');

/** A valid patch with no output. */
export const SILENT_PATCH = new TextEncoder().encode('{"format":1,"nodes":[{"id":1,"box":"osc~"}],"connections":[]}');

/** The names of every function the engine module exports. */
export const FUNCTIONS = [
  "engine_input",
  "engine_load",
  "engine_message_ptr",
  "engine_message_len",
  "engine_channels",
  "engine_block_size",
  "engine_process",
] as const;

/** Encodes a WebAssembly name. */
function name(text: string): number[] {
  return [text.length, ...new TextEncoder().encode(text)];
}

/** Encodes an unsigned integer in LEB128. */
function leb128(value: number): number[] {
  const bytes: number[] = [];
  let rest = value;
  do {
    const low = rest % 128;
    rest = Math.floor(rest / 128);
    bytes.push(rest > 0 ? low + 128 : low);
  } while (rest > 0);
  return bytes;
}

/** Wraps section contents with their id and size. */
function section(id: number, contents: number[]): number[] {
  return [id, ...leb128(contents.length), ...contents];
}

/**
 * Builds a small module by hand: optionally a memory, and optionally one function
 * returning an i64 (a BigInt in JavaScript), exported under each of `functions`.
 */
export function handMadeModule(options: { memory: boolean; functions: readonly string[] }): WebAssembly.Module {
  const exports: number[][] = [];
  if (options.memory) {
    exports.push([...name("memory"), 0x02, 0x00]);
  }
  for (const exported of options.functions) {
    exports.push([...name(exported), 0x00, 0x00]);
  }
  const bytes = [
    ...[0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00],
    ...(options.functions.length > 0 ? section(0x01, [0x01, 0x60, 0x00, 0x01, 0x7e]) : []),
    ...(options.functions.length > 0 ? section(0x03, [0x01, 0x00]) : []),
    ...(options.memory ? section(0x05, [0x01, 0x00, 0x01]) : []),
    ...section(0x07, [exports.length, ...exports.flat()]),
    ...(options.functions.length > 0 ? section(0x0a, [0x01, 0x04, 0x00, 0x42, 0x00, 0x0b]) : []),
  ];
  return new WebAssembly.Module(new Uint8Array(bytes));
}

/** Builds a patch: one oscillator per frequency, each into its own inlet of one output. */
export function oscillators(frequencies: readonly number[]): Uint8Array {
  const nodes = [{ id: 0, box: "dac~" }, ...frequencies.map((frequency, index) => ({ id: index + 1, box: "osc~", params: { frequency } }))];
  const connections = frequencies.map((_, index) => ({ from: [index + 1, 0], to: [0, index] }));
  return new TextEncoder().encode(JSON.stringify({ format: 1, nodes, connections }));
}
