import { assert, describe, expect, test } from "vitest";
import { ABI_VERSION, MAX_COUNT, MAX_INDEX, SHADERS, type SquareSetup, THRESHOLDS, u32, VisualModuleError, VisualSetupError, VisualTrap } from "../ts/driver.ts";
import { VisualModule } from "../ts/visual.ts";
import { TestDriver } from "./test-driver.ts";

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
  visual_display: 2,
  visual_display_len: 0,
  visual_placement: 2,
  visual_shader_ptr: 1,
  visual_shader_len: 1,
  visual_thresholds: 0,
  visual_encode_values: 1,
  visual_encode: 0,
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

/** The values a fake module renders for a square of `count` values: 0.25 and 1, then zeros. */
function image(count: number): number[] {
  return [0.25, 1, ...Array.from({ length: count - 2 }, () => 0)];
}

/** The bytes a fake module displays for a square of `count` bytes: 10, 20, 30 and 40, then zeros. */
function displayBytes(count: number): number[] {
  return [10, 20, 30, 40, ...Array.from({ length: count - 4 }, () => 0)];
}

/**
 * A fake module: a memory holding the values of an image at byte 8 (0.25 and 1, then
 * zeros), a 32-byte hash at byte 4096, the message "no" at byte 8192 and the bytes of an
 * image for display at byte 12 289 (10, 20, 30 and 40, then zeros), a placement at byte
 * 16 384 (1, 2, 3 and 4), the shader "fn a() {}" at byte 20 480 and 510 thresholds at byte
 * 24 576 (0, 1, 2…), room for values to encode at byte 28 672 and their bytes encoded at
 * byte 32 768 (50, 60, 70, 80, 90, 100, 110 and 120), and the functions
 * of the interface with their arities, each recording its call and answering what
 * `answers` gives (by default, the addresses of those values, the length of an image of the
 * last square set up, and every setup accepted).
 */
