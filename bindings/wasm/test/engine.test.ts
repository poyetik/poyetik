import { describe, expect, test, vi } from "vitest";
import { ABI_VERSION, EngineTrap, ModuleError, PatchError, RENDER_FORMATS, WasmEngine, givenAsBinary32, type RenderFormat } from "../ts/engine.ts";
import { interleave, interleave64, renderHash } from "../ts/hash.ts";
import { Counted, created } from "./counted.ts";
import { CORPUS, engineFunctions, HOST_BUFFER_SIZES, FUNCTIONS, oscillators, PATCH_NAMES, PRECISION_NAMES, REFERENCES, SILENT_PATCH, SINE_PATCH, UNKNOWN_BOX, handMadeModule, NAN_BITS, reference, wasmBytes, type CorpusCase } from "./fixtures.ts";

/** The built engine module, compiled once and instantiated afresh by every test. */
const module = new WebAssembly.Module(wasmBytes());
/** The sample rate of the tests, in hertz: the one of the sine-440 case. */
const RATE = 48_000;

/** Renders `frames` frames of the sine case through buffers of `size` frames. */
function renderSine(frames: number, size: number): Float32Array[] {
  const engine = WasmEngine.create(module, SINE_PATCH, RATE, "binary64-as-binary32");
  const out = [new Float32Array(frames), new Float32Array(frames)];
  for (let start = 0; start < frames; start += size) {
    const buffers = [new Float32Array(Math.min(size, frames - start)), new Float32Array(Math.min(size, frames - start))];
    engine.render(buffers);
    buffers.forEach((buffer, channel) => out[channel]?.set(buffer, start));
  }
  return out;
}

/** The render hash of `testCase` rendered in `format` through buffers of `size` frames. */
async function corpusHash(testCase: CorpusCase, size: number, format: RenderFormat): Promise<string> {
  const { rate, frames, patch } = testCase;
  if (format === "binary64") {
    const wide = WasmEngine.create(module, patch, rate, format);
    const outputs = Array.from({ length: wide.channels }, () => new Float64Array(frames));
    for (let start = 0; start < frames; start += size) {
      wide.render(outputs.map((output) => output.subarray(start, Math.min(start + size, frames))));
    }
    return renderHash(rate, wide.channels, interleave64(outputs), format);
  }
  const engine = WasmEngine.create(module, patch, rate, format);
  const channels = Array.from({ length: engine.channels }, () => new Float32Array(frames));
  for (let start = 0; start < frames; start += size) {
    engine.render(channels.map((channel) => channel.subarray(start, Math.min(start + size, frames))));
  }
  return renderHash(rate, engine.channels, interleave(channels), format);
}

/**
 * The sum of `terms` in the reduction order of `spec/engine-design.md`, decision 28, written
 * literally from the rule, apart from the engine: no term is +0; term `k` goes to lane
 * `k mod 16`, each lane summing its terms in increasing `k` from -0; then lane `i` with lane
 * `i + 8`, then `i + 4`, `i + 2`, `i + 1`. `add` is the addition of the precision.
 */
function reductionOrder(terms: readonly number[], add: (left: number, right: number) => number): number {
  if (terms.length === 0) {
    return 0;
  }
  const lanes = Array.from({ length: 16 }, () => -0);
  terms.forEach((term, index) => {
    lanes[index % 16] = add(lanes[index % 16] ?? Number.NaN, term);
  });
  for (const half of [8, 4, 2, 1]) {
    for (let lane = 0; lane < half; lane += 1) {
      lanes[lane] = add(lanes[lane] ?? Number.NaN, lanes[lane + half] ?? Number.NaN);
    }
  }
  return lanes[0] ?? Number.NaN;
}

/**
 * A patch of oscillators at `frequencies` (ids 1 to n) summed onto channel 0: all into the
 * left inlet of one `dac~` (id n + 1), or each into the left inlet of its own `dac~` (ids
 * n + 1 to 2n, run in that order).
 */
