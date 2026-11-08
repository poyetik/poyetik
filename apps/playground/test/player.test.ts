import { loadPatch } from "@poyetik/engine-wasm/web";
import { readFileSync } from "node:fs";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { SINE_440 } from "../src/patch.ts";
import { ENGINE_RATES, EnginePlayer, explain, missing, type Environment, type Failure, type Missing, type PlayerOptions } from "../src/player.ts";
import { SAMPLE_RATES } from "./matrix.ts";

/** The built engine module. */
const WASM = readFileSync(new URL(import.meta.resolve("@poyetik/engine-wasm/engine.wasm")));

/** Everything the fake Web Audio world did, in order. */
let log: string[] = [];

/** What the next engine processor answers: its first message, or nothing at all. */
let engineAnswer: unknown = { type: "ready" };

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

/** Fake `AudioWorkletNode`: named after its processor; the engine one answers `engineAnswer`. */
class FakeWorkletNode extends FakeNode {
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
  /** The current value of the parameter, updated at the end of a ramp. */
  value: number;
  /** The node the parameter belongs to, prefixing its log lines. */
  readonly owner: string;

  constructor(owner: string, value: number) {
    this.owner = owner;
    this.value = value;
  }

  /** Logs `value` scheduled at `time`, in seconds of the context clock. */
  setValueAtTime(value: number, time: number): void {
    log.push(`${this.owner} set ${String(value)} at ${String(time)}`);
  }

  /** Logs a ramp to `value` ending at `time`, in seconds, and takes that value at once. */
  linearRampToValueAtTime(value: number, time: number): void {
    log.push(`${this.owner} ramp to ${String(value)} at ${String(time)}`);
    this.value = value;
  }

