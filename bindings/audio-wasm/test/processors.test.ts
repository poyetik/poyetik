import { afterEach, beforeAll, describe, expect, test, vi } from "vitest";
import { PROTECTED_OUTPUT, RAW_OUTPUT, WasmEngine, givenAsBinary32 } from "../ts/engine.ts";
import { ENGINE_PROCESSOR, MARKER_PERIOD, SOURCE_STOPPED, TAP_PROCESSOR } from "../ts/protocol.ts";
import { Counted, created } from "./counted.ts";
import { engineFunctions, handMadeBytes, type HandMadeFunction, PRECISION_NAMES, READY, SAMPLE_RATES, SILENT_PATCH, SINE_PATCH, UNKNOWN_BOX, wasmBytes } from "./fixtures.ts";
import { loadProcessors, processorClass, type Processor, type ProcessorClass } from "./worklet.ts";

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
    const node = new Engine({ processorOptions: { wasm: wasmBytes(), patch: SINE_PATCH, marker: false, precision: "binary64", output: RAW_OUTPUT } });
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
    const node = new Engine({ processorOptions: { wasm: wasmBytes(), patch: SINE_PATCH, marker: false, precision, output: RAW_OUTPUT } });
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
    const node = new Engine({ processorOptions: { wasm: wasmBytes(), patch: SILENT_PATCH, marker: false, precision: "binary64", output: RAW_OUTPUT } });
    const outputs = [[new Float32Array(128)]];
    expect(node.process([], outputs)).toBe(true);
    expect(outputs[0]?.[0]).toStrictEqual(new Float32Array(128));
  });

  test("keeps running with no output", () => {
    const Engine = processor(ENGINE_PROCESSOR);
    const node = new Engine({ processorOptions: { wasm: wasmBytes(), patch: SINE_PATCH, marker: false, precision: "binary64", output: RAW_OUTPUT } });
    expect(node.process([], [])).toBe(true);
  });

  test("counts every frame it renders on the channel after the patch channels", () => {
    const Engine = processor(ENGINE_PROCESSOR);
    const node = new Engine({ processorOptions: { wasm: wasmBytes(), patch: SINE_PATCH, marker: true, precision: "binary64", output: RAW_OUTPUT } });
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
    const node = new Engine({ processorOptions: { wasm: wasmBytes(), patch: SILENT_PATCH, marker: true, precision: "binary64", output: RAW_OUTPUT } });
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
      const node = new Engine({ processorOptions: { wasm: wasmBytes(), patch: SINE_PATCH, marker: true, precision: "binary64", output: RAW_OUTPUT } });
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
    const node = new Engine({ processorOptions: { wasm: wasmBytes(), patch: SINE_PATCH, marker: false, precision: "binary64", output: RAW_OUTPUT } });
    expect(node.port.posted).toStrictEqual([READY]);
  });

  test("tells the main thread what the module observed of subnormals on its thread", () => {
    // A module observing code 0b1001: binary32 operands and binary64 results kept, the rest flushed.
    const functions = engineFunctions().map((func) => (func.name === "engine_subnormals" ? { ...func, body: { i32: 0b1001 } } : func));
    const node = new (processor(ENGINE_PROCESSOR))({ processorOptions: { wasm: handMadeBytes({ memory: true, functions }).buffer, patch: SINE_PATCH, marker: false, precision: "binary64", output: RAW_OUTPUT } });
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
    const node = new Engine({ processorOptions: { wasm: wasmBytes(), patch: UNKNOWN_BOX, marker: false, precision: "binary64", output: RAW_OUTPUT } });
    const [message] = node.port.posted;
    expect(message).toMatchObject({ type: "rejected" });
    const detail: unknown = typeof message === "object" && message !== null ? Reflect.get(message, "detail") : undefined;
    expect(detail instanceof Uint8Array ? new TextDecoder().decode(detail) : detail).toBe('box 1: unknown box type "nope~"');
  });

  test("reports a failure while rendering once, then stops", () => {
    const Engine = processor(ENGINE_PROCESSOR);
    const node = new Engine({ processorOptions: { wasm: wasmBytes(), patch: SINE_PATCH, marker: false, precision: "binary64", output: RAW_OUTPUT } });
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

describe("engine processor reports of replaced samples", () => {
  /**
   * The bytes of an engine module of one channel whose blocks sit at address 0, whose
   * output stage reports a replaced count of 1, 2, 3… at each call of `engine_replaced`, its first sample at
   * frame 2^32 + 5 of channel 2; or, with `count` 0, nothing replaced.
   */
  function replacing(count: "counting" | 0): ArrayBuffer {
    const report = count === 0 ? { report: [0, 5, 1, 2] as const } : { report: [0, 5, 1, 2] as const, counting: true as const };
    const answers: Readonly<Record<string, HandMadeFunction["body"]>> = { engine_process: { i32: 0 }, engine_replaced: report };
    return handMadeBytes({ memory: true, functions: engineFunctions().map((func) => ({ ...func, body: answers[func.name] ?? func.body })) }).buffer;
  }

  test("tells nothing while nothing is replaced", () => {
    const node = new (processor(ENGINE_PROCESSOR))({ processorOptions: { wasm: replacing(0), patch: SINE_PATCH, marker: false, precision: "binary64", output: RAW_OUTPUT } });
    for (let quantum = 0; quantum !== 4; quantum += 1) {
      expect(node.process([], [[new Float32Array(128)]])).toBe(true);
    }
    expect(node.port.posted).toStrictEqual([READY]);
  });

  test("tells the first replacement at once, then more at most once per REPLACED_REPORT_SECONDS", () => {
    // Two quanta of 128 frames per second: the interval is two quanta.
    vi.stubGlobal("sampleRate", 256);
    const node = new (processor(ENGINE_PROCESSOR))({ processorOptions: { wasm: replacing("counting"), patch: SINE_PATCH, marker: false, precision: "binary64", output: RAW_OUTPUT } });
    const first = { frame: 2 ** 32 + 5, channel: 2 };
    // Each call counts on: the processor reads the count once per quantum (1, then 3, then 4),
    // and the first sample once more when it tells (2, then 5).
    expect(node.process([], [[new Float32Array(128)]])).toBe(true);
    expect(node.port.posted).toStrictEqual([READY, { type: "replaced", replaced: { count: 1, first } }]);
    expect(node.process([], [[new Float32Array(128)]])).toBe(true);
    expect(node.port.posted).toHaveLength(2);
    expect(node.process([], [[new Float32Array(128)]])).toBe(true);
    expect(node.port.posted).toStrictEqual([READY, { type: "replaced", replaced: { count: 1, first } }, { type: "replaced", replaced: { count: 4, first } }]);
  });
});

describe("engine processor output stage", () => {
  /** An engine processor running the sine through `output`. */
  function engine(output: unknown): Processor {
    const Engine = processor(ENGINE_PROCESSOR);
    return new Engine({ processorOptions: { wasm: wasmBytes(), patch: SINE_PATCH, marker: false, precision: "binary32", output } });
  }

  /** The largest magnitude of `quanta` render quanta of `node`'s first channel. */
  function peak(node: Processor, quanta: number): number {
    let largest = 0;
    for (let quantum = 0; quantum < quanta; quantum += 1) {
      const outputs = [[new Float32Array(128), new Float32Array(128)]];
      expect(node.process([], outputs)).toBe(true);
      largest = Math.max(largest, ...(outputs[0]?.[0] ?? []).map(Math.abs));
    }
    return largest;
  }

  test("creates the engine with the output stage of its options, delayed by its latency", () => {
    const node = engine(PROTECTED_OUTPUT);
    const outputs = [[new Float32Array(128), new Float32Array(128)], [new Float32Array(128), new Float32Array(128)]];
    expect(node.process([], [outputs[0] ?? []])).toBe(true);
    expect(node.process([], [outputs[1] ?? []])).toBe(true);
    const first = [...(outputs[0]?.[0] ?? []), ...(outputs[1]?.[0] ?? [])];
    // 5 ms at 48 kHz and the precise detector's 16 frames.
    expect(first.slice(0, 256).every((sample) => sample === 0)).toBe(true);
    expect(first[257]).not.toBe(0);
    expect(peak(node, 400)).toBeLessThanOrEqual(0.891_250_938_133_745_6);
  });

  test("applies the commands of the main thread, telling nothing on success", () => {
    const node = engine(PROTECTED_OUTPUT);
    expect(node.port.posted).toStrictEqual([READY]);
    node.port.onmessage?.({ data: { type: "limiter", settings: { ceiling: -6, lookahead: 1, release: 50, adaptivity: 0 } } });
    expect(peak(node, 400)).toBeLessThanOrEqual(10 ** (-6 / 20));
    node.port.onmessage?.({ data: { type: "protect", protect: false } });
    expect(peak(node, 400)).toBeGreaterThan(0.999);
    expect(node.port.posted).toStrictEqual([READY]);
  });

  test("tells a refused command back as bytes, and renders on", () => {
    const node = engine(RAW_OUTPUT);
    node.port.onmessage?.({ data: { type: "protect", protect: true } });
    expect(node.port.posted).toStrictEqual([READY, { type: "refused", detail: new TextEncoder().encode("the output stage is raw: it has no limiter to set") }]);
    expect(peak(node, 4)).toBeGreaterThan(0.1);
  });

  test("refuses a malformed command as the engine refuses a setting, its words as bytes, and renders on with the settings it had", () => {
    const node = engine(PROTECTED_OUTPUT);
    const malformed = [
      [{ type: "mute" }, "invalid command to the engine processor: unknown command"],
      [null, "invalid command to the engine processor: not an object"],
      [{ type: "limiter", settings: { ceiling: "-6", lookahead: 1, release: 50, adaptivity: 0 } }, "invalid command to the engine processor: limiter ceiling is not a number"],
      [{ type: "protect", protect: 1 }, "invalid command to the engine processor: protect is not a boolean"],
    ] as const;
    for (const [data] of malformed) {
      node.port.onmessage?.({ data });
    }
    expect(node.port.posted).toStrictEqual([READY, ...malformed.map(([, words]) => ({ type: "refused", detail: new TextEncoder().encode(words) }))]);
    // Still protected at the default ceiling, and still taking commands.
    expect(peak(node, 400)).toBeLessThanOrEqual(0.891_250_938_133_745_6);
    expect(peak(node, 4)).toBeGreaterThan(0.1);
    node.port.onmessage?.({ data: { type: "limiter", settings: { ceiling: -6, lookahead: 1, release: 50, adaptivity: 0 } } });
    // Ten quanta to settle: the samples limited before the command leave the stage's latency.
    peak(node, 10);
    expect(peak(node, 400)).toBeLessThanOrEqual(10 ** (-6 / 20));
    expect(node.port.posted).toHaveLength(1 + malformed.length);
  });
});

describe("engine processor commands to a module that breaks the interface", () => {
  test("stops on a module error of a live setting, as on a failure while rendering, then ignores every command", () => {
    const functions = engineFunctions().map((func) => (func.name === "engine_limiter" ? { ...func, body: { i32: 2 } } : func));
    // Built raw: the module is asked nothing about its stage until the command.
    const node = new (processor(ENGINE_PROCESSOR))({ processorOptions: { wasm: handMadeBytes({ memory: true, functions }).buffer, patch: SINE_PATCH, marker: false, precision: "binary64", output: RAW_OUTPUT } });
    node.port.onmessage?.({ data: { type: "limiter", settings: { ceiling: -6, lookahead: 1, release: 50, adaptivity: 0 } } });
    expect(node.port.posted).toStrictEqual([READY, { type: "failed", message: "ModuleError: not an engine module: engine_limiter returned 2, not 1 (succeeded) or 0 (refused)" }]);
    expect(node.process([], [[new Float32Array(128)]])).toBe(false);
    node.port.onmessage?.({ data: { type: "mute" } });
    node.port.onmessage?.({ data: { type: "protect", protect: true } });
    expect(node.port.posted).toHaveLength(2);
  });
});

describe("engine processor refusals of its output stage", () => {
  test("keeps the engine's explanation in its failure, decoded without a TextDecoder", () => {
    const Engine = processor(ENGINE_PROCESSOR);
    // As in a worklet scope without one: constructing a TextDecoder fails.
    const decoder = TextDecoder;
    vi.stubGlobal("TextDecoder", function absent() {
      throw new Error("no TextDecoder in this scope");
    });
    let node: Processor;
    try {
      expect(() => new TextDecoder()).toThrow(new Error("no TextDecoder in this scope"));
      node = new Engine({ processorOptions: { wasm: wasmBytes(), patch: SINE_PATCH, marker: false, precision: "binary64", output: { ...PROTECTED_OUTPUT, maximumLookahead: 9 } } });
    } finally {
      vi.stubGlobal("TextDecoder", decoder);
    }
    expect(node.port.posted).toStrictEqual([{ type: "failed", message: "OutputError: the engine refused the output settings: limiter maximum lookahead 9 ms is out of range (0.5 to 5 ms)" }]);
  });

  test("says so when the explanation is not UTF-8, rather than failing to report", () => {
    // engine_limiter writes the binary32 NaN 0x7f800001 at address 0, bytes 01 00 80 7f (a
    // lone continuation byte), and refuses; its explanation is those four bytes.
    const answers: Readonly<Record<string, HandMadeFunction["body"]>> = { engine_limiter: "nan", engine_message_ptr: { i32: 0 }, engine_message_len: { i32: 4 } };
    const functions = engineFunctions().map((func) => ({ ...func, body: answers[func.name] ?? func.body }));
    const node = new (processor(ENGINE_PROCESSOR))({ processorOptions: { wasm: handMadeBytes({ memory: true, functions }).buffer, patch: SINE_PATCH, marker: false, precision: "binary64", output: PROTECTED_OUTPUT } });
    expect(node.port.posted).toStrictEqual([{ type: "failed", message: "OutputError: the engine refused the output settings: an explanation that is not UTF-8 (01 00 80 7f)" }]);
  });
});

describe("engine processor traps", () => {
  /** The bytes of an engine module whose `name` export traps. */
  function trapping(name: string): ArrayBuffer {
    return handMadeBytes({ memory: true, functions: engineFunctions().map((func) => (func.name === name ? { ...func, body: "trap" as const } : func)) }).buffer;
  }

  test("reports a trap while loading with the trap itself, the only clue under panic = abort", () => {
    const node = new (processor(ENGINE_PROCESSOR))({ processorOptions: { wasm: trapping("engine_load"), patch: SINE_PATCH, marker: false, precision: "binary64", output: RAW_OUTPUT } });
    expect(node.port.posted).toStrictEqual([{ type: "failed", message: "EngineTrap: the engine module trapped and cannot be used again (RuntimeError: unreachable)" }]);
  });

  test("reports a trap while rendering with the trap itself", () => {
    const node = new (processor(ENGINE_PROCESSOR))({ processorOptions: { wasm: trapping("engine_process"), patch: SINE_PATCH, marker: false, precision: "binary64", output: RAW_OUTPUT } });
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
    tap.port.onmessage?.({ data: "stop" });
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
    tap.port.onmessage?.({ data: "stop" });
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
