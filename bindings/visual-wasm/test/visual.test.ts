import { describe, expect, test } from "vitest";
import { ABI_VERSION, MAX_COUNT, MAX_INDEX, type SquareSetup, VisualModule, VisualModuleError } from "../ts/visual.ts";

/** A valid square. */
const SQUARE: SquareSetup = { width: 2, height: 1, rate: [60, 1], side: 65_536, cycle: [1, 2] };

/** Every call a fake module received: the function's name and its arguments. */
type Calls = [string, number[]][];

/**
 * A fake module: a memory holding two values of an image at byte 8 and a 32-byte hash at
 * byte 64, and the functions of the interface, each recording its call and answering what
 * `answers` gives (by default, the addresses and lengths of those values).
 */
function fake(answers: Partial<Record<string, (...args: number[]) => unknown>> = {}): { exports: Record<string, unknown>; calls: Calls; memory: WebAssembly.Memory } {
  const memory = new WebAssembly.Memory({ initial: 1 });
  new Float32Array(memory.buffer, 8, 2).set([0.25, 1]);
  new Uint8Array(memory.buffer, 64, 32).set(Array.from({ length: 32 }, (_slot, index) => index * 8));
  const calls: Calls = [];
  const defaults: Record<string, (...args: number[]) => unknown> = {
    visual_abi_version: () => ABI_VERSION,
    visual_setup: () => 1,
    visual_render: () => 8,
    visual_hash: () => 64,
    visual_values_len: () => 2,
  };
  const exports: Record<string, unknown> = { memory };
  for (const name of Object.keys(defaults)) {
    exports[name] = (...args: number[]): unknown => {
      calls.push([name, args]);
      return (answers[name] ?? defaults[name] ?? (() => 0))(...args);
    };
  }
  return { exports, calls, memory };
}

describe("VisualModule", () => {
  test("drives the interface: setup, render and hash, every argument in its place", () => {
    const { exports, calls, memory } = fake();
    const module = new VisualModule(exports);
    expect(module.setup({ width: 7, height: 5, rate: [24_000, 1001], side: 40_000, cycle: [1001, 1000] })).toBe(true);
    const values = module.render(5n + (7n << 32n));
    expect(Array.from(values)).toStrictEqual([0.25, 1]);
    new Float32Array(memory.buffer, 8, 2).set([9, 9]);
    expect(Array.from(values), "a copy, not a view of the memory").toStrictEqual([0.25, 1]);
    expect(module.hash(MAX_INDEX - BigInt(MAX_COUNT) + 1n, MAX_COUNT)).toBe(Array.from({ length: 32 }, (_slot, index) => (index * 8).toString(16).padStart(2, "0")).join(""));
    expect(calls).toStrictEqual([
      ["visual_abi_version", []],
      ["visual_setup", [7, 5, 24_000, 1001, 40_000, 1001, 1000]],
      ["visual_render", [5, 7]],
      ["visual_values_len", []],
      ["visual_hash", [1, 0xffff_ffff, 0xffff_ffff]],
    ]);
  });

  test("says whether the module accepted the square", () => {
    expect(new VisualModule(fake({ visual_setup: () => 0 }).exports).setup(SQUARE)).toBe(false);
    expect(new VisualModule(fake({ visual_setup: () => 2 }).exports).setup(SQUARE)).toBe(false);
  });

  test("refuses images the module refuses", () => {
    expect(() => new VisualModule(fake({ visual_hash: () => 0 }).exports).hash(0n, 1)).toThrow(new VisualModuleError("visual_hash refused the images"));
  });

  test("reads every result as unsigned, as the interface returns it", () => {
    const module = new VisualModule(fake({ visual_render: () => -1, visual_values_len: () => 0 }).exports);
    expect(() => module.render(0n)).toThrow(RangeError);
  });

  test("refuses a module that is not the visual module it drives", () => {
    const { exports } = fake();
    for (const [edit, reason] of [
      [{ memory: undefined }, "visual module: no memory"],
      [{ memory: {} }, "visual module: no memory"],
      [{ visual_abi_version: () => ABI_VERSION + 1 }, "visual module: interface version 0x50560003, not 0x50560002"],
      [{ visual_abi_version: () => 0 }, "visual module: interface version 0x0, not 0x50560002"],
      [{ visual_abi_version: 3 }, "visual module: no function visual_abi_version"],
      [{ visual_abi_version: () => "2" }, "visual module: visual_abi_version returned a string, not a number"],
      [{ visual_setup: undefined }, "visual module: no function visual_setup"],
      [{ visual_render: undefined }, "visual module: no function visual_render"],
      [{ visual_hash: undefined }, "visual module: no function visual_hash"],
      [{ visual_values_len: undefined }, "visual module: no function visual_values_len"],
    ] as const) {
      expect(() => new VisualModule({ ...exports, ...edit }), reason).toThrow(new VisualModuleError(reason.slice("visual module: ".length)));
    }
    expect(new VisualModuleError("x")).toMatchObject({ name: "VisualModuleError", message: "visual module: x" });
  });

  test("refuses values the interface cannot take, before any call", () => {
    const { exports, calls } = fake();
    const module = new VisualModule(exports);
    for (const [setup, what] of [
      [{ ...SQUARE, width: -1 }, "width"],
      [{ ...SQUARE, height: 2 ** 32 }, "height"],
      [{ ...SQUARE, rate: [1.5, 1] }, "rate[0]"],
      [{ ...SQUARE, rate: [60, Number.NaN] }, "rate[1]"],
      [{ ...SQUARE, side: 0.5 }, "side"],
      [{ ...SQUARE, cycle: [-1, 2] }, "cycle[0]"],
      [{ ...SQUARE, cycle: [1, 2 ** 32] }, "cycle[1]"],
    ] as const) {
      expect(() => module.setup(setup), what).toThrow(new VisualModuleError(`${what} is not an unsigned 32-bit integer`));
    }
    expect(() => module.setup({ ...SQUARE, width: 0xffff_ffff })).not.toThrow();
    expect(() => module.render(-1n)).toThrow(new VisualModuleError("image -1 is not an index from 0 to 2^64 - 1"));
    expect(() => module.render(MAX_INDEX + 1n)).toThrow(new VisualModuleError("image 18446744073709551616 is not an index from 0 to 2^64 - 1"));
    expect(() => module.hash(0n, MAX_COUNT + 1)).toThrow(new VisualModuleError("count is not an unsigned 32-bit integer"));
    expect(() => module.hash(-1n, 1)).toThrow(VisualModuleError);
    expect(() => module.hash(MAX_INDEX, 2)).toThrow(new VisualModuleError("2 images from image 18446744073709551615 pass the largest index, 2^64 - 1"));
    expect(() => module.hash(MAX_INDEX - 3n, 5)).toThrow(new VisualModuleError("5 images from image 18446744073709551612 pass the largest index, 2^64 - 1"));
    expect(() => module.hash(MAX_INDEX, 1)).not.toThrow();
    expect(() => module.hash(MAX_INDEX, 0)).not.toThrow();
    expect(calls.map(([name]) => name)).toStrictEqual(["visual_abi_version", "visual_setup", "visual_hash", "visual_hash"]);
  });

  test("names the limits of the interface", () => {
    expect([ABI_VERSION, MAX_INDEX, MAX_COUNT]).toStrictEqual([0x5056_0002, 18_446_744_073_709_551_615n, 4_294_967_295]);
  });
});
