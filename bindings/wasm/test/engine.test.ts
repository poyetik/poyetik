import { describe, expect, test } from "vitest";
import { ABI_VERSION, EngineTrap, ModuleError, PatchError, WasmEngine } from "../ts/engine.ts";
import { interleave, renderHash } from "../ts/hash.ts";
import { CORPUS, engineFunctions, HOST_BUFFER_SIZES, FUNCTIONS, oscillators, PATCH_NAMES, REFERENCES, SILENT_PATCH, SINE_PATCH, UNKNOWN_BOX, handMadeModule, wasmBytes } from "./fixtures.ts";

/** The built engine module, compiled once and instantiated afresh by every test. */
const module = new WebAssembly.Module(wasmBytes());
/** The sample rate of the tests, in hertz: the one of the sine-440 case. */
const RATE = 48_000;

/** Renders `frames` frames of the sine case through buffers of `size` frames. */
function renderSine(frames: number, size: number): Float32Array[] {
  const engine = WasmEngine.create(module, SINE_PATCH, RATE);
  const out = [new Float32Array(frames), new Float32Array(frames)];
  for (let start = 0; start < frames; start += size) {
    const buffers = [new Float32Array(Math.min(size, frames - start)), new Float32Array(Math.min(size, frames - start))];
    engine.render(buffers);
    buffers.forEach((buffer, channel) => out[channel]?.set(buffer, start));
  }
  return out;
}

