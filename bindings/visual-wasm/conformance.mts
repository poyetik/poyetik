/**
 * The visual conformance cases rendered by the WebAssembly module, in Node: every case of
 * conformance/visual/cases must give the image hash of conformance/visual/references.json,
 * which the native renderer (visual-cli) recorded. Same bits natively and in WebAssembly.
 *
 * Usage: node bindings/visual-wasm/conformance.mts <visual.wasm> <conformance/visual>
 * Exit codes: 0 when every hash matches, 1 when one differs or cannot be computed, 2 for a
 * usage error; every error is one `error:` line on stderr.
 */
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

/** The version of the module's interface this script drives. */
const ABI_VERSION = 0x5056_0002;

/** Reads the export `name` of `exports` as a function of numbers, or throws. */
function exported(exports: WebAssembly.Exports, name: string): (...args: readonly number[]) => number {
  const value = exports[name];
  if (typeof value !== "function") {
    throw new Error(`the visual module has no function ${name}`);
  }
  return (...args) => {
    const result: unknown = Reflect.apply(value, undefined, args);
    if (typeof result !== "number") {
      throw new TypeError(`${name} returned a ${typeof result}, not a number`);
    }
    return result >>> 0;
  };
}

/** A whole number read from `value`, or throws naming `what`. */
function whole(value: unknown, what: string): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) {
    throw new Error(`${what} must be a whole number below 2^53`);
  }
  return value;
}

/** A pair of whole numbers read from `value`, or throws naming `what`. */
function pair(value: unknown, what: string): [number, number] {
  if (!Array.isArray(value) || value.length !== 2) {
    throw new Error(`${what} must be a pair of numbers`);
  }
  return [whole(value[0], what), whole(value[1], what)];
}

/** The image hash of the case `text`, rendered by the module `bytes`, in hexadecimal. */
async function hashOf(bytes: Uint8Array<ArrayBuffer>, text: string): Promise<string> {
  const { instance } = await WebAssembly.instantiate(bytes, {});
  const { exports } = instance;
  const memory = exports["memory"];
  if (!(memory instanceof WebAssembly.Memory)) {
    throw new Error("the visual module exports no memory");
  }
  if (exported(exports, "visual_abi_version")() !== ABI_VERSION) {
    throw new Error("the visual module has another interface version");
  }
  const case_: unknown = JSON.parse(text);
  const field = (name: string): unknown => Reflect.get(Object(case_), name);
  const square: unknown = field("square");
  const [rateNumerator, rateDenominator] = pair(field("rate"), "rate");
  const [cycleNumerator, cycleDenominator] = pair(Reflect.get(Object(square), "cycle"), "square.cycle");
  const side = whole(Reflect.get(Object(square), "side"), "square.side");
  const ok = exported(exports, "visual_setup")(whole(field("width"), "width"), whole(field("height"), "height"), rateNumerator, rateDenominator, side, cycleNumerator, cycleDenominator);
  if (ok !== 1) {
    throw new Error("the module refused the case");
  }
  const first = whole(field("first"), "first");
  const images = whole(field("images"), "images");
  const address = exported(exports, "visual_hash")(first % 2 ** 32, Math.floor(first / 2 ** 32), images);
  return Array.from(new Uint8Array(memory.buffer, address, 32), (byte) => byte.toString(16).padStart(2, "0")).join("");
}

/** Checks every case; returns the exit code. */
async function main(args: readonly string[]): Promise<number> {
  const [wasm, dir] = args;
  if (wasm === undefined || dir === undefined || args.length !== 2) {
    process.stderr.write("error: usage: node conformance.mts <visual.wasm> <conformance/visual>\n");
    return 2;
  }
  const bytes = new Uint8Array(readFileSync(wasm));
  const references: unknown = JSON.parse(readFileSync(join(dir, "references.json"), "utf8"));
  let problems = 0;
  const names = readdirSync(join(dir, "cases")).filter((file) => file.endsWith(".json")).sort();
  for (const file of names) {
    const name = file.slice(0, -".json".length);
    const expected: unknown = Reflect.get(Object(references), name);
    const hash = await hashOf(bytes, readFileSync(join(dir, "cases", file), "utf8"));
    if (hash === expected) {
      process.stdout.write(`ok ${name} ${hash}\n`);
    } else {
      process.stdout.write(`differs ${name}: ${hash} in WebAssembly, reference ${String(expected)}\n`);
      problems += 1;
    }
  }
  if (problems > 0 || names.length === 0) {
    process.stderr.write(`error: ${String(problems)} of ${String(names.length)} visual cases differ in WebAssembly, listed above\n`);
    return 1;
  }
  return 0;
}

process.exitCode = await main(process.argv.slice(2)).catch((error: unknown) => {
  process.stderr.write(`error: ${error instanceof Error ? error.message : String(error)}\n`);
  return 1;
});
