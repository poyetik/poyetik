import { describe, expect, test, vi } from "vitest";
import { ABI_VERSION, EngineTrap, ModuleError, PatchError, WasmEngine } from "../ts/engine.ts";
import { interleave, renderHash } from "../ts/hash.ts";
import { Counted, created } from "./counted.ts";
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
    expect(new TextDecoder().decode(detail)).toBe('box 1: unknown box type "nope~"');
  });

  test("hands JavaScript an explanation on one escaped, bounded line", () => {
    const explanation = (patch: string): string => {
      try {
        WasmEngine.create(module, new TextEncoder().encode(patch), RATE);
      } catch (thrown) {
        return thrown instanceof PatchError ? new TextDecoder().decode(thrown.detail) : String(thrown);
      }
      return "loaded";
    };
    expect(explanation(String.raw`{"format":1,"nodes":[{"id":1,"box":"x\u001b[31m\nsecond‮"}],"connections":[]}`)).toBe(
      String.raw`box 1: unknown box type "x\u{1b}[31m\u{a}second\u{202e}"`,
    );
    const long = explanation(`{"format":1,"nodes":[{"id":1,"box":"${"y".repeat(5000)}"}],"connections":[]}`);
    // 1,000 characters, each one UTF-16 unit here (ASCII and the closing "…").
    expect(long).toHaveLength(1000);
    expect(long).toMatch(/^box 1: unknown box type "y+…$/u);
  });

  test("refuses a patch longer than the engine reads, as the engine words it", () => {
    const exports = new WebAssembly.Instance(module, {}).exports;
    const call = (name: string, ...args: number[]): number => {
      const exported = exports[name];
      return typeof exported === "function" ? Number(Reflect.apply(exported, undefined, args)) : Number.NaN;
    };
    // The limit of the patch format (spec/patch-format.md), as the module reports it.
    const max = call("engine_max_patch_bytes");
    expect(max).toBe(16 * 1024 * 1024);
    // What the engine itself says of one byte more, read through the raw interface.
    call("engine_input", max + 1);
    expect(call("engine_load", RATE)).toBe(0);
    const memory = exports["memory"];
    const said = memory instanceof WebAssembly.Memory ? new Uint8Array(memory.buffer, call("engine_message_ptr"), call("engine_message_len")) : new Uint8Array();
    const engineSays = new TextDecoder().decode(said);
    expect(engineSays).toBe(`patch too large: ${String(max + 1)} bytes (at most ${String(max)})`);
    const refusal = (patch: Uint8Array): string => {
      try {
        WasmEngine.create(module, patch, RATE);
      } catch (thrown) {
        return thrown instanceof PatchError ? new TextDecoder().decode(thrown.detail) : String(thrown);
      }
      return "loaded";
    };
    expect(refusal(new Uint8Array(max + 1))).toBe(engineSays);
    // At the limit, the patch reaches the engine, which reads it (blank: not JSON).
    expect(refusal(new Uint8Array(max).fill(0x20))).toMatch(/^invalid patch: EOF while parsing/u);
  });

  test("refuses a patch longer than the module reads before asking it for memory", () => {
    // engine_input traps: calling it at all is an EngineTrap, never a PatchError.
    const functions = engineFunctions().map((func) =>
      func.name === "engine_input" ? { ...func, body: "trap" as const } : func.name === "engine_max_patch_bytes" ? { ...func, body: { i32: 4 } } : func,
    );
    const small = handMadeModule({ memory: true, functions });
    expect(() => WasmEngine.create(small, new Uint8Array(5), RATE)).toThrow(PatchError);
    expect(() => WasmEngine.create(small, new Uint8Array(4), RATE)).toThrow(EngineTrap);
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
    expect(ABI_VERSION).toBe(0x5059_0002);
    // engineFunctions gives every function the same result: here, the version answered.
    // Far from ours, then one step from ours in the version both ways, and in the identity
    // both ways.
    for (const [answer, shown] of [
      [1, "0x1"],
      [0x5059_0001, "0x50590001"],
      [0x5059_0003, "0x50590003"],
      [0x505a_0002, "0x505a0002"],
      [0x5058_0002, "0x50580002"],
    ] as const) {
      const other = handMadeModule({ memory: true, functions: engineFunctions({ i32: answer }) });
      expect(() => WasmEngine.create(other, SINE_PATCH, RATE)).toThrow(new ModuleError(`interface ${shown}, expected 0x50590002`));
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

  /** An engine of `channels` channels of `blockSize`-frame blocks, whose `engine_process` gives `pointer`. */
  function placing(channels: number, blockSize: number, pointer: number): WasmEngine {
    const answers: Readonly<Record<string, number>> = { engine_channels: channels, engine_block_size: blockSize, engine_process: pointer };
    const functions = engineFunctions().map((func) => {
      const answer = answers[func.name];
      return answer === undefined ? func : { ...func, body: { i32: answer } };
    });
    return WasmEngine.create(handMadeModule({ memory: true, functions }), SINE_PATCH, RATE);
  }

  test("refuses a module whose blocks have no frame, which would render forever", () => {
    expect(() => placing(1, 0, 4)).toThrow(new ModuleError("engine_block_size returned 0: a block has at least one frame"));
    expect(placing(1, 1, 4).blocks).toBe(0);
  });

  test("reads a block that ends at the end of the memory", () => {
    // One page: 16 384 samples; two channels of 4 frames from sample 16 376 fill its end.
    const outputs = [new Float32Array(4).fill(9), new Float32Array(4).fill(9)];
    placing(2, 4, 65_536 - 32).render(outputs);
    expect(outputs).toStrictEqual([new Float32Array(4), new Float32Array(4)]);
  });

  test("reads a first block at the first sample index 1", () => {
    // The module copies the patch from address 1: its bytes 3 to 6 are the sample at address 4.
    const outputs = [new Float32Array(1)];
    placing(1, 1, 4).render(outputs);
    expect(outputs[0]).toStrictEqual(new Float32Array([new DataView(SINE_PATCH.buffer, SINE_PATCH.byteOffset).getFloat32(3, true)]));
    expect(outputs[0]?.[0]).not.toBe(0);
  });

  test.each([
    [65_536 - 28, "outside the memory by one sample"],
    [70_000, "outside the memory"],
    [65_536 - 34, "unaligned"],
  ])("refuses a block at the address %i (%s), then never calls the module again", (pointer) => {
    const engine = placing(2, 4, pointer);
    const outputs = [new Float32Array(4).fill(9), new Float32Array(4).fill(9)];
    expect(() => {
      engine.render(outputs);
    }).toThrow(new ModuleError(`engine_process returned the block address ${String(pointer)}, unaligned or outside the memory`));
    expect(() => {
      engine.render(outputs);
    }).toThrow(new EngineTrap());
    expect(engine.blocks).toBe(0);
  });

  test("follows a block that moves without the memory growing", () => {
    // One channel of one-frame blocks, at address 4096 (silent), then 2048 (0.25), and so on.
    const functions = engineFunctions().map((func) => (func.name === "engine_process" ? { ...func, body: "alternate" as const } : func));
    const engine = WasmEngine.create(handMadeModule({ memory: true, functions }), SINE_PATCH, RATE);
    const outputs = [new Float32Array(3)];
    engine.render(outputs);
    expect(outputs[0]).toStrictEqual(new Float32Array([0, 0.25, 0]));
  });

  test("creates its views of the memory once, not at every block", () => {
    vi.stubGlobal("Float32Array", Counted);
    try {
      const engine = WasmEngine.create(module, SINE_PATCH, RATE);
      const outputs = [new Float32Array(64), new Float32Array(64)];
      // The first block: one view of each channel; then none, block after block.
      expect(created(() => { engine.render(outputs); })).toBe(2);
      expect(created(() => { for (let block = 0; block < 100; block += 1) engine.render(outputs); })).toBe(0);

      const one = [new Float32Array(1)];
      // A block that moves: new views of the same memory.
      const moving = engineFunctions().map((func) => (func.name === "engine_process" ? { ...func, body: "alternate" as const } : func));
      const alternating = WasmEngine.create(handMadeModule({ memory: true, functions: moving }), SINE_PATCH, RATE);
      expect(created(() => { alternating.render(one); })).toBe(1);
      expect(created(() => { alternating.render(one); })).toBe(1);

      // A memory that grows: a new view of it, and of the block.
      const growing = engineFunctions().map((func) => (func.name === "engine_process" ? { ...func, body: "grow" as const } : func));
      const grown = WasmEngine.create(handMadeModule({ memory: true, functions: growing }), SINE_PATCH, RATE);
      expect(created(() => { grown.render(one); })).toBe(2);
      expect(created(() => { grown.render(one); })).toBe(2);
    } finally {
      vi.unstubAllGlobals();
    }
  });

  test("copies a whole block at once, and a partial one sample by sample", () => {
    /** An output that counts the copies made into it with `set`. */
    class Recorded extends Float32Array {
      /** Calls of `set`. */
      copies = 0;

      /** Copies `array` into this one from `offset`, and counts the copy. */
      override set(array: ArrayLike<number>, offset?: number): void {
        this.copies += 1;
        super.set(array, offset);
      }
    }
    const engine = WasmEngine.create(module, SINE_PATCH, RATE);
    // 160 frames: two whole blocks, then half of one.
    const outputs = [new Recorded(160), new Recorded(160)];
    engine.render(outputs);
    expect(outputs.map((output) => output.copies)).toStrictEqual([2, 2]);
    expect(Array.from(outputs[0] ?? [])).toStrictEqual(Array.from(renderSine(160, 160)[0] ?? []));
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