function fake(answers: Partial<Record<string, (...args: number[]) => unknown>> = {}): { exports: Record<string, unknown>; calls: Calls; memory: WebAssembly.Memory } {
  const memory = new WebAssembly.Memory({ initial: 1 });
  new Float32Array(memory.buffer, 8, 2).set([0.25, 1]);
  new Uint8Array(memory.buffer, 4096, 32).set(Array.from({ length: 32 }, (_slot, index) => index * 8));
  new Uint8Array(memory.buffer, 8192, 2).set([0x6e, 0x6f]);
  new Uint8Array(memory.buffer, 12_289, 4).set([10, 20, 30, 40]);
  new Uint32Array(memory.buffer, 16_384, 4).set([1, 2, 3, 4]);
  new Uint8Array(memory.buffer, 20_480, 9).set(new TextEncoder().encode("fn a() {}"));
  new Uint32Array(memory.buffer, 24_576, 510).set(Array.from({ length: 510 }, (_slot, index) => index));
  new Uint8Array(memory.buffer, 32_768, 8).set([50, 60, 70, 80, 90, 100, 110, 120]);
  const calls: Calls = [];
  let values = 0;
  const defaults: Record<string, (...args: number[]) => unknown> = {
    visual_abi_version: () => ABI_VERSION,
    visual_setup: (width = 0, height = 0) => {
      values = width * height * 4;
      return 1;
    },
    visual_message_ptr: () => 8192,
    visual_message_len: () => 2,
    visual_render: () => 8,
    visual_hash: () => 4096,
    visual_values_len: () => values,
    visual_display: () => 12_289,
    visual_display_len: () => values,
    visual_placement: () => 16_384,
    visual_shader_ptr: () => 20_480,
    visual_shader_len: () => 9,
    visual_thresholds: () => 24_576,
    visual_encode_values: () => 28_672,
    visual_encode: () => 32_768,
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

describe("VisualDriver", () => {
  test("drives the interface: setup, render and hash, every argument in its place", () => {
    const { exports, calls, memory } = fake();
    const module = new TestDriver(exports, { width: 7, height: 5, rate: [24_000, 1001], side: 40_000, cycle: [1001, 1000] });
    const values = module.render(5n + (7n << 32n));
    expect(Array.from(values)).toStrictEqual(image(7 * 5 * 4));
    new Float32Array(memory.buffer, 8, 2).set([9, 9]);
    expect(Array.from(values), "a copy, not a view of the memory").toStrictEqual(image(7 * 5 * 4));
    const shown = module.display(3n + (2n << 32n));
    expect(Array.from(shown)).toStrictEqual(displayBytes(7 * 5 * 4));
    new Uint8Array(memory.buffer, 12_289, 1).set([99]);
    expect(shown[0], "a view of the memory, not a copy").toBe(99);
    expect(shown).toBeInstanceOf(Uint8ClampedArray);
    expect(module.hash(MAX_INDEX - BigInt(MAX_COUNT) + 1n, MAX_COUNT)).toBe(Array.from({ length: 32 }, (_slot, index) => (index * 8).toString(16).padStart(2, "0")).join(""));
    module.setup(SQUARE);
    expect(calls).toStrictEqual([
      ["visual_abi_version", []],
      ["visual_setup", [7, 5, 24_000, 1001, 40_000, 1001, 1000]],
      ["visual_render", [5, 7]],
      ["visual_values_len", []],
      ["visual_display", [3, 2]],
      ["visual_display_len", []],
      ["visual_hash", [1, 0xffff_ffff, 0xffff_ffff]],
      ["visual_setup", [2, 1, 60, 1, 65_536, 1, 2]],
    ]);
  });

  test("throws the module's explanation when it refuses a square, at creation or later", () => {
    expect(() => new TestDriver(fake({ visual_setup: () => 0 }).exports, SQUARE)).toThrow(new VisualSetupError("no"));
    let refuse = false;
    const { exports, calls } = fake({ visual_setup: () => (refuse ? 0 : 1) });
    const module = new TestDriver(exports, SQUARE);
    refuse = true;
    expect(() => {
      module.setup(SQUARE);
    }).toThrow(new VisualSetupError("no"));
    expect(calls.slice(-4).map(([name]) => name)).toStrictEqual(["visual_setup", "visual_message_ptr", "visual_message_len", "visual_values_len"]);
    expect(new VisualSetupError("x")).toMatchObject({ name: "VisualSetupError", message: "x" });
  });

  test("is left with no square when the module cannot hold even the one it had, until a setup succeeds", () => {
    let result = 1;
    let held = 8;
    const { exports, calls } = fake({ visual_setup: () => result, visual_values_len: () => held, visual_display_len: () => held });
    const module = new TestDriver(exports, SQUARE);
    result = 0;
    expect(() => {
      module.setup(SQUARE);
    }, "refused, the square it had kept").toThrow(new VisualSetupError("no"));
    expect(Array.from(module.render(0n)), "the square kept").toStrictEqual(image(8));
    held = 4;
    expect(() => {
      module.setup(SQUARE);
    }, "a count that is neither").toThrow(new VisualModuleError("visual_values_len answered 4 after a refused setup: neither the square it had (8 values) nor none (0)"));
    held = 0;
    expect(() => {
      module.setup(SQUARE);
    }, "refused, no square left").toThrow(new VisualSetupError("no"));
    const before = calls.length;
    const none = new VisualSetupError("no square set up: the last setup was refused, and the module could not hold the square it had again; set one up");
    expect(() => module.render(0n)).toThrow(none);
    expect(() => module.display(0n)).toThrow(none);
    expect(() => module.placement(0n)).toThrow(none);
    expect(() => module.hash(0n, 1)).toThrow(none);
    expect(calls.length, "refused before any call").toBe(before);
    result = 1;
    held = 8;
    module.setup(SQUARE);
    expect(Array.from(module.render(0n)), "a square again").toStrictEqual(image(8));
  });

  test("refuses an explanation that is not UTF-8", () => {
    const { exports, memory } = fake({ visual_setup: () => 0 });
    new Uint8Array(memory.buffer, 8192, 2).set([0x6e, 0xff]);
    expect(() => new TestDriver(exports, SQUARE)).toThrow(new VisualModuleError("visual_setup explained its refusal in bytes that are not UTF-8"));
  });

  test("refuses a setup result outside the interface", () => {
    expect(() => new TestDriver(fake({ visual_setup: () => 2 }).exports, SQUARE)).toThrow(new VisualModuleError("visual_setup returned 2, not 0 or 1"));
  });

  test("reads every result as unsigned, as the interface returns it", () => {
    // 8 − 2^32 is how a signed i32 carries address 8: only read as unsigned does it reach the values.
    const module = new TestDriver(fake({ visual_render: () => 8 - 2 ** 32, visual_hash: () => 4096 - 2 ** 32, visual_display: () => 12_289 - 2 ** 32 }).exports, SQUARE);
    expect(Array.from(module.render(0n))).toStrictEqual(image(8));
    expect(Array.from(module.display(0n))).toStrictEqual(displayBytes(8));
    expect(module.hash(0n, 1)).toBe(Array.from({ length: 32 }, (_slot, index) => (index * 8).toString(16).padStart(2, "0")).join(""));
  });

  test("refuses images the module refuses", () => {
    expect(() => new TestDriver(fake({ visual_hash: () => 0 }).exports, SQUARE).hash(0n, 1)).toThrow(new VisualModuleError("visual_hash refused the images"));
  });

  test("dies when the module traps, and stays dead", () => {
    const trap = new WebAssembly.RuntimeError("unreachable");
    let trapping = false;
    const module = new TestDriver(
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
    for (const call of [() => module.render(0n), () => module.display(0n), () => module.hash(0n, 1), () => { module.setup(SQUARE); }]) {
      expect(call).toThrow(new VisualTrap());
    }
    expect(new VisualTrap({ cause: trap })).toMatchObject({ name: "VisualTrap", message: "the visual module trapped and cannot be used again", cause: trap });
  });

  test("keeps the trap as the cause of the error it throws", () => {
    const trap = new WebAssembly.RuntimeError("unreachable");
    const module = new TestDriver(fake({ visual_hash: () => { throw trap; } }).exports, SQUARE);
    let thrown: unknown;
    try {
      module.hash(0n, 1);
    } catch (error) {
      thrown = error;
    }
    expect(thrown).toBeInstanceOf(VisualTrap);
    expect(thrown).toMatchObject({ cause: trap });
  });

  test("dies when a call into the module fails in any way, a host stack overflow included", () => {
    for (const failure of [new RangeError("Maximum call stack size exceeded"), new Error("InternalError: too much recursion"), new WebAssembly.RuntimeError("unreachable")]) {
      let failing = true;
      const { exports, calls } = fake({
        visual_render: () => {
          if (failing) {
            throw failure;
          }
          return 8;
        },
      });
      const module = new TestDriver(exports, SQUARE);
      let thrown: unknown;
      try {
        module.render(0n);
      } catch (error) {
        thrown = error;
      }
      expect(thrown, String(failure)).toBeInstanceOf(VisualTrap);
      expect(thrown, String(failure)).toMatchObject({ cause: failure });
      failing = false;
      const before = calls.length;
      expect(() => module.render(0n), String(failure)).toThrow(new VisualTrap());
      expect(calls.length, `${String(failure)}: never called again`).toBe(before);
    }
  });

  test("reads the explanation of a refusal from the memory as it is after asking where it is", () => {
    const { exports, memory } = fake({
      visual_setup: () => 0,
      visual_message_ptr: () => {
        memory.grow(1);
        return 8192;
      },
    });
    expect(() => new TestDriver(exports, SQUARE)).toThrow(new VisualSetupError("no"));
  });

  test("refuses what the module answers outside its memory, unaligned or of the wrong length", () => {
    const end = 65_536;
    for (const [answers, call, reason] of [
      [{ visual_values_len: () => 5 }, "render", "visual_values_len answered 5, not the 8 values of the square set up"],
      [{ visual_render: () => 3 }, "render", "visual_render answered 8 values of 4 bytes at 3, unaligned or outside the module memory"],
      [{ visual_render: () => end - 28 }, "render", `visual_render answered 8 values of 4 bytes at ${String(end - 28)}, unaligned or outside the module memory`],
      [{ visual_hash: () => end - 31 }, "hash", `visual_hash answered 32 values of 1 bytes at ${String(end - 31)}, unaligned or outside the module memory`],
      [{ visual_display_len: () => 7 }, "display", "visual_display_len answered 7, not the 8 bytes of the square set up"],
      [{ visual_display: () => end - 7 }, "display", `visual_display answered 8 values of 1 bytes at ${String(end - 7)}, unaligned or outside the module memory`],
    ] as const) {
      const module = new TestDriver(fake(answers).exports, SQUARE);
      const calls = { render: () => module.render(0n), display: () => module.display(0n), hash: () => module.hash(0n, 1) };
      expect(calls[call], reason).toThrow(new VisualModuleError(reason));
      expect(() => {
        module.setup(SQUARE);
      }, "not a trap: the instance stays alive").not.toThrow();
    }
    const atTheEnd = new TestDriver(fake({ visual_render: () => end - 32, visual_hash: () => end - 32, visual_display: () => end - 8 }).exports, SQUARE);
    expect(Array.from(atTheEnd.render(0n)), "ending exactly at the end of the memory").toStrictEqual(image(8).map(() => 0));
    expect(Array.from(atTheEnd.display(0n)), "ending exactly at the end of the memory").toStrictEqual(image(8).map(() => 0));
    expect(atTheEnd.hash(0n, 1)).toBe("0".repeat(64));
    expect(() => new TestDriver(fake({ visual_setup: () => 0, visual_message_ptr: () => end - 1 }).exports, SQUARE)).toThrow(
      new VisualModuleError(`visual_message_ptr answered 2 values of 1 bytes at ${String(end - 1)}, unaligned or outside the module memory`),
    );
  });

  test("refuses bytes that are not the visual module, and keeps the reason", async () => {
    const garbage: unknown = await VisualModule.instantiate(new Uint8Array([1, 2, 3]), SQUARE).catch((error: unknown) => error);
    expect(garbage).toBeInstanceOf(VisualModuleError);
    assert(garbage instanceof VisualModuleError, "a VisualModuleError");
    expect(garbage.message).toMatch(/^visual module: cannot be compiled: CompileError: /u);
    expect(garbage.cause, "the reason kept").toBeInstanceOf(WebAssembly.CompileError);
    // (module (import "a" "b" (func))): a valid module that needs an import.
    const needsImports = new Uint8Array([0x00, 0x61, 0x73, 0x6d, 1, 0, 0, 0, 1, 4, 1, 0x60, 0, 0, 2, 7, 1, 1, 0x61, 1, 0x62, 0, 0]);
    const unlinked: unknown = await VisualModule.instantiate(needsImports, SQUARE).catch((error: unknown) => error);
    assert(unlinked instanceof VisualModuleError, "a VisualModuleError");
    expect(unlinked.message).toMatch(/^visual module: cannot be instantiated with no imports: /u);
    expect(unlinked.cause, "the reason kept: no import object for module a").toBeInstanceOf(TypeError);
    // (module): a valid module with nothing the interface needs.
    await expect(VisualModule.instantiate(new Uint8Array([0x00, 0x61, 0x73, 0x6d, 1, 0, 0, 0]), SQUARE)).rejects.toThrow(new VisualModuleError("no memory"));
  });

  test("dies when the module traps while its version is asked", () => {
    const trap = new WebAssembly.RuntimeError("unreachable");
    expect(
      () =>
        new TestDriver(
          fake({
            visual_abi_version: () => {
              throw trap;
            },
          }).exports,
          SQUARE,
        ),
    ).toThrow(new VisualTrap());
  });

  test("refuses a module that is not the visual module it drives", () => {
    const { exports } = fake();
    const edits: (readonly [Record<string, unknown>, string])[] = [
      [{ memory: undefined }, "no memory"],
      [{ memory: {} }, "no memory"],
      [{ visual_abi_version: () => ABI_VERSION + 1 }, "interface 0x50560005, expected 0x50560004: build the module again from this source (pnpm build:wasm)"],
      [{ visual_abi_version: () => 0 }, "interface 0x0, expected 0x50560004: build the module again from this source (pnpm build:wasm)"],
      [{ visual_abi_version: 3 }, "no function visual_abi_version"],
      [{ visual_abi_version: () => "2" }, "visual_abi_version did not return a number"],
      [{ visual_abi_version: withArity(1, () => ABI_VERSION) }, "visual_abi_version takes 1 argument(s), expected 0"],
    ];
    for (const [name, arity] of Object.entries(ARITIES).filter(([each]) => each !== "visual_abi_version")) {
      edits.push([{ [name]: undefined }, `no function ${name}`], [{ [name]: withArity(arity + 1, () => 1) }, `${name} takes ${String(arity + 1)} argument(s), expected ${String(arity)}`]);
    }
    for (const [edit, reason] of edits) {
      expect(() => new TestDriver({ ...exports, ...edit }, SQUARE), reason).toThrow(new VisualModuleError(reason));
    }
    expect(new VisualModuleError("x")).toMatchObject({ name: "VisualModuleError", message: "visual module: x" });
  });

  test("refuses values the interface cannot take, before any call", () => {
    const { exports, calls } = fake();
    const module = new TestDriver(exports, SQUARE);
    for (const [setup, what, value] of [
      [{ ...SQUARE, width: -1 }, "width", "-1"],
      [{ ...SQUARE, height: 2 ** 32 }, "height", "4294967296"],
      [{ ...SQUARE, rate: [1.5, 1] }, "rate[0]", "1.5"],
      [{ ...SQUARE, rate: [60, Number.NaN] }, "rate[1]", "NaN"],
      [{ ...SQUARE, side: 0.5 }, "side", "0.5"],
      [{ ...SQUARE, cycle: [-1, 2] }, "cycle[0]", "-1"],
      [{ ...SQUARE, cycle: [1, 2 ** 32] }, "cycle[1]", "4294967296"],
    ] as const) {
      expect(() => { module.setup(setup); }, what).toThrow(new VisualModuleError(`${what} is not an unsigned 32-bit integer: ${value}`));
    }
    expect(() => { module.setup({ ...SQUARE, width: 0xffff_ffff }); }).not.toThrow();
    expect(() => module.render(-1n)).toThrow(new VisualModuleError("image -1 is not an index from 0 to 2^64 - 1"));
    expect(() => module.render(MAX_INDEX + 1n)).toThrow(new VisualModuleError("image 18446744073709551616 is not an index from 0 to 2^64 - 1"));
    expect(() => module.display(-1n)).toThrow(new VisualModuleError("image -1 is not an index from 0 to 2^64 - 1"));
    expect(() => module.hash(0n, MAX_COUNT + 1)).toThrow(new VisualModuleError("count is not an unsigned 32-bit integer: 4294967296"));
    expect(() => module.hash(-1n, 1)).toThrow(new VisualModuleError("image -1 is not an index from 0 to 2^64 - 1"));
    expect(() => module.hash(MAX_INDEX, 2)).toThrow(new VisualModuleError("2 images from image 18446744073709551615 pass the largest index, 2^64 - 1"));
    expect(() => module.hash(MAX_INDEX - 3n, 5)).toThrow(new VisualModuleError("5 images from image 18446744073709551612 pass the largest index, 2^64 - 1"));
    expect(() => module.hash(MAX_INDEX, 1)).not.toThrow();
    expect(() => module.hash(MAX_INDEX, 0)).not.toThrow();
    expect(calls.map(([name]) => name)).toStrictEqual(["visual_abi_version", "visual_setup", "visual_setup", "visual_hash", "visual_hash"]);
  });

  test("names the limits of the interface", () => {
    expect([ABI_VERSION, MAX_INDEX, MAX_COUNT]).toStrictEqual([0x5056_0004, 18_446_744_073_709_551_615n, 4_294_967_295]);
  });

  test("gives a GPU what it needs: the placement, the shaders and the thresholds", () => {
    const { exports, calls, memory } = fake();
    const module = new TestDriver(exports, SQUARE);
    expect(module.placement(9n + (3n << 32n))).toStrictEqual([1, 2, 3, 4]);
    expect([module.shader("square"), module.shader("display")]).toStrictEqual(["fn a() {}", "fn a() {}"]);
    const thresholds = module.thresholds();
    expect(Array.from(thresholds)).toStrictEqual(Array.from({ length: 510 }, (_slot, index) => index));
    new Uint32Array(memory.buffer, 24_576, 1).set([99]);
    expect(thresholds[0], "a copy, not a view of the memory").toBe(0);
    expect(calls.slice(2)).toStrictEqual([
      ["visual_placement", [9, 3]],
      ["visual_shader_ptr", [0]],
      ["visual_shader_len", [0]],
      ["visual_shader_ptr", [1]],
      ["visual_shader_len", [1]],
      ["visual_thresholds", []],
    ]);
    expect(SHADERS).toStrictEqual({ square: 0, display: 1 });
    expect(THRESHOLDS).toBe(510);
  });

  test("reads the placement and the shaders' addresses as unsigned", () => {
    const module = new TestDriver(fake({ visual_placement: () => 16_384 - 2 ** 32, visual_shader_ptr: () => 20_480 - 2 ** 32, visual_thresholds: () => 24_576 - 2 ** 32 }).exports, SQUARE);
    expect(module.placement(0n)).toStrictEqual([1, 2, 3, 4]);
    expect(module.shader("square")).toBe("fn a() {}");
    expect(module.thresholds()).toHaveLength(510);
  });

  test("encodes any values for display through the module: written into its room, its bytes copied", () => {
    const { exports, calls, memory } = fake();
    const module = new TestDriver(exports, SQUARE);
    calls.length = 0;
    const values = Float32Array.of(0.25, 0.5, 1, 1, -0, Number.NaN, 2, 0.5);
    const bytes = module.encode(values);
    expect(Array.from(bytes)).toStrictEqual([50, 60, 70, 80, 90, 100, 110, 120]);
    expect(bytes, "the type of the square's display bytes, the same encoding").toBeInstanceOf(Uint8ClampedArray);
    expect(Array.from(new Float32Array(memory.buffer, 28_672, 8)), "the values written in the room, as given").toStrictEqual(Array.from(values));
    new Uint8Array(memory.buffer, 32_768, 1).set([7]);
    expect(bytes[0], "a copy, not a view of the memory").toBe(50);
    expect(calls).toStrictEqual([
      ["visual_encode_values", [2]],
      ["visual_encode", []],
    ]);
  });

  test("reads the encoded bytes from the memory as it is after the encoding call", () => {
    const { exports, memory } = fake({
      visual_encode: () => {
        memory.grow(1);
        new Uint8Array(memory.buffer, 32_768, 8).set([1, 2, 3, 4, 5, 6, 7, 8]);
        return 32_768;
      },
    });
    expect(Array.from(new TestDriver(exports, SQUARE).encode(new Float32Array(8)))).toStrictEqual([1, 2, 3, 4, 5, 6, 7, 8]);
  });

  test("refuses values to encode that are not whole pixels, and a room or bytes outside the contract", () => {
    for (const length of [1, 3, 5, 7]) {
      const { exports, calls } = fake();
      const module = new TestDriver(exports, SQUARE);
      calls.length = 0;
      expect(() => module.encode(new Float32Array(length)), String(length)).toThrow(new VisualModuleError(`${String(length)} values are not whole pixels of 4`));
      expect(calls, "before any call").toStrictEqual([]);
    }
    const empty = fake();
    const emptied = new TestDriver(empty.exports, SQUARE);
    empty.calls.length = 0;
    expect(() => emptied.encode(new Float32Array(0)), "no pixel").toThrow(new VisualModuleError("no pixel to encode"));
    expect(empty.calls, "refused before any call").toStrictEqual([]);
    expect(() => new TestDriver(fake({ visual_encode_values: () => 0 }).exports, SQUARE).encode(new Float32Array(8))).toThrow(new VisualModuleError("the module has no room for 2 pixels to encode: more than the largest image, or its memory full"));
    const end = 65_536;
    for (const [answers, message] of [
      [{ visual_encode_values: () => end - 28 }, `visual_encode_values answered 8 values of 4 bytes at ${String(end - 28)}, unaligned or outside the module memory`],
      [{ visual_encode_values: () => 28_674 }, "visual_encode_values answered 8 values of 4 bytes at 28674, unaligned or outside the module memory"],
      [{ visual_encode: () => end - 7 }, `visual_encode answered 8 values of 1 bytes at ${String(end - 7)}, unaligned or outside the module memory`],
    ] as const) {
      expect(() => new TestDriver(fake(answers).exports, SQUARE).encode(new Float32Array(8)), message).toThrow(new VisualModuleError(message));
    }
    const ending = fake({ visual_encode_values: () => end - 32, visual_encode: () => end - 8 });
    expect(Array.from(new TestDriver(ending.exports, SQUARE).encode(new Float32Array(8))), "values and bytes ending at the memory's end").toStrictEqual([0, 0, 0, 0, 0, 0, 0, 0]);
  });

  test("refuses a placement, a shader or thresholds the module answers outside its contract", () => {
    const end = 65_536;
    for (const [answers, call, reason] of [
      [{ visual_placement: () => 0 }, "placement", "visual_placement answered no placement for the square set up"],
      [{ visual_placement: () => end - 12 }, "placement", `visual_placement answered 4 values of 4 bytes at ${String(end - 12)}, unaligned or outside the module memory`],
      [{ visual_placement: () => 16_386 }, "placement", "visual_placement answered 4 values of 4 bytes at 16386, unaligned or outside the module memory"],
      [{ visual_shader_len: () => 0 }, "shader", "visual_shader_len answered no square shader"],
      [{ visual_shader_ptr: () => end - 8 }, "shader", `visual_shader_ptr answered 9 values of 1 bytes at ${String(end - 8)}, unaligned or outside the module memory`],
      [{ visual_thresholds: () => end - 2036 }, "thresholds", `visual_thresholds answered 510 values of 4 bytes at ${String(end - 2036)}, unaligned or outside the module memory`],
    ] as const) {
      const module = new TestDriver(fake(answers).exports, SQUARE);
      const calls = { placement: () => module.placement(0n), shader: () => module.shader("square"), thresholds: () => module.thresholds() };
      expect(calls[call], reason).toThrow(new VisualModuleError(reason));
    }
    const { exports, memory } = fake();
    new Uint8Array(memory.buffer, 20_480, 1).set([0xff]);
    expect(() => new TestDriver(exports, SQUARE).shader("display")).toThrow(new VisualModuleError("the display shader is not UTF-8"));
    expect(() => new TestDriver(fake().exports, SQUARE).placement(-1n)).toThrow(new VisualModuleError("image -1 is not an index from 0 to 2^64 - 1"));
    const ending = new TestDriver(fake({ visual_placement: () => end - 16, visual_shader_ptr: () => end - 9, visual_thresholds: () => end - 2040 }).exports, SQUARE);
    expect([ending.placement(0n), ending.shader("square"), ending.thresholds().length], "ending exactly at the end of the memory").toStrictEqual([[0, 0, 0, 0], "\0".repeat(9), 510]);
  });
});

describe("the package built to JavaScript", () => {
  test("is what a product module's wrapper imports: the built driver, with its types", async () => {
    const built = await import("@poyetik/visual-wasm/driver");
    expect(built.ABI_VERSION).toBe(ABI_VERSION);
    const { exports } = fake();
    /** The built driver, its constructor callable here. */
    class Built extends built.VisualDriver {
      /**
       * The built driver of `fakeExports`.
       * @param fakeExports A fake instance's exports.
       * @param square The square to set up.
       */
      public constructor(fakeExports: Record<string, unknown>, square: SquareSetup) {
        super(fakeExports, square);
      }
    }
    expect(new Built(exports, SQUARE).placement(0n)).toStrictEqual([1, 2, 3, 4]);
    expect([import.meta.resolve("@poyetik/visual-wasm/driver"), import.meta.resolve("@poyetik/visual-wasm/visual")]).toStrictEqual([expect.stringMatching(/\/dist\/driver\.js$/u), expect.stringMatching(/\/dist\/visual\.js$/u)]);
  });
});

describe("one driver per instance", () => {
  test("refuses a second driver of an instance, by its memory", () => {
    const { exports } = fake();
    new TestDriver(exports, SQUARE);
    expect(() => new TestDriver(exports, SQUARE)).toThrow(new VisualModuleError("this instance already has its driver: two drivers of one instance would draw and hash each other's square"));
  });

  test("drives instances of their own side by side", () => {
    expect(() => [new TestDriver(fake().exports, SQUARE), new TestDriver(fake().exports, SQUARE)]).not.toThrow();
  });
});

describe("u32", () => {
  test("passes an unsigned 32-bit integer, and names any other with its value, in the caller's error", () => {
    expect([0, 0xffff_ffff].map((value) => u32(value, "x"))).toStrictEqual([0, 0xffff_ffff]);
    for (const value of [-1, 0.5, 2 ** 32, Number.NaN]) {
      expect(() => u32(value, "x"), String(value)).toThrow(new VisualModuleError(`x is not an unsigned 32-bit integer: ${String(value)}`));
    }
    const own = (detail: string): RangeError => new RangeError(detail);
    expect(() => u32(-1, "a page's term", own)).toThrow(new RangeError("a page's term is not an unsigned 32-bit integer: -1"));
    expect(u32(7, "a page's term", own)).toBe(7);
  });
});

describe("a product module's own calls", () => {
  test("die with the instance as the driver's own calls do", () => {
    const { exports } = fake();
    const module = new TestDriver(exports, SQUARE);
    expect(module.call(() => 7)).toBe(7);
    const cause = new Error("unreachable");
    expect(() => module.call(() => { throw cause; })).toThrow(new VisualTrap({ cause }));
    expect(() => module.call(() => 7), "never called again").toThrow(new VisualTrap());
    expect(() => module.render(0n), "nor the driver's own").toThrow(new VisualTrap());
  });
});
