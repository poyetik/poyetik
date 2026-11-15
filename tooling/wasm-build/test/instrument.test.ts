import { describe, expect, test } from "vitest";
import { COUNTER, InstrumentError, instrument, signedLeb, unsignedLeb } from "../src/instrument.ts";
import { assemble, bump, charge, COUNTER_ENTRY, I32_GLOBAL, SCRATCH_ENTRY, type Func } from "./assemble.ts";

/** A function of no parameter and no result, exported as `f`, with `code`. */
function plain(code: readonly number[]): Func {
  return { params: 0, result: false, locals: [0x00], code, exported: "f" };
}

/** The globals the instrumenter appends: the counter, then the scratch of the bulk memory charges. */
const ADDED = [COUNTER_ENTRY, SCRATCH_ENTRY] as const;

/** The expected instrumentation of a module of one function `f` and no global: the counter is global 0, the scratch global 1. */
function expectedOne(code: readonly number[]): Uint8Array {
  return assemble({ functions: [plain(code)], globals: ADDED, exports: [[COUNTER, 0x03, 0]] });
}

/** Instruments a module of one exported function with `code`, and no global. */
function instrumentOne(code: readonly number[]): Uint8Array {
  return instrument(assemble({ functions: [plain(code)] }));
}

/** Every instruction without a prefix that does not open a structure nor branch, with representative immediates. */
const STRAIGHT: readonly (readonly number[])[] = [
  [0x01],
  [0x10, 0x00],
  [0x11, 0x81, 0x01, 0x00],
  [0x1a],
  [0x1b],
  [0x1c, 0x02, 0x7f, 0x7e],
  ...[0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26].map((opcode) => [opcode, 0x85, 0x01]),
  ...Array.from({ length: 0x3e - 0x28 + 1 }, (_entry, index) => [0x28 + index, 0x02, 0x90, 0x03]),
  [0x3f, 0x00],
  [0x40, 0x00],
  [0x41, 0x80, 0x80, 0x80, 0x80, 0x78],
  [0x42, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x7f],
  [0x43, 0x00, 0x00, 0x80, 0x3f],
  [0x44, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xf0, 0x3f],
  ...Array.from({ length: 0xc4 - 0x45 + 1 }, (_entry, index) => [0x45 + index]),
  [0xd0, 0x70],
  [0xd1],
  [0xd2, 0x83, 0x01],
  ...[0, 1, 2, 3, 4, 5, 6, 7].map((code) => [0xfc, code]),
  [0xfc, 9, 0x82, 0x01],
  [0xfc, 13, 0x82, 0x01],
  [0xfc, 15, 0x82, 0x01],
  [0xfc, 16, 0x82, 0x01],
];

/**
 * Every bulk memory instruction, whose work grows with the length it is given: `memory.init`,
 * `memory.copy` and `memory.fill`, and `memory.copy` with its sub-opcode written in two bytes.
 */
const BULK: readonly (readonly number[])[] = [
  [0xfc, 8, 0x82, 0x01, 0x00],
  [0xfc, 10, 0x00, 0x00],
  [0xfc, 11, 0x00],
  [0xfc, 0x8a, 0x00, 0x00, 0x00],
];

/** Every counted instruction after which execution may go elsewhere, but `if`. */
const BRANCHING: readonly (readonly number[])[] = [[0x00], [0x0c, 0x81, 0x01], [0x0d, 0x81, 0x01], [0x0e, 0x02, 0x00, 0x81, 0x01, 0x00], [0x0f]];

/** Every way of writing a block type: empty, each value type, a type index in one byte and in two. */
const BLOCK_TYPES: readonly (readonly number[])[] = [[0x40], [0x7f], [0x7e], [0x7d], [0x7c], [0x7b], [0x70], [0x6f], [0x00], [0x80, 0x01]];

