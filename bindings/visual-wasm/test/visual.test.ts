import { describe, expect, test } from "vitest";
import { ABI_VERSION, MAX_COUNT, MAX_INDEX, type SquareSetup, VisualModule, VisualModuleError, VisualSetupError, VisualTrap } from "../ts/visual.ts";

/** A valid square. */
const SQUARE: SquareSetup = { width: 2, height: 1, rate: [60, 1], side: 65_536, cycle: [1, 2] };

/** Every call a fake module received: the function's name and its arguments. */
type Calls = [string, number[]][];

/** The functions of the interface and their arities. */
const ARITIES = {
  visual_abi_version: 0,
  visual_setup: 7,
  visual_message_ptr: 0,
  visual_message_len: 0,
  visual_render: 2,
  visual_hash: 3,
  visual_values_len: 0,
} as const;

/** A function taking exactly `arity` arguments (0 to 8), as an export of a real module does, that runs `body`. */
function withArity(arity: number, body: (...args: number[]) => unknown): (...args: number[]) => unknown {
  // A function's `length` is the number of parameters it is written with.
  switch (arity) {
    case 0:
      return () => body();
    case 1:
      return (p0: number) => body(p0);
    case 2:
      return (p0: number, p1: number) => body(p0, p1);
    case 3:
      return (p0: number, p1: number, p2: number) => body(p0, p1, p2);
    case 4:
      return (p0: number, p1: number, p2: number, p3: number) => body(p0, p1, p2, p3);
    case 7:
      return (p0: number, p1: number, p2: number, p3: number, p4: number, p5: number, p6: number) => body(p0, p1, p2, p3, p4, p5, p6);
    default:
      return (p0: number, p1: number, p2: number, p3: number, p4: number, p5: number, p6: number, p7: number) => body(p0, p1, p2, p3, p4, p5, p6, p7);
  }
}

/**
 * A fake module: a memory holding two values of an image at byte 8, a 32-byte hash at
 * byte 64 and the message "no" at byte 128, and the functions of the interface with their
 * arities, each recording its call and answering what `answers` gives (by default, the
 * addresses and lengths of those values, and every setup accepted).
 */
function fake(answers: Partial<Record<string, (...args: number[]) => unknown>> = {}): { exports: Record<string, unknown>; calls: Calls; memory: WebAssembly.Memory } {
  const memory = new WebAssembly.Memory({ initial: 1 });
  new Float32Array(memory.buffer, 8, 2).set([0.25, 1]);
  new Uint8Array(memory.buffer, 64, 32).set(Array.from({ length: 32 }, (_slot, index) => index * 8));
  new Uint8Array(memory.buffer, 128, 2).set([0x6e, 0x6f]);
  const calls: Calls = [];
  const defaults: Record<string, (...args: number[]) => unknown> = {
    visual_abi_version: () => ABI_VERSION,
    visual_setup: () => 1,
    visual_message_ptr: () => 128,
    visual_message_len: () => 2,
    visual_render: () => 8,
    visual_hash: () => 64,
    visual_values_len: () => 2,
  };
  const exports: Record<string, unknown> = { memory };
  for (const [name, arity] of Object.entries(ARITIES)) {
    exports[name] = withArity(arity, (...args: number[]): unknown => {
      calls.push([name, args]);
      return (answers[name] ?? defaults[name] ?? (() => 0))(...args);
    });
  }
  return { exports, calls, memory };
}

