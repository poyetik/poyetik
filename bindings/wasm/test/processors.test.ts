import { afterEach, beforeAll, describe, expect, test, vi } from "vitest";
import { WasmEngine, givenAsBinary32 } from "../ts/engine.ts";
import { ENGINE_PROCESSOR, MARKER_PERIOD, SOURCE_STOPPED, TAP_PROCESSOR } from "../ts/protocol.ts";
import { Counted, created } from "./counted.ts";
import { engineFunctions, handMadeBytes, PRECISION_NAMES, READY, SAMPLE_RATES, SILENT_PATCH, SINE_PATCH, UNKNOWN_BOX, wasmBytes } from "./fixtures.ts";
import { loadProcessors, processorClass, type ProcessorClass } from "./worklet.ts";

/** The sample rate of the fake worklet scope, in hertz, restored after a test changes it. */
const RATE = 48_000;

/** The processor classes registered by the worklet module, by name, filled when it is imported. */
let registered: ReadonlyMap<string, ProcessorClass> = new Map();

beforeAll(async () => {
  registered = await loadProcessors(RATE);
});

// Restored whether the test passed or not: a failing test must not change the rate of the next ones.
afterEach(() => {
  vi.stubGlobal("sampleRate", RATE);
});

/** The registered processor class of this name. */
function processor(name: string): ProcessorClass {
  return processorClass(registered, name);
}