function summedPatch(frequencies: readonly number[], outputs: "one" | "each"): Uint8Array {
  const count = frequencies.length;
  const oscillatorNodes = frequencies.map((frequency, index) => ({ id: index + 1, box: "osc~", params: { frequency } }));
  const dacs = outputs === "one" ? [{ id: count + 1, box: "dac~" }] : frequencies.map((_frequency, index) => ({ id: count + index + 1, box: "dac~" }));
  const connections = frequencies.map((_frequency, index) => ({ from: [index + 1, 0], to: [outputs === "one" ? count + 1 : count + index + 1, 0] }));
  return new TextEncoder().encode(JSON.stringify({ format: 1, nodes: [...oscillatorNodes, ...dacs], connections }));
}

/** The first `frames` frames of channel 0 of `patch`, computed in `precision`, as binary64 values, and the bytes of their samples. */
function channelZero(patch: Uint8Array, precision: "binary64" | "binary32", frames: number): { readonly values: number[]; readonly bytes: Uint8Array } {
  if (precision === "binary64") {
    const wide = WasmEngine.create(module, patch, RATE, "binary64");
    const wideOutputs = Array.from({ length: wide.channels }, () => new Float64Array(frames));
    wide.render(wideOutputs);
    const wideLeft = wideOutputs[0] ?? new Float64Array(0);
    return { values: Array.from(wideLeft), bytes: new Uint8Array(wideLeft.buffer) };
  }
  const engine = WasmEngine.create(module, patch, RATE, "binary32");
  const outputs = Array.from({ length: engine.channels }, () => new Float32Array(frames));
  engine.render(outputs);
  const left = outputs[0] ?? new Float32Array(0);
  return { values: Array.from(left), bytes: new Uint8Array(left.buffer) };
}

