import { describe, expect, test } from "vitest";
import { ModuleError, PatchError, WasmEngine } from "../ts/engine.ts";
import { interleave, renderHash } from "../ts/hash.ts";
import { FUNCTIONS, oscillators, SILENT_PATCH, SINE_FRAMES, SINE_HASH, SINE_PATCH, UNKNOWN_BOX, handMadeModule, wasmBytes } from "./fixtures.ts";

const module = new WebAssembly.Module(wasmBytes());
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
  test("renders sine-440 in Node bit-identical to the native engine", async () => {
    const channels = renderSine(SINE_FRAMES, 128);
    expect(await renderHash(RATE, 2, interleave(channels))).toBe(SINE_HASH);
  });

  test.each([1, 37, 64, 100, 4096])("gives the same samples whatever the buffer size (%i)", (size) => {
    expect(renderSine(1000, size)).toStrictEqual(renderSine(1000, 128));
  });

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

  test.each([1, 37, 64, 100, 4096])("never renders a block ahead (buffers of %i frames)", (size) => {
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
    const empty = handMadeModule({ memory: false, functions: FUNCTIONS });
    expect(() => WasmEngine.create(empty, SINE_PATCH, RATE)).toThrow(new ModuleError("no memory"));
  });

  test.each(FUNCTIONS)("rejects a module without %s", (missing) => {
    const partial = handMadeModule({ memory: true, functions: FUNCTIONS.filter((name) => name !== missing) });
    expect(() => WasmEngine.create(partial, SINE_PATCH, RATE)).toThrow(new ModuleError(`no function ${missing}`));
  });

  test("rejects a module whose functions do not return numbers", () => {
    const bigints = handMadeModule({ memory: true, functions: FUNCTIONS });
    expect(() => WasmEngine.create(bigints, SINE_PATCH, RATE)).toThrow(new ModuleError("engine_input did not return a number"));
  });

  test("names its errors", () => {
    expect(new ModuleError("x")).toMatchObject({ name: "ModuleError", message: "not an engine module: x" });
  });
});
