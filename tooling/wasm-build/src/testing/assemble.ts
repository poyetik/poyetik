// Hand-assembled WebAssembly modules for the tests of the instrumenter: every byte is
// written here, independently of src/instrument.ts, so that an expected module is a
// fact and never what the instrumenter produced. Exported to other packages' tests
// (`@poyetik/wasm-build/testing/assemble`): measured and mutated as the sources are
// (test/testing.test.ts).

/** Encodes an unsigned integer in LEB128 (an independent copy: tests never learn their answer from the code under test). */
export function leb(value: number): number[] {
  const bytes: number[] = [];
  let rest = value;
  do {
    const low = rest % 128;
    rest = Math.floor(rest / 128);
    bytes.push(rest > 0 ? low + 128 : low);
  } while (rest > 0);
  return bytes;
}

/** Encodes a name. */
export function name(text: string): number[] {
  const bytes = new TextEncoder().encode(text);
  return [...leb(bytes.length), ...bytes];
}

/** Wraps contents into a section. */
export function section(id: number, contents: readonly number[]): number[] {
  return [id, ...leb(contents.length), ...contents];
}

/** One function of an assembled module. */
export interface Func {
  /** Number of `i32` parameters. */
  readonly params: number;
  /** Whether it returns an `i32`. */
  readonly result: boolean;
  /** The local declarations, encoded (count of groups first). */
  readonly locals: readonly number[];
  /** The code, final `end` included. */
  readonly code: readonly number[];
  /** Its export name, if exported. */
  readonly exported?: string;
}

/** What an assembled module holds. */
export interface Assembly {
  /** The functions, each with its own type. */
  readonly functions: readonly Func[];
  /** Global entries, encoded; no global section when absent. */
  readonly globals?: readonly (readonly number[])[];
  /** Exports added after those of the functions: name, kind, index. */
  readonly exports?: readonly (readonly [string, number, number])[];
  /** Whether to write the export section (even empty). */
  readonly exportSection?: boolean;
  /** An import section, encoded contents, placed after the types. */
  readonly imports?: readonly number[];
  /** Whether to write the code section. */
  readonly codeSection?: boolean;
  /** Raw bytes appended to the code section, after the bodies. */
  readonly codeTrailer?: readonly number[];
  /** A custom section at the end: its name and contents. */
  readonly custom?: readonly [string, readonly number[]];
}

/** The counter global the instrumenter appends: a mutable `i64` starting at 0. */
export const COUNTER_ENTRY = [0x7e, 0x01, 0x42, 0x00, 0x0b] as const;

/** A mutable `i32` global starting at 0, as a module's own global. */
export const I32_GLOBAL = [0x7f, 0x01, 0x41, 0x00, 0x0b] as const;

/** The scratch global the instrumenter appends after the counter: a mutable `i32` starting at 0. */
export const SCRATCH_ENTRY = I32_GLOBAL;

/**
 * The instructions charging a bulk memory instruction, inserted right before it: its length
 * (the top of the stack) is kept in the scratch global `scratch`, the counter `counter` gains
 * ceil(length / 8) computed in 64 bits (`i64.extend_i32_u`, 7, `i64.add`, 3, `i64.shr_u`), and
 * the length is pushed back. Both indices below 128.
 */
export function charge(counter: number, scratch: number): number[] {
  return [0x24, scratch, 0x23, counter, 0x23, scratch, 0xad, 0x42, 0x07, 0x7c, 0x42, 0x03, 0x88, 0x7c, 0x24, counter, 0x23, scratch];
}

/** The instructions adding `count` (below 64) to global `index` (below 128). */
export function bump(index: number, count: number): number[] {
  return [0x23, index, 0x42, count, 0x7c, 0x24, index];
}

/** Assembles a module: types, imports, functions, a memory, globals, exports, code, custom. */
export function assemble(assembly: Assembly): Uint8Array<ArrayBuffer> {
  const { functions } = assembly;
  const types = functions.map((func) => [0x60, func.params, ...Array<number>(func.params).fill(0x7f), ...(func.result ? [0x01, 0x7f] : [0x00])]);
  const exports: number[][] = [];
  functions.forEach((func, index) => {
    if (func.exported !== undefined) {
      exports.push([...name(func.exported), 0x00, index]);
    }
  });
  for (const [exportName, kind, index] of assembly.exports ?? []) {
    exports.push([...name(exportName), kind, ...leb(index)]);
  }
  const bodies = functions.map((func) => {
    const body = [...func.locals, ...func.code];
    return [...leb(body.length), ...body];
  });
  const globals = assembly.globals;
  return new Uint8Array([
    ...[0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00],
    ...section(0x01, [...leb(types.length), ...types.flat()]),
    ...(assembly.imports === undefined ? [] : section(0x02, assembly.imports)),
    ...section(0x03, [...leb(functions.length), ...functions.map((_func, index) => index)]),
    ...section(0x05, [0x01, 0x00, 0x01]),
    ...(globals === undefined ? [] : section(0x06, [...leb(globals.length), ...globals.flat()])),
    ...(assembly.exportSection === false ? [] : section(0x07, [...leb(exports.length), ...exports.flat()])),
    ...(assembly.codeSection === false ? [] : section(0x0a, [...leb(bodies.length), ...bodies.flat(), ...(assembly.codeTrailer ?? [])])),
    ...(assembly.custom === undefined ? [] : section(0x00, [...name(assembly.custom[0]), ...assembly.custom[1]])),
  ]);
}