describe("VisualModule", () => {
  test("drives the interface: setup, render and hash, every argument in its place", () => {
    const { exports, calls, memory } = fake();
    const module = new VisualModule(exports, { width: 7, height: 5, rate: [24_000, 1001], side: 40_000, cycle: [1001, 1000] });
    const values = module.render(5n + (7n << 32n));
    expect(Array.from(values)).toStrictEqual([0.25, 1]);
    new Float32Array(memory.buffer, 8, 2).set([9, 9]);
    expect(Array.from(values), "a copy, not a view of the memory").toStrictEqual([0.25, 1]);
    expect(module.hash(MAX_INDEX - BigInt(MAX_COUNT) + 1n, MAX_COUNT)).toBe(Array.from({ length: 32 }, (_slot, index) => (index * 8).toString(16).padStart(2, "0")).join(""));
    module.setup(SQUARE);
    expect(calls).toStrictEqual([
      ["visual_abi_version", []],
      ["visual_setup", [7, 5, 24_000, 1001, 40_000, 1001, 1000]],
      ["visual_render", [5, 7]],
      ["visual_values_len", []],
      ["visual_hash", [1, 0xffff_ffff, 0xffff_ffff]],
      ["visual_setup", [2, 1, 60, 1, 65_536, 1, 2]],
    ]);
  });

  test("throws the module's explanation when it refuses a square, at creation or later", () => {
    expect(() => new VisualModule(fake({ visual_setup: () => 0 }).exports, SQUARE)).toThrow(new VisualSetupError("no"));
    let refuse = false;
    const { exports, calls } = fake({ visual_setup: () => (refuse ? 0 : 1) });
    const module = new VisualModule(exports, SQUARE);
    refuse = true;
    expect(() => {
      module.setup(SQUARE);
    }).toThrow(new VisualSetupError("no"));
    expect(calls.slice(-3).map(([name]) => name)).toStrictEqual(["visual_setup", "visual_message_ptr", "visual_message_len"]);
    expect(new VisualSetupError("x")).toMatchObject({ name: "VisualSetupError", message: "x" });
  });

  test("refuses an explanation that is not UTF-8", () => {
    const { exports, memory } = fake({ visual_setup: () => 0 });
    new Uint8Array(memory.buffer, 128, 2).set([0x6e, 0xff]);
    expect(() => new VisualModule(exports, SQUARE)).toThrow(new VisualModuleError("visual_setup explained its refusal in bytes that are not UTF-8"));
  });

  test("refuses a setup result outside the interface", () => {
    expect(() => new VisualModule(fake({ visual_setup: () => 2 }).exports, SQUARE)).toThrow(new VisualModuleError("visual_setup returned 2, not 0 or 1"));
  });

  test("reads every result as unsigned, as the interface returns it", () => {
    // 8 − 2^32 is how a signed i32 carries address 8: only read as unsigned does it reach the values.
    const module = new VisualModule(fake({ visual_render: () => 8 - 2 ** 32, visual_hash: () => 64 - 2 ** 32 }).exports, SQUARE);
    expect(Array.from(module.render(0n))).toStrictEqual([0.25, 1]);
    expect(module.hash(0n, 1)).toBe(Array.from({ length: 32 }, (_slot, index) => (index * 8).toString(16).padStart(2, "0")).join(""));
  });

  test("refuses images the module refuses", () => {
    expect(() => new VisualModule(fake({ visual_hash: () => 0 }).exports, SQUARE).hash(0n, 1)).toThrow(new VisualModuleError("visual_hash refused the images"));
  });

  test("dies when the module traps, and stays dead", () => {
    const trap = new WebAssembly.RuntimeError("unreachable");
    let trapping = false;
    const module = new VisualModule(
      fake({
        visual_render: () => {
          if (trapping) {
            throw trap;
          }
          return 8;
        },
      }).exports,
      SQUARE,
    );
    trapping = true;
    expect(() => module.render(0n)).toThrow(new VisualTrap());
    try {
      module.render(0n);
    } catch (error) {
      expect(error).toBeInstanceOf(VisualTrap);
      expect(error).not.toHaveProperty("cause");
    }
    trapping = false;
    for (const call of [() => module.render(0n), () => module.hash(0n, 1), () => { module.setup(SQUARE); }]) {
      expect(call).toThrow(new VisualTrap());
    }
    expect(new VisualTrap({ cause: trap })).toMatchObject({ name: "VisualTrap", message: "the visual module trapped and cannot be used again", cause: trap });
  });

  test("keeps the trap as the cause of the error it throws", () => {
    const trap = new WebAssembly.RuntimeError("unreachable");
    const module = new VisualModule(fake({ visual_hash: () => { throw trap; } }).exports, SQUARE);
    let thrown: unknown;
    try {
      module.hash(0n, 1);
    } catch (error) {
      thrown = error;
    }
    expect(thrown).toBeInstanceOf(VisualTrap);
    expect(thrown).toMatchObject({ cause: trap });
  });

  test("lets an error that is not a trap through, and stays alive", () => {
    const failure = new Error("not a trap");
    let failing = true;
    const module = new VisualModule(fake({ visual_render: () => { if (failing) { throw failure; } return 8; } }).exports, SQUARE);
    expect(() => module.render(0n)).toThrow(failure);
    failing = false;
    expect(Array.from(module.render(0n))).toStrictEqual([0.25, 1]);
  });

  test("refuses a module that is not the visual module it drives", () => {
    const { exports } = fake();
    const edits: (readonly [Record<string, unknown>, string])[] = [
      [{ memory: undefined }, "no memory"],
      [{ memory: {} }, "no memory"],
      [{ visual_abi_version: () => ABI_VERSION + 1 }, "interface 0x50560004, expected 0x50560003"],
      [{ visual_abi_version: () => 0 }, "interface 0x0, expected 0x50560003"],
      [{ visual_abi_version: 3 }, "no function visual_abi_version"],
      [{ visual_abi_version: () => "2" }, "visual_abi_version did not return a number"],
      [{ visual_abi_version: withArity(1, () => ABI_VERSION) }, "visual_abi_version takes 1 argument(s), expected 0"],
    ];
    for (const [name, arity] of Object.entries(ARITIES).filter(([each]) => each !== "visual_abi_version")) {
      edits.push([{ [name]: undefined }, `no function ${name}`], [{ [name]: withArity(arity + 1, () => 1) }, `${name} takes ${String(arity + 1)} argument(s), expected ${String(arity)}`]);
    }
    for (const [edit, reason] of edits) {
      expect(() => new VisualModule({ ...exports, ...edit }, SQUARE), reason).toThrow(new VisualModuleError(reason));
    }
    expect(new VisualModuleError("x")).toMatchObject({ name: "VisualModuleError", message: "visual module: x" });
  });

  test("refuses values the interface cannot take, before any call", () => {
    const { exports, calls } = fake();
    const module = new VisualModule(exports, SQUARE);
    for (const [setup, what] of [
      [{ ...SQUARE, width: -1 }, "width"],
      [{ ...SQUARE, height: 2 ** 32 }, "height"],
      [{ ...SQUARE, rate: [1.5, 1] }, "rate[0]"],
      [{ ...SQUARE, rate: [60, Number.NaN] }, "rate[1]"],
      [{ ...SQUARE, side: 0.5 }, "side"],
      [{ ...SQUARE, cycle: [-1, 2] }, "cycle[0]"],
      [{ ...SQUARE, cycle: [1, 2 ** 32] }, "cycle[1]"],
    ] as const) {
      expect(() => { module.setup(setup); }, what).toThrow(new VisualModuleError(`${what} is not an unsigned 32-bit integer`));
    }
    expect(() => { module.setup({ ...SQUARE, width: 0xffff_ffff }); }).not.toThrow();
    expect(() => module.render(-1n)).toThrow(new VisualModuleError("image -1 is not an index from 0 to 2^64 - 1"));
    expect(() => module.render(MAX_INDEX + 1n)).toThrow(new VisualModuleError("image 18446744073709551616 is not an index from 0 to 2^64 - 1"));
    expect(() => module.hash(0n, MAX_COUNT + 1)).toThrow(new VisualModuleError("count is not an unsigned 32-bit integer"));
    expect(() => module.hash(-1n, 1)).toThrow(VisualModuleError);
    expect(() => module.hash(MAX_INDEX, 2)).toThrow(new VisualModuleError("2 images from image 18446744073709551615 pass the largest index, 2^64 - 1"));
    expect(() => module.hash(MAX_INDEX - 3n, 5)).toThrow(new VisualModuleError("5 images from image 18446744073709551612 pass the largest index, 2^64 - 1"));
    expect(() => module.hash(MAX_INDEX, 1)).not.toThrow();
    expect(() => module.hash(MAX_INDEX, 0)).not.toThrow();
    expect(calls.map(([name]) => name)).toStrictEqual(["visual_abi_version", "visual_setup", "visual_setup", "visual_hash", "visual_hash"]);
  });

  test("names the limits of the interface", () => {
    expect([ABI_VERSION, MAX_INDEX, MAX_COUNT]).toStrictEqual([0x5056_0003, 18_446_744_073_709_551_615n, 4_294_967_295]);
  });
});
