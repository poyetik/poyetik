import { describe, expect, test } from "vitest";
import { CountedInstance, CountError, countedEngine, countedModule } from "../../benchmarks/counter.ts";
import { instrument } from "../../benchmarks/instrument.ts";
import { ModuleError, RENDER_FORMATS, WasmEngine } from "../../ts/engine.ts";
import { interleave, interleave64, renderHash, STRICT } from "../../ts/hash.ts";
import { CORPUS, SINE_PATCH, UNKNOWN_BOX, reference, wasmBytes } from "../fixtures.ts";
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
  test.each(CORPUS.map((testCase) => [testCase.name, testCase] as const))("renders %s bit-identical to the native engine, as the module does, in every render format", async (name, testCase) => {
    const module = new WebAssembly.Module(instrument(new Uint8Array(wasmBytes())));
    const { patch, rate, frames, output } = testCase;
    const wide = WasmEngine.create(module, patch, rate, "binary64", output);
    const wideChannels = Array.from({ length: wide.channels }, () => new Float64Array(frames));
    wide.render(wideChannels);
    expect(await renderHash(rate, wide.channels, interleave64(wideChannels), "binary64", STRICT)).toBe(reference(name, "binary64"));
    for (const format of RENDER_FORMATS.filter((candidate) => candidate !== "binary64")) {
      const engine = WasmEngine.create(module, patch, rate, format, output);
      const channels = Array.from({ length: engine.channels }, () => new Float32Array(frames));
      engine.render(channels);
      expect(await renderHash(rate, engine.channels, interleave(channels), format, STRICT)).toBe(reference(name, format));
    }
  });
});

test("measures the engine as the bytes it was given, instrumented", () => {
  const bytes = new Uint8Array(wasmBytes());
  const engine = countedEngine(bytes);
  expect(engine.bytes).toBe(bytes.length);
  expect(() => new CountedInstance(engine.module)).not.toThrow();
});

