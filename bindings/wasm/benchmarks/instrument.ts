// Counting the WebAssembly instructions a module executes, deterministically: the module is
// rewritten so that every straight-line run of its code adds its number of instructions to
// a global counter before it runs. The count depends only on the bytes of the module and on
// what it is asked to do, never on the machine, the browser engine or the load of the
// machine: the same on a developer's Mac and on CI. What it does not see (native code,
// memory and caches, vector width) is stated in spec/benchmarks.md.
//
// Only the instructions of the features the engine module is built with are understood
// (the MVP, sign extension, non-trapping conversions, bulk memory, reference types and
// multiple values); any other instruction is refused, never skipped, so that a new
// compiler feature cannot make the counts silently wrong.

/** The name under which the instrumented module exports its counter: a mutable `i64` global. */
export const COUNTER = "instructions";

/** Thrown when a module cannot be instrumented: one line saying why, and where. */
export class InstrumentError extends Error {
  /**
   * Describes why the module cannot be instrumented.
   * @param message What is wrong, with the byte offset when there is one.
   */
  constructor(message: string) {
    super(message);
    this.name = "InstrumentError";
  }
}

/** Reads bytes in order, refusing to read past the end. */
class Reader {
  /** The bytes read. */
  readonly #bytes: Uint8Array;
  /** Offset of these bytes in the module, for the error messages. */
  readonly #origin: number;
  /** Index of the next byte to read. */
  #position = 0;

  /**
   * Reads `bytes`, which start at byte `origin` of the module.
   * @param bytes The bytes to read.
   * @param origin Their offset in the module.
   */
  constructor(bytes: Uint8Array, origin: number) {
    this.#bytes = bytes;
    this.#origin = origin;
  }

  /** The offset in the module of the next byte to read. */
  get offset(): number {
    return this.#origin + this.#position;
  }

  /** Whether every byte has been read. */
  get done(): boolean {
    return this.#position === this.#bytes.length;
  }

  /** The bytes read from `start` (an index of this reader) to the current position. */
  since(start: number): Uint8Array {
    return this.#bytes.subarray(start, this.#position);
  }

  /** The index of the next byte to read, for {@link Reader.since}. */
  get position(): number {
    return this.#position;
  }

  /**
   * Reads one byte.
   * @throws {InstrumentError} at the end of the bytes.
   */
  byte(): number {
    const value = this.#bytes[this.#position];
    if (value === undefined) {
      throw new InstrumentError(`truncated at byte ${String(this.offset)}`);
    }
    this.#position += 1;
    return value;
  }

  /**
   * Reads `length` bytes.
   * @throws {InstrumentError} when fewer remain.
   */
  bytes(length: number): Uint8Array {
    if (length > this.#bytes.length - this.#position) {
      throw new InstrumentError(`truncated at byte ${String(this.offset)}: ${String(length)} bytes expected`);
    }
    const start = this.#position;
    this.#position += length;
    return this.#bytes.subarray(start, this.#position);
  }

  /**
   * Reads an unsigned LEB128 integer of at most 32 bits (at most 5 bytes).
   * @throws {InstrumentError} when it is longer, larger, or truncated.
   */
  u32(): number {
    const at = this.offset;
    let value = 0;
    for (let shift = 0; shift !== 35; shift += 7) {
      const byte = this.byte();
      value += (byte & 0x7f) * 2 ** shift;
      if (byte < 0x80) {
        if (value > 0xff_ff_ff_ff) {
          throw new InstrumentError(`integer at byte ${String(at)} exceeds 32 bits`);
        }
        return value;
      }
    }
    throw new InstrumentError(`integer at byte ${String(at)} is longer than 5 bytes`);
  }

