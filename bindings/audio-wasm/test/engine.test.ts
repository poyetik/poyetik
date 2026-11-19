import { describe, expect, test, vi } from "vitest";
import { ABI_VERSION, DEFAULT_LIMITER, DETECTORS, EngineTrap, MAX_LOOKAHEAD, MAX_REPLACED_COUNT, ModuleError, OUTPUT_MODES, OutputError, PROTECTED_OUTPUT, PatchError, RAW_OUTPUT, RENDER_FORMATS, WasmEngine, givenAsBinary32, parsePrecision, subnormalArithmetic, type Output, type RenderFormat } from "../ts/engine.ts";
import { DEFAULT_ENGINE_BLOCK, ENGINE_BLOCKS } from "../ts/blocks.ts";
import { interleave, interleave64, renderHash, STRICT } from "../ts/hash.ts";
import { Counted, created } from "./counted.ts";
import { CORPUS, IEEE_SUBNORMALS, INVALID_LIVE, LIVE_VECTOR, liveRefusal, engineFunctions, HOST_BUFFER_SIZES, FUNCTIONS, oscillators, PATCH_NAMES, PRECISION_NAMES, REFERENCES, SAMPLE_RATES, SILENT_PATCH, SINE_PATCH, UNKNOWN_BOX, handMadeModule, NAN_BITS, REPORT_ADDRESS, type HandMadeFunction, reference, wasmBytes, wasmBytesOf, type CorpusCase } from "./fixtures.ts";

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
async function corpusHash(testCase: CorpusCase, size: number, format: RenderFormat, engineModule: WebAssembly.Module = module): Promise<string> {
  const { rate, frames, patch, output: stage } = testCase;
  if (format === "binary64") {
    const wide = WasmEngine.create(engineModule, patch, rate, format, stage);
    const outputs = Array.from({ length: wide.channels }, () => new Float64Array(frames));
    for (let start = 0; start < frames; start += size) {
      wide.render(outputs.map((output) => output.subarray(start, Math.min(start + size, frames))));
    }
    return renderHash(rate, wide.channels, interleave64(outputs), format, STRICT);
  }
  const engine = WasmEngine.create(engineModule, patch, rate, format, stage);
  const channels = Array.from({ length: engine.channels }, () => new Float32Array(frames));
  for (let start = 0; start < frames; start += size) {
    engine.render(channels.map((channel) => channel.subarray(start, Math.min(start + size, frames))));
  }
  return renderHash(rate, engine.channels, interleave(channels), format, STRICT);
}

/**
 * The render hash of the live vector in `format`: block by block of the default engine
 * block, each change set before its block, as the native engine renders it in
 * `audio-host`'s tests and in `audio-cli check` and `bless`, which writes the hashes this
 * one is compared with.
 */
async function liveHash(format: RenderFormat): Promise<string> {
  const { rate, frames, patch, output, changes } = LIVE_VECTOR;
  const blocks = frames / DEFAULT_ENGINE_BLOCK;
  const changed = (live: WasmEngine<Float64Array> | WasmEngine<Float32Array>, block: number): void => {
    for (const change of changes.filter((each) => each.block === block)) {
      live.setLimiter(change.limiter);
    }
  };
  if (format === "binary64") {
    const wide = WasmEngine.create(module, patch, rate, format, output);
    const outputs = Array.from({ length: wide.channels }, () => new Float64Array(frames));
    for (let block = 0; block < blocks; block += 1) {
      changed(wide, block);
      wide.render(outputs.map((each) => each.subarray(block * DEFAULT_ENGINE_BLOCK, (block + 1) * DEFAULT_ENGINE_BLOCK)));
    }
    return renderHash(rate, wide.channels, interleave64(outputs), format, STRICT);
  }
  const engine = WasmEngine.create(module, patch, rate, format, output);
  const channels = Array.from({ length: engine.channels }, () => new Float32Array(frames));
  for (let block = 0; block < blocks; block += 1) {
    changed(engine, block);
    engine.render(channels.map((each) => each.subarray(block * DEFAULT_ENGINE_BLOCK, (block + 1) * DEFAULT_ENGINE_BLOCK)));
  }
  return renderHash(rate, engine.channels, interleave(channels), format, STRICT);
}

/**
 * The sum of `terms` in the reduction order of `spec/engine-design.md`, decision 28, written
 * literally from the rule, apart from the engine: a sum of no terms is +0; term `k` goes to
 * lane `k mod 16`, each lane summing its terms in increasing `k` from -0; then lane `i` with
 * lane `i + 8`, then `i + 4`, `i + 2`, `i + 1`. `add` is the addition of the precision.
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

/** What `action` throws. */
function thrownBy(action: () => unknown): unknown {
  try {
    action();
  } catch (error) {
    return error;
  }
  return undefined;
}