describe("engine processor", () => {
  test("is registered under its name", () => {
    expect([...registered.keys()]).toStrictEqual([ENGINE_PROCESSOR, TAP_PROCESSOR]);
  });

  test.each(SAMPLE_RATES)("renders the patch at the rate of the worklet: %i Hz", (rate) => {
    // The worklet's global sampleRate, never a default: each rate gives other samples.
    vi.stubGlobal("sampleRate", rate);
    const Engine = processor(ENGINE_PROCESSOR);
    const node = new Engine({ processorOptions: { wasm: wasmBytes(), patch: SINE_PATCH, marker: false, precision: "binary64" } });
    const outputs = [[new Float32Array(128), new Float32Array(128)]];
    expect(node.process([], outputs)).toBe(true);
    const expected = [new Float32Array(128), new Float32Array(128)];
    WasmEngine.create(new WebAssembly.Module(wasmBytes()), SINE_PATCH, rate, "binary64-as-binary32").render(expected);
    expect(outputs[0]).toStrictEqual(expected);
    const other = [new Float32Array(128), new Float32Array(128)];
    WasmEngine.create(new WebAssembly.Module(wasmBytes()), SINE_PATCH, rate === 48_000 ? 44_100 : 48_000, "binary64-as-binary32").render(other);
    expect(outputs[0]).not.toStrictEqual(other);
  });

  test.each(PRECISION_NAMES)("computes in the precision of its options, %s, given as binary32", (precision) => {
    const Engine = processor(ENGINE_PROCESSOR);
    const node = new Engine({ processorOptions: { wasm: wasmBytes(), patch: SINE_PATCH, marker: false, precision } });
    const outputs = [[new Float32Array(128), new Float32Array(128)]];
    expect(node.process([], outputs)).toBe(true);
    const expected = [new Float32Array(128), new Float32Array(128)];
    WasmEngine.create(new WebAssembly.Module(wasmBytes()), SINE_PATCH, RATE, givenAsBinary32(precision)).render(expected);
    expect(outputs[0]).toStrictEqual(expected);
    const other = [new Float32Array(128), new Float32Array(128)];
    WasmEngine.create(new WebAssembly.Module(wasmBytes()), SINE_PATCH, RATE, precision === "binary64" ? "binary32" : "binary64-as-binary32").render(other);
    expect(outputs[0]).not.toStrictEqual(other);
  });

  test("leaves the one silent channel of a patch without output alone, without a marker", () => {
    const Engine = processor(ENGINE_PROCESSOR);
    const node = new Engine({ processorOptions: { wasm: wasmBytes(), patch: SILENT_PATCH, marker: false, precision: "binary64" } });
    const outputs = [[new Float32Array(128)]];
    expect(node.process([], outputs)).toBe(true);
    expect(outputs[0]?.[0]).toStrictEqual(new Float32Array(128));
  });

  test("keeps running with no output", () => {
    const Engine = processor(ENGINE_PROCESSOR);
    const node = new Engine({ processorOptions: { wasm: wasmBytes(), patch: SINE_PATCH, marker: false, precision: "binary64" } });
    expect(node.process([], [])).toBe(true);
  });

  test("counts every frame it renders on the channel after the patch channels", () => {
    const Engine = processor(ENGINE_PROCESSOR);
    const node = new Engine({ processorOptions: { wasm: wasmBytes(), patch: SINE_PATCH, marker: true, precision: "binary64" } });
    const outputs = [[new Float32Array(128), new Float32Array(128), new Float32Array(128)]];
    expect(node.process([], outputs)).toBe(true);
    const expected = [new Float32Array(256), new Float32Array(256)];
    WasmEngine.create(new WebAssembly.Module(wasmBytes()), SINE_PATCH, RATE, "binary64-as-binary32").render(expected);
    expect(outputs[0]?.slice(0, 2)).toStrictEqual(expected.map((channel) => channel.subarray(0, 128)));
    expect(outputs[0]?.[2]).toStrictEqual(Float32Array.from({ length: 128 }, (_empty, frame) => frame + 1));
    expect(node.process([], outputs)).toBe(true);
    expect(outputs[0]?.[2]).toStrictEqual(Float32Array.from({ length: 128 }, (_empty, frame) => frame + 129));
    // Without room for the marker (a host giving fewer channels), it renders the same
    // samples, and keeps counting.
    const fewer = [[new Float32Array(128), new Float32Array(128)]];
    expect(node.process([], fewer)).toBe(true);
    const later = [new Float32Array(384), new Float32Array(384)];
    WasmEngine.create(new WebAssembly.Module(wasmBytes()), SINE_PATCH, RATE, "binary64-as-binary32").render(later);
    expect(fewer[0]).toStrictEqual(later.map((channel) => channel.slice(256, 384)));
    expect(node.process([], [])).toBe(true);
    expect(node.process([], outputs)).toBe(true);
    expect(outputs[0]?.[2]).toStrictEqual(Float32Array.from({ length: 128 }, (_empty, frame) => frame + 385));
  });

  // 2^24 frames through the processor, which Stryker instruments: a fixed amount of work
  // (the period of the marker is the point), which took 0.5 to 1 s plain on a loaded
  // 10-core Mac, alone or beside the package's other files, and several times that
  // instrumented on a loaded machine (Vitest's default is 5 s). 60 s leaves a margin of
  // 60 times the slowest plain run.
  test("counts from 1 again after 2^24 frames, the largest count a float holds exactly", () => {
    const Engine = processor(ENGINE_PROCESSOR);
    const node = new Engine({ processorOptions: { wasm: wasmBytes(), patch: SILENT_PATCH, marker: true, precision: "binary64" } });
    // A patch without output: the marker is its first channel. 4096 quanta of 4096 frames reach 2^24.
    const outputs = [[new Float32Array(4096)]];
    for (let quantum = 1; quantum < 4096; quantum += 1) {
      node.process([], outputs);
    }
    expect(MARKER_PERIOD).toBe(2 ** 24);
    expect(outputs[0]?.[0]?.at(-1)).toBe(2 ** 24 - 4096);
    node.process([], [[new Float32Array(4095)]]);
    node.process([], outputs);
    expect(Array.from(outputs[0]?.[0]?.subarray(0, 3) ?? [])).toStrictEqual([2 ** 24, 1, 2]);
    expect(outputs[0]?.[0]?.at(-1)).toBe(4095);
  }, 60_000);

  test("creates one view per patch channel at its first quantum, a known deviation, then none", () => {
    // Recorded in CLAUDE.md: the first quantum on the audio thread creates the views of
    // the first block, which a garbage collection count cannot see (they are kept).
    const Engine = processor(ENGINE_PROCESSOR);
    const original = Float32Array;
    vi.stubGlobal("Float32Array", Counted);
    try {
      const node = new Engine({ processorOptions: { wasm: wasmBytes(), patch: SINE_PATCH, marker: true, precision: "binary64" } });
      const outputs = [[new Float32Array(128), new Float32Array(128), new Float32Array(128)]];
      expect(
        created(() => {
          node.process([], outputs);
        }),
      ).toBe(2);
      expect(
        created(() => {
          for (let quantum = 0; quantum < 100; quantum += 1) {
            node.process([], outputs);
          }
        }),
      ).toBe(0);
    } finally {
      vi.stubGlobal("Float32Array", original);
    }
  });

  test("tells the main thread it is ready", () => {
    const Engine = processor(ENGINE_PROCESSOR);
    const node = new Engine({ processorOptions: { wasm: wasmBytes(), patch: SINE_PATCH, marker: false, precision: "binary64" } });
    expect(node.port.posted).toStrictEqual([READY]);
  });

  test("tells the main thread what the module observed of subnormals on its thread", () => {
    // A module observing code 0b1001: binary32 operands and binary64 results kept, the rest flushed.
    const functions = engineFunctions().map((func) => (func.name === "engine_subnormals" ? { ...func, body: { i32: 0b1001 } } : func));
    const node = new (processor(ENGINE_PROCESSOR))({ processorOptions: { wasm: handMadeBytes({ memory: true, functions }).buffer, patch: SINE_PATCH, marker: false, precision: "binary64" } });
    expect(node.port.posted).toStrictEqual([
      { type: "ready", subnormals: { binary32: { operands: true, results: false }, binary64: { operands: false, results: true } } },
    ]);
  });

  test("reports invalid options, then outputs silence and stops", () => {
    const Engine = processor(ENGINE_PROCESSOR);
    const node = new Engine({});
    expect(node.port.posted).toStrictEqual([{ type: "failed", message: "OptionsError: invalid processor options: not an object" }]);
    const outputs = [[new Float32Array(4).fill(9)]];
    expect(node.process([], outputs)).toBe(false);
    expect(outputs[0]?.[0]).toStrictEqual(new Float32Array(4).fill(9));
  });

  test("reports the explanation of a rejected patch, as bytes", () => {
    const Engine = processor(ENGINE_PROCESSOR);
    const node = new Engine({ processorOptions: { wasm: wasmBytes(), patch: UNKNOWN_BOX, marker: false, precision: "binary64" } });
    const [message] = node.port.posted;
    expect(message).toMatchObject({ type: "rejected" });
    const detail: unknown = typeof message === "object" && message !== null ? Reflect.get(message, "detail") : undefined;
    expect(detail instanceof Uint8Array ? new TextDecoder().decode(detail) : detail).toBe('box 1: unknown box type "nope~"');
  });

  test("reports a failure while rendering once, then stops", () => {
    const Engine = processor(ENGINE_PROCESSOR);
    const node = new Engine({ processorOptions: { wasm: wasmBytes(), patch: SINE_PATCH, marker: false, precision: "binary64" } });
    // Outputs of different lengths make the engine refuse to render.
    expect(node.process([], [[new Float32Array(2), new Float32Array(3)]])).toBe(false);
    expect(node.port.posted).toStrictEqual([
      READY,
      { type: "failed", message: "RangeError: every output must have the same length: 3 instead of 2" },
    ]);
    expect(node.process([], [[new Float32Array(2)]])).toBe(false);
    expect(node.port.posted).toHaveLength(2);
  });
});

