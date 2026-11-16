// The fake Web Audio world of the player tests, shared by test/player.test.ts and
// test/player.property.test.ts: a context with a clock that runs only while it runs,
// worklet nodes, gains, constant sources and a fetch, every action logged in order.
import { readFileSync } from "node:fs";
import { vi } from "vitest";
import { SINE_440 } from "../src/patch.ts";
import type { Environment, EnginePlayer, PlayerOptions } from "../src/player.ts";

/** The built engine module. */
export const WASM = readFileSync(new URL(import.meta.resolve("@poyetik/audio-wasm/engine-128.wasm")));

/** Everything the fake Web Audio world did, in order. */
export const log: string[] = [];

/** The first message of an engine processor whose audio thread keeps every subnormal (IEEE arithmetic). */
export const READY = Object.freeze({
  type: "ready",
  subnormals: Object.freeze({ binary32: Object.freeze({ operands: true, results: true }), binary64: Object.freeze({ operands: true, results: true }) }),
});

/** What the next engine processor answers (`answer`): its first message, or nothing at all. */
export const processor: { answer: unknown } = { answer: READY };

/** A fake message port. */
class FakePort {
  /** The handler the player sets for messages of the processor, called by {@link FakePort.deliver}. */
  onmessage: ((event: { data: unknown }) => void) | null = null;

  /** Logs the message sent to the processor instead of sending it. */
  postMessage(message: unknown): void {
    log.push(`post ${String(message)}`);
  }

  /** Delivers `data` as a message from the processor. */
  deliver(data: unknown): void {
    this.onmessage?.({ data });
  }
}

/** A fake node: logs its connections, returns its target like the real `connect`. */
class FakeNode {
  /** The port of the node; only worklet nodes use it, to talk to their processor. */
  readonly port = new FakePort();
  /** How the node appears in the log: its processor name, or its kind and setting. */
  readonly name: string;

  constructor(name: string) {
    this.name = name;
  }

  /** Logs the connection to `target`; returns it, so that connections chain as in Web Audio. */
  connect<T>(target: T): T {
    log.push(`connect ${this.name} -> ${target instanceof FakeNode ? target.name : "destination"}`);
    return target;
  }

  /** Logs the disconnection from `target`. */
  disconnect(target: unknown): void {
    log.push(`disconnect ${this.name} -> ${target instanceof FakeNode ? target.name : "?"}`);
  }
}

/** Fake `AudioWorkletNode`: named after its processor; the engine one answers `processor.answer`. */
export class FakeWorkletNode extends FakeNode {
  /** Every worklet node created, in order; emptied before each test. */
  static readonly created: FakeWorkletNode[] = [];
  /** The options the player built the node with, checked by the tests. */
  readonly options: unknown;
  /** The handler the player sets for a processor failure; a test calls it to simulate a crash of the audio thread. */
  onprocessorerror: (() => void) | null = null;

  constructor(_context: unknown, name: string, options: unknown) {
    super(name);
    this.options = options;
    FakeWorkletNode.created.push(this);
    const answer = processor.answer;
    if (name === "poyetik-engine" && answer !== undefined) {
      queueMicrotask(() => {
        this.port.deliver(answer);
      });
    }
  }
}

/** A fake `AudioParam`: logs every scheduled value. */
class FakeParam {
  /** The current value of the parameter, updated at the end of a ramp or when set. */
  #value: number;
  /** The node the parameter belongs to, prefixing its log lines. */
  readonly owner: string;

  constructor(owner: string, value: number) {
    this.owner = owner;
    this.#value = value;
  }

  /** The current value. */
  get value(): number {
    return this.#value;
  }

  /** Sets the value at once, logged; refuses a non-finite one, as a browser does. */
  set value(value: number) {
    finite(value);
    log.push(`${this.owner} value ${String(value)}`);
    this.#value = value;
  }

  /** Logs `value` scheduled at `time`, in seconds of the context clock. */
  setValueAtTime(value: number, time: number): void {
    finite(value, time);
    log.push(`${this.owner} set ${String(value)} at ${String(time)}`);
  }

  /** Logs a ramp to `value` ending at `time`, in seconds, and takes that value at once. */
  linearRampToValueAtTime(value: number, time: number): void {
    finite(value, time);
    log.push(`${this.owner} ramp to ${String(value)} at ${String(time)}`);
    this.#value = value;
  }

  /** Logs the cancellation of the values scheduled from `time`, in seconds. */
  cancelScheduledValues(time: number): void {
    finite(time);
    log.push(`${this.owner} cancel from ${String(time)}`);
  }
}

/**
 * Refuses a non-finite argument as a browser's bindings refuse it for an `AudioParam`
 * (Web IDL `float` and `double`, not `unrestricted`): a `TypeError` with WebKit's words.
 */
function finite(...values: readonly number[]): void {
  if (!values.every(Number.isFinite)) {
    throw new TypeError("The provided value is non-finite");
  }
}

