// Drives the real audio-thread entry point, `EngineProcessor.process`, through the fake
// worklet scope from a fresh processor, as a browser drives it from its first quantum.
import { expect } from "vitest";
import { ENGINE_PROCESSOR } from "../../ts/protocol.ts";
import { SINE_PATCH, wasmBytes } from "../fixtures.ts";
import { loadProcessors, processorClass } from "../worklet.ts";
import { collections, repeat } from "./collections.ts";

/** The sample rate of the fake worklet scope, in hertz. */
const RATE = 44_100;
/** Quanta rendered before measuring: only the first calls compile and allocate. */
const WARM_UP = 100;
/** Frames per block of the engine: quanta that are whole blocks are copied without allocating. */
export const BLOCK = 64;

/** Collections caused by rendering `frames` frames through quanta of `size` frames, from a fresh processor. */
export async function processing(size: number, frames: number): Promise<number> {
  const EngineProcessor = processorClass(await loadProcessors(RATE), ENGINE_PROCESSOR);
  const node = new EngineProcessor({ processorOptions: { wasm: wasmBytes(), patch: SINE_PATCH, marker: false } });
  // Made once, as a browser hands the processor the same arrays at every quantum.
  const inputs: Float32Array[][] = [];
  const outputs = [[new Float32Array(size), new Float32Array(size)]];
  const run = (): void => {
    if (!node.process(inputs, outputs)) {
      throw new Error("the processor stopped");
    }
  };
  repeat(WARM_UP, run);
  const count = await collections(() => {
    repeat(Math.ceil(frames / size), run);
  });
  expect(node.port.posted).toStrictEqual([{ type: "ready" }]);
  return count;
}