  /**
   * Skips a signed LEB128 integer of at most `limit` bytes: its value is the compiler's
   * concern, its length is the instrumenter's.
   * @throws {InstrumentError} when it is longer, or truncated.
   */
  signed(limit: number): void {
    const at = this.offset;
    for (let read = 0; read !== limit; read += 1) {
      if (this.byte() < 0x80) {
        return;
      }
    }
    throw new InstrumentError(`integer at byte ${String(at)} is longer than ${String(limit)} bytes`);
  }

  /**
   * Reads an index that must be zero (a memory index, as the engine has one memory).
   * @throws {InstrumentError} when it is not.
   */
  zero(): void {
    const at = this.offset;
    if (this.u32() !== 0) {
      throw new InstrumentError(`memory index at byte ${String(at)} is not 0: only one memory is supported`);
    }
  }
}

/** Encodes an unsigned integer in LEB128. */
export function unsignedLeb(value: number): number[] {
  const bytes: number[] = [];
  let rest = value;
  for (;;) {
    const low = rest % 128;
    rest = Math.floor(rest / 128);
    if (rest === 0) {
      bytes.push(low);
      return bytes;
    }
    bytes.push(low | 0x80);
  }
}

/** Encodes a non-negative integer in signed LEB128 (the immediate of `i64.const`). */
export function signedLeb(value: number): number[] {
  const bytes: number[] = [];
  let rest = value;
  for (;;) {
    const low = rest % 128;
    rest = Math.floor(rest / 128);
    if (rest === 0 && low < 0x40) {
      bytes.push(low);
      return bytes;
    }
    bytes.push(low | 0x80);
  }
}

/** Appends `bytes` to `out`, one by one: spreading a large array into a call overflows the stack. */
function append(out: number[], bytes: Iterable<number>): void {
  for (const byte of bytes) {
    out.push(byte);
  }
}

/** Reads the immediates of an instruction, after its opcode. */
type Immediates = (reader: Reader) => void;

/** No immediate. */
const NONE: Immediates = () => undefined;

/** One index (a label, a function, a local, a global, a table, a type…). */
const INDEX: Immediates = (reader) => {
  reader.u32();
};

/** Two indices. */
const TWO_INDICES: Immediates = (reader) => {
  reader.u32();
  reader.u32();
};

/** A memory index, which must be 0. */
const MEMORY: Immediates = (reader) => {
  reader.zero();
};

/**
 * A block type: a signed 33-bit integer, the empty type (0x40) and the value types being
 * its one-byte negative values, a type index its non-negative ones.
 */
const BLOCK_TYPE: Immediates = (reader) => {
  reader.signed(5);
};

/** A memory argument: alignment, then offset (the alignment's bit 6 would name a memory). */
const MEMARG: Immediates = (reader) => {
  const at = reader.offset;
  if (reader.u32() >= 0x40) {
    throw new InstrumentError(`memory argument at byte ${String(at)} names a memory: only one memory is supported`);
  }
  reader.u32();
};

/** The immediates of the instructions without a prefix, by opcode, but the ranges of {@link immediatesOf}. */
const INSTRUCTIONS: ReadonlyMap<number, Immediates> = new Map<number, Immediates>([
  [0x00, NONE], // unreachable
  [0x01, NONE], // nop
  [0x02, BLOCK_TYPE], // block
  [0x03, BLOCK_TYPE], // loop
  [0x04, BLOCK_TYPE], // if
  [0x05, NONE], // else
  [0x0b, NONE], // end
  [0x0c, INDEX], // br
  [0x0d, INDEX], // br_if
  [
    0x0e, // br_table: the labels, then the default one
    (reader) => {
      const count = reader.u32();
      for (let label = 0; label <= count; label += 1) {
        reader.u32();
      }
    },
  ],
  [0x0f, NONE], // return
  [0x10, INDEX], // call
  [0x11, TWO_INDICES], // call_indirect: type, table
  [0x1a, NONE], // drop
  [0x1b, NONE], // select
  [
    0x1c, // select with its value types
    (reader) => {
      reader.bytes(reader.u32());
    },
  ],
  [0x20, INDEX], // local.get
  [0x21, INDEX], // local.set
  [0x22, INDEX], // local.tee
  [0x23, INDEX], // global.get
  [0x24, INDEX], // global.set
  [0x25, INDEX], // table.get
  [0x26, INDEX], // table.set
  [0x3f, MEMORY], // memory.size
  [0x40, MEMORY], // memory.grow
  [
    0x41, // i32.const
    (reader) => {
      reader.signed(5);
    },
  ],
  [
    0x42, // i64.const
    (reader) => {
      reader.signed(10);
    },
  ],
  [0x43, (reader) => void reader.bytes(4)], // f32.const
  [0x44, (reader) => void reader.bytes(8)], // f64.const
  [0xd0, (reader) => void reader.byte()], // ref.null: a reference type
  [0xd1, NONE], // ref.is_null
  [0xd2, INDEX], // ref.func
]);