describe("subnormalArithmetic", () => {
  /** What a format keeps, written out. */
  const kept = (operands: boolean, results: boolean): { operands: boolean; results: boolean } => ({ operands, results });

  test("reads each bit of the code as one observation, against literals", () => {
    expect(subnormalArithmetic(0)).toStrictEqual({ binary32: kept(false, false), binary64: kept(false, false) });
    expect(subnormalArithmetic(0b0001)).toStrictEqual({ binary32: kept(true, false), binary64: kept(false, false) });
    expect(subnormalArithmetic(0b0010)).toStrictEqual({ binary32: kept(false, true), binary64: kept(false, false) });
    expect(subnormalArithmetic(0b0100)).toStrictEqual({ binary32: kept(false, false), binary64: kept(true, false) });
    expect(subnormalArithmetic(0b1000)).toStrictEqual({ binary32: kept(false, false), binary64: kept(false, true) });
    expect(subnormalArithmetic(0b1010)).toStrictEqual({ binary32: kept(false, true), binary64: kept(false, true) });
    expect(subnormalArithmetic(0b1111)).toStrictEqual(IEEE_SUBNORMALS);
  });

  test.each([16, -1, 0.5, 15.5, Number.NaN, Infinity])("refuses %o, which is no code", (code) => {
    expect(subnormalArithmetic(code)).toBeUndefined();
  });
});

