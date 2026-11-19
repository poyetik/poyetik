// Drives the real audio-thread entry points, `EngineProcessor.process` and
// `TapProcessor.process`, through the fake worklet scope: from the first quantum of a fresh
// processor on, as a browser drives it, every quantum counted.
import { expect } from "vitest";
import { PROTECTED_OUTPUT, RAW_OUTPUT, type Output, type OutputMode } from "../../ts/engine.ts";
import { ENGINE_PROCESSOR, TAP_PROCESSOR } from "../../ts/protocol.ts";
import { READY, SINE_PATCH, wasmBytes } from "../fixtures.ts";
import { loadProcessors, processorClass } from "../worklet.ts";
import { collections, repeat } from "./collections.ts";

/** The sample rate of the fake worklet scope, in hertz. */
const RATE = 44_100;
/** Frames per block of the engine: quanta that are whole blocks are copied without allocating. */
export const BLOCK = 64;

/**
 * An output stage of each mode: raw, protected, and switchable (raw first, its limiter's
 * latency kept), a record, so that a new mode does not compile without one.
 */
export const OUTPUTS: Readonly<Record<OutputMode, Output>> = { raw: RAW_OUTPUT, protected: PROTECTED_OUTPUT, switchable: { ...PROTECTED_OUTPUT, mode: "switchable" } };

/**
 * Collections caused by rendering `frames` frames through quanta of `size` frames, through
 * the output stage `output` (raw by default), from the first quantum of a fresh processor,
 * calling `alongside` after every quantum (the positive control: something that
 * allocates, to prove that the count sees it). Given a `command`, the processor first
 * renders ten quanta uncounted, then receives it, and the count starts after it.
 */
export async function processing(size: number, frames: number, alongside: () => void = () => undefined, output: Output = RAW_OUTPUT, command?: unknown): Promise<number> {
  const EngineProcessor = processorClass(await loadProcessors(RATE), ENGINE_PROCESSOR);
  const node = new EngineProcessor({ processorOptions: { wasm: wasmBytes(), patch: SINE_PATCH, marker: false, precision: "binary64", output } });
  // Made once, as a browser hands the processor the same arrays at every quantum.
  const inputs: Float32Array[][] = [];
  const outputs = [[new Float32Array(size), new Float32Array(size)]];
  if (command !== undefined) {
    repeat(10, () => node.process(inputs, outputs));
    node.port.onmessage?.({ data: command });
  }
  const count = await collections(() => {
    repeat(Math.ceil(frames / size), () => {
      if (!node.process(inputs, outputs)) {
        throw new Error("the processor stopped");
      }
      alongside();
    });
  });
  expect(node.port.posted).toStrictEqual([READY]);
  return count;
}

/** Collections caused by tapping `quanta` quanta of two channels of 128 frames, from a fresh tap processor. */
export async function tapping(quanta: number): Promise<number> {
  const TapProcessor = processorClass(await loadProcessors(RATE), TAP_PROCESSOR);
  const node = new TapProcessor();
  const inputs = [[new Float32Array(128), new Float32Array(128)]];
  const count = await collections(() => {
    repeat(quanta, () => {
      node.process(inputs, []);
    });
  });
  expect(node.port.posted).toHaveLength(quanta);
  return count;
}