describe("instrument", () => {
  test.each(STRAIGHT.map((instruction) => [instruction.map((byte) => byte.toString(16)).join(" "), instruction]))(
    "counts %s with the instruction after it, in one run",
    (_name, instruction) => {
      expect(instrumentOne([...instruction, 0x01, 0x0b])).toStrictEqual(expectedOne([...bump(0, 2), ...instruction, 0x01, 0x0b]));
    },
  );

  test.each(BULK.map((instruction) => [instruction.map((byte) => byte.toString(16)).join(" "), instruction]))(
    "counts %s as one instruction of its run, and charges the length it is given right before it",
    (_name, instruction) => {
      expect(instrumentOne([...instruction, 0x01, 0x0b])).toStrictEqual(expectedOne([...bump(0, 2), ...charge(0, 1), ...instruction, 0x01, 0x0b]));
    },
  );

  test("charges every bulk memory instruction of a run, the scratch global following the module's own globals and the counter", () => {
    const globals = Array.from({ length: 3 }, () => I32_GLOBAL);
    const code = [0xfc, 11, 0x00, 0xfc, 10, 0x00, 0x00, 0x0b];
    expect(instrument(assemble({ functions: [plain(code)], globals }))).toStrictEqual(
      assemble({
        functions: [plain([...bump(3, 2), ...charge(3, 4), 0xfc, 11, 0x00, ...charge(3, 4), 0xfc, 10, 0x00, 0x00, 0x0b])],
        globals: [...globals, ...ADDED],
        exports: [[COUNTER, 0x03, 3]],
      }),
    );
  });

  test.each(BRANCHING.map((instruction) => [instruction.map((byte) => byte.toString(16)).join(" "), instruction]))(
    "counts %s, then starts a new run after it",
    (_name, instruction) => {
      expect(instrumentOne([...instruction, 0x01, 0x0b])).toStrictEqual(expectedOne([...bump(0, 1), ...instruction, ...bump(0, 1), 0x01, 0x0b]));
    },
  );

  test.each(BLOCK_TYPES.flatMap((type) => [0x02, 0x03].map((opcode) => [opcode, type] as const)))("does not count the marker 0x%s of block type %j, and starts a run inside it", (opcode, type) => {
    expect(instrumentOne([0x01, opcode, ...type, 0x01, 0x0b, 0x01, 0x0b])).toStrictEqual(
      expectedOne([...bump(0, 1), 0x01, opcode, ...type, ...bump(0, 1), 0x01, 0x0b, ...bump(0, 1), 0x01, 0x0b]),
    );
  });

  test.each(BLOCK_TYPES)("counts `if` of block type %j, then each arm as its own run, `else` and `end` uncounted", (...type) => {
    expect(instrumentOne([0x01, 0x04, ...type, 0x01, 0x01, 0x05, 0x01, 0x0b, 0x0b])).toStrictEqual(
      expectedOne([...bump(0, 2), 0x01, 0x04, ...type, ...bump(0, 2), 0x01, 0x01, 0x05, ...bump(0, 1), 0x01, 0x0b, 0x0b]),
    );
  });

  test("adds nothing to a function that does nothing", () => {
    expect(instrumentOne([0x0b])).toStrictEqual(expectedOne([0x0b]));
  });

  test("keeps the locals, the module's own globals and exports, and its custom sections", () => {
    const locals = [0x02, 0x01, 0x7f, 0x83, 0x01, 0x7c];
    const func: Func = { params: 1, result: false, locals, code: [0x01, 0x0b], exported: "g" };
    const globals = Array.from({ length: 130 }, () => I32_GLOBAL);
    const custom = ["name", [0x01, 0x02, 0x03]] as const;
    expect(instrument(assemble({ functions: [func, func], globals, exports: [["memory", 0x02, 0]], custom }))).toStrictEqual(
      assemble({
        functions: [
          { ...func, code: [0x23, 0x82, 0x01, 0x42, 0x01, 0x7c, 0x24, 0x82, 0x01, 0x01, 0x0b] },
          { ...func, code: [0x23, 0x82, 0x01, 0x42, 0x01, 0x7c, 0x24, 0x82, 0x01, 0x01, 0x0b] },
        ],
        globals: [...globals, ...ADDED],
        exports: [
          ["memory", 0x02, 0],
          [COUNTER, 0x03, 130],
        ],
        custom,
      }),
    );
  });

  test("accepts an empty import section", () => {
    expect(instrument(assemble({ functions: [plain([0x0b])], imports: [0x00] }))).toStrictEqual(
      assemble({ functions: [plain([0x0b])], imports: [0x00], globals: ADDED, exports: [[COUNTER, 0x03, 0]] }),
    );
  });

  test("accepts names close to the counter's", () => {
    for (const close of ["instruction", "instructionz", "instructionss", "Instructions"]) {
      expect(instrument(assemble({ functions: [plain([0x0b])], exports: [[close, 0x02, 0]] }))).toStrictEqual(
        assemble({ functions: [plain([0x0b])], globals: ADDED, exports: [[close, 0x02, 0], [COUNTER, 0x03, 0]] }),
      );
    }
  });
});

