import { beforeAll, describe, expect, test, vi } from "vitest";
import { WasmEngine } from "../ts/engine.ts";
import { ENGINE_PROCESSOR, TAP_PROCESSOR } from "../ts/protocol.ts";
import { SINE_PATCH, UNKNOWN_BOX, wasmBytes } from "./fixtures.ts";

const RATE = 48_000;

/** Stands in for the processor port: records messages. */
class FakePort {
  onmessage: (() => void) | null = null;
  readonly posted: unknown[] = [];

  postMessage(message: unknown): void {
    this.posted.push(message);
  }
}

/** Stands in for the `AudioWorkletProcessor` base class. */
class FakeProcessor {
  readonly port = new FakePort();
}

/** A registered processor, as the tests drive it. */
interface Processor {
  readonly port: FakePort;
  process(inputs: Float32Array[][], outputs: Float32Array[][]): boolean;
}

const registered = new Map<string, new (options?: unknown) => Processor>();

beforeAll(async () => {
  vi.stubGlobal("AudioWorkletProcessor", FakeProcessor);
  vi.stubGlobal("sampleRate", RATE);
  vi.stubGlobal("registerProcessor", (name: string, constructor: new (options?: unknown) => Processor) => {
    registered.set(name, constructor);
  });
  await import("#worklet/processors");
});

/** The registered processor class of this name. */
function processor(name: string): new (options?: unknown) => Processor {
  const found = registered.get(name);
  if (found === undefined) {
    throw new Error(`${name} is not registered`);
  }
  return found;
}

describe("engine processor", () => {
  test("is registered under its name", () => {
    expect([...registered.keys()]).toStrictEqual([ENGINE_PROCESSOR, TAP_PROCESSOR]);
  });

  test("renders the patch at the rate of the worklet", () => {
    const Engine = processor(ENGINE_PROCESSOR);
    const node = new Engine({ processorOptions: { wasm: wasmBytes(), patch: SINE_PATCH } });
    const outputs = [[new Float32Array(128), new Float32Array(128)]];
    expect(node.process([], outputs)).toBe(true);
    const expected = [new Float32Array(128), new Float32Array(128)];
    WasmEngine.create(new WebAssembly.Module(wasmBytes()), SINE_PATCH, RATE).render(expected);
    expect(outputs[0]).toStrictEqual(expected);
  });

  test("keeps running with no output", () => {
    const Engine = processor(ENGINE_PROCESSOR);
    const node = new Engine({ processorOptions: { wasm: wasmBytes(), patch: SINE_PATCH } });
    expect(node.process([], [])).toBe(true);
  });

  test("tells the main thread it is ready", () => {
    const Engine = processor(ENGINE_PROCESSOR);
    const node = new Engine({ processorOptions: { wasm: wasmBytes(), patch: SINE_PATCH } });
    expect(node.port.posted).toStrictEqual([{ type: "ready" }]);
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
    const node = new Engine({ processorOptions: { wasm: wasmBytes(), patch: UNKNOWN_BOX } });
    const [message] = node.port.posted;
    expect(message).toMatchObject({ type: "rejected" });
    const detail: unknown = typeof message === "object" && message !== null ? Reflect.get(message, "detail") : undefined;
    expect(detail instanceof Uint8Array ? new TextDecoder().decode(detail) : detail).toBe('node 1: unknown box type "nope~"');
  });

  test("reports a failure while rendering once, then stops", () => {
    const Engine = processor(ENGINE_PROCESSOR);
    const node = new Engine({ processorOptions: { wasm: wasmBytes(), patch: SINE_PATCH } });
    // Outputs of different lengths make the engine refuse to render.
    expect(node.process([], [[new Float32Array(2), new Float32Array(3)]])).toBe(false);
    expect(node.port.posted).toStrictEqual([
      { type: "ready" },
      { type: "failed", message: "RangeError: every output must have the same length: 3 instead of 2" },
    ]);
    expect(node.process([], [[new Float32Array(2)]])).toBe(false);
    expect(node.port.posted).toHaveLength(2);
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

  test("posts nothing without input", () => {
    const tap = new (processor(TAP_PROCESSOR))();
    expect(tap.process([[]], [])).toBe(true);
    expect(tap.process([], [])).toBe(true);
    expect(tap.port.posted).toStrictEqual([]);
  });
});