/** Fake `GainNode`, with a recording gain. */
class FakeGainNode extends FakeNode {
  /** The gain parameter, logged under the name "gain". */
  readonly gain: FakeParam;

  constructor(_context: unknown, options: { gain: number }) {
    super(`gain ${String(options.gain)}`);
    this.gain = new FakeParam("gain", options.gain);
  }
}

/** Fake `ConstantSourceNode`: ends when the clock of its context reaches its stop time, as in browsers. */
class FakeConstantSource extends FakeNode {
  /** The handler the player sets for the end of the source, called by {@link FakeContext.advance}. */
  onended: (() => void) | null = null;
  /** When the source ends, in seconds of the context clock; undefined until stopped. */
  end: number | undefined;

  constructor(context: FakeContext, options: { offset?: number }) {
    if (FakeContext.refuses.has("fade")) {
      throw new DOMException("cannot schedule", "InvalidStateError");
    }
    super(`clock offset ${String(options.offset)}`);
    context.sources.push(this);
  }

  /** Logs the start of the source. */
  start(): void {
    log.push("clock start");
  }

  /** Logs and records the stop at `time`, in seconds: the source ends once the clock reaches it. */
  stop(time: number): void {
    log.push(`clock stop at ${String(time)}`);
    this.end = time;
  }
}