  /** Logs the cancellation of the values scheduled from `time`, in seconds. */
  cancelScheduledValues(time: number): void {
    log.push(`${this.owner} cancel from ${String(time)}`);
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
class FakeContext {
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
  /** The handler the player sets for state changes, called by {@link FakeContext.become}. */
  onstatechange: (() => void) | null = null;
  /** The worklet of the context: logs the modules added, and always loads them. */
  readonly audioWorklet = {
    addModule: (url: string) => {
      log.push(`add module ${url}`);
      return Promise.resolve();
    },
  };

  /** Creates the context, at the rate asked for or the device rate, unless the fake browser refuses to (no audio device). */
  constructor(options?: { sampleRate?: number }) {
    if (FakeContext.refuses.has("create")) {
      throw new DOMException("no device", "NotSupportedError");
    }
    this.sampleRate = options?.sampleRate ?? FakeContext.rate;
    FakeContext.created.push(this);
    log.push("new context");
  }

  /** What the fake browser refuses, to check that the player never rejects nor throws. */
  static readonly refuses = new Set<"create" | "resume" | "close" | "fade">();
  /** When set, `resume` answers only once the test resolves it. */
  static resuming: PromiseWithResolvers<undefined> | undefined;

  /** Logs the resume and moves to {@link FakeContext.resumesTo}; answers at once, or when {@link FakeContext.resuming} is resolved. */
  resume(): Promise<void> {
    if (FakeContext.refuses.has("resume")) {
      return Promise.reject(new DOMException("closing", "InvalidStateError"));
    }
    log.push("resume");
    this.state = FakeContext.resumesTo;
    return FakeContext.resuming?.promise ?? Promise.resolve();
  }

  /** Rejects on a closed context, like browsers, or when the fake browser refuses. */
  close(): Promise<void> {
    if (this.state === "closed" || FakeContext.refuses.has("close")) {
      return Promise.reject(new DOMException("already closed", "InvalidStateError"));
    }
    log.push("close");
    this.state = "closed";
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
    this.onstatechange?.();
  }
}

/** Whether `text` names a lack or a failure the player explains. */
function isProblem(text: string): text is Missing | Failure {
  return ["secure-context", "web-audio", "audio-worklet", "webassembly", "load", "engine", "device"].includes(text);
}

/** A browser with everything the player needs. */
const CAPABLE: Environment = { isSecureContext: true, AudioContext: FakeContext, AudioWorkletNode: FakeWorkletNode, WebAssembly };

/** Options of a player; `changes` replace some of them. */
function playerOptions(patch = SINE_440, environment = CAPABLE, changes: Partial<PlayerOptions> = {}): PlayerOptions {
  return { wasm: "engine.wasm", worklet: "processors.js", patch, gain: 0.25, fade: 0.02, historySeconds: 4 / 48_000, environment, ...changes };
}

/** The signals of the requests made, in order: a session aborts its downloads when it ends. */
let signals: AbortSignal[] = [];

/** Fetch answering every request with the engine module, or with `status`, or never (`"never"`). */
function stubFetch(status: number | "never" = 200): void {
  vi.stubGlobal("fetch", (url: string, init: { signal: AbortSignal }) => {
    log.push(`fetch ${url}`);
    signals.push(init.signal);
    if (status === "never") {
      return new Promise(() => undefined);
    }
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

/** Stops `player` while it plays, letting the audio clock of its context run through the fade out. */
async function stopPlaying(player: EnginePlayer): Promise<void> {
  const stopping = player.stop();
  lastContext()?.advance(1);
  await stopping;
}

/** Lets every pending callback run. */
function flush(): Promise<void> {
  return new Promise((resolve) => setImmediate(resolve));
}

beforeEach(() => {
  log = [];
  signals = [];
  engineAnswer = { type: "ready" };
  FakeContext.created.length = 0;
  FakeContext.resumesTo = "running";
  FakeContext.resuming = undefined;
  FakeContext.refuses.clear();
  FakeContext.rate = 48_000;
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
    const player = new EnginePlayer(playerOptions());
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
      "gain cancel from 1",
      "gain set 0 at 1",
      "gain ramp to 0.25 at 1.02",
    ]);
    expect(workletNode("poyetik-engine")?.options).toMatchObject({ outputChannelCount: [2], processorOptions: { patch: SINE_440 } });
    expect(workletNode("poyetik-tap")?.options).toMatchObject({ channelCount: 1 });
    expect(player.state()).toStrictEqual({ kind: "playing" });
    expect(player.latest()).toStrictEqual({ samples: new Float32Array(), rate: 48_000 });
  });

  test("tells its listeners about every change of state, until they unsubscribe", async () => {
    const player = new EnginePlayer(playerOptions());
    const seen: string[] = [];
    const unsubscribe = player.subscribe(() => seen.push(player.state().kind));
    await player.start();
    await stopPlaying(player);
    unsubscribe();
    await player.start();
    expect(seen).toStrictEqual(["starting", "playing", "stopping", "stopped"]);
    expect(player.state()).toBe(player.state());
  });

  test("keeps the latest samples of the first channel", async () => {
    const player = new EnginePlayer(playerOptions());
    await player.start();
    const tap = workletNode("poyetik-tap")?.port;
    tap?.deliver([new Float32Array([1, 2, 3]), new Float32Array([9, 9, 9])]);
    tap?.deliver([new Float32Array([4, 5])]);
    expect(player.latest()?.samples).toStrictEqual(new Float32Array([2, 3, 4, 5]));
    tap?.deliver([]);
    expect(player.latest()?.samples).toStrictEqual(new Float32Array([2, 3, 4, 5]));
  });

  test("fails, closing the context, on a malformed block from the tap", async () => {
    const player = new EnginePlayer(playerOptions());
    await player.start();
    log = [];
    workletNode("poyetik-tap")?.port.deliver("garbage");
    await flush();
    expect(player.state()).toStrictEqual({ kind: "failed", cause: "engine", detail: "MessageError: invalid message from the processor: tap block is not an array" });
    expect(log).toStrictEqual(["post stop", "disconnect poyetik-engine -> poyetik-tap", "close"]);
    expect(player.latest()).toBeUndefined();
  });

  test("fails when the engine processor fails while playing, once", async () => {
    const player = new EnginePlayer(playerOptions());
    await player.start();
    const engine = workletNode("poyetik-engine");
    engine?.port.deliver({ type: "failed", message: "EngineTrap: trapped" });
    engine?.onprocessorerror?.();
    await flush();
    expect(player.state()).toStrictEqual({ kind: "failed", cause: "engine", detail: "Error: the engine failed: EngineTrap: trapped" });
    expect(log.filter((entry) => entry === "close")).toHaveLength(1);
  });

  test("fails when the engine processor cannot load the patch", async () => {
    // Built in this realm, as the structured clone of a real port would deliver it.
    engineAnswer = { type: "rejected", detail: Uint8Array.from(new TextEncoder().encode("bad patch")) };
    const player = new EnginePlayer(playerOptions());
    await player.start();
    expect(player.state()).toStrictEqual({ kind: "failed", cause: "engine", detail: "Error: bad patch" });
    expect(log.at(-1)).toBe("close");
  });

  test("starts once, even when asked twice at once", async () => {
    const player = new EnginePlayer(playerOptions());
    const first = player.start();
    const second = player.start();
    expect(second).toBe(first);
    await Promise.all([first, second]);
    await player.start();
    expect(FakeContext.created).toHaveLength(1);
  });

  test("cancels a start at once: closes its context, and settles the start", async () => {
    const player = new EnginePlayer(playerOptions());
    const starting = player.start();
    const stopping = player.stop();
    expect(player.state()).toStrictEqual({ kind: "stopped" });
    await Promise.all([starting, stopping]);
    expect(player.state()).toStrictEqual({ kind: "stopped" });
    expect(lastContext()?.state).toBe("closed");
    expect(signals.map((signal) => signal.aborted)).toStrictEqual([true]);
  });

  test("cancels a download that never answers, then downloads anew and plays", async () => {
    stubFetch("never");
    const player = new EnginePlayer(playerOptions());
    const starting = player.start();
    await flush();
    expect(player.state()).toStrictEqual({ kind: "starting" });
    await player.stop();
    await starting;
    expect(player.state()).toStrictEqual({ kind: "stopped" });
    expect(signals.map((signal) => signal.aborted)).toStrictEqual([true]);
    stubFetch();
    await player.start();
    expect(player.state()).toStrictEqual({ kind: "playing" });
    expect(log.filter((entry) => entry.startsWith("fetch"))).toStrictEqual(["fetch engine.wasm", "fetch engine.wasm"]);
    expect(FakeContext.created.map((context) => context.state)).toStrictEqual(["closed", "running"]);
  });

  test("cancels a start whose engine never answers", async () => {
    engineAnswer = undefined;
    const player = new EnginePlayer(playerOptions());
    const starting = player.start();
    await vi.waitFor(() => {
      expect(workletNode("poyetik-engine")).toBeDefined();
    });
    await player.stop();
    await starting;
    expect(player.state()).toStrictEqual({ kind: "stopped" });
    expect(log.slice(-3)).toStrictEqual(["post stop", "disconnect poyetik-engine -> poyetik-tap", "close"]);
  });

  test("cancelled right after its last wait, never plays", async () => {
    FakeContext.resuming = Promise.withResolvers();
    const player = new EnginePlayer(playerOptions());
    const starting = player.start();
    await vi.waitFor(() => {
      expect(log).toContain("resume");
    });
    // The resume answers, and a cancel comes before the start sees the answer.
    FakeContext.resuming.resolve(undefined);
    queueMicrotask(() => {
      void player.stop();
    });
    await starting;
    expect(player.state()).toStrictEqual({ kind: "stopped" });
    expect(lastContext()?.state).toBe("closed");
  });

  test("stops by fading out on the audio clock, then closing; once", async () => {
    const player = new EnginePlayer(playerOptions());
    await player.start();
    log = [];
    const first = player.stop();
    expect(player.stop()).toBe(first);
    expect(player.state()).toStrictEqual({ kind: "stopping" });
    // The context closes only once its clock reaches the end of the ramp.
    lastContext()?.advance(0.01);
    await flush();
    expect(player.state()).toStrictEqual({ kind: "stopping" });
    lastContext()?.advance(0.01);
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

  test.each(["suspended", "interrupted"] as const)("stops at once when the browser makes the context %s during the fade out", async (state) => {
    const player = new EnginePlayer(playerOptions());
    await player.start();
    const stopping = player.stop();
    lastContext()?.become(state);
    await stopping;
    expect(player.state()).toStrictEqual({ kind: "stopped" });
    expect(lastContext()?.state).toBe("closed");
  });

  test("fails during the fade out, settles the stop, then starts and stops a new session", async () => {
    const player = new EnginePlayer(playerOptions());
    await player.start();
    const stopping = player.stop();
    workletNode("poyetik-engine")?.onprocessorerror?.();
    await stopping;
    expect(player.state()).toStrictEqual({ kind: "failed", cause: "engine", detail: "Error: the engine processor failed" });
    await player.start();
    expect(player.state()).toStrictEqual({ kind: "playing" });
    const again = player.stop();
    expect(again).not.toBe(stopping);
    lastContext()?.advance(1);
    await again;
    expect(player.state()).toStrictEqual({ kind: "stopped" });
    expect(FakeContext.created.map((context) => context.state)).toStrictEqual(["closed", "closed"]);
  });

  test("fails, without closing twice, when the browser closes the context during the fade out", async () => {
    const player = new EnginePlayer(playerOptions());
    await player.start();
    log = [];
    const stopping = player.stop();
    lastContext()?.become("closed");
    await stopping;
    expect(player.state()).toStrictEqual({ kind: "failed", cause: "device", detail: "Error: the browser closed the audio device" });
    expect(log.filter((entry) => entry === "close")).toStrictEqual([]);
  });

  test("follows the browser: suspended, resumed, closed", async () => {
    const player = new EnginePlayer(playerOptions());
    await player.start();
    const context = lastContext();
    context?.become("suspended");
    expect(player.state()).toStrictEqual({ kind: "suspended", refused: false });
    expect(player.latest()).toBeUndefined();
    context?.become("suspended");
    expect(player.state()).toStrictEqual({ kind: "suspended", refused: false });
    await player.resume();
    expect(player.state()).toStrictEqual({ kind: "playing" });
    context?.become("running");
    expect(player.state()).toStrictEqual({ kind: "playing" });
    log = [];
    context?.become("closed");
    await flush();
    expect(player.state()).toStrictEqual({ kind: "failed", cause: "device", detail: "Error: the browser closed the audio device" });
    expect(log).toStrictEqual(["post stop", "disconnect poyetik-engine -> poyetik-tap"]);
  });

  test("ignores the browser while starting: the start decides", async () => {
    engineAnswer = undefined;
    const player = new EnginePlayer(playerOptions());
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
    const player = new EnginePlayer(playerOptions());
    await player.start();
    const old = lastContext();
    await stopPlaying(player);
    await player.start();
    const seen: string[] = [];
    player.subscribe(() => seen.push(player.state().kind));
    old?.become("suspended");
    old?.become("running");
    expect(player.state()).toStrictEqual({ kind: "playing" });
    expect(seen).toStrictEqual([]);
  });

  test("stops again after starting again", async () => {
    const player = new EnginePlayer(playerOptions());
    await player.start();
    await stopPlaying(player);
    await player.start();
    await stopPlaying(player);
    expect(player.state()).toStrictEqual({ kind: "stopped" });
    expect(FakeContext.created.map((context) => context.state)).toStrictEqual(["closed", "closed"]);
  });

  test("stays suspended when the browser refuses to resume, says so, and closes at once when stopped", async () => {
    FakeContext.resumesTo = "suspended";
    const player = new EnginePlayer(playerOptions());
    await player.start();
    expect(player.state()).toStrictEqual({ kind: "suspended", refused: false });
    await player.resume();
    expect(player.state()).toStrictEqual({ kind: "suspended", refused: true });
    expect(player.state()).toBe(player.state());
    log = [];
    await player.stop();
    expect(log).toStrictEqual(["post stop", "disconnect poyetik-engine -> poyetik-tap", "close"]);
    expect(player.state()).toStrictEqual({ kind: "stopped" });
  });

  test("forgets a refused resume once the browser resumes", async () => {
    FakeContext.resumesTo = "suspended";
    const player = new EnginePlayer(playerOptions());
    await player.start();
    await player.resume();
    lastContext()?.become("running");
    expect(player.state()).toStrictEqual({ kind: "playing" });
    lastContext()?.become("suspended");
    expect(player.state()).toStrictEqual({ kind: "suspended", refused: false });
  });

  test.each(["user", "browser"] as const)("fades in again after a suspension, resumed by the %s", async (by) => {
    // Neither the default gain nor the default fade: values the code cannot hard-code.
    const player = new EnginePlayer(playerOptions(SINE_440, CAPABLE, { gain: 0.5, fade: 0.005 }));
    await player.start();
    const context = lastContext();
    context?.advance(0.5);
    log = [];
    // The ramp waits on the frozen clock: it plays from where the sound stopped, at the resume.
    context?.become("suspended");
    expect(log).toStrictEqual(["gain cancel from 1.5", "gain set 0 at 1.5", "gain ramp to 0.5 at 1.505"]);
    log = [];
    if (by === "user") {
      await player.resume();
    } else {
      context?.become("running");
    }
    expect(player.state()).toStrictEqual({ kind: "playing" });
    expect(log.filter((entry) => entry.startsWith("gain"))).toStrictEqual([]);
  });

  test("schedules no fade in when suspended while stopping", async () => {
    const player = new EnginePlayer(playerOptions());
    await player.start();
    const stopping = player.stop();
    log = [];
    lastContext()?.become("suspended");
    await stopping;
    expect(log.filter((entry) => entry.startsWith("gain"))).toStrictEqual([]);
  });

  test("resumes only when suspended", async () => {
    const player = new EnginePlayer(playerOptions());
    await player.resume();
    await player.start();
    log = [];
    await player.resume();
    expect(log).toStrictEqual([]);
  });

  test("ignores what a closed session reports", async () => {
    const player = new EnginePlayer(playerOptions());
    await player.start();
    const context = lastContext();
    const engine = workletNode("poyetik-engine");
    await stopPlaying(player);
    context?.become("closed");
    engine?.port.deliver({ type: "failed", message: "late" });
    await flush();
    expect(player.state()).toStrictEqual({ kind: "stopped" });
  });

  test("fetches the engine once, and starts again in a new context", async () => {
    const player = new EnginePlayer(playerOptions());
    await player.start();
    await stopPlaying(player);
    log = [];
    await player.start();
    expect(log.filter((entry) => entry.startsWith("fetch"))).toStrictEqual([]);
    expect(FakeContext.created).toHaveLength(2);
  });

  test("fails, closing the context, when the engine cannot be fetched; and can start again", async () => {
    stubFetch(404);
    const player = new EnginePlayer(playerOptions());
    await player.start();
    expect(player.state()).toStrictEqual({ kind: "failed", cause: "load", detail: "Error: cannot load the engine from engine.wasm (HTTP 404)" });
    expect(log).toStrictEqual(["new context", "fetch engine.wasm", "close"]);
    expect(player.latest()).toBeUndefined();
    stubFetch();
    await player.start();
    expect(player.state()).toStrictEqual({ kind: "playing" });
  });

  test("acknowledges a failure when stopped", async () => {
    stubFetch(404);
    const player = new EnginePlayer(playerOptions());
    await player.start();
    await player.stop();
    expect(player.state()).toStrictEqual({ kind: "stopped" });
  });

  test("fails when the patch is rejected", async () => {
    const player = new EnginePlayer(playerOptions(new TextEncoder().encode("{}")));
    await player.start();
    expect(player.state()).toStrictEqual({ kind: "failed", cause: "engine", detail: "Error: invalid patch: missing field `format` at line 1 column 2" });
    expect(log.at(-1)).toBe("close");
  });

  test("does not leave the start state when a failure came first", async () => {
    // The engine answers nothing: the tap fails before the start completes.
    engineAnswer = undefined;
    const player = new EnginePlayer(playerOptions());
    const starting = player.start();
    await vi.waitFor(() => {
      expect(workletNode("poyetik-tap")).toBeDefined();
    });
    workletNode("poyetik-tap")?.port.deliver("garbage");
    workletNode("poyetik-engine")?.port.deliver({ type: "ready" });
    await starting;
    expect(player.state()).toStrictEqual({ kind: "failed", cause: "engine", detail: "MessageError: invalid message from the processor: tap block is not an array" });
  });

  test("never throws nor rejects when no context can be created: fails instead", async () => {
    FakeContext.refuses.add("create");
    const player = new EnginePlayer(playerOptions());
    await player.start();
    expect(player.state()).toStrictEqual({ kind: "failed", cause: "device", detail: "NotSupportedError: no device" });
    expect(log).toStrictEqual([]);
  });

  test("never rejects when the browser refuses to resume: fails, closing the context once", async () => {
    FakeContext.resumesTo = "suspended";
    const player = new EnginePlayer(playerOptions());
    await player.start();
    FakeContext.refuses.add("resume");
    log = [];
    await player.resume();
    expect(player.state()).toStrictEqual({ kind: "failed", cause: "device", detail: "InvalidStateError: closing" });
    expect(log).toStrictEqual(["post stop", "disconnect poyetik-engine -> poyetik-tap", "close"]);
  });

  test("never rejects when the context cannot be closed", async () => {
    FakeContext.refuses.add("close");
    stubFetch(404);
    const player = new EnginePlayer(playerOptions());
    await player.start();
    expect(player.state()).toStrictEqual({ kind: "failed", cause: "load", detail: "Error: cannot load the engine from engine.wasm (HTTP 404)" });
    stubFetch();
    await player.start();
    await stopPlaying(player);
    expect(player.state()).toStrictEqual({ kind: "stopped" });
  });

  test("never rejects when the fade out cannot be scheduled: fails, closing the context", async () => {
    const player = new EnginePlayer(playerOptions());
    await player.start();
    FakeContext.refuses.add("fade");
    await player.stop();
    expect(player.state()).toStrictEqual({ kind: "failed", cause: "device", detail: "InvalidStateError: cannot schedule" });
    expect(lastContext()?.state).toBe("closed");
  });

  test.each(["secure-context", "web-audio", "audio-worklet", "webassembly"] as const)("knows from the start that the browser lacks %s, and never starts", async (lacking) => {
    const environments: Record<Missing, Environment> = {
      "secure-context": { ...CAPABLE, isSecureContext: false },
      "web-audio": { isSecureContext: true },
      "audio-worklet": { ...CAPABLE, AudioWorkletNode: undefined },
      webassembly: { ...CAPABLE, WebAssembly: undefined },
    };
    const player = new EnginePlayer(playerOptions(SINE_440, environments[lacking]));
    const unsupported = player.state();
    expect(unsupported).toStrictEqual({ kind: "unsupported", lacking });
    await player.start();
    await player.stop();
    await player.resume();
    expect(player.state()).toBe(unsupported);
    expect(FakeContext.created).toHaveLength(0);
    expect(log).toStrictEqual([]);
  });

  test("fails on a download the network refuses, naming what it could not load", async () => {
    vi.stubGlobal("fetch", () => Promise.reject(new TypeError("Failed to fetch")));
    const player = new EnginePlayer(playerOptions());
    await player.start();
    expect(player.state()).toStrictEqual({ kind: "failed", cause: "load", detail: "Error: cannot load the engine from engine.wasm (TypeError: Failed to fetch)" });
  });

  test("fails when the processors cannot be loaded: a download", async () => {
    vi.stubGlobal("AudioContext", class extends FakeContext {
      /** A worklet whose module cannot be loaded. */
      override readonly audioWorklet = { addModule: () => Promise.reject(new Error("no module")) };
    });
    const player = new EnginePlayer(playerOptions());
    await player.start();
    expect(player.state()).toStrictEqual({ kind: "failed", cause: "load", detail: "Error: cannot load the processors from processors.js: Error: no module" });
  });

  test("fails when the audio device cannot start: the device", async () => {
    const player = new EnginePlayer(playerOptions());
    FakeContext.refuses.add("resume");
    await player.start();
    expect(player.state()).toStrictEqual({ kind: "failed", cause: "device", detail: "InvalidStateError: closing" });
  });
});

/** A seeded pseudo-random generator (mulberry32): the same sequence for the same seed, on every run. */
function random(seed: number): () => number {
  let state = seed;
  return () => {
    state = (state + 0x6d2b79f5) | 0;
    let mixed = Math.imul(state ^ (state >>> 15), 1 | state);
    mixed = (mixed + Math.imul(mixed ^ (mixed >>> 7), 61 | mixed)) ^ mixed;
    return ((mixed ^ (mixed >>> 14)) >>> 0) / 4_294_967_296;
  };
}

/**
 * How long a wait for the engine to compile may last: it compiles in real time, so a
 * loaded machine (the full check runs packages side by side) must slow it, not fail it.
 */
const COMPILING = { timeout: 10_000, interval: 1 };

/** What the user, the browser and the audio thread may do, in any order. */
const EVENTS = ["start", "stop", "resume", "suspend", "interrupt", "browser resume", "fail", "advance", "flush", "loaded"] as const;

/** Every kind of state the player may be in. */
const KINDS: readonly string[] = ["stopped", "starting", "playing", "suspended", "stopping", "failed"];

describe("EnginePlayer under any interleaving", () => {
  test.each(Array.from({ length: 100 }, (_unused, seed) => seed))("keeps its promises with seed %i", async (seed) => {
    const next = random(seed);
    const player = new EnginePlayer(playerOptions());
    const promises: { settled: boolean }[] = [];
    /** Records a public promise of the player, to check that it settles. */
    const watch = (promise: Promise<void>): void => {
      const watched = { settled: false };
      promises.push(watched);
      void promise.then(() => {
        watched.settled = true;
      });
    };
    /** The browser changes the state of the current context, unless it is closed. */
    const become = (state: AudioContextState | "interrupted"): void => {
      const context = lastContext();
      if (context !== undefined && context.state !== "closed") {
        context.become(state);
      }
    };
    /** A context is open exactly while its session is live: only the last one, and only while the player is not idle. */
    const check = (): void => {
      const { kind } = player.state();
      expect(KINDS).toContain(kind);
      const states = FakeContext.created.map((context) => context.state);
      expect(states.slice(0, -1).every((state) => state === "closed")).toBe(true);
      expect(states.length === 0 || states.at(-1) === "closed").toBe(kind === "stopped" || kind === "failed");
    };
    for (let step = 0; step < 20; step += 1) {
      const event = EVENTS[Math.floor(next() * EVENTS.length)];
      switch (event) {
        case "start":
          watch(player.start());
          break;
        case "stop":
          watch(player.stop());
          break;
        case "resume":
          watch(player.resume());
          break;
        case "suspend":
          become("suspended");
          break;
        case "interrupt":
          become("interrupted");
          break;
        case "browser resume":
          become("running");
          break;
        case "fail":
          workletNode("poyetik-engine")?.onprocessorerror?.();
          break;
        case "advance":
          lastContext()?.advance(0.01);
          break;
        case "loaded":
          // The engine compiles in real time: waits until a start in progress has ended.
          await vi.waitFor(() => {
            expect(player.state().kind).not.toBe("starting");
          }, COMPILING);
          break;
        case "flush":
        case undefined:
          await flush();
          break;
      }
      check();
    }
    // The world drains: the clock runs and every pending event is delivered (the engine
    // compiles in real time, hence the retries), until every promise has settled.
    await vi.waitFor(async () => {
      lastContext()?.advance(1);
      await flush();
      check();
      expect(promises.filter((promise) => !promise.settled)).toStrictEqual([]);
    }, COMPILING);
  }, 60_000);
});

describe("EnginePlayer at the rate of the device", () => {
  test.each([3_000, ENGINE_RATES.min - 1, ENGINE_RATES.max + 1, 768_000])("plays a device at %i Hz, outside the engine's rates, at 48 kHz: the browser resamples", async (rate) => {
    FakeContext.rate = rate;
    const player = new EnginePlayer(playerOptions());
    await player.start();
    expect(player.state()).toStrictEqual({ kind: "playing" });
    expect(player.latest()?.rate).toBe(48_000);
    expect(FakeContext.created.map((context) => [context.sampleRate, context.state])).toStrictEqual([
      [rate, "closed"],
      [48_000, "running"],
    ]);
  });

  test("plays at 48 kHz even when the device context cannot be closed", async () => {
    FakeContext.rate = 768_000;
    FakeContext.refuses.add("close");
    const player = new EnginePlayer(playerOptions());
    await player.start();
    expect(player.state()).toStrictEqual({ kind: "playing" });
    expect(player.latest()?.rate).toBe(48_000);
  });

  test("knows the rates of the engine: its limits are those of the test matrix, and the engine refuses beyond them", async () => {
    expect([ENGINE_RATES.min, ENGINE_RATES.max]).toStrictEqual([Math.min(...SAMPLE_RATES), Math.max(...SAMPLE_RATES)]);
    const wasm = (): ArrayBuffer => WASM.buffer.slice(WASM.byteOffset, WASM.byteOffset + WASM.byteLength);
    for (const rate of [ENGINE_RATES.min, ENGINE_RATES.max]) {
      expect((await loadPatch(wasm(), SINE_440, rate)).rate).toBe(rate);
    }
    for (const rate of [ENGINE_RATES.min - 1, ENGINE_RATES.max + 1]) {
      await expect(loadPatch(wasm(), SINE_440, rate)).rejects.toThrow(`invalid sample rate: ${String(rate)} Hz`);
    }
  });

  test.each(SAMPLE_RATES)("plays at %i Hz, the rate of its context, never a default", async (rate) => {
    FakeContext.rate = rate;
    const player = new EnginePlayer(playerOptions());
    await player.start();
    expect(player.state()).toStrictEqual({ kind: "playing" });
    expect(player.latest()?.rate).toBe(rate);
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

  test("explains every lack and every failure, differently, with what to do", () => {
    // A record: a new member of either union without its expected text does not compile.
    const expected: Record<Missing | Failure, string> = {
      "secure-context": "Audio needs a secure page: open this page over HTTPS (or from localhost).",
      "web-audio": "This browser cannot play audio from a page (no Web Audio). Try a recent Chrome, Firefox, Safari or Edge.",
      "audio-worklet": "This browser cannot run the engine in real time (no AudioWorklet). Try a recent Chrome, Firefox, Safari or Edge.",
      webassembly: "This browser cannot run the engine (no WebAssembly). Try a recent Chrome, Firefox, Safari or Edge.",
      load: "Could not load the audio engine. Check your connection, then press Start to try again.",
      engine: "The audio engine stopped on an error. Press Start to try again; if it happens again, reload the page.",
      device: "The browser could not play audio on this device. Check the sound output, then press Start to try again.",
    };
    for (const [problem, text] of Object.entries(expected)) {
      expect(isProblem(problem) ? explain(problem) : undefined).toBe(text);
    }
    expect(new Set(Object.values(expected)).size).toBe(Object.keys(expected).length);
  });
});
