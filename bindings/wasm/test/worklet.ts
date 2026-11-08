// A fake `AudioWorkletGlobalScope`, enough to load the real worklet module in Node and to
// drive its processors as the audio thread would: shared by the unit tests and the
// allocation tests.
import { vi } from "vitest";

/** Stands in for the processor port: records messages. */
export class FakePort {
  /** Set by the processor to receive messages from the main thread; the tests call it to simulate one. */
  onmessage: (() => void) | null = null;
  /** Every message the processor posted, in order. */
  readonly posted: unknown[] = [];

  /** Records `message` instead of sending it to the main thread. */
  postMessage(message: unknown): void {
    this.posted.push(message);
  }
}

/** Stands in for the `AudioWorkletProcessor` base class. */
class FakeProcessor {
  /** The fake port of this processor, through which the tests read what it posted. */
  readonly port = new FakePort();
}

/** A registered processor, as the tests drive it. */
export interface Processor {
  readonly port: FakePort;
  process(inputs: Float32Array[][], outputs: Float32Array[][]): boolean;
}

/** A registered processor class. */
export type ProcessorClass = new (options?: unknown) => Processor;

/** The worklet module being loaded, with what it registered: loaded once per test file. */
let loading: Promise<ReadonlyMap<string, ProcessorClass>> | undefined;

/**
 * Stubs the worklet globals (the base class, `sampleRate` at `rate` hertz and
 * `registerProcessor`), then imports the worklet module, once: the processor classes it
 * registered, by name, in order. Later calls only set the rate, as does
 * `vi.stubGlobal("sampleRate", …)`.
 */
export function loadProcessors(rate: number): Promise<ReadonlyMap<string, ProcessorClass>> {
  vi.stubGlobal("sampleRate", rate);
  loading ??= (async () => {
    const registered = new Map<string, ProcessorClass>();
    vi.stubGlobal("AudioWorkletProcessor", FakeProcessor);
    vi.stubGlobal("registerProcessor", (name: string, constructor: ProcessorClass) => {
      registered.set(name, constructor);
    });
    await import("#worklet/processors");
    return registered;
  })();
  return loading;
}

/** The processor class registered under `name`. */
export function processorClass(registered: ReadonlyMap<string, ProcessorClass>, name: string): ProcessorClass {
  const found = registered.get(name);
  if (found === undefined) {
    throw new Error(`${name} is not registered`);
  }
  return found;
}
