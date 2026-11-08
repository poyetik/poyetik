// Shared test data: the built engine module, the conformance corpus and the shared test
// matrix, read from their files and never retyped.
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

/** The conformance corpus directory, resolved once when the fixtures load. */
const CONFORMANCE = conformance();

/** The built engine module (`pnpm build` writes it). */
export function wasmBytes(): ArrayBuffer {
  const file = readFileSync(new URL("../dist/engine.wasm", import.meta.url));
  return file.buffer.slice(file.byteOffset, file.byteOffset + file.byteLength);
}

/** Reads a field of a JSON object. */
function field(json: unknown, key: string): unknown {
  return typeof json === "object" && json !== null ? Reflect.get(json, key) : undefined;
}

/** The native reference hashes of the corpus, by case name, as parsed JSON. */
const references: unknown = JSON.parse(readFileSync(new URL("references.json", CONFORMANCE), "utf8"));

/** The patch file of the sine-440 case, byte for byte: never parsed and written again. */
export const SINE_PATCH = new Uint8Array(readFileSync(new URL("patches/sine-440.json", CONFORMANCE)));

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

/** The keys of the shared test matrix, each read once below: a key added to the file fails here until it is read. */
const MATRIX_KEYS = ["host_buffer_sizes", "sample_rates"] as const;

/** The shared test matrix (conformance/test-matrix.json), parsed JSON. */
const matrix: unknown = JSON.parse(readFileSync(new URL("test-matrix.json", CONFORMANCE), "utf8"));
if (typeof matrix !== "object" || matrix === null || Object.keys(matrix).sort().join() !== MATRIX_KEYS.join()) {
  throw new Error(`the test matrix must have exactly the keys ${MATRIX_KEYS.join(", ")}`);
}

/** One key of the shared test matrix: a list of positive whole numbers. */
function matrixValues(key: (typeof MATRIX_KEYS)[number]): readonly number[] {
  const values = field(matrix, key);
  if (!Array.isArray(values) || !values.every((value): value is number => Number.isSafeInteger(value) && Number(value) > 0)) {
    throw new Error(`invalid test matrix: ${key} is not a list of positive whole numbers`);
  }
  return values;
}

/** Every sample rate of the shared test matrix, in hertz. */
export const SAMPLE_RATES: readonly number[] = matrixValues("sample_rates");

/** Every host buffer size of the shared test matrix, in frames: how hosts split time. */
export const HOST_BUFFER_SIZES: readonly number[] = matrixValues("host_buffer_sizes");

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
  "engine_abi_version",
  "engine_input",
  "engine_load",
  "engine_message_ptr",
  "engine_message_len",
  "engine_channels",
  "engine_block_size",
  "engine_process",
] as const;