/** The immediates of the instructions of the 0xfc prefix after the saturating truncations, by sub-opcode. */
const FC_INSTRUCTIONS: ReadonlyMap<number, Immediates> = new Map<number, Immediates>([
  [
    8, // memory.init: data segment, memory
    (reader) => {
      reader.u32();
      reader.zero();
    },
  ],
  [9, INDEX], // data.drop
  [
    10, // memory.copy: two memories
    (reader) => {
      reader.zero();
      reader.zero();
    },
  ],
  [11, MEMORY], // memory.fill
  [12, TWO_INDICES], // table.init: element segment, table
  [13, INDEX], // elem.drop
  [14, TWO_INDICES], // table.copy
  [15, INDEX], // table.grow
  [16, INDEX], // table.size
  [17, INDEX], // table.fill
]);

/**
 * The immediates of an instruction without a prefix: those listed, then the loads and
 * stores (0x28 to 0x3e: a memory argument) and the numeric instructions, sign extension
 * included (0x45 to 0xc4: none). `undefined` for any other.
 */
function immediatesOf(opcode: number): Immediates | undefined {
  if (opcode >= 0x28 && opcode <= 0x3e) {
    return MEMARG;
  }
  if (opcode >= 0x45 && opcode <= 0xc4) {
    return NONE;
  }
  return INSTRUCTIONS.get(opcode);
}

/**
 * The immediates of an instruction of the 0xfc prefix: the saturating truncations (0 to 7:
 * none), then those listed. `undefined` for any other.
 */
function fcImmediatesOf(code: number): Immediates | undefined {
  return code <= 7 ? NONE : FC_INSTRUCTIONS.get(code);
}

/**
 * Instructions that only mark structure (`block`, `loop`, `else`, `end`): they compute
 * nothing and are not counted, but they end a straight-line run, since a branch may land
 * right after them.
 */
const MARKERS: ReadonlySet<number> = new Set([0x02, 0x03, 0x05, 0x0b]);

/**
 * Counted instructions after which execution may continue elsewhere than at the next
 * instruction (`unreachable`, `if`, `br`, `br_if`, `br_table`, `return`): they end a
 * straight-line run. A call does not: execution comes back after it.
 */
const BRANCHES: ReadonlySet<number> = new Set([0x00, 0x04, 0x0c, 0x0d, 0x0e, 0x0f]);

/** The opcodes that open a structure closed by an `end`. */
const OPENERS: ReadonlySet<number> = new Set([0x02, 0x03, 0x04]);

/** One decoded instruction: its opcode (the first byte) and all its bytes. */
interface Instruction {
  /** The first byte. */
  readonly opcode: number;
  /** The whole instruction, immediates included. */
  readonly bytes: Uint8Array;
}

/**
 * Reads one instruction.
 * @throws {InstrumentError} for an instruction the instrumenter does not understand.
 */
