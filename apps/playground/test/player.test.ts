import { readFileSync } from "node:fs";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { SINE_440 } from "../src/patch.ts";
import { EnginePlayer, explain, missing, type Environment, type Missing, type PlayerOptions } from "../src/player.ts";

/** The built engine module. */
const WASM = readFileSync(new URL(import.meta.resolve("@poyetik/engine-wasm/engine.wasm")));

/** Everything the fake Web Audio world did, in order. */
let log: string[] = [];

/** What the next engine processor answers: its first message, or nothing at all. */
let engineAnswer: unknown = { type: "ready" };

/** A fake message port. */
class FakePort {
  onmessage: ((event: { data: unknown }) => void) | null = null;

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
  readonly port = new FakePort();
  readonly name: string;

  constructor(name: string) {
    this.name = name;
  }

  connect<T>(target: T): T {
    log.push(`connect ${this.name} -> ${target instanceof FakeNode ? target.name : "destination"}`);
    return target;
  }

  disconnect(target: unknown): void {
    log.push(`disconnect ${this.name} -> ${target instanceof FakeNode ? target.name : "?"}`);
  }
}

/** Fake `AudioWorkletNode`: named after its processor; the engine one answers `engineAnswer`. */
class FakeWorkletNode extends FakeNode {
  static readonly created: FakeWorkletNode[] = [];
  readonly options: unknown;
  onprocessorerror: (() => void) | null = null;

  constructor(_context: unknown, name: string, options: unknown) {
    super(name);
    this.options = options;
    FakeWorkletNode.created.push(this);
    const answer = engineAnswer;
    if (name === "poyetik-engine" && answer !== undefined) {
      queueMicrotask(() => {
        this.port.deliver(answer);
      });
    }
  }
}

/** A fake `AudioParam`: logs every scheduled value. */
class FakeParam {
  value: number;
  readonly owner: string;

  constructor(owner: string, value: number) {
    this.owner = owner;
    this.value = value;
  }

  setValueAtTime(value: number, time: number): void {
    log.push(`${this.owner} set ${String(value)} at ${String(time)}`);
  }

  linearRampToValueAtTime(value: number, time: number): void {
    log.push(`${this.owner} ramp to ${String(value)} at ${String(time)}`);
    this.value = value;
  }

  cancelScheduledValues(time: number): void {
    log.push(`${this.owner} cancel from ${String(time)}`);
  }
}

/** Fake `GainNode`, with a recording gain. */
class FakeGainNode extends FakeNode {
  readonly gain: FakeParam;

  constructor(_context: unknown, options: { gain: number }) {
    super(`gain ${String(options.gain)}`);
    this.gain = new FakeParam("gain", options.gain);
  }
}

/** Fake `ConstantSourceNode`: ends as soon as it is stopped, as if its time had come. */
class FakeConstantSource extends FakeNode {
  onended: (() => void) | null = null;

  constructor(_context: unknown, options: { offset?: number }) {
    super(`clock offset ${String(options.offset)}`);
  }

  start(): void {
    log.push("clock start");
  }

  stop(time: number): void {
    log.push(`clock stop at ${String(time)}`);
    queueMicrotask(() => this.onended?.());
  }
}

/** Fake `AudioContext`, at 48 kHz, 1 s into its clock. */
class FakeContext {
  static readonly created: FakeContext[] = [];
  /** The state `resume` leads to: a browser may keep a context suspended. */
  static resumesTo: AudioContextState = "running";
  readonly sampleRate = 48_000;
  readonly currentTime = 1;
  readonly destination = {};
  state: AudioContextState = "suspended";
  onstatechange: (() => void) | null = null;
  readonly audioWorklet = {
    addModule: (url: string) => {
      log.push(`add module ${url}`);
      return Promise.resolve();
    },
  };

  constructor() {
    FakeContext.created.push(this);
    log.push("new context");
  }

  resume(): Promise<void> {
    log.push("resume");
    this.state = FakeContext.resumesTo;
    return Promise.resolve();
  }

  /** Rejects on a closed context, like browsers. */
  close(): Promise<void> {
    if (this.state === "closed") {
      return Promise.reject(new DOMException("already closed", "InvalidStateError"));
    }
    log.push("close");
    this.state = "closed";
    return Promise.resolve();
  }

  /** The browser changes the state. */
  become(state: AudioContextState): void {
    this.state = state;
    this.onstatechange?.();
  }
}

/** A browser with everything the player needs. */
const CAPABLE: Environment = { isSecureContext: true, AudioContext: FakeContext, AudioWorkletNode: FakeWorkletNode, WebAssembly };