/** Fake `AudioContext`, at 48 kHz, 1 s into its clock. */
export class FakeContext extends EventTarget {
  /** Every context created, in order; emptied before each test. */
  static readonly created: FakeContext[] = [];
  /** The state `resume` leads to: a browser may keep a context suspended. */
  static resumesTo: AudioContextState = "running";
  /** The rate of the device: each test may pick another one. */
  static rate = 48_000;
  /** Sample rate of the context, in hertz: the rate asked for, else the device rate when it was created. */
  readonly sampleRate: number;
  /** Time of the context clock, in seconds: 1 at creation, so that the scheduled times in the log are predictable; moved only by {@link FakeContext.advance}. */
  currentTime = 1;
  /** The constant sources of the context, whose ends {@link FakeContext.advance} fires. */
  readonly sources: FakeConstantSource[] = [];
  /** The speakers, as an opaque target of connections. */
  readonly destination = {};
  /** State of the context: suspended until `resume`, as browsers create them outside a user gesture. */
  state: AudioContextState | "interrupted" = "suspended";
  /** The handler attribute's callback; registered as a listener when first set, as in browsers. */
  #handler: (() => void) | null = null;
  /** Calls the current handler: the listener the attribute holds its place with. */
  readonly #invoke = (): void => {
    // A browser reports what a handler throws on the window, never to the code that fired
    // the event: kept here for the tests to see, rather than crashing their run.
    try {
      this.#handler?.();
    } catch (error) {
      FakeContext.reported.push(error);
    }
  };
  /** The event handler attribute: takes its place among the listeners when first set, keeps it when replaced, leaves when cleared. */
  get onstatechange(): (() => void) | null {
    return this.#handler;
  }
  /** Sets the handler attribute. */
  set onstatechange(handler: (() => void) | null) {
    if (handler === null) {
      this.removeEventListener("statechange", this.#invoke);
    } else if (this.#handler === null) {
      this.addEventListener("statechange", this.#invoke);
    }
    this.#handler = handler;
  }
  /** The worklet of the context: logs the modules added, and always loads them. */
  readonly audioWorklet = {
    addModule: (url: string): Promise<void> => {
      log.push(`add module ${url}`);
      return Promise.resolve();
    },
  };

  /** Creates the context, at the rate asked for or the device rate, unless the fake browser refuses to (no audio device). */
  constructor(options?: { sampleRate?: number }) {
    if (FakeContext.refuses.has("create")) {
      throw new DOMException("no device", "NotSupportedError");
    }
    super();
    this.sampleRate = options?.sampleRate ?? FakeContext.rate;
    FakeContext.created.push(this);
    log.push("new context");
  }

  /** What the fake browser refuses, to check that the player never rejects nor throws. */
  static readonly refuses = new Set<"create" | "resume" | "close" | "fade">();
  /** What the handlers of every context threw, in order: a browser reports it on the window; emptied before each test. */
  static readonly reported: unknown[] = [];
  /** When set, `resume` answers only once the test resolves it. */
  static resuming: Promise<undefined> | undefined;

  /** Logs the resume and moves to {@link FakeContext.resumesTo}; answers at once, or when {@link FakeContext.resuming} is resolved. */
  resume(): Promise<void> {
    if (FakeContext.refuses.has("resume")) {
      return Promise.reject(new DOMException("closing", "InvalidStateError"));
    }
    log.push("resume");
    this.state = FakeContext.resumesTo;
    return FakeContext.resuming ?? Promise.resolve();
  }

  /**
   * Rejects on a closed context, like browsers, or when the fake browser refuses; otherwise
   * closes as browsers do (measured in Chrome 157 and Firefox 134, 2026-11-12; the Web Audio
   * specification queues it so): its promise settles first, and `statechange` fires in a
   * later task, `setImmediate` here, which runs before a later `setImmediate` of the test.
   * Its state reads "closed" as soon as it returns, as in Chrome and the specification
   * (Firefox changes it only when the promise settles). WebKit is read from its source,
   * not measured, until a browser test of this order (roadmap #24 step 94).
   */
  close(): Promise<void> {
    if (this.state === "closed" || FakeContext.refuses.has("close")) {
      return Promise.reject(new DOMException("already closed", "InvalidStateError"));
    }
    log.push("close");
    this.state = "closed";
    setImmediate(() => this.dispatchEvent(new Event("statechange")));
    return Promise.resolve();
  }

  /**
   * Lets `seconds` of audio time pass, as the audio thread would: only while the context
   * runs (a suspended or closed context has a frozen clock). Sources whose stop time is
   * reached end, asynchronously as browser events do.
   */
  advance(seconds: number): void {
    if (this.state !== "running") {
      return;
    }
    this.currentTime += seconds;
    for (const source of this.sources) {
      if (source.end !== undefined && source.end <= this.currentTime) {
        source.end = undefined;
        queueMicrotask(() => source.onended?.());
      }
    }
  }

  /** The browser changes the state; Safari also has "interrupted", which the DOM types do not list. */
  become(state: AudioContextState | "interrupted"): void {
    this.state = state;
    this.dispatchEvent(new Event("statechange"));
  }
}


/** A browser with everything the player needs. */
export const CAPABLE: Environment = { isSecureContext: true, AudioContext: FakeContext, AudioWorkletNode: FakeWorkletNode, WebAssembly };

/** Options of a player; `changes` replace some of them. */
export function playerOptions(patch = SINE_440, environment = CAPABLE, changes: Partial<PlayerOptions> = {}): PlayerOptions {
  return { wasm: "engine.wasm", worklet: "processors.js", patch, gain: 0.25, fade: 0.02, historySeconds: 4 / 48_000, environment, ...changes };
}

/** The signals of the requests made, in order: a session aborts its downloads when it ends. */
export const signals: AbortSignal[] = [];

/** Fetch answering every request with the engine module, or with `status`, or never (`"never"`). */
export function stubFetch(status: number | "never" = 200): void {
  vi.stubGlobal("fetch", (url: string, init: { signal: AbortSignal }) => {
    log.push(`fetch ${url}`);
    signals.push(init.signal);
    if (status === "never") {
      return new Promise(() => undefined);
    }
    return Promise.resolve({ ok: status === 200, status, arrayBuffer: () => Promise.resolve(WASM.buffer.slice(WASM.byteOffset, WASM.byteOffset + WASM.byteLength)) });
  });
}

/** Fetch holding every request until the function returned answers them all with the engine module. */
export function holdFetch(): () => void {
  const held: ((response: unknown) => void)[] = [];
  vi.stubGlobal("fetch", (url: string) => {
    log.push(`fetch ${url}`);
    return new Promise((resolve) => {
      held.push(resolve);
    });
  });
  return () => {
    for (const answer of held.splice(0)) {
      answer({ ok: true, status: 200, arrayBuffer: () => Promise.resolve(WASM.buffer.slice(WASM.byteOffset, WASM.byteOffset + WASM.byteLength)) });
    }
  };
}

/** The worklet node of this processor, from the last start. */
export function workletNode(name: string): FakeWorkletNode | undefined {
  return FakeWorkletNode.created.filter((node) => node.name === name).pop();
}

/** The context of the last start. */
export function lastContext(): FakeContext | undefined {
  return FakeContext.created.at(-1);
}

/** Stops `player` while it plays, letting the audio clock of its context run through the fade out. */
export async function stopPlaying(player: EnginePlayer): Promise<void> {
  const stopping = player.stop();
  lastContext()?.advance(1);
  await stopping;
}

/** Lets every pending callback run. */
export function flush(): Promise<void> {
  return new Promise((resolve) => setImmediate(resolve));
}

/** Empties the logs and puts the fake browser back in its default state; stubs the Web Audio globals and fetch. */
export function resetFakes(): void {
  log.length = 0;
  signals.length = 0;
  processor.answer = READY;
  FakeContext.created.length = 0;
  FakeContext.resumesTo = "running";
  FakeContext.resuming = undefined;
  FakeContext.refuses.clear();
  FakeContext.reported.length = 0;
  FakeContext.rate = 48_000;
  FakeWorkletNode.created.length = 0;
  vi.stubGlobal("AudioContext", FakeContext);
  vi.stubGlobal("AudioWorkletNode", FakeWorkletNode);
  vi.stubGlobal("GainNode", FakeGainNode);
  vi.stubGlobal("ConstantSourceNode", FakeConstantSource);
  stubFetch();
}
