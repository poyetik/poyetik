import { describe, expect, test } from "vitest";
import { CountedInstance, CountError, countedModule } from "../../benchmarks/counter.ts";
import { instrument } from "../../benchmarks/instrument.ts";
import { WasmEngine } from "../../ts/engine.ts";
import { interleave, renderHash } from "../../ts/hash.ts";
import { CORPUS, REFERENCES, SINE_PATCH, UNKNOWN_BOX, wasmBytes } from "../fixtures.ts";
import { assemble, I32_GLOBAL } from "./assemble.ts";

/** The built engine module, instrumented and compiled at the first use (a mutant that breaks it fails a test, not the file). */
let compiled: WebAssembly.Module | undefined;

/** {@link compiled}, compiled if needed. */
function counted(): WebAssembly.Module {
  compiled ??= countedModule(new Uint8Array(wasmBytes()));
  return compiled;
}

/** The patch of no box. */
const EMPTY = new TextEncoder().encode('{"format":1,"nodes":[],"connections":[]}');

describe("the instrumented engine module", () => {
  test.each(CORPUS.map((testCase) => [testCase.name, testCase] as const))("renders %s bit-identical to the native engine, as the module does", async (_name, testCase) => {
    const engine = WasmEngine.create(new WebAssembly.Module(instrument(new Uint8Array(wasmBytes()))), testCase.patch, testCase.rate);
    const channels = Array.from({ length: engine.channels }, () => new Float32Array(testCase.frames));
    engine.render(channels);
    expect(await renderHash(testCase.rate, engine.channels, interleave(channels))).toBe(REFERENCES.get(testCase.name));
  });
});

describe("CountedInstance", () => {
  test("counts the same instructions for the same work, in every instance", () => {
    const counts = [0, 1].map(() => {
      const instance = new CountedInstance(counted());
      return [instance.load(SINE_PATCH, 48_000), instance.process(3)];
    });
    expect(counts[0]).toStrictEqual(counts[1]);
    expect(counts[0]?.every((count) => count > 0n)).toBe(true);
  });

  test("counts nothing for no block, and each block of a patch of no box alike", () => {
    const once = new CountedInstance(counted());
    once.load(EMPTY, 48_000);
    expect(once.process(0)).toBe(0n);
    const block = once.process(1);
    expect(block).toBeGreaterThan(0n);
    const thrice = new CountedInstance(counted());
    thrice.load(EMPTY, 48_000);
    expect(thrice.process(3)).toBe(3n * block);
  });

  test("counts a load that grows with the patch", () => {
    const small = new CountedInstance(counted()).load(EMPTY, 48_000);
    expect(new CountedInstance(counted()).load(SINE_PATCH, 48_000)).toBeGreaterThan(small);
  });

  test("refuses a patch with the engine's words", () => {
    const instance = new CountedInstance(counted());
    expect(() => instance.load(UNKNOWN_BOX, 48_000)).toThrow(CountError);
    expect(() => instance.load(UNKNOWN_BOX, 48_000)).toThrow(/^the engine refused the patch: .*"nope~"/);
    expect(() => instance.load(SINE_PATCH, 7)).toThrow(/^the engine refused the patch: .+/);
  });

  test("refuses a module without its memory and counter, of another interface, without a function, or whose counter is not an i64", () => {
    const version = { params: 0, result: true, locals: [0x00], code: [0x41, 0x82, 0x80, 0xe4, 0x82, 0x05, 0x0b], exported: "engine_abi_version" };
    const other = { ...version, code: [0x41, 0x83, 0x80, 0xe4, 0x82, 0x05, 0x0b] };
    const counter = (kind: readonly number[]): readonly (readonly [string, number, number])[] => [
      ["memory", 0x02, 0],
      ["instructions", kind[0] ?? 0, 0],
    ];
    const i64 = [0x7e, 0x01, 0x42, 0x00, 0x0b];
    const module = (functions: readonly (typeof version)[], globals: readonly (readonly number[])[], exports: readonly (readonly [string, number, number])[]): WebAssembly.Module =>
      new WebAssembly.Module(assemble({ functions, globals, exports }));
    expect(() => new CountedInstance(module([version], [i64], [["memory", 0x02, 0]]))).toThrow(new CountError('the module does not export its memory and its counter "instructions"'));
    expect(() => new CountedInstance(module([version], [i64], [["instructions", 0x03, 0]]))).toThrow(new CountError('the module does not export its memory and its counter "instructions"'));
    expect(() => new CountedInstance(module([other], [i64], counter([0x03])))).toThrow(new CountError("the module has the interface 0x50590003, expected 0x50590002"));
    expect(() => new CountedInstance(module([version], [i64], counter([0x03])))).toThrow(new CountError("the module has no function engine_input"));
    const stubs = ["engine_input", "engine_load", "engine_message_ptr", "engine_message_len", "engine_process"].map((name) => ({
      params: name === "engine_input" || name === "engine_load" ? 1 : 0,
      result: true,
      locals: [0x00],
      code: [0x41, 0x01, 0x0b],
      exported: name,
    }));
    const wrong = new CountedInstance(module([version, ...stubs], [I32_GLOBAL], counter([0x03])));
    expect(() => wrong.process(1)).toThrow(new CountError('the counter "instructions" is not an i64'));
  });

  test("names itself", () => {
    expect(new CountError("x").name).toBe("CountError");
  });
});