describe("CountedInstance", () => {
  test("counts the same instructions for the same work, in every instance", () => {
    const counts = [0, 1].map(() => {
      const instance = new CountedInstance(counted());
      return [instance.load(SINE_PATCH, 48_000, "binary64-as-binary32"), instance.process(3)];
    });
    expect(counts[0]).toStrictEqual(counts[1]);
    expect(counts[0]?.every((count) => count > 0n)).toBe(true);
  });

  test("counts nothing for no block, and each block of a patch of no box alike", () => {
    const once = new CountedInstance(counted());
    once.load(EMPTY, 48_000, "binary64-as-binary32");
    expect(once.process(0)).toBe(0n);
    const block = once.process(1);
    expect(block).toBeGreaterThan(0n);
    const thrice = new CountedInstance(counted());
    thrice.load(EMPTY, 48_000, "binary64-as-binary32");
    expect(thrice.process(3)).toBe(3n * block);
  });

  test("loads in the render format asked, which changes what a block costs", () => {
    const cost = (format: "binary64" | "binary64-as-binary32" | "binary32"): bigint => {
      const instance = new CountedInstance(counted());
      instance.load(SINE_PATCH, 48_000, format);
      return instance.process(2);
    };
    const [wide, given, narrow] = [cost("binary64"), cost("binary64-as-binary32"), cost("binary32")];
    expect(given).toBeGreaterThan(wide);
    expect(narrow).not.toBe(given);
  });

  test("counts a load that grows with the patch", () => {
    const small = new CountedInstance(counted()).load(EMPTY, 48_000, "binary64-as-binary32");
    expect(new CountedInstance(counted()).load(SINE_PATCH, 48_000, "binary64-as-binary32")).toBeGreaterThan(small);
  });

  test("refuses a patch with the engine's words", () => {
    const instance = new CountedInstance(counted());
    expect(() => instance.load(UNKNOWN_BOX, 48_000, "binary64-as-binary32")).toThrow(CountError);
    expect(() => instance.load(UNKNOWN_BOX, 48_000, "binary64-as-binary32")).toThrow(/^the engine refused the patch: .*"nope~"/);
    expect(() => instance.load(SINE_PATCH, 7, "binary64-as-binary32")).toThrow(/^the engine refused the patch: .+/);
  });

  test("refuses a module without its counter or memory, of another interface, without a function or with another arity (read as the engine reads them), or whose counter is not an i64", () => {
    const version = { params: 0, result: true, locals: [0x00], code: [0x41, 0x85, 0x80, 0xe4, 0x82, 0x05, 0x0b], exported: "engine_abi_version" };
    const other = { ...version, code: [0x41, 0x86, 0x80, 0xe4, 0x82, 0x05, 0x0b] };
    const counter = (kind: readonly number[]): readonly (readonly [string, number, number])[] => [
      ["memory", 0x02, 0],
      ["instructions", kind[0] ?? 0, 0],
    ];
    const i64 = [0x7e, 0x01, 0x42, 0x00, 0x0b];
    const module = (functions: readonly (typeof version)[], globals: readonly (readonly number[])[], exports: readonly (readonly [string, number, number])[]): WebAssembly.Module =>
      new WebAssembly.Module(assemble({ functions, globals, exports }));
    expect(() => new CountedInstance(module([version], [i64], [["memory", 0x02, 0]]))).toThrow(new CountError('the module does not export its counter "instructions"'));
    expect(() => new CountedInstance(module([version], [i64], [["instructions", 0x03, 0]]))).toThrow(new ModuleError("no memory"));
    expect(() => new CountedInstance(module([other], [i64], counter([0x03])))).toThrow(new ModuleError("interface 0x50590006, expected 0x50590005"));
    expect(() => new CountedInstance(module([version], [i64], counter([0x03])))).toThrow(new ModuleError("no function engine_input"));
    const arities: Readonly<Record<string, number>> = {
      engine_input: 1,
      engine_load: 2,
      engine_message_ptr: 0,
      engine_message_len: 0,
      engine_channels: 0,
      engine_block_size: 0,
      engine_process: 0,
      engine_max_patch_bytes: 0,
      engine_subnormals: 0,
      engine_replaced: 0,
      engine_output: 7,
      engine_limiter: 4,
      engine_protect: 1,
      engine_latency: 0,
      engine_lookahead_pending: 0,
    };
    const stubbed = (arity: (name: string, expected: number) => number): (typeof version)[] =>
      Object.entries(arities).map(([name, expected]) => ({ params: arity(name, expected), result: true, locals: [0x00], code: [0x41, 0x01, 0x0b], exported: name }));
    const stubs = stubbed((_name, expected) => expected);
    expect(() => new CountedInstance(module([version, ...stubbed((name, expected) => (name === "engine_load" ? 1 : expected))], [i64], counter([0x03])))).toThrow(
      new ModuleError("engine_load takes 1 argument(s), expected 2"),
    );
    expect(() => new CountedInstance(module([version, ...stubbed((name, expected) => (name === "engine_process" ? 1 : expected))], [i64], counter([0x03])))).toThrow(
      new ModuleError("engine_process takes 1 argument(s), expected 0"),
    );
    for (const [result, unsigned] of [[2, 2], [-1, 0xff_ff_ff_ff]] as const) {
      const loading = stubs.map((stub) => (stub.exported === "engine_load" ? { ...stub, code: [0x41, result & 0x7f, 0x0b] } : stub));
      expect(() => new CountedInstance(module([version, ...loading], [i64], counter([0x03]))).load(SINE_PATCH, 48_000, "binary64-as-binary32")).toThrow(
        new ModuleError(`engine_load returned ${String(unsigned)}, not 1 (succeeded) or 0 (refused)`),
      );
    }
    const wrong = new CountedInstance(module([version, ...stubs], [I32_GLOBAL], counter([0x03])));
    expect(() => wrong.process(1)).toThrow(new CountError('the counter "instructions" is not an i64'));
  });

  test("names itself", () => {
    expect(new CountError("x").name).toBe("CountError");
  });
});