describe("counting", () => {
  /**
   * Three functions, each counted by hand. `sum(n)` adds n + (n - 1) + … + 1 in a loop: each
   * pass runs `local.get`, `i32.eqz`, `br_if` (3), then nine instructions; the last test
   * runs the first three only, then `local.get` returns: 12n + 4. `pick(c)` runs
   * `local.get`, `if`, then `i32.const` when c is not 0 (3), else `i32.const`, `i32.const`,
   * `i32.add` (5). `twice()` runs `i32.const`, `call` and `pick(0)`: 7. `copy(n)` and
   * `fill(n)` run `i32.const`, `i32.const`, `local.get` and their bulk instruction (4), which
   * is charged ceil(n / 8) more. `fills(k)` fills 9 bytes k times: each pass runs
   * `local.get`, `i32.eqz`, `br_if` (3), three `i32.const` and `memory.fill` (4, charged 2
   * more), then `local.get`, `i32.const`, `i32.sub`, `local.set`, `br` (5); the last test
   * runs the first three only: 14k + 3.
   */
  const functions: readonly Func[] = [
    {
      params: 1,
      result: true,
      locals: [0x01, 0x01, 0x7f],
      code: [
        ...[0x02, 0x40, 0x03, 0x40],
        ...[0x20, 0x00, 0x45, 0x0d, 0x01],
        ...[0x20, 0x01, 0x20, 0x00, 0x6a, 0x21, 0x01, 0x20, 0x00, 0x41, 0x01, 0x6b, 0x21, 0x00, 0x0c, 0x00],
        ...[0x0b, 0x0b, 0x20, 0x01, 0x0b],
      ],
      exported: "sum",
    },
    { params: 1, result: true, locals: [0x00], code: [0x20, 0x00, 0x04, 0x7f, 0x41, 0x0a, 0x05, 0x41, 0x14, 0x41, 0x01, 0x6a, 0x0b, 0x0b], exported: "pick" },
    { params: 0, result: true, locals: [0x00], code: [0x41, 0x00, 0x10, 0x01, 0x0b], exported: "twice" },
    { params: 1, result: false, locals: [0x00], code: [0x41, 0x00, 0x41, 0x00, 0x20, 0x00, 0xfc, 10, 0x00, 0x00, 0x0b], exported: "copy" },
    { params: 1, result: false, locals: [0x00], code: [0x41, 0x00, 0x41, 0x00, 0x20, 0x00, 0xfc, 11, 0x00, 0x0b], exported: "fill" },
    {
      params: 1,
      result: false,
      locals: [0x00],
      code: [
        ...[0x02, 0x40, 0x03, 0x40],
        ...[0x20, 0x00, 0x45, 0x0d, 0x01],
        ...[0x41, 0x00, 0x41, 0x00, 0x41, 0x09, 0xfc, 11, 0x00],
        ...[0x20, 0x00, 0x41, 0x01, 0x6b, 0x21, 0x00, 0x0c, 0x00],
        ...[0x0b, 0x0b, 0x0b],
      ],
      exported: "fills",
    },
  ];
  /** The instructions `name(...args)` executes, and its result, in a fresh instance. */
  function run(name: string, ...args: number[]): [bigint, unknown] {
    const { exports } = new WebAssembly.Instance(new WebAssembly.Module(instrument(assemble({ functions }))), {});
    const counter = exports[COUNTER];
    const func = exports[name];
    if (!(counter instanceof WebAssembly.Global) || typeof func !== "function") {
      throw new Error("not instrumented");
    }
    const before: unknown = counter.value;
    const result: unknown = Reflect.apply(func, undefined, args);
    const after: unknown = counter.value;
    if (typeof before !== "bigint" || typeof after !== "bigint") {
      throw new Error("the counter is not an i64");
    }
    return [after - before, result];
  }

  test.each([0, 1, 2, 10, 1000])("counts 12n + 4 instructions for a loop of n = %i passes, and computes what it did", (passes) => {
    expect(run("sum", passes)).toStrictEqual([BigInt(12 * passes + 4), (passes * (passes + 1)) / 2]);
  });

  test("counts only the arm of an `if` that runs", () => {
    expect(run("pick", 1)).toStrictEqual([3n, 10]);
    expect(run("pick", 0)).toStrictEqual([5n, 21]);
  });

  test("counts the instructions of a function called", () => {
    expect(run("twice")).toStrictEqual([7n, 21]);
  });

  test.each([
    [0, 0],
    [1, 1],
    [7, 1],
    [8, 1],
    [9, 2],
    [512, 64],
    [65_536, 8192],
  ])("charges a copy and a fill of %i bytes %i more than their instructions, as the words they move", (bytes, words) => {
    expect(run("copy", bytes)).toStrictEqual([BigInt(4 + words), undefined]);
    expect(run("fill", bytes)).toStrictEqual([BigInt(4 + words), undefined]);
  });

  test.each([0, 1, 3, 100])("charges a fill each time it runs: 14k + 3 for k = %i fills of 9 bytes", (passes) => {
    expect(run("fills", passes)).toStrictEqual([BigInt(14 * passes + 3), undefined]);
  });

  test("charges a length of 2^32 - 1 bytes in 64 bits, unsigned, before the module traps", () => {
    const { exports } = new WebAssembly.Instance(new WebAssembly.Module(instrument(assemble({ functions }))), {});
    const counter = exports[COUNTER];
    const fill = exports["fill"];
    if (!(counter instanceof WebAssembly.Global) || typeof fill !== "function") {
      throw new Error("not instrumented");
    }
    expect(() => {
      Reflect.apply(fill, undefined, [-1]);
    }).toThrow(WebAssembly.RuntimeError);
    const counted: unknown = counter.value;
    expect(counted).toBe(4n + 2n ** 29n);
  });
});