describe("WasmEngine", () => {
  test("knows the whole corpus: every case has its patch and its reference, and nothing else", () => {
    const names = CORPUS.map((testCase) => testCase.name);
    expect(names.length).toBeGreaterThan(1);
    expect(PATCH_NAMES).toStrictEqual(names);
    expect([...REFERENCES.keys()].sort()).toStrictEqual(names);
  });

  test.each(CORPUS.flatMap((testCase) => HOST_BUFFER_SIZES.map((size) => [testCase.name, size, testCase] as const)))(
    "renders %s in Node bit-identical to the native engine, through buffers of %i frames",
    async (_name, size, testCase) => {
      const engine = WasmEngine.create(module, testCase.patch, testCase.rate);
      const channels = Array.from({ length: engine.channels }, () => new Float32Array(testCase.frames));
      for (let start = 0; start < testCase.frames; start += size) {
        const buffers = channels.map((channel) => channel.subarray(start, Math.min(start + size, testCase.frames)));
        engine.render(buffers);
      }
      expect(await renderHash(testCase.rate, engine.channels, interleave(channels))).toBe(REFERENCES.get(testCase.name));
    },
  );

  test("keeps every channel apart", () => {
    const render = (frequencies: readonly number[]): Float32Array[] => {
      const outputs = [new Float32Array(300), new Float32Array(300)];
      WasmEngine.create(module, oscillators(frequencies), RATE).render(outputs);
      return outputs;
    };
    const [left, right] = render([440, 880]);
    expect(left).toStrictEqual(render([440])[0]);
    expect(right).toStrictEqual(render([880])[0]);
    expect(left).not.toStrictEqual(right);
  });

  test.each(HOST_BUFFER_SIZES)("never renders a block ahead (buffers of %i frames)", (size) => {
    const engine = WasmEngine.create(module, SINE_PATCH, RATE);
    expect(engine.blocks).toBe(0);
    for (let output = size; output <= 1000; output += size) {
      engine.render([new Float32Array(size)]);
      expect(engine.blocks).toBe(Math.ceil(output / 64));
    }
    engine.render([]);
    engine.render([new Float32Array(0)]);
    expect(engine.blocks).toBe(Math.ceil(Math.floor(1000 / size) * size / 64));
  });

  test("reports its channels", () => {
    expect(WasmEngine.create(module, SINE_PATCH, RATE).channels).toBe(2);
    expect(WasmEngine.create(module, SILENT_PATCH, RATE).channels).toBe(0);
  });

  test("fills extra outputs with silence and drops extra channels", () => {
    const engine = WasmEngine.create(module, SINE_PATCH, RATE);
    const outputs = [new Float32Array(200), new Float32Array(200), new Float32Array(200).fill(7)];
    engine.render(outputs);
    const [reference] = renderSine(200, 200);
    expect(outputs[0]).toStrictEqual(reference);
    expect(outputs[1]).toStrictEqual(reference);
    expect(outputs[2]).toStrictEqual(new Float32Array(200));
    const mono = [new Float32Array(200)];
    WasmEngine.create(module, SINE_PATCH, RATE).render(mono);
    expect(mono[0]).toStrictEqual(reference);
  });

  test("renders nothing into no outputs", () => {
    const engine = WasmEngine.create(module, SINE_PATCH, RATE);
    engine.render([]);
    engine.render([new Float32Array(0)]);
    const next = [new Float32Array(10)];
    engine.render(next);
    expect(next[0]).toStrictEqual(renderSine(10, 10)[0]);
  });

  test("runs a patch with no output", () => {
    const outputs = [new Float32Array(100).fill(1)];
    WasmEngine.create(module, SILENT_PATCH, RATE).render(outputs);
    expect(outputs[0]).toStrictEqual(new Float32Array(100));
  });

  test("rejects a patch with the explanation of the engine", () => {
    const error = (() => {
      try {
        WasmEngine.create(module, UNKNOWN_BOX, RATE);
      } catch (thrown) {
        return thrown;
      }
      return undefined;
    })();
    expect(error).toBeInstanceOf(PatchError);
    expect(error).toMatchObject({ name: "PatchError", message: "the engine rejected the patch" });
    const detail = error instanceof PatchError ? error.detail : new Uint8Array();
    // A copy: it does not keep the memory of the module alive.
    expect(detail.buffer.byteLength).toBe(detail.length);
    expect(new TextDecoder().decode(detail)).toBe('node 1: unknown box type "nope~"');
  });

  test("rejects an unsupported sample rate", () => {
    expect(() => WasmEngine.create(module, SINE_PATCH, 0)).toThrow(PatchError);
  });

  test("rejects a module without memory", () => {
    const empty = handMadeModule({ memory: false, functions: engineFunctions() });
    expect(() => WasmEngine.create(empty, SINE_PATCH, RATE)).toThrow(new ModuleError("no memory"));
  });

  test.each(FUNCTIONS)("rejects a module without %s", (missing) => {
    const partial = handMadeModule({ memory: true, functions: engineFunctions().filter((func) => func.name !== missing) });
    expect(() => WasmEngine.create(partial, SINE_PATCH, RATE)).toThrow(new ModuleError(`no function ${missing}`));
  });

  test("rejects a function with the wrong number of arguments", () => {
    const functions = engineFunctions().map((func) => (func.name === "engine_load" ? { ...func, params: 0 } : func));
    expect(() => WasmEngine.create(handMadeModule({ memory: true, functions }), SINE_PATCH, RATE)).toThrow(
      new ModuleError("engine_load takes 0 argument(s), expected 1"),
    );
  });

  test("rejects a module whose functions do not return numbers", () => {
    const bigints = handMadeModule({ memory: true, functions: engineFunctions("i64") });
    expect(() => WasmEngine.create(bigints, SINE_PATCH, RATE)).toThrow(new ModuleError("engine_abi_version did not return a number"));
  });

  test("checks the interface identity and version before anything else", () => {
    expect(ABI_VERSION).toBe(0x5059_0001);
    // engineFunctions gives every function the same result: here, the version answered.
    // Far from ours, then one step from ours in the version, and in the identity both ways.
    for (const [answer, shown] of [
      [1, "0x1"],
      [0x5059_0002, "0x50590002"],
      [0x505a_0001, "0x505a0001"],
      [0x5058_0001, "0x50580001"],
    ] as const) {
      const other = handMadeModule({ memory: true, functions: engineFunctions({ i32: answer }) });
      expect(() => WasmEngine.create(other, SINE_PATCH, RATE)).toThrow(new ModuleError(`interface ${shown}, expected 0x50590001`));
    }
  });

  test("reads results from 2^31 on as unsigned", () => {
    // A module reporting 2^31 channels (i32 -2^31), with every other call returning 1.
    const functions = engineFunctions().map((func) => (func.name === "engine_channels" ? { ...func, body: { i32: -(2 ** 31) } } : func));
    expect(WasmEngine.create(handMadeModule({ memory: true, functions }), SINE_PATCH, RATE).channels).toBe(2 ** 31);
  });

  test("explains a module that cannot be instantiated", () => {
    const importing = handMadeModule({ memory: true, imports: true, functions: engineFunctions() });
    const error = (() => {
      try {
        WasmEngine.create(importing, SINE_PATCH, RATE);
      } catch (thrown) {
        return thrown;
      }
      return undefined;
    })();
    expect(error).toBeInstanceOf(ModuleError);
    expect(error instanceof Error ? error.message : "").toMatch(/^not an engine module: cannot be instantiated: TypeError: \S/);
    // Instantiated without imports: the WebAssembly engine refuses the missing import.
    expect(error instanceof Error ? error.cause : undefined).toBeInstanceOf(TypeError);
  });

  test("turns a trap into an EngineTrap, and never calls a trapped module again", () => {
    const loadTraps = engineFunctions().map((func) => (func.name === "engine_load" ? { ...func, body: "trap" as const } : func));
    const loadError = (() => {
      try {
        WasmEngine.create(handMadeModule({ memory: true, functions: loadTraps }), SINE_PATCH, RATE);
      } catch (thrown) {
        return thrown;
      }
      return undefined;
    })();
    expect(loadError).toBeInstanceOf(EngineTrap);
    expect(loadError instanceof Error ? loadError.cause : undefined).toBeInstanceOf(WebAssembly.RuntimeError);
    const processTraps = engineFunctions().map((func) => (func.name === "engine_process" ? { ...func, body: "trap" as const } : func));
    const engine = WasmEngine.create(handMadeModule({ memory: true, functions: processTraps }), SINE_PATCH, RATE);
    const error = (() => {
      try {
        engine.render([new Float32Array(1)]);
      } catch (thrown) {
        return thrown;
      }
      return undefined;
    })();
    expect(error).toBeInstanceOf(EngineTrap);
    expect(error instanceof Error ? error.cause : undefined).toBeInstanceOf(WebAssembly.RuntimeError);
    expect(() => {
      engine.render([new Float32Array(1)]);
    }).toThrow(new EngineTrap());
    expect(new EngineTrap()).toMatchObject({ name: "EngineTrap", message: "the engine module trapped and cannot be used again" });
  });

  test("lets other errors of the module through, and stops calling it", () => {
    const functions = engineFunctions().map((func) => (func.name === "engine_process" ? { ...func, body: "i64" as const } : func));
    const engine = WasmEngine.create(handMadeModule({ memory: true, functions }), SINE_PATCH, RATE);
    expect(() => {
      engine.render([new Float32Array(1)]);
    }).toThrow(new ModuleError("engine_process did not return a number"));
    expect(() => {
      engine.render([new Float32Array(1)]);
    }).toThrow(EngineTrap);
  });

  test("reads the block from the memory as it is after it grew", () => {
    // One channel of one-frame blocks; engine_process grows the memory, then writes 0.5.
    const functions = engineFunctions().map((func) => (func.name === "engine_process" ? { ...func, body: "grow" as const } : func));
    const engine = WasmEngine.create(handMadeModule({ memory: true, functions }), SINE_PATCH, RATE);
    const outputs = [new Float32Array(1)];
    engine.render(outputs);
    expect(outputs[0]?.[0]).toBe(0.5);
  });

  test("outputs silence for a block placed outside the memory", () => {
    // A module pointing its block past the end of its one page of memory.
    const functions = engineFunctions().map((func) => (func.name === "engine_process" ? { ...func, body: { i32: 70_000 } } : func));
    const engine = WasmEngine.create(handMadeModule({ memory: true, functions }), SINE_PATCH, RATE);
    const outputs = [new Float32Array(1).fill(9)];
    engine.render(outputs);
    expect(outputs[0]?.[0]).toBe(0);
  });

  test.each([48_000.5, -1, 2 ** 32 + 48_000, Number.NaN, Number.POSITIVE_INFINITY])("refuses the sample rate %d", (rate) => {
    expect(() => WasmEngine.create(module, SINE_PATCH, rate)).toThrow(new RangeError(`the sample rate must be a whole number of hertz, not ${String(rate)}`));
  });

  test("accepts the largest 32-bit sample rate, rejected by the engine", () => {
    expect(() => WasmEngine.create(module, SINE_PATCH, 0xff_ff_ff_ff)).toThrow(PatchError);
  });

  test("refuses outputs of different lengths, without rendering", () => {
    const engine = WasmEngine.create(module, SINE_PATCH, RATE);
    expect(() => {
      engine.render([new Float32Array(10), new Float32Array(9)]);
    }).toThrow(new RangeError("every output must have the same length: 9 instead of 10"));
    expect(engine.blocks).toBe(0);
  });

  test("names its errors", () => {
    expect(new ModuleError("x")).toMatchObject({ name: "ModuleError", message: "not an engine module: x" });
  });
});