function instruction(reader: Reader): Instruction {
  const start = reader.position;
  const at = reader.offset;
  const opcode = reader.byte();
  if (opcode === 0xfc) {
    const code = reader.u32();
    const immediates = fcImmediatesOf(code);
    if (immediates === undefined) {
      throw new InstrumentError(`unknown instruction 0xfc ${String(code)} at byte ${String(at)}`);
    }
    immediates(reader);
  } else {
    const immediates = immediatesOf(opcode);
    if (immediates === undefined) {
      throw new InstrumentError(`unknown instruction 0x${opcode.toString(16).padStart(2, "0")} at byte ${String(at)}`);
    }
    immediates(reader);
  }
  return { opcode, bytes: reader.since(start) };
}

/** The instructions adding `count` to the global `counter`. */
function bump(counter: number, count: number): number[] {
  const index = unsignedLeb(counter);
  return [0x23, ...index, 0x42, ...signedLeb(count), 0x7c, 0x24, ...index];
}

/**
 * Instruments one function body (its locals and its code, without its size).
 *
 * Every straight-line run of the code is preceded by the instructions adding its number
 * of counted instructions to the counter: a run starts where a branch may land (the
 * start of the function, after a structure marker, after a branch), so whenever any of
 * its instructions runs, all of them do, unless the module traps.
 * @throws {InstrumentError} when the body is truncated, holds an instruction the
 * instrumenter does not understand, or goes on after the end of the function.
 */
function instrumentBody(reader: Reader, counter: number): number[] {
  const localsStart = reader.position;
  const groups = reader.u32();
  for (let group = 0; group !== groups; group += 1) {
    reader.u32();
    reader.byte();
  }
  const out = [...reader.since(localsStart)];
  let run: number[] = [];
  let counted = 0;
  let depth = 0;
  while (depth >= 0) {
    const { opcode, bytes } = instruction(reader);
    append(run, bytes);
    if (!MARKERS.has(opcode)) {
      counted += 1;
    }
    if (OPENERS.has(opcode)) {
      depth += 1;
    } else if (opcode === 0x0b) {
      depth -= 1;
    }
    if (MARKERS.has(opcode) || BRANCHES.has(opcode)) {
      if (counted !== 0) {
        append(out, bump(counter, counted));
      }
      append(out, run);
      run = [];
      counted = 0;
    }
  }
  if (!reader.done) {
    throw new InstrumentError(`code after the end of the function at byte ${String(reader.offset)}`);
  }
  return out;
}

/** A section of a module: its id and its contents. */
interface Section {
  /** The section id. */
  readonly id: number;
  /** The contents, without the id and the size. */
  readonly contents: Uint8Array;
  /** Offset of the contents in the module. */
  readonly origin: number;
}

/** The magic number and version 1 that every module starts with. */
const PREAMBLE = [0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00] as const;

/** The import section id. */
const IMPORT = 2;
/** The global section id. */
const GLOBAL = 6;
/** The export section id. */
const EXPORT = 7;
/** The code section id. */
const CODE = 10;

/**
 * Splits a module into its sections.
 * @throws {InstrumentError} when it is not a version 1 module, or is truncated.
 */
function sections(module: Uint8Array): Section[] {
  const reader = new Reader(module, 0);
  if (PREAMBLE.some((byte, index) => module[index] !== byte)) {
    throw new InstrumentError("not a WebAssembly module of version 1");
  }
  reader.bytes(PREAMBLE.length);
  const found: Section[] = [];
  while (!reader.done) {
    const id = reader.byte();
    const size = reader.u32();
    const origin = reader.offset;
    found.push({ id, contents: reader.bytes(size), origin });
  }
  return found;
}

/** Wraps contents into a section. */
function section(id: number, contents: readonly number[]): number[] {
  return [id, ...unsignedLeb(contents.length), ...contents];
}

/** Reads a vector's count, then returns it with the bytes that follow it. */
function vector(found: Section): { readonly count: number; readonly rest: Uint8Array } {
  const reader = new Reader(found.contents, found.origin);
  const count = reader.u32();
  return { count, rest: reader.bytes(found.contents.length - reader.position) };
}