describe("WasmEngine", () => {
  test("knows the whole corpus: every case has its patch and its reference, and nothing else", () => {
    const names = CORPUS.map((testCase) => testCase.name);
    expect(names.length).toBeGreaterThan(1);
    expect(PATCH_NAMES).toStrictEqual(names);
    expect([...REFERENCES.keys()].sort()).toStrictEqual(names);
  });

  test.each(RENDER_FORMATS)("renders the live vector of limiter settings in %s with the hash the native engine gives", async (format) => {
    // Lookaheads set while it limits, held pending and taken at its rest, longer and shorter,
    // two set between the same blocks and one set back: the bits of `audio-host`'s native
    // render, through its hash.
    expect(Number.isInteger(LIVE_VECTOR.frames / DEFAULT_ENGINE_BLOCK)).toBe(true);
    expect(LIVE_VECTOR.changes.length).toBeGreaterThan(0);
    await expect(liveHash(format)).resolves.toBe(LIVE_VECTOR.hashes[format]);
  });

  test("refuses every live vector of the shared invalid corpus, as audio-cli and audio-host do", () => {
    // Every file of the corpus, and none other, for the reason its name gives, the same in
    // every reader; and a vector at the limits is taken: 2^29 frames, a change at its last
    // block, two at the same block.
    const reasons: Readonly<Record<string, string>> = {
      "change-at-the-end": "a change at block 10, past the end of its 10 blocks",
      "change-past-the-end": "a change at block 4294967296, past the end of its 10 blocks",
      "changes-out-of-order": "a change at block 2 after one at block 3, out of order",
      "frames-above-the-limit": "frames must be at most 536870912, got 536870976",
    };
    expect(INVALID_LIVE.map(({ name }) => name)).toStrictEqual(Object.keys(reasons));
    for (const { name, frames, blocks } of INVALID_LIVE) {
      expect(liveRefusal(frames, blocks), name).toBe(reasons[name]);
    }
    expect(liveRefusal(536_870_912, [0, 8_388_607, 8_388_607])).toBeUndefined();
    expect(liveRefusal(536_870_912, [8_388_608])).toBe("a change at block 8388608, past the end of its 8388608 blocks");
  });

  test("renders the corpus through every detector of the engine, by name", () => {
    // A detector no case renders through has no bits proven the same in WebAssembly as natively.
    const named = new Set(CORPUS.flatMap((testCase) => (testCase.output.mode === "raw" ? [] : [testCase.output.detector])));
    expect([...named].sort()).toStrictEqual([...DETECTORS].sort());
  });

  test.each(RENDER_FORMATS.flatMap((format) => CORPUS.flatMap((testCase) => HOST_BUFFER_SIZES.map((size) => [testCase.name, format, size, testCase] as const))))(
    "renders %s in %s in Node bit-identical to the native engine, through buffers of %i frames",
    async (name, format, size, testCase) => {
      expect(await corpusHash(testCase, size, format)).toBe(reference(name, format));
    },
  );

  /**
   * The first host buffer size of the test matrix, beyond one frame, that is a whole number
   * of no engine block: every module renders parts of its blocks through it.
   */
  const SPLIT = HOST_BUFFER_SIZES.find((size) => size > 1 && ENGINE_BLOCKS.every((block) => size % block !== 0 && block % size !== 0));

  test("splits every engine block of every module through a host buffer size of the test matrix", () => {
    expect(SPLIT).toBeDefined();
  });

  /** Each shipped module, compiled once, by its engine block. */
  const MODULES = new Map(ENGINE_BLOCKS.map((block) => [block, new WebAssembly.Module(wasmBytesOf(block))] as const));

  test.each(ENGINE_BLOCKS.flatMap((block) => RENDER_FORMATS.flatMap((format) => CORPUS.map((testCase) => [testCase.name, format, block, testCase] as const))))(
    "renders %s in %s bit-identical to the native engine with the module of %i-frame blocks",
    async (name, format, block, testCase) => {
      const sized = MODULES.get(block);
      expect(sized).toBeDefined();
      if (sized !== undefined && SPLIT !== undefined) {
        expect(await corpusHash(testCase, SPLIT, format, sized)).toBe(reference(name, format));
      }
    },
  );

  test("renders every precision of the test matrix, given as binary32 or as binary64", () => {
    expect(PRECISION_NAMES).toStrictEqual(["binary64", "binary32"]);
    expect(PRECISION_NAMES.map(givenAsBinary32)).toStrictEqual(["binary64-as-binary32", "binary32"]);
    expect(RENDER_FORMATS).toStrictEqual(["binary64", "binary64-as-binary32", "binary32"]);
  });

  test("gives a binary64 render in binary64, or rounded to binary32, and a binary32 one in binary32", () => {
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

  test("reports the frames of its blocks, as the module gives them", () => {
    expect(WasmEngine.create(module, SINE_PATCH, RATE, "binary64-as-binary32").blockSize).toBe(64);
    expect(WasmEngine.create(module, SINE_PATCH, RATE, "binary64").blockSize).toBe(64);
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

  test.each([2, -1])("refuses a load result %i that is neither 1 (loaded) nor 0 (refused) as a module error, never a refused patch", (result) => {
    const functions = engineFunctions().map((func) => (func.name === "engine_load" ? { ...func, body: { i32: result } } : func));
    const answering = handMadeModule({ memory: true, functions });
    expect(() => WasmEngine.create(answering, SINE_PATCH, RATE, "binary64-as-binary32")).toThrow(
      new ModuleError(`engine_load returned ${String(result >>> 0)}, not 1 (succeeded) or 0 (refused)`),
    );
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
    expect(ABI_VERSION).toBe(0x5059_0005);
    // engineFunctions gives every function the same result: here, the version answered.
    // Far from ours, then one step from ours in the version both ways, and in the identity
    // both ways.
    for (const [answer, shown] of [
      [1, "0x1"],
      [0x5059_0004, "0x50590004"],
      [0x5059_0006, "0x50590006"],
      [0x505a_0004, "0x505a0004"],
      [0x5058_0004, "0x50580004"],
    ] as const) {
      const other = handMadeModule({ memory: true, functions: engineFunctions({ i32: answer }) });
      expect(() => WasmEngine.create(other, SINE_PATCH, RATE, "binary64-as-binary32")).toThrow(new ModuleError(`interface ${shown}, expected 0x50590005`));
    }
  });

  test("observes IEEE subnormal arithmetic in Node, whose thread keeps every subnormal", () => {
    expect(WasmEngine.create(new WebAssembly.Module(wasmBytes()), SINE_PATCH, RATE, "binary64").subnormals).toStrictEqual(IEEE_SUBNORMALS);
  });

  test("reads the module's observation of subnormals, and refuses a code that is none", () => {
    const observing = (code: number): WebAssembly.Module =>
      handMadeModule({ memory: true, functions: engineFunctions().map((func) => (func.name === "engine_subnormals" ? { ...func, body: { i32: code } } : func)) });
    expect(WasmEngine.create(observing(0b0110), SINE_PATCH, RATE, "binary64-as-binary32").subnormals).toStrictEqual({
      binary32: { operands: false, results: true },
      binary64: { operands: true, results: false },
    });
    expect(() => WasmEngine.create(observing(16), SINE_PATCH, RATE, "binary64-as-binary32")).toThrow(new ModuleError("engine_subnormals returned 16, not a code from 0 to 15"));
    // -1 as an i32 is read back as unsigned, as every result.
    expect(() => WasmEngine.create(observing(-1), SINE_PATCH, RATE, "binary64-as-binary32")).toThrow(new ModuleError("engine_subnormals returned 4294967295, not a code from 0 to 15"));
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

  test("reads what the output stage replaced: nothing, or the count and the first sample", () => {
    const nothing = WasmEngine.create(handMadeModule({ memory: true, functions: engineFunctions() }), SINE_PATCH, RATE, "binary64-as-binary32");
    expect(nothing.replacedCount).toBe(0);
    expect(nothing.replaced()).toStrictEqual({ count: 0 });
    expect(Object.isFrozen(nothing.replaced())).toBe(true);
    expect(nothing.replaced()).toBe(nothing.replaced());
    const reporting = (body: HandMadeFunction["body"]): WasmEngine<Float32Array> => {
      const functions = engineFunctions().map((func) => (func.name === "engine_replaced" ? { ...func, body } : func));
      return WasmEngine.create(handMadeModule({ memory: true, functions }), SINE_PATCH, RATE, "binary64-as-binary32");
    };
    // Every word read as unsigned and little-endian: the count 2^32 - 1, the frame the largest exact one, 2^53 - 2.
    const engine = reporting({ report: [-1, -2, 0x1f_ffff, 7] });
    expect(engine.replacedCount).toBe(0xffff_ffff);
    // The largest count, where it saturates: 2^32 - 1, written apart.
    expect(MAX_REPLACED_COUNT).toBe(4_294_967_295);
    expect(engine.replacedFirst()).toStrictEqual({ frame: 2 ** 53 - 2, channel: 7 });
    expect(engine.replaced()).toStrictEqual({ count: 0xffff_ffff, first: { frame: 2 ** 53 - 2, channel: 7 } });
    // Each word in its place.
    const spread = reporting({ report: [3, 0x0102_0304, 2, 0x0506] });
    expect(spread.replaced()).toStrictEqual({ count: 3, first: { frame: 2 * 2 ** 32 + 0x0102_0304, channel: 0x0506 } });
  });

  test("reads its report through one view while the memory stays, and one new view once it grew", () => {
    // The engine processor reads the count every render quantum: reading it allocates nothing.
    const functions = engineFunctions().map((func) =>
      func.name === "engine_process" ? { ...func, body: "grow" as const } : func.name === "engine_replaced" ? { ...func, body: { report: [2, 0, 0, 0] as const } } : func,
    );
    const engine = WasmEngine.create(handMadeModule({ memory: true, functions }), SINE_PATCH, RATE, "binary64-as-binary32");
    const views = vi.spyOn(globalThis, "DataView");
    try {
      expect([engine.replacedCount, engine.replacedCount, engine.replacedCount]).toStrictEqual([2, 2, 2]);
      expect(views).toHaveBeenCalledTimes(0);
      engine.render([new Float32Array(1)]);
      expect([engine.replacedCount, engine.replacedCount]).toStrictEqual([2, 2]);
      expect(views).toHaveBeenCalledTimes(1);
    } finally {
      views.mockRestore();
    }
  });

  test("reads the report wherever the module places it, after the memory grew", () => {
    // engine_process grows the memory: the report, read after, is read from the memory as it is then.
    const functions = engineFunctions().map((func) =>
      func.name === "engine_process" ? { ...func, body: "grow" as const } : func.name === "engine_replaced" ? { ...func, body: { report: [4, 0, 0, 1] as const } } : func,
    );
    const engine = WasmEngine.create(handMadeModule({ memory: true, functions }), SINE_PATCH, RATE, "binary64-as-binary32");
    engine.render([new Float32Array(1)]);
    expect(engine.replaced()).toStrictEqual({ count: 4, first: { frame: 0, channel: 1 } });
  });

  test.each([
    ["unaligned", REPORT_ADDRESS + 1],
    ["its last word outside the memory", 65_536 - 12],
    ["entirely outside the memory", 65_536],
    ["negative, read as an unsigned address beyond the memory", -16],
  ])("refuses a report %s, and stops calling the module", (_case, address) => {
    const functions = engineFunctions().map((func) => (func.name === "engine_replaced" ? { ...func, body: { i32: address } } : func));
    const engine = WasmEngine.create(handMadeModule({ memory: true, functions }), SINE_PATCH, RATE, "binary64-as-binary32");
    const unsigned = address >>> 0;
    expect(() => engine.replacedCount).toThrow(new ModuleError(`engine_replaced returned the address ${String(unsigned)}, unaligned or outside the memory`));
    expect(() => engine.replacedCount).toThrow(new EngineTrap());
  });

  test("reads the last aligned report the memory holds", () => {
    const functions = engineFunctions().map((func) => (func.name === "engine_replaced" ? { ...func, body: { i32: 65_536 - 16 } } : func));
    const engine = WasmEngine.create(handMadeModule({ memory: true, functions }), SINE_PATCH, RATE, "binary64-as-binary32");
    expect(engine.replaced()).toStrictEqual({ count: 0 });
  });

  test("turns a trap of engine_replaced into an EngineTrap, and never calls the module again", () => {
    const functions = engineFunctions().map((func) => (func.name === "engine_replaced" ? { ...func, body: "trap" as const } : func));
    const engine = WasmEngine.create(handMadeModule({ memory: true, functions }), SINE_PATCH, RATE, "binary64-as-binary32");
    const error = (() => {
      try {
        engine.replaced();
      } catch (thrown) {
        return thrown;
      }
      return undefined;
    })();
    expect(error).toBeInstanceOf(EngineTrap);
    expect(error instanceof Error ? error.cause : undefined).toBeInstanceOf(WebAssembly.RuntimeError);
    expect(() => engine.replacedCount).toThrow(new EngineTrap());
    expect(() => engine.replacedFirst()).toThrow(new EngineTrap());
    expect(() => {
      engine.render([new Float32Array(1)]);
    }).toThrow(EngineTrap);
  });

  test("lets another error of a report through, and stops calling the module", () => {
    const functions = engineFunctions().map((func) => (func.name === "engine_replaced" ? { ...func, body: "i64" as const } : func));
    const engine = WasmEngine.create(handMadeModule({ memory: true, functions }), SINE_PATCH, RATE, "binary64-as-binary32");
    expect(() => engine.replacedCount).toThrow(new ModuleError("engine_replaced did not return a number"));
    expect(() => engine.replacedCount).toThrow(new EngineTrap());
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

  test("refuses a module whose blocks have no frame, which would render forever, or more frames than an engine block has", () => {
    expect(() => placing(1, 0, 4)).toThrow(new ModuleError("engine_block_size returned 0, not 1 to 1048576 frames"));
    expect(placing(1, 1, 4).blocks).toBe(0);
    expect(placing(1, 1, 4).blockSize).toBe(1);
    expect(placing(2, 48, 4).blockSize).toBe(48);
    // 2^20, the largest engine block (`audio_core::MAX_ENGINE_BLOCK`), and one more; -1 as
    // the module's i32, read unsigned.
    expect(placing(0, 2 ** 20, 4).blockSize).toBe(2 ** 20);
    expect(() => placing(0, 2 ** 20 + 1, 4)).toThrow(new ModuleError("engine_block_size returned 1048577, not 1 to 1048576 frames"));
    expect(() => placing(0, -1, 4)).toThrow(new ModuleError("engine_block_size returned 4294967295, not 1 to 1048576 frames"));
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

test("reads a precision only in its exact name, at a boundary", () => {
  expect(["binary64", "binary32"].map(parsePrecision)).toStrictEqual(["binary64", "binary32"]);
  for (const value of ["binary-64", "Binary64", "binary16", "", undefined, null, 64, ["binary64"]]) {
    expect(parsePrecision(value), String(value)).toBeUndefined();
  }
  expect([givenAsBinary32("binary64"), givenAsBinary32("binary32")]).toStrictEqual(["binary64-as-binary32", "binary32"]);
});

describe("the output stage", () => {
  /** 10^(-1/20): the default ceiling of -1 dBTP as an amplitude, computed apart. */
  const CEILING = 0.891_250_938_133_745_6;

  /** The largest magnitude of `frames` frames of the sine from `engine`, after `skip` frames. */
  function peak(engine: WasmEngine<Float32Array>, frames: number, skip = 0): number {
    const out = new Float32Array(frames);
    engine.render([out]);
    return out.subarray(skip).reduce((largest, sample) => Math.max(largest, Math.abs(sample)), 0);
  }

  /** The engine's explanation of a refusal, as text. */
  function detail(error: unknown): string {
    return error instanceof OutputError ? new TextDecoder().decode(error.detail) : String(error);
  }

  test("takes the longest lookahead as a maximum and as a lookahead, and refuses the next number up", () => {
    // 5 ms, written apart (`audio_core::Setting::Lookahead`'s range), and the next binary64
    // up, 5 + 2^-50.
    expect(MAX_LOOKAHEAD).toBe(5);
    const longest = { mode: "protected", maximumLookahead: 5, detector: "precise", limiter: { ...DEFAULT_LIMITER, lookahead: 5 } } as const;
    expect(WasmEngine.create(module, SINE_PATCH, RATE, "binary32", longest).latency).toBe(240 + 16);
    for (const [output, words] of [
      [{ ...longest, maximumLookahead: 5 + 2 ** -50 }, "limiter maximum lookahead 5.000000000000001 ms is out of range (0.5 to 5 ms)"],
      [{ ...longest, limiter: { ...DEFAULT_LIMITER, lookahead: 5 + 2 ** -50 } }, "limiter lookahead 5.000000000000001 ms is out of range (0.5 to 5 ms)"],
    ] as const) {
      expect(detail(thrownBy(() => WasmEngine.create(module, SINE_PATCH, RATE, "binary32", output)))).toBe(words);
    }
  });

  test("names its modes in the order of their codes, and its defaults as the engine's", () => {
    expect(OUTPUT_MODES).toStrictEqual(["raw", "protected", "switchable"]);
    expect(DEFAULT_LIMITER).toStrictEqual({ ceiling: -1, lookahead: 1.5, release: 100, adaptivity: 0.5 });
    expect(DETECTORS).toStrictEqual(["standard", "precise", "mastering"]);
    expect(RAW_OUTPUT).toStrictEqual({ mode: "raw" });
    expect(PROTECTED_OUTPUT).toStrictEqual({ mode: "protected", maximumLookahead: 5, detector: "precise", limiter: DEFAULT_LIMITER });
    expect([Object.isFrozen(DEFAULT_LIMITER), Object.isFrozen(RAW_OUTPUT), Object.isFrozen(PROTECTED_OUTPUT)]).toStrictEqual([true, true, true]);
  });

  test("is raw by default: no latency, the sine at full scale", () => {
    const engine = WasmEngine.create(module, SINE_PATCH, RATE, "binary32");
    expect(engine.latency).toBe(0);
    expect(peak(engine, 4_800)).toBeGreaterThan(0.999);
  });

  test("sets nothing for a raw stage, which has no limiter: a fresh instance builds one", () => {
    // A module whose output-stage setters trap: only a limited stage calls them.
    const functions = engineFunctions().map((func) => (func.name === "engine_limiter" || func.name === "engine_output" ? { ...func, body: "trap" as const } : func));
    const setless = handMadeModule({ memory: true, functions });
    expect(WasmEngine.create(setless, SINE_PATCH, RATE, "binary32", RAW_OUTPUT).channels).toBe(1);
    expect(WasmEngine.create(setless, SINE_PATCH, RATE, "binary32").channels).toBe(1);
    for (const mode of ["protected", "switchable"] as const) {
      expect(thrownBy(() => WasmEngine.create(setless, SINE_PATCH, RATE, "binary32", { ...PROTECTED_OUTPUT, mode }))).toStrictEqual(new EngineTrap());
    }
  });

  test("protects under the ceiling, delayed by its declared latency", () => {
    // The maximum lookahead in frames at 48 kHz, and each detector's delay.
    for (const [maximumLookahead, detector, latency] of [[5, "precise", 256], [0.5, "standard", 30], [0.5, "mastering", 88]] as const) {
      const engine = WasmEngine.create(module, SINE_PATCH, RATE, "binary64-as-binary32", { ...PROTECTED_OUTPUT, maximumLookahead, detector, limiter: { ...DEFAULT_LIMITER, lookahead: 0.5 } });
      expect(engine.latency).toBe(latency);
      const out = new Float32Array(4_800);
      engine.render([out]);
      expect(out.subarray(0, latency).every((sample) => sample === 0)).toBe(true);
      expect(out[latency + 1]).not.toBe(0);
      expect(out.reduce((largest, sample) => Math.max(largest, Math.abs(sample)), 0)).toBeLessThanOrEqual(CEILING);
    }
  });

  test.each<[string, Output, string]>([
    ["a maximum lookahead out of range", { ...PROTECTED_OUTPUT, maximumLookahead: 9 }, "limiter maximum lookahead 9 ms is out of range (0.5 to 5 ms)"],
    ["a lookahead above the maximum", { mode: "protected", maximumLookahead: 1, detector: "precise", limiter: DEFAULT_LIMITER }, "limiter lookahead 1.5 ms is above its maximum of 1 ms"],
    ["a ceiling out of range", { mode: "protected", maximumLookahead: 5, detector: "precise", limiter: { ...DEFAULT_LIMITER, ceiling: 1 } }, "limiter ceiling 1 dBTP is out of range (-20 to 0 dBTP)"],
    ["a release out of range", { mode: "switchable", maximumLookahead: 5, detector: "precise", limiter: { ...DEFAULT_LIMITER, release: 9 } }, "limiter release 9 ms is out of range (10 to 1000 ms)"],
    ["an adaptivity out of range", { mode: "protected", maximumLookahead: 5, detector: "precise", limiter: { ...DEFAULT_LIMITER, adaptivity: 1.5 } }, "limiter adaptivity 1.5 is out of range (0 to 1)"],
  ])("refuses %s with the engine's words", (_case, output, words) => {
    const error = thrownBy(() => WasmEngine.create(module, SINE_PATCH, RATE, "binary32", output));
    expect(error).toBeInstanceOf(OutputError);
    expect(error).toMatchObject({ name: "OutputError", message: "the engine refused the output settings" });
    expect(detail(error)).toBe(words);
  });

  test.each(SAMPLE_RATES)("changes its limiter live, holding the new ceiling without silencing the sine, and renders on after a refusal, at %i Hz", (rate) => {
    const engine = WasmEngine.create(module, SINE_PATCH, rate, "binary32", PROTECTED_OUTPUT);
    // -6 dBTP as an amplitude, computed apart.
    const lower = 0.501_187_233_627_272_2;
    engine.setLimiter({ ...DEFAULT_LIMITER, ceiling: -6 });
    // The latency counted apart before it sizes anything: the maximum lookahead, 5 ms, in
    // frames rounded to the nearest (half up), and the precise detector's delay, 16 frames.
    expect(engine.latency).toBe(Math.floor((rate + 100) / 200) + 16);
    // Phases sized from the rate: the stage's latency and four lookahead windows to settle,
    // then four periods of the 440 Hz sine, so that every phase holds crests.
    const settle = engine.latency + 4 * Math.ceil((DEFAULT_LIMITER.lookahead / 1000) * rate);
    // At least 0.98 of the new ceiling: the shortfall the playground's 440 Hz sine is allowed
    // under the default ceiling (spec/measurements.md, "A period longer than the window",
    // twice the shortfall measured there), its true-peak bound underived until
    // roadmap #24 step 112. A gain stuck a fifth of a decibel low fails, a sine silenced too.
    const shortfall = 0.98;
    const periods = 4 * Math.ceil(rate / 440);
    const settled = peak(engine, settle + periods, settle);
    expect(settled).toBeLessThanOrEqual(lower);
    expect(settled).toBeGreaterThanOrEqual(shortfall * lower);
    const refusal = thrownBy(() => {
      engine.setLimiter({ ...DEFAULT_LIMITER, lookahead: 6 });
    });
    expect(detail(refusal)).toBe("limiter lookahead 6 ms is out of range (0.5 to 5 ms)");
    const after = peak(engine, periods);
    expect(after).toBeLessThanOrEqual(lower);
    expect(after).toBeGreaterThanOrEqual(shortfall * lower);
  });

  test("tells whether a lookahead set live is pending, until the limiter's rest takes it", () => {
    // Raw: never. Protected, on the full-scale sine over its -1 dBTP ceiling, which keeps it
    // limiting: 5 ms set once it limits is pending, quantum after quantum; set back, none.
    // Before its first block, at rest: pending until that block, whose first frame takes it.
    expect(WasmEngine.create(module, SINE_PATCH, RATE, "binary32").lookaheadPending).toBe(false);
    const limiting = WasmEngine.create(module, SINE_PATCH, RATE, "binary32", PROTECTED_OUTPUT);
    limiting.render([new Float32Array(4_800)]);
    expect(limiting.lookaheadPending).toBe(false);
    limiting.setLimiter({ ...DEFAULT_LIMITER, lookahead: 5 });
    for (let quantum = 0; quantum !== 20; quantum += 1) {
      expect(limiting.lookaheadPending, `quantum ${String(quantum)}`).toBe(true);
      limiting.render([new Float32Array(128)]);
    }
    limiting.setLimiter(DEFAULT_LIMITER);
    expect(limiting.lookaheadPending).toBe(false);
    const resting = WasmEngine.create(module, SINE_PATCH, RATE, "binary32", PROTECTED_OUTPUT);
    resting.setLimiter({ ...DEFAULT_LIMITER, lookahead: 5 });
    expect(resting.lookaheadPending).toBe(true);
    resting.render([new Float32Array(1)]);
    expect(resting.lookaheadPending).toBe(false);
  });

  test("dies on an answer of engine_lookahead_pending neither 1 nor 0, and on a trap", () => {
    for (const [body, error] of [[{ i32: 2 }, new ModuleError("engine_lookahead_pending returned 2, not 1 (pending) or 0 (in force)")], ["trap", new EngineTrap()]] as const) {
      const functions = engineFunctions().map((func) => (func.name === "engine_lookahead_pending" ? { ...func, body } : func));
      const engine = WasmEngine.create(handMadeModule({ memory: true, functions }), SINE_PATCH, RATE, "binary32");
      expect(() => engine.lookaheadPending).toThrow(error);
      expect(() => engine.lookaheadPending).toThrow(new EngineTrap());
    }
    // A hand-made module answering 0 and 1: read as such.
    for (const [answer, pending] of [[0, false], [1, true]] as const) {
      const functions = engineFunctions().map((func) => (func.name === "engine_lookahead_pending" ? { ...func, body: { i32: answer } } : func));
      expect(WasmEngine.create(handMadeModule({ memory: true, functions }), SINE_PATCH, RATE, "binary32").lookaheadPending).toBe(pending);
    }
  });

  test("switches its protection by a crossfade, raw first when switchable", () => {
    const engine = WasmEngine.create(module, SINE_PATCH, RATE, "binary32", { ...PROTECTED_OUTPUT, mode: "switchable" });
    expect(engine.latency).toBe(256);
    expect(peak(engine, 4_800, 256)).toBeGreaterThan(0.999);
    engine.setProtected(true);
    expect(peak(engine, 48_000, 24_000)).toBeLessThanOrEqual(CEILING);
    engine.setProtected(false);
    expect(peak(engine, 48_000, 24_000)).toBeGreaterThan(0.999);
  });

  test("keeps a protected stage protected, refusing to switch it to raw", () => {
    const engine = WasmEngine.create(module, SINE_PATCH, RATE, "binary32", PROTECTED_OUTPUT);
    expect(detail(thrownBy(() => { engine.setProtected(false); }))).toBe("the output stage is protected: only a switchable stage is switched to raw");
    engine.setProtected(true);
    expect(peak(engine, 48_000, 256)).toBeLessThanOrEqual(CEILING);
  });

  test("refuses to set a limiter it does not have", () => {
    const engine = WasmEngine.create(module, SINE_PATCH, RATE, "binary32");
    for (const action of [() => { engine.setProtected(true); }, () => { engine.setLimiter(DEFAULT_LIMITER); }]) {
      expect(detail(thrownBy(action))).toBe("the output stage is raw: it has no limiter to set");
    }
    expect(peak(engine, 4_800)).toBeGreaterThan(0.999);
  });

  test.each(["engine_output"])("refuses an answer of %s neither 1 nor 0 as a module error", (name) => {
    const functions = engineFunctions().map((func) => (func.name === name ? { ...func, body: { i32: 2 } } : func));
    expect(() => WasmEngine.create(handMadeModule({ memory: true, functions }), SINE_PATCH, RATE, "binary32", PROTECTED_OUTPUT)).toThrow(
      new ModuleError(`${name} returned 2, not 1 (succeeded) or 0 (refused)`),
    );
  });

  test("dies on a live answer neither 1 nor 0, and on a trap", () => {
    for (const [body, error] of [[{ i32: 2 }, new ModuleError("engine_protect returned 2, not 1 (succeeded) or 0 (refused)")], ["trap", new EngineTrap()]] as const) {
      const functions = engineFunctions().map((func) => (func.name === "engine_protect" ? { ...func, body } : func));
      const engine = WasmEngine.create(handMadeModule({ memory: true, functions }), SINE_PATCH, RATE, "binary32");
      expect(() => { engine.setProtected(false); }).toThrow(error);
      expect(() => { engine.setLimiter(DEFAULT_LIMITER); }).toThrow(new EngineTrap());
    }
  });

  test("dies on a live answer of engine_limiter neither 1 nor 0", () => {
    const functions = engineFunctions().map((func) => (func.name === "engine_limiter" ? { ...func, body: { i32: 2 } } : func));
    const engine = WasmEngine.create(handMadeModule({ memory: true, functions }), SINE_PATCH, RATE, "binary32");
    expect(() => { engine.setLimiter(DEFAULT_LIMITER); }).toThrow(new ModuleError("engine_limiter returned 2, not 1 (succeeded) or 0 (refused)"));
    expect(() => { engine.setProtected(true); }).toThrow(new EngineTrap());
  });
});

describe("the engine's explanation", () => {
  /** A module that refuses the calls `refused`, its explanation `length` bytes at `pointer` of its one-page memory. */
  const explaining = (pointer: number, length: number, refused: readonly string[] = ["engine_load"]): WebAssembly.Module =>
    handMadeModule({
      memory: true,
      functions: engineFunctions().map((func) =>
        func.name === "engine_message_ptr" ? { ...func, body: { i32: pointer } } : func.name === "engine_message_len" ? { ...func, body: { i32: length } } : refused.includes(func.name) ? { ...func, body: { i32: 0 } } : func,
      ),
    });

  /** One page of memory, the hand-made modules' whole memory, in bytes. */
  const PAGE = 65_536;

  test("is read up to the last byte of the memory, and refused one byte past it as a module error", () => {
    expect(thrownBy(() => WasmEngine.create(explaining(PAGE - 4, 4), SINE_PATCH, RATE, "binary32"))).toStrictEqual(new PatchError(new Uint8Array(4)));
    expect(thrownBy(() => WasmEngine.create(explaining(PAGE - 3, 4), SINE_PATCH, RATE, "binary32"))).toStrictEqual(
      new ModuleError(`engine_message_ptr returned ${String(PAGE - 3)} and engine_message_len 4: an explanation outside the memory`),
    );
    expect(thrownBy(() => WasmEngine.create(explaining(0, PAGE + 1), SINE_PATCH, RATE, "binary32"))).toStrictEqual(
      new ModuleError(`engine_message_ptr returned 0 and engine_message_len ${String(PAGE + 1)}: an explanation outside the memory`),
    );
  });

  test("of a live refusal, outside the memory, kills the engine as any module error does", () => {
    /** The live settings, refused. */
    const LIVE = ["engine_limiter", "engine_protect"];
    for (const set of [(engine: WasmEngine<Float32Array>) => { engine.setLimiter(DEFAULT_LIMITER); }, (engine: WasmEngine<Float32Array>) => { engine.setProtected(true); }]) {
      // Built raw: a fresh module is never asked to set a raw stage.
      const inside = WasmEngine.create(explaining(PAGE - 4, 4, LIVE), SINE_PATCH, RATE, "binary32");
      expect(thrownBy(() => { set(inside); })).toStrictEqual(new OutputError(new Uint8Array(4)));
      expect(inside.replacedCount).toBe(0);
      const outside = WasmEngine.create(explaining(PAGE - 3, 4, LIVE), SINE_PATCH, RATE, "binary32");
      expect(thrownBy(() => { set(outside); })).toStrictEqual(new ModuleError(`engine_message_ptr returned ${String(PAGE - 3)} and engine_message_len 4: an explanation outside the memory`));
      expect(thrownBy(() => outside.replacedCount)).toStrictEqual(new EngineTrap());
    }
  });

  test("traps as an engine trap, live, and kills the engine", () => {
    const functions = engineFunctions().map((func) => (func.name === "engine_message_len" ? { ...func, body: "trap" as const } : func.name === "engine_protect" ? { ...func, body: { i32: 0 } } : func));
    const engine = WasmEngine.create(handMadeModule({ memory: true, functions }), SINE_PATCH, RATE, "binary32");
    const trap = thrownBy(() => { engine.setProtected(true); });
    expect(trap).toStrictEqual(new EngineTrap());
    expect(trap).toHaveProperty("cause.name", "RuntimeError");
    expect(thrownBy(() => engine.replacedCount)).toStrictEqual(new EngineTrap());
  });
});

describe("the module of every engine block", () => {
  /** The bytes of `outputs`, as rendered. */
  const bytesOf = (outputs: readonly (Float64Array | Float32Array)[]): number[] => outputs.flatMap((output) => [...new Uint8Array(output.buffer)]);

  /** 3000 frames of the sine through `engine` (no whole number of any block: a last part of a block is read). */
  const wide = (engine: WasmEngine<Float64Array>): number[] => {
    const outputs = [new Float64Array(3000), new Float64Array(3000)];
    engine.render(outputs);
    return bytesOf(outputs);
  };

  /** The same, given as binary32. */
  const narrow = (engine: WasmEngine<Float32Array>): number[] => {
    const outputs = [new Float32Array(3000), new Float32Array(3000)];
    engine.render(outputs);
    return bytesOf(outputs);
  };

  // The engine blocks the package ships a module for (`audio_core::ENGINE_BLOCKS`, decision 2).
  test.each(ENGINE_BLOCKS)("blocks of %i frames render the default module's samples bit for bit, limited", (frames) => {
    const sized = new WebAssembly.Module(wasmBytesOf(frames));
    const engine = WasmEngine.create(sized, SINE_PATCH, RATE, "binary64", PROTECTED_OUTPUT);
    expect(engine.blockSize).toBe(frames);
    expect(wide(engine)).toStrictEqual(wide(WasmEngine.create(module, SINE_PATCH, RATE, "binary64", PROTECTED_OUTPUT)));
    for (const format of ["binary64-as-binary32", "binary32"] as const) {
      expect(narrow(WasmEngine.create(sized, SINE_PATCH, RATE, format, PROTECTED_OUTPUT))).toStrictEqual(narrow(WasmEngine.create(module, SINE_PATCH, RATE, format, PROTECTED_OUTPUT)));
    }
  });
});