describe("refusals", () => {
  /** What `instrument` throws for `module`, as an {@link InstrumentError}. */
  function refusal(module: Uint8Array): string {
    try {
      instrument(module);
    } catch (error) {
      expect(error).toBeInstanceOf(InstrumentError);
      expect(Reflect.get(Object(error), "name")).toBe("InstrumentError");
      return String(Reflect.get(Object(error), "message"));
    }
    throw new Error("accepted");
  }

  /** The offset in `module` of the first occurrence of `bytes`. */
  function offsetOf(module: Uint8Array, bytes: readonly number[]): number {
    return Buffer.from(module).indexOf(Buffer.from(bytes));
  }

  test("refuses what is not a module of version 1", () => {
    for (const bytes of [[], [0x00, 0x61, 0x73], [0x00, 0x61, 0x73, 0x6d, 0x02, 0x00, 0x00, 0x00], [0x01, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00], [0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x01]]) {
      expect(refusal(new Uint8Array(bytes))).toBe("not a WebAssembly module of version 1");
    }
  });

  test("refuses a truncated section, naming the byte", () => {
    expect(refusal(new Uint8Array([0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01]))).toBe("truncated at byte 9");
    expect(refusal(new Uint8Array([0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x05, 0x00]))).toBe("truncated at byte 10: 5 bytes expected");
  });

  test("refuses a module that imports anything, exports nothing or has no code", () => {
    expect(refusal(assemble({ functions: [plain([0x0b])], imports: [0x01, 0x01, 0x65, 0x01, 0x66, 0x00, 0x00] }))).toBe(
      "the module imports something: only self-contained modules are counted",
    );
    expect(refusal(assemble({ functions: [plain([0x0b])], exportSection: false }))).toBe("the module exports nothing");
    expect(refusal(assemble({ functions: [plain([0x0b])], codeSection: false }))).toBe("the module has no code");
  });

  test("refuses a module that already exports a counter", () => {
    expect(refusal(assemble({ functions: [plain([0x0b])], exports: [[COUNTER, 0x00, 0]] }))).toBe('the module already exports "instructions"');
  });

  test("refuses bytes after the exports and after the bodies", () => {
    const module = assemble({ functions: [plain([0x0b])] });
    // The export section, rewritten with one more byte: its size and contents.
    const at = offsetOf(module, [0x07, 0x05, 0x01, 0x01, 0x66]);
    const padded = new Uint8Array([...module.subarray(0, at), 0x07, 0x06, 0x01, 0x01, 0x66, 0x00, 0x00, 0xff, ...module.subarray(at + 7)]);
    expect(refusal(padded)).toBe(`bytes after the exports at byte ${String(at + 7)}`);
    const trailer = assemble({ functions: [plain([0x0b])], codeTrailer: [0x02] });
    expect(refusal(trailer)).toBe(`bytes after the function bodies at byte ${String(trailer.length - 1)}`);
  });

  test("refuses an instruction it does not understand, naming it and its byte", () => {
    for (const [code, words] of [
      [[0xfd, 0x0c], "0xfd"],
      [[0x12, 0x00], "0x12"],
      [[0x06, 0x40], "0x06"],
      [[0xc5], "0xc5"],
      [[0x27], "0x27"],
      [[0xfc, 0x12], "0xfc 18"],
    ] as const) {
      const module = assemble({ functions: [plain([0x01, ...code, 0x0b])] });
      expect(refusal(module)).toBe(`unknown instruction ${words} at byte ${String(offsetOf(module, [0x01, ...code]) + 1)}`);
    }
  });

  test("refuses a bulk table instruction, whose work grows with a length it is given and is not charged", () => {
    for (const code of [
      [0xfc, 12, 0x00, 0x00],
      [0xfc, 14, 0x00, 0x00],
      [0xfc, 17, 0x00],
    ]) {
      const module = assemble({ functions: [plain([0x01, ...code, 0x0b])] });
      expect(refusal(module)).toBe(
        `instruction 0xfc ${String(code[1])} at byte ${String(offsetOf(module, [0x01, ...code]) + 1)} does work that grows with its operand, which the count does not charge`,
      );
    }
  });

  test("refuses a second memory, in a memory argument or a memory index", () => {
    const argument = assemble({ functions: [plain([0x28, 0x40, 0x00, 0x0b])] });
    expect(refusal(argument)).toBe(`memory argument at byte ${String(offsetOf(argument, [0x28, 0x40]) + 1)} names a memory: only one memory is supported`);
    for (const [code, index] of [
      [[0x3f, 0x01], 1],
      [[0x40, 0x01], 1],
      [[0xfc, 8, 0x00, 0x01], 3],
      [[0xfc, 10, 0x01, 0x00], 2],
      [[0xfc, 10, 0x00, 0x01], 3],
      [[0xfc, 11, 0x01], 2],
    ] as const) {
      const module = assemble({ functions: [plain([...code, 0x0b])] });
      expect(refusal(module)).toBe(`memory index at byte ${String(offsetOf(module, [...code]) + index)} is not 0: only one memory is supported`);
    }
  });

  test("refuses integers longer or larger than their type", () => {
    const long = assemble({ functions: [plain([0x20, 0x80, 0x80, 0x80, 0x80, 0x80, 0x00, 0x0b])] });
    expect(refusal(long)).toBe(`integer at byte ${String(offsetOf(long, [0x20, 0x80]) + 1)} is longer than 5 bytes`);
    const large = assemble({ functions: [plain([0x20, 0xff, 0xff, 0xff, 0xff, 0x10, 0x0b])] });
    expect(refusal(large)).toBe(`integer at byte ${String(offsetOf(large, [0x20, 0xff]) + 1)} exceeds 32 bits`);
    // The largest index is accepted: 2^32 - 1.
    expect(() => instrumentOne([0x20, 0xff, 0xff, 0xff, 0xff, 0x0f, 0x1a, 0x0b])).not.toThrow();
    for (const [code, limit] of [
      [[0x41, 0x80, 0x80, 0x80, 0x80, 0x80, 0x00], 5],
      [[0x42, ...Array<number>(10).fill(0x80), 0x00], 10],
      [[0x02, 0x80, 0x80, 0x80, 0x80, 0x80, 0x00], 5],
    ] as const) {
      const module = assemble({ functions: [plain([...code, 0x0b])] });
      expect(refusal(module)).toBe(`integer at byte ${String(offsetOf(module, [code[0], 0x80]) + 1)} is longer than ${String(limit)} bytes`);
    }
  });

  test("refuses a truncated function, and code after its end", () => {
    for (const code of [[0x41], [0x44, 0x00, 0x00], [0x02, 0x40, 0x0b], [0x01]]) {
      expect(refusal(assemble({ functions: [plain(code)] }))).toMatch(/^truncated at byte \d+/);
    }
    const after = assemble({ functions: [plain([0x0b, 0x01])] });
    expect(refusal(after)).toBe(`code after the end of the function at byte ${String(offsetOf(after, [0x0b, 0x01]) + 1)}`);
  });
});

describe("LEB128", () => {
  test("encodes unsigned integers, at each length", () => {
    expect([0, 127, 128, 16_383, 16_384, 0xff_ff_ff_ff].map(unsignedLeb)).toStrictEqual([
      [0x00],
      [0x7f],
      [0x80, 0x01],
      [0xff, 0x7f],
      [0x80, 0x80, 0x01],
      [0xff, 0xff, 0xff, 0xff, 0x0f],
    ]);
  });

  test("encodes non-negative integers signed: a sign bit set needs one more byte", () => {
    expect([0, 63, 64, 127, 128, 8191, 8192].map(signedLeb)).toStrictEqual([[0x00], [0x3f], [0xc0, 0x00], [0xff, 0x00], [0x80, 0x01], [0xff, 0x3f], [0x80, 0xc0, 0x00]]);
  });
});