/**
 * The export section with the counter appended, as global `counter`.
 * @throws {InstrumentError} when the module already exports something under its name.
 */
function exportsWithCounter(found: Section, counter: number): number[] {
  const reader = new Reader(found.contents, found.origin);
  const count = reader.u32();
  const name = new TextEncoder().encode(COUNTER);
  for (let entry = 0; entry !== count; entry += 1) {
    const exported = reader.bytes(reader.u32());
    reader.byte();
    reader.u32();
    if (exported.length === name.length && exported.every((byte, index) => byte === name[index])) {
      throw new InstrumentError(`the module already exports "${COUNTER}"`);
    }
  }
  if (!reader.done) {
    throw new InstrumentError(`bytes after the exports at byte ${String(reader.offset)}`);
  }
  return [...unsignedLeb(count + 1), ...vector(found).rest, ...unsignedLeb(name.length), ...name, 0x03, ...unsignedLeb(counter)];
}

/**
 * The code section with every function body instrumented.
 * @throws {InstrumentError} as {@link instrumentBody}, or when the section is malformed.
 */
function instrumentedCode(found: Section, counter: number): number[] {
  const reader = new Reader(found.contents, found.origin);
  const count = reader.u32();
  const out = unsignedLeb(count);
  for (let index = 0; index !== count; index += 1) {
    const size = reader.u32();
    const origin = reader.offset;
    const body = instrumentBody(new Reader(reader.bytes(size), origin), counter);
    append(out, unsignedLeb(body.length));
    append(out, body);
  }
  if (!reader.done) {
    throw new InstrumentError(`bytes after the function bodies at byte ${String(reader.offset)}`);
  }
  return out;
}

/** The counter global: a mutable `i64` starting at 0. */
const COUNTER_GLOBAL = [0x7e, 0x01, 0x42, 0x00, 0x0b] as const;

/**
 * Returns `module` rewritten to count the instructions it executes, in a mutable `i64`
 * global exported as {@link COUNTER}.
 *
 * What is counted: every instruction executed, except the structure markers `block`,
 * `loop`, `else` and `end`, which compute nothing. Everything else of the module is kept
 * as it is (custom sections included), so it computes exactly the same results.
 * @throws {InstrumentError} when the module is not a version 1 module, imports anything
 * (the counter's index would have to follow the imported globals: only self-contained
 * modules are counted), exports nothing or already exports a {@link COUNTER}, has no
 * code, or holds an instruction the instrumenter does not understand.
 */
export function instrument(module: Uint8Array): Uint8Array<ArrayBuffer> {
  const found = sections(module);
  const imports = found.find((candidate) => candidate.id === IMPORT);
  if (imports !== undefined && vector(imports).count !== 0) {
    throw new InstrumentError("the module imports something: only self-contained modules are counted");
  }
  const globals = found.find((candidate) => candidate.id === GLOBAL);
  const counter = globals === undefined ? 0 : vector(globals).count;
  const exports = found.find((candidate) => candidate.id === EXPORT);
  if (exports === undefined) {
    throw new InstrumentError("the module exports nothing");
  }
  if (!found.some((candidate) => candidate.id === CODE)) {
    throw new InstrumentError("the module has no code");
  }
  const out: number[] = [...PREAMBLE];
  for (const current of found) {
    if (current.id === EXPORT) {
      if (globals === undefined) {
        append(out, section(GLOBAL, [1, ...COUNTER_GLOBAL]));
      }
      append(out, section(EXPORT, exportsWithCounter(current, counter)));
    } else if (current.id === GLOBAL) {
      const { rest } = vector(current);
      append(out, section(GLOBAL, [...unsignedLeb(counter + 1), ...rest, ...COUNTER_GLOBAL]));
    } else if (current.id === CODE) {
      append(out, section(CODE, instrumentedCode(current, counter)));
    } else {
      append(out, section(current.id, [...current.contents]));
    }
  }
  return Uint8Array.from(out);
}