describe("engine processor traps", () => {
  /** The bytes of an engine module whose `name` export traps. */
  function trapping(name: string): ArrayBuffer {
    return handMadeBytes({ memory: true, functions: engineFunctions().map((func) => (func.name === name ? { ...func, body: "trap" as const } : func)) }).buffer;
  }

  test("reports a trap while loading with the trap itself, the only clue under panic = abort", () => {
    const node = new (processor(ENGINE_PROCESSOR))({ processorOptions: { wasm: trapping("engine_load"), patch: SINE_PATCH, marker: false, precision: "binary64" } });
    expect(node.port.posted).toStrictEqual([{ type: "failed", message: "EngineTrap: the engine module trapped and cannot be used again (RuntimeError: unreachable)" }]);
  });

  test("reports a trap while rendering with the trap itself", () => {
    const node = new (processor(ENGINE_PROCESSOR))({ processorOptions: { wasm: trapping("engine_process"), patch: SINE_PATCH, marker: false, precision: "binary64" } });
    expect(node.process([], [[new Float32Array(128)]])).toBe(false);
    expect(node.port.posted).toStrictEqual([
      READY,
      { type: "failed", message: "EngineTrap: the engine module trapped and cannot be used again (RuntimeError: unreachable)" },
    ]);
  });
});

describe("tap processor", () => {
  test("posts a copy of every input block until told to stop", () => {
    const tap = new (processor(TAP_PROCESSOR))();
    const left = new Float32Array([1, 2]);
    expect(tap.process([[left, new Float32Array([3, 4])]], [])).toBe(true);
    left[0] = 9;
    expect(tap.port.posted).toStrictEqual([[new Float32Array([1, 2]), new Float32Array([3, 4])]]);
    tap.port.onmessage?.();
    expect(tap.process([[left]], [])).toBe(false);
    expect(tap.port.posted).toHaveLength(1);
  });

  test("posts once that its source stopped, when its input goes from present to empty", () => {
    const tap = new (processor(TAP_PROCESSOR))();
    const quantum = [new Float32Array([1])];
    expect(tap.process([quantum], [])).toBe(true);
    expect(tap.process([[]], [])).toBe(true);
    expect(tap.process([[]], [])).toBe(true);
    expect(tap.process([], [])).toBe(true);
    expect(tap.port.posted).toStrictEqual([quantum, SOURCE_STOPPED]);
    // A source that starts again, then stops again: told again.
    expect(tap.process([quantum], [])).toBe(true);
    expect(tap.process([], [])).toBe(true);
    expect(tap.port.posted).toStrictEqual([quantum, SOURCE_STOPPED, quantum, SOURCE_STOPPED]);
    // Once told to stop, nothing.
    expect(tap.process([quantum], [])).toBe(true);
    tap.port.onmessage?.();
    expect(tap.process([], [])).toBe(false);
    expect(tap.port.posted).toHaveLength(5);
  });

  test("posts nothing without input", () => {
    const tap = new (processor(TAP_PROCESSOR))();
    expect(tap.process([[]], [])).toBe(true);
    expect(tap.process([], [])).toBe(true);
    expect(tap.port.posted).toStrictEqual([]);
  });
});