/** Encodes a WebAssembly name. */
function encodedName(text: string): number[] {
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

/** A function of a hand-made module. */
export interface HandMadeFunction {
  readonly name: string;
  /** Number of `i32` parameters. */
  readonly params: number;
  /**
   * An `i32` result (a JavaScript number), an `i64` one (a BigInt), a trap (typed `i32`),
   * `grow`: grows the memory by one page, writes 0.5 (f32) at address 0, returns 0; or
   * `alternate`: writes 0.25 (f32) at address 2048, then returns 4096 and 2048 in turn
   * (which one is kept at address 1024, away from the patch copied from address 1): a
   * block that moves without the memory growing.
   */
  readonly body: { readonly i32: number } | "i64" | "trap" | "grow" | "alternate";
}

/** The arity of each engine export. */
const ARITY: Readonly<Record<string, number>> = { engine_input: 1, engine_load: 1 };

/**
 * The engine functions with their arity, each returning `i32` 1 (or `body`), except the
 * interface version, which gives the expected one unless `body` is given.
 */
export function engineFunctions(body?: HandMadeFunction["body"]): HandMadeFunction[] {
  return FUNCTIONS.map((name) => ({
    name,
    params: ARITY[name] ?? 0,
    body: body ?? (name === "engine_abi_version" ? { i32: 0x5059_0001 } : { i32: 1 }),
  }));
}

/** Encodes a signed integer in LEB128. */
function sleb128(value: number): number[] {
  const bytes: number[] = [];
  let rest = value;
  for (;;) {
    const low = rest & 0x7f;
    rest >>= 7;
    const done = (rest === 0 && (low & 0x40) === 0) || (rest === -1 && (low & 0x40) !== 0);
    bytes.push(done ? low : low | 0x80);
    if (done) {
      return bytes;
    }
  }
}

/** What a hand-made module holds: see {@link handMadeBytes}. */
export interface HandMade {
  readonly memory: boolean;
  readonly imports?: boolean;
  readonly functions: readonly HandMadeFunction[];
}

/**
 * Encodes a small module by hand: optionally a memory, an imported function (which no
 * engine module has), and `functions`, each with its own type and body.
 */
export function handMadeBytes(options: HandMade): Uint8Array<ArrayBuffer> {
  const types = options.functions.map((func) => [0x60, func.params, ...Array<number>(func.params).fill(0x7f), 0x01, func.body === "i64" ? 0x7e : 0x7f]);
  const bodies = options.functions.map((func) => {
    const grow = [0x41, 0x01, 0x40, 0x00, 0x1a, 0x41, 0x00, 0x43, 0x00, 0x00, 0x00, 0x3f, 0x38, 0x02, 0x00, 0x41, 0x00];
    const alternate = [
      ...[0x41, 0x80, 0x10, 0x43, 0x00, 0x00, 0x80, 0x3e, 0x38, 0x02, 0x00],
      ...[0x41, 0x80, 0x08, 0x41, 0x80, 0x10, 0x41, 0x80, 0x08, 0x28, 0x02, 0x00, 0x6b, 0x36, 0x02, 0x00],
      ...[0x41, 0x80, 0x08, 0x28, 0x02, 0x00, 0x41, 0x80, 0x10, 0x6a],
    ];
    const code =
      func.body === "trap"
        ? [0x00]
        : func.body === "i64"
          ? [0x42, 0x00]
          : func.body === "grow"
            ? grow
            : func.body === "alternate"
              ? alternate
              : [0x41, ...sleb128(func.body.i32)];
    return [code.length + 2, 0x00, ...code, 0x0b];
  });
  const imported = options.imports === true ? 1 : 0;
  const exports: number[][] = [];
  if (options.memory) {
    exports.push([...encodedName("memory"), 0x02, 0x00]);
  }
  options.functions.forEach((func, index) => {
    exports.push([...encodedName(func.name), 0x00, index + imported]);
  });
  const typeSection = options.imports === true ? [...types, [0x60, 0x00, 0x00]] : types;
  const bytes = [
    ...[0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00],
    ...section(0x01, [typeSection.length, ...typeSection.flat()]),
    ...(options.imports === true ? section(0x02, [0x01, ...encodedName("env"), ...encodedName("f"), 0x00, types.length]) : []),
    ...section(0x03, [options.functions.length, ...options.functions.map((_func, index) => index)]),
    ...(options.memory ? section(0x05, [0x01, 0x00, 0x01]) : []),
    ...section(0x07, [exports.length, ...exports.flat()]),
    ...section(0x0a, [bodies.length, ...bodies.flat()]),
  ];
  return new Uint8Array(bytes);
}

/** Compiles the module {@link handMadeBytes} encodes. */
export function handMadeModule(options: HandMade): WebAssembly.Module {
  return new WebAssembly.Module(handMadeBytes(options));
}

/** Builds a patch: one oscillator per frequency, each into its own inlet of one output. */
export function oscillators(frequencies: readonly number[]): Uint8Array {
  const nodes = [{ id: 0, box: "dac~" }, ...frequencies.map((frequency, index) => ({ id: index + 1, box: "osc~", params: { frequency } }))];
  const connections = frequencies.map((_frequency, index) => ({ from: [index + 1, 0], to: [0, index] }));
  return new TextEncoder().encode(JSON.stringify({ format: 1, nodes, connections }));
}