describe("WasmEngine", () => {
  test("knows the whole corpus: every case has its patch and its reference, and nothing else", () => {
    const names = CORPUS.map((testCase) => testCase.name);
    expect(names.length).toBeGreaterThan(1);
    expect(PATCH_NAMES).toStrictEqual(names);
    expect([...REFERENCES.keys()].sort()).toStrictEqual(names);
  });

  test.each(RENDER_FORMATS.flatMap((format) => CORPUS.flatMap((testCase) => HOST_BUFFER_SIZES.map((size) => [testCase.name, format, size, testCase] as const))))(
    "renders %s in %s in Node bit-identical to the native engine, through buffers of %i frames",
    async (name, format, size, testCase) => {
      expect(await corpusHash(testCase, size, format)).toBe(reference(name, format));
    },
  );

  test("renders every precision of the test matrix, given as binary32 or as binary64", () => {
    expect(PRECISION_NAMES).toStrictEqual(["binary64", "binary32"]);
    expect(PRECISION_NAMES.map(givenAsBinary32)).toStrictEqual(["binary64-as-binary32", "binary32"]);
    expect(RENDER_FORMATS).toStrictEqual(["binary64", "binary64-as-binary32", "binary32"]);
  });

  test("gives a 64-bit render in binary64, or rounded to binary32, and a 32-bit one in binary32", () => {
    const frames = 300;
    const wide = [new Float64Array(frames)];
    WasmEngine.create(module, oscillators([1000.5]), RATE, "binary64").render(wide);
    const given = [new Float32Array(frames)];
    WasmEngine.create(module, oscillators([1000.5]), RATE, "binary64-as-binary32").render(given);
    const narrow = [new Float32Array(frames)];
    WasmEngine.create(module, oscillators([1000.5]), RATE, "binary32").render(narrow);
    expect(Array.from(given[0] ?? [])).toStrictEqual(Array.from(wide[0] ?? [], (sample) => Math.fround(sample)));
    expect(Array.from(wide[0] ?? []).some((sample) => Math.fround(sample) !== sample)).toBe(true);
    expect(narrow[0]).not.toStrictEqual(given[0]);
  });

  test.each(PRECISION_NAMES)("sums cables and output boxes in the reduction order in %s, at every count around the multiples of 16", (precision) => {
    // Each oscillator alone gives its terms bit for bit (one cable is a copy); the sum of n
    // of them, through one inlet or n output boxes, must be the reduction order of those
    // terms, computed here in the precision of the render (binary32: each sum rounded).
    const frames = 64;
    const add = precision === "binary64" ? (left: number, right: number) => left + right : (left: number, right: number) => Math.fround(left + right);
    const frequencies = Array.from({ length: 100 }, (_entry, index) => 100 + 173.5 * index);
    const alone = frequencies.map((frequency) => channelZero(summedPatch([frequency], "one"), precision, frames).values);
    let revealed = false;
    for (const count of [0, 1, 2, 3, 15, 16, 17, 31, 32, 33, 48, 49, 100]) {
      const terms = alone.slice(0, count);
      const expected = Array.from({ length: frames }, (_entry, frame) => reductionOrder(terms.map((samples) => samples[frame] ?? Number.NaN), add));
      const bytes = precision === "binary64" ? new Uint8Array(Float64Array.from(expected).buffer) : new Uint8Array(Float32Array.from(expected).buffer);
      // No term: one unconnected output (n output boxes would be none, and no channel).
      for (const outputs of count === 0 ? (["one"] as const) : (["one", "each"] as const)) {
        expect(channelZero(summedPatch(frequencies.slice(0, count), outputs), precision, frames).bytes, `${String(count)} terms, ${outputs}`).toStrictEqual(bytes);
      }
      const sequential = Array.from({ length: frames }, (_entry, frame) => terms.reduce((total, samples) => add(total, samples[frame] ?? Number.NaN), 0));
      revealed ||= sequential.some((value, frame) => !Object.is(value, expected[frame]));
    }
    // The terms do reveal the order: a sequential sum differs somewhere.
    expect(revealed).toBe(true);
  });

  test("keeps every channel apart", () => {
    const render = (frequencies: readonly number[]): Float32Array[] => {
      const outputs = [new Float32Array(300), new Float32Array(300)];
      WasmEngine.create(module, oscillators(frequencies), RATE, "binary64-as-binary32").render(outputs);
      return outputs;
    };
    const [left, right] = render([440, 880]);
    expect(left).toStrictEqual(render([440])[0]);
    expect(right).toStrictEqual(render([880])[0]);
    expect(left).not.toStrictEqual(right);
  });

  test.each(HOST_BUFFER_SIZES)("never renders a block ahead (buffers of %i frames)", (size) => {
    const engine = WasmEngine.create(module, SINE_PATCH, RATE, "binary64-as-binary32");
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
    expect(WasmEngine.create(module, SINE_PATCH, RATE, "binary64-as-binary32").channels).toBe(2);
    expect(WasmEngine.create(module, SILENT_PATCH, RATE, "binary64-as-binary32").channels).toBe(0);
  });

  test("fills extra outputs with silence and drops extra channels", () => {
    const engine = WasmEngine.create(module, SINE_PATCH, RATE, "binary64-as-binary32");
    const outputs = [new Float32Array(200), new Float32Array(200), new Float32Array(200).fill(7)];
    engine.render(outputs);
    const [sine] = renderSine(200, 200);
    expect(outputs[0]).toStrictEqual(sine);
    expect(outputs[1]).toStrictEqual(sine);
    expect(outputs[2]).toStrictEqual(new Float32Array(200));
    const mono = [new Float32Array(200)];
    WasmEngine.create(module, SINE_PATCH, RATE, "binary64-as-binary32").render(mono);
    expect(mono[0]).toStrictEqual(sine);
  });

  test("renders nothing into no outputs", () => {
    const engine = WasmEngine.create(module, SINE_PATCH, RATE, "binary64-as-binary32");
    engine.render([]);
    engine.render([new Float32Array(0)]);
    const next = [new Float32Array(10)];
    engine.render(next);
    expect(next[0]).toStrictEqual(renderSine(10, 10)[0]);
  });

  test("runs a patch with no output", () => {
    const outputs = [new Float32Array(100).fill(1)];
    WasmEngine.create(module, SILENT_PATCH, RATE, "binary64-as-binary32").render(outputs);
    expect(outputs[0]).toStrictEqual(new Float32Array(100));
  });

  test("rejects a patch with the explanation of the engine", () => {
    const error = (() => {
      try {
        WasmEngine.create(module, UNKNOWN_BOX, RATE, "binary64-as-binary32");
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
        WasmEngine.create(module, new TextEncoder().encode(patch), RATE, "binary64-as-binary32");
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
    expect(call("engine_load", RATE, 1)).toBe(0);
    const memory = exports["memory"];
    const said = memory instanceof WebAssembly.Memory ? new Uint8Array(memory.buffer, call("engine_message_ptr"), call("engine_message_len")) : new Uint8Array();
    const engineSays = new TextDecoder().decode(said);
    expect(engineSays).toBe(`patch too large: ${String(max + 1)} bytes (at most ${String(max)})`);
    const refusal = (patch: Uint8Array): string => {
      try {
        WasmEngine.create(module, patch, RATE, "binary64-as-binary32");
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
    expect(() => WasmEngine.create(small, new Uint8Array(5), RATE, "binary64-as-binary32")).toThrow(PatchError);
    expect(() => WasmEngine.create(small, new Uint8Array(4), RATE, "binary64-as-binary32")).toThrow(EngineTrap);
  });

  test("rejects an unsupported sample rate", () => {
    expect(() => WasmEngine.create(module, SINE_PATCH, 0, "binary64-as-binary32")).toThrow(PatchError);
  });

  test("rejects a module without memory", () => {
    const empty = handMadeModule({ memory: false, functions: engineFunctions() });
    expect(() => WasmEngine.create(empty, SINE_PATCH, RATE, "binary64-as-binary32")).toThrow(new ModuleError("no memory"));
  });

  test.each(FUNCTIONS)("rejects a module without %s", (missing) => {
    const partial = handMadeModule({ memory: true, functions: engineFunctions().filter((func) => func.name !== missing) });
    expect(() => WasmEngine.create(partial, SINE_PATCH, RATE, "binary64-as-binary32")).toThrow(new ModuleError(`no function ${missing}`));
  });

  test("rejects a function with the wrong number of arguments", () => {
    const functions = engineFunctions().map((func) => (func.name === "engine_load" ? { ...func, params: 0 } : func));
    expect(() => WasmEngine.create(handMadeModule({ memory: true, functions }), SINE_PATCH, RATE, "binary64-as-binary32")).toThrow(
      new ModuleError("engine_load takes 0 argument(s), expected 2"),
    );
  });

  test("rejects a module whose functions do not return numbers", () => {
    const bigints = handMadeModule({ memory: true, functions: engineFunctions("i64") });
    expect(() => WasmEngine.create(bigints, SINE_PATCH, RATE, "binary64-as-binary32")).toThrow(new ModuleError("engine_abi_version did not return a number"));
  });

  test("checks the interface identity and version before anything else", () => {
    expect(ABI_VERSION).toBe(0x5059_0003);
    // engineFunctions gives every function the same result: here, the version answered.
    // Far from ours, then one step from ours in the version both ways, and in the identity
    // both ways.
    for (const [answer, shown] of [
      [1, "0x1"],
      [0x5059_0002, "0x50590002"],
      [0x5059_0004, "0x50590004"],
      [0x505a_0003, "0x505a0003"],
      [0x5058_0003, "0x50580003"],
    ] as const) {
      const other = handMadeModule({ memory: true, functions: engineFunctions({ i32: answer }) });
      expect(() => WasmEngine.create(other, SINE_PATCH, RATE, "binary64-as-binary32")).toThrow(new ModuleError(`interface ${shown}, expected 0x50590003`));
    }
  });

  test("reads results from 2^31 on as unsigned", () => {
    // A module reporting 2^31 channels (i32 -2^31), with every other call returning 1.
    const functions = engineFunctions().map((func) => (func.name === "engine_channels" ? { ...func, body: { i32: -(2 ** 31) } } : func));
    expect(WasmEngine.create(handMadeModule({ memory: true, functions }), SINE_PATCH, RATE, "binary64-as-binary32").channels).toBe(2 ** 31);
  });

  test("explains a module that cannot be instantiated", () => {
    const importing = handMadeModule({ memory: true, imports: true, functions: engineFunctions() });
    const error = (() => {
      try {
        WasmEngine.create(importing, SINE_PATCH, RATE, "binary64-as-binary32");
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
        WasmEngine.create(handMadeModule({ memory: true, functions: loadTraps }), SINE_PATCH, RATE, "binary64-as-binary32");
      } catch (thrown) {
        return thrown;
      }
      return undefined;
    })();
    expect(loadError).toBeInstanceOf(EngineTrap);
    expect(loadError instanceof Error ? loadError.cause : undefined).toBeInstanceOf(WebAssembly.RuntimeError);
    const processTraps = engineFunctions().map((func) => (func.name === "engine_process" ? { ...func, body: "trap" as const } : func));
    const engine = WasmEngine.create(handMadeModule({ memory: true, functions: processTraps }), SINE_PATCH, RATE, "binary64-as-binary32");
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
    const engine = WasmEngine.create(handMadeModule({ memory: true, functions }), SINE_PATCH, RATE, "binary64-as-binary32");
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
    const engine = WasmEngine.create(handMadeModule({ memory: true, functions }), SINE_PATCH, RATE, "binary64-as-binary32");
    const outputs = [new Float32Array(1)];
    engine.render(outputs);
    expect(outputs[0]?.[0]).toBe(0.5);
  });

  /** A hand-made module of `channels` channels of `blockSize`-frame blocks, whose `engine_process` gives `pointer`. */
  function placed(channels: number, blockSize: number, pointer: number): WebAssembly.Module {
    const answers: Readonly<Record<string, number>> = { engine_channels: channels, engine_block_size: blockSize, engine_process: pointer };
    const functions = engineFunctions().map((func) => {
      const answer = answers[func.name];
      return answer === undefined ? func : { ...func, body: { i32: answer } };
    });
    return handMadeModule({ memory: true, functions });
  }

  /** A binary32 engine of `channels` channels of `blockSize`-frame blocks, whose `engine_process` gives `pointer`. */
  function placing(channels: number, blockSize: number, pointer: number): WasmEngine<Float32Array> {
    return WasmEngine.create(placed(channels, blockSize, pointer), SINE_PATCH, RATE, "binary64-as-binary32");
  }

  /** The same, rendering binary64: 8 bytes per sample. */
  function placing64(channels: number, blockSize: number, pointer: number): WasmEngine<Float64Array> {
    return WasmEngine.create(placed(channels, blockSize, pointer), SINE_PATCH, RATE, "binary64");
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

  test("reads a binary64 block of 8-byte samples, whole or in part, up to the end of the memory", () => {
    // One page: 8192 binary64 samples; two channels of 4 frames from sample 8184 fill its end.
    const outputs = [new Float64Array(4).fill(9), new Float64Array(4).fill(9)];
    placing64(2, 4, 65_536 - 64).render(outputs);
    expect(outputs).toStrictEqual([new Float64Array(4), new Float64Array(4)]);
    // The patch is copied from address 1: bytes 7 to 14 are the sample at address 8, read
    // whole (a block of one frame) and in part (one frame of a block of two).
    const expected = new DataView(SINE_PATCH.buffer, SINE_PATCH.byteOffset).getFloat64(7, true);
    const whole = [new Float64Array(1)];
    placing64(1, 1, 8).render(whole);
    expect(whole[0]).toStrictEqual(new Float64Array([expected]));
    const part = [new Float64Array(1)];
    placing64(1, 2, 8).render(part);
    expect(part[0]).toStrictEqual(new Float64Array([expected]));
    expect(expected).not.toBe(0);
  });

  test.each([
    [65_536 - 56, "outside the memory by one sample"],
    [65_536 - 60, "unaligned on 8 bytes"],
  ])("refuses a binary64 block at the address %i (%s)", (pointer) => {
    expect(() => {
      placing64(2, 4, pointer).render([new Float64Array(4)]);
    }).toThrow(new ModuleError(`engine_process returned the block address ${String(pointer)}, unaligned or outside the memory`));
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
    const engine = WasmEngine.create(handMadeModule({ memory: true, functions }), SINE_PATCH, RATE, "binary64-as-binary32");
    const outputs = [new Float32Array(3)];
    engine.render(outputs);
    expect(outputs[0]).toStrictEqual(new Float32Array([0, 0.25, 0]));
  });

  test("creates its views of the memory once, not at every block", () => {
    vi.stubGlobal("Float32Array", Counted);
    try {
      const engine = WasmEngine.create(module, SINE_PATCH, RATE, "binary64-as-binary32");
      const outputs = [new Float32Array(64), new Float32Array(64)];
      // The first block: one view of each channel; then none, block after block.
      expect(created(() => { engine.render(outputs); })).toBe(2);
      expect(created(() => { for (let block = 0; block < 100; block += 1) engine.render(outputs); })).toBe(0);

      const one = [new Float32Array(1)];
      // A block that moves: new views of the same memory.
      const moving = engineFunctions().map((func) => (func.name === "engine_process" ? { ...func, body: "alternate" as const } : func));
      const alternating = WasmEngine.create(handMadeModule({ memory: true, functions: moving }), SINE_PATCH, RATE, "binary64-as-binary32");
      expect(created(() => { alternating.render(one); })).toBe(1);
      expect(created(() => { alternating.render(one); })).toBe(1);

      // A memory that grows: a new view of it, and of the block.
      const growing = engineFunctions().map((func) => (func.name === "engine_process" ? { ...func, body: "grow" as const } : func));
      const grown = WasmEngine.create(handMadeModule({ memory: true, functions: growing }), SINE_PATCH, RATE, "binary64-as-binary32");
      expect(created(() => { grown.render(one); })).toBe(2);
      expect(created(() => { grown.render(one); })).toBe(2);
    } finally {
      vi.unstubAllGlobals();
    }
  });

  test("copies a part of a block bit for bit, a NaN payload included", () => {
    // Two-frame blocks of one channel holding a NaN with a payload: one frame, a part of the
    // block, keeps its bits, as does the next frame.
    const answers: Readonly<Record<string, number>> = { engine_channels: 1, engine_block_size: 2 };
    const functions = engineFunctions().map((func) => {
      const answer = answers[func.name];
      return func.name === "engine_process" ? { ...func, body: "nan" as const } : answer === undefined ? func : { ...func, body: { i32: answer } };
    });
    const engine = WasmEngine.create(handMadeModule({ memory: true, functions }), SINE_PATCH, RATE, "binary64-as-binary32");
    const first = new Float32Array(1);
    engine.render([first]);
    const second = new Float32Array(1);
    engine.render([second]);
    expect([...new Uint32Array(first.buffer), ...new Uint32Array(second.buffer)]).toStrictEqual([NAN_BITS, NAN_BITS]);
  });

  test("copies a whole block at once, and a part of one in one copy", () => {
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
    const engine = WasmEngine.create(module, SINE_PATCH, RATE, "binary64-as-binary32");
    // 160 frames: two whole blocks, then half of one.
    const outputs = [new Recorded(160), new Recorded(160)];
    engine.render(outputs);
    expect(outputs.map((output) => output.copies)).toStrictEqual([3, 3]);
    expect(Array.from(outputs[0] ?? [])).toStrictEqual(Array.from(renderSine(160, 160)[0] ?? []));
  });

  test.each([48_000.5, -1, 2 ** 32 + 48_000, Number.NaN, Number.POSITIVE_INFINITY])("refuses the sample rate %d", (rate) => {
    expect(() => WasmEngine.create(module, SINE_PATCH, rate, "binary64-as-binary32")).toThrow(new RangeError(`the sample rate must be a whole number of hertz, not ${String(rate)}`));
  });

  test("accepts the largest 32-bit sample rate, rejected by the engine", () => {
    expect(() => WasmEngine.create(module, SINE_PATCH, 0xff_ff_ff_ff, "binary64-as-binary32")).toThrow(PatchError);
  });

  test("refuses outputs of different lengths, without rendering", () => {
    const engine = WasmEngine.create(module, SINE_PATCH, RATE, "binary64-as-binary32");
    expect(() => {
      engine.render([new Float32Array(10), new Float32Array(9)]);
    }).toThrow(new RangeError("every output must have the same length: 9 instead of 10"));
    expect(engine.blocks).toBe(0);
  });

  test("names its errors", () => {
    expect(new ModuleError("x")).toMatchObject({ name: "ModuleError", message: "not an engine module: x" });
  });
});