/** Options of a player. */
function options(patch = SINE_440, environment = CAPABLE): PlayerOptions {
  return { wasm: "engine.wasm", worklet: "processors.js", patch, gain: 0.25, fade: 0.02, historySeconds: 4 / 48_000, environment };
}

/** Fetch answering every request with the engine module, or with `status`. */
function stubFetch(status = 200): void {
  vi.stubGlobal("fetch", (url: string) => {
    log.push(`fetch ${url}`);
    return Promise.resolve({ ok: status === 200, status, arrayBuffer: () => Promise.resolve(WASM.buffer.slice(WASM.byteOffset, WASM.byteOffset + WASM.byteLength)) });
  });
}

/** The worklet node of this processor, from the last start. */
function workletNode(name: string): FakeWorkletNode | undefined {
  return FakeWorkletNode.created.filter((node) => node.name === name).pop();
}

/** The context of the last start. */
function lastContext(): FakeContext | undefined {
  return FakeContext.created.at(-1);
}

/** Lets every pending callback run. */
function flush(): Promise<void> {
  return new Promise((resolve) => setImmediate(resolve));
}

beforeEach(() => {
  log = [];
  engineAnswer = { type: "ready" };
  FakeContext.created.length = 0;
  FakeContext.resumesTo = "running";
  FakeWorkletNode.created.length = 0;
  vi.stubGlobal("AudioContext", FakeContext);
  vi.stubGlobal("AudioWorkletNode", FakeWorkletNode);
  vi.stubGlobal("GainNode", FakeGainNode);
  vi.stubGlobal("ConstantSourceNode", FakeConstantSource);
  stubFetch();
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("EnginePlayer", () => {
  test("plays the patch, fading in: engine to speakers through the gain, and to the tap", async () => {
    const player = new EnginePlayer(options());
    expect(player.state()).toStrictEqual({ kind: "stopped" });
    expect(player.latest()).toBeUndefined();
    await player.start();
    expect(log).toStrictEqual([
      "new context",
      "fetch engine.wasm",
      "add module processors.js",
      "connect poyetik-engine -> poyetik-tap",
      "connect poyetik-engine -> gain 0",
      "connect gain 0 -> destination",
      "resume",
      "gain set 0 at 1",
      "gain ramp to 0.25 at 1.02",
    ]);
    expect(workletNode("poyetik-engine")?.options).toMatchObject({ outputChannelCount: [2], processorOptions: { patch: SINE_440 } });
    expect(workletNode("poyetik-tap")?.options).toMatchObject({ channelCount: 1 });
    expect(player.state()).toStrictEqual({ kind: "playing" });
    expect(player.latest()).toStrictEqual({ samples: new Float32Array(), rate: 48_000 });
  });

  test("tells its listeners about every change of state, until they unsubscribe", async () => {
    const player = new EnginePlayer(options());
    const seen: string[] = [];
    const unsubscribe = player.subscribe(() => seen.push(player.state().kind));
    await player.start();
    await player.stop();
    unsubscribe();
    await player.start();
    expect(seen).toStrictEqual(["starting", "playing", "stopping", "stopped"]);
    expect(player.state()).toBe(player.state());
  });

  test("keeps the latest samples of the first channel", async () => {
    const player = new EnginePlayer(options());
    await player.start();
    const tap = workletNode("poyetik-tap")?.port;
    tap?.deliver([new Float32Array([1, 2, 3]), new Float32Array([9, 9, 9])]);
    tap?.deliver([new Float32Array([4, 5])]);
    expect(player.latest()?.samples).toStrictEqual(new Float32Array([2, 3, 4, 5]));
    tap?.deliver([]);
    expect(player.latest()?.samples).toStrictEqual(new Float32Array([2, 3, 4, 5]));
  });

  test("fails, closing the context, on a malformed block from the tap", async () => {
    const player = new EnginePlayer(options());
    await player.start();
    log = [];
    workletNode("poyetik-tap")?.port.deliver("garbage");
    await flush();
    expect(player.state()).toStrictEqual({ kind: "failed", reason: "OptionsError: invalid processor options: tap block is not an array" });
    expect(log).toStrictEqual(["post stop", "disconnect poyetik-engine -> poyetik-tap", "close"]);
    expect(player.latest()).toBeUndefined();
  });

  test("fails when the engine processor fails while playing, once", async () => {
    const player = new EnginePlayer(options());
    await player.start();
    const engine = workletNode("poyetik-engine");
    engine?.port.deliver({ type: "failed", message: "EngineTrap: trapped" });
    engine?.onprocessorerror?.();
    await flush();
    expect(player.state()).toStrictEqual({ kind: "failed", reason: "Error: the engine failed: EngineTrap: trapped" });
    expect(log.filter((entry) => entry === "close")).toHaveLength(1);
  });

  test("fails when the engine processor cannot load the patch", async () => {
    // Built in this realm, as the structured clone of a real port would deliver it.
    engineAnswer = { type: "rejected", detail: Uint8Array.from(new TextEncoder().encode("bad patch")) };
    const player = new EnginePlayer(options());
    await player.start();
    expect(player.state()).toStrictEqual({ kind: "failed", reason: "Error: bad patch" });
    expect(log.at(-1)).toBe("close");
  });

  test("starts once, even when asked twice at once", async () => {
    const player = new EnginePlayer(options());
    const first = player.start();
    const second = player.start();
    expect(second).toBe(first);
    await Promise.all([first, second]);
    await player.start();
    expect(FakeContext.created).toHaveLength(1);
  });

  test("stops during a start: waits for it, then stops", async () => {
    const player = new EnginePlayer(options());
    const starting = player.start();
    const stopping = player.stop();
    await Promise.all([starting, stopping]);
    expect(player.state()).toStrictEqual({ kind: "stopped" });
    expect(lastContext()?.state).toBe("closed");
  });

  test("stops by fading out on the audio clock, then closing; once", async () => {
    const player = new EnginePlayer(options());
    await player.start();
    log = [];
    const first = player.stop();
    expect(player.stop()).toBe(first);
    expect(player.state()).toStrictEqual({ kind: "stopping" });
    await first;
    expect(log).toStrictEqual([
      "gain cancel from 1",
      "gain set 0.25 at 1",
      "gain ramp to 0 at 1.02",
      "connect clock offset 0 -> destination",
      "clock start",
      "clock stop at 1.02",
      "post stop",
      "disconnect poyetik-engine -> poyetik-tap",
      "close",
    ]);
    expect(player.state()).toStrictEqual({ kind: "stopped" });
    expect(player.latest()).toBeUndefined();
    log = [];
    await player.stop();
    expect(log).toStrictEqual([]);
  });

  test("fails, without closing twice, when the browser closes the context during the fade out", async () => {
    const player = new EnginePlayer(options());
    await player.start();
    log = [];
    const stopping = player.stop();
    lastContext()?.become("closed");
    await stopping;
    expect(player.state()).toStrictEqual({ kind: "failed", reason: "Error: the browser closed the audio device" });
    expect(log.filter((entry) => entry === "close")).toStrictEqual([]);
  });

  test("follows the browser: suspended, resumed, closed", async () => {
    const player = new EnginePlayer(options());
    await player.start();
    const context = lastContext();
    context?.become("suspended");
    expect(player.state()).toStrictEqual({ kind: "suspended" });
    expect(player.latest()).toBeUndefined();
    context?.become("suspended");
    expect(player.state()).toStrictEqual({ kind: "suspended" });
    await player.resume();
    expect(player.state()).toStrictEqual({ kind: "playing" });
    context?.become("running");
    expect(player.state()).toStrictEqual({ kind: "playing" });
    log = [];
    context?.become("closed");
    await flush();
    expect(player.state()).toStrictEqual({ kind: "failed", reason: "Error: the browser closed the audio device" });
    expect(log).toStrictEqual(["post stop", "disconnect poyetik-engine -> poyetik-tap"]);
  });

  test("ignores the browser while starting: the start decides", async () => {
    engineAnswer = undefined;
    const player = new EnginePlayer(options());
    const starting = player.start();
    await vi.waitFor(() => {
      expect(workletNode("poyetik-engine")).toBeDefined();
    });
    lastContext()?.become("running");
    lastContext()?.become("suspended");
    expect(player.state()).toStrictEqual({ kind: "starting" });
    workletNode("poyetik-engine")?.port.deliver({ type: "ready" });
    await starting;
    expect(player.state()).toStrictEqual({ kind: "playing" });
  });

  test("ignores the context of an earlier session", async () => {
    const player = new EnginePlayer(options());
    await player.start();
    const old = lastContext();
    await player.stop();
    await player.start();
    const seen: string[] = [];
    player.subscribe(() => seen.push(player.state().kind));
    old?.become("suspended");
    old?.become("running");
    expect(player.state()).toStrictEqual({ kind: "playing" });
    expect(seen).toStrictEqual([]);
  });

  test("stops again after starting again", async () => {
    const player = new EnginePlayer(options());
    await player.start();
    await player.stop();
    await player.start();
    await player.stop();
    expect(player.state()).toStrictEqual({ kind: "stopped" });
    expect(FakeContext.created.map((context) => context.state)).toStrictEqual(["closed", "closed"]);
  });

  test("stays suspended when the browser refuses to resume, and closes at once when stopped", async () => {
    FakeContext.resumesTo = "suspended";
    const player = new EnginePlayer(options());
    await player.start();
    expect(player.state()).toStrictEqual({ kind: "suspended" });
    await player.resume();
    expect(player.state()).toStrictEqual({ kind: "suspended" });
    log = [];
    await player.stop();
    expect(log).toStrictEqual(["post stop", "disconnect poyetik-engine -> poyetik-tap", "close"]);
    expect(player.state()).toStrictEqual({ kind: "stopped" });
  });

  test("resumes only when suspended", async () => {
    const player = new EnginePlayer(options());
    await player.resume();
    await player.start();
    log = [];
    await player.resume();
    expect(log).toStrictEqual([]);
  });

  test("ignores what a closed session reports", async () => {
    const player = new EnginePlayer(options());
    await player.start();
    const context = lastContext();
    const engine = workletNode("poyetik-engine");
    await player.stop();
    context?.become("closed");
    engine?.port.deliver({ type: "failed", message: "late" });
    await flush();
    expect(player.state()).toStrictEqual({ kind: "stopped" });
  });

  test("fetches the engine once, and starts again in a new context", async () => {
    const player = new EnginePlayer(options());
    await player.start();
    await player.stop();
    log = [];
    await player.start();
    expect(log.filter((entry) => entry.startsWith("fetch"))).toStrictEqual([]);
    expect(FakeContext.created).toHaveLength(2);
  });

  test("fails, closing the context, when the engine cannot be fetched; and can start again", async () => {
    stubFetch(404);
    const player = new EnginePlayer(options());
    await player.start();
    expect(player.state()).toStrictEqual({ kind: "failed", reason: "Error: cannot load the engine from engine.wasm (HTTP 404)" });
    expect(log).toStrictEqual(["new context", "fetch engine.wasm", "close"]);
    expect(player.latest()).toBeUndefined();
    stubFetch();
    await player.start();
    expect(player.state()).toStrictEqual({ kind: "playing" });
  });

  test("acknowledges a failure when stopped", async () => {
    stubFetch(404);
    const player = new EnginePlayer(options());
    await player.start();
    await player.stop();
    expect(player.state()).toStrictEqual({ kind: "stopped" });
  });

  test("fails when the patch is rejected", async () => {
    const player = new EnginePlayer(options(new TextEncoder().encode("{}")));
    await player.start();
    expect(player.state().kind).toBe("failed");
    expect(log.at(-1)).toBe("close");
  });

  test("does not leave the start state when a failure came first", async () => {
    // The engine answers nothing: the tap fails before the start completes.
    engineAnswer = undefined;
    const player = new EnginePlayer(options());
    const starting = player.start();
    await vi.waitFor(() => {
      expect(workletNode("poyetik-tap")).toBeDefined();
    });
    workletNode("poyetik-tap")?.port.deliver("garbage");
    workletNode("poyetik-engine")?.port.deliver({ type: "ready" });
    await starting;
    expect(player.state().kind).toBe("failed");
  });

  test("explains what the browser lacks instead of starting", async () => {
    const player = new EnginePlayer(options(SINE_440, { ...CAPABLE, isSecureContext: false }));
    await player.start();
    expect(player.state()).toStrictEqual({ kind: "failed", reason: explain("secure-context") });
    expect(FakeContext.created).toHaveLength(0);
  });
});

describe("missing", () => {
  test.each([
    [CAPABLE, undefined],
    [{ ...CAPABLE, isSecureContext: false }, "secure-context"],
    [{ isSecureContext: true }, "web-audio"],
    [{ ...CAPABLE, AudioWorkletNode: undefined }, "audio-worklet"],
    [{ ...CAPABLE, WebAssembly: undefined }, "webassembly"],
  ] as const)("finds what %o lacks: %s", (environment, expected) => {
    expect(missing(environment)).toBe(expected);
  });

  test("explains each lack, differently", () => {
    const all: Missing[] = ["secure-context", "web-audio", "audio-worklet", "webassembly"];
    const texts = all.map(explain);
    expect(new Set(texts).size).toBe(all.length);
    expect(texts[0]).toContain("HTTPS");
    expect(texts[1]).toContain("Web Audio");
    expect(texts[2]).toContain("AudioWorklet");
    expect(texts[3]).toContain("WebAssembly");
  });
});
