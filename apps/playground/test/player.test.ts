import { readFileSync } from "node:fs";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { SINE_440 } from "../src/patch.ts";
import { EnginePlayer, type PlayerOptions } from "../src/player.ts";

/** The built engine module. */
const WASM = readFileSync(new URL(import.meta.resolve("@poyetik/engine-wasm/engine.wasm")));

/** Everything the fake Web Audio world did, in order. */
let log: string[] = [];

/** A fake message port. */
class FakePort {
  onmessage: ((event: { data: unknown }) => void) | null = null;

  postMessage(message: unknown): void {
    log.push(`post ${String(message)}`);
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

/** Fake `AudioWorkletNode`: named after its processor. */
class FakeWorkletNode extends FakeNode {
  static readonly created: FakeWorkletNode[] = [];
  readonly options: unknown;

  constructor(_context: unknown, name: string, options: unknown) {
    super(name);
    this.options = options;
    FakeWorkletNode.created.push(this);
  }
}

/** Fake `GainNode`. */
class FakeGainNode extends FakeNode {
  constructor(_context: unknown, options: { gain: number }) {
    super(`gain ${String(options.gain)}`);
  }
}

/** Fake `AudioContext`, at 48 kHz. */
class FakeContext {
  static readonly created: FakeContext[] = [];
  readonly sampleRate = 48_000;
  readonly destination = {};
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
    return Promise.resolve();
  }

  close(): Promise<void> {
    log.push("close");
    return Promise.resolve();
  }
}

/** Options of a player, with errors collected. */
function options(errors: Error[] = [], patch = SINE_440): PlayerOptions {
  return { wasm: "engine.wasm", worklet: "processors.js", patch, gain: 0.25, history: 4, onError: (error) => errors.push(error) };
}

/** Fetch answering every request with the engine module, or with `status`. */
function stubFetch(status = 200): void {
  vi.stubGlobal("fetch", (url: string) => {
    log.push(`fetch ${url}`);
    return Promise.resolve({ ok: status === 200, status, arrayBuffer: () => Promise.resolve(WASM.buffer.slice(WASM.byteOffset, WASM.byteOffset + WASM.byteLength)) });
  });
}

/** The tap node of the last start. */
function tapNode(): FakeWorkletNode | undefined {
  return FakeWorkletNode.created.filter((node) => node.name === "poyetik-tap").pop();
}

beforeEach(() => {
  log = [];
  FakeContext.created.length = 0;
  FakeWorkletNode.created.length = 0;
  vi.stubGlobal("AudioContext", FakeContext);
  vi.stubGlobal("AudioWorkletNode", FakeWorkletNode);
  vi.stubGlobal("GainNode", FakeGainNode);
  stubFetch();
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("EnginePlayer", () => {
  test("plays the patch: engine to speakers through the gain, and to the tap", async () => {
    const player = new EnginePlayer(options());
    expect(player.latest()).toBeUndefined();
    await player.start();
    expect(log).toStrictEqual([
      "new context",
      "fetch engine.wasm",
      "add module processors.js",
      "connect poyetik-engine -> poyetik-tap",
      "connect poyetik-engine -> gain 0.25",
      "connect gain 0.25 -> destination",
      "resume",
    ]);
    expect(FakeWorkletNode.created[0]?.options).toMatchObject({ outputChannelCount: [2], processorOptions: { patch: SINE_440 } });
    expect(tapNode()?.options).toMatchObject({ channelCount: 1 });
    expect(player.latest()).toStrictEqual({ samples: new Float32Array(), rate: 48_000 });
  });

  test("keeps the latest samples of the first channel", async () => {
    const player = new EnginePlayer(options());
    await player.start();
    tapNode()?.port.onmessage?.({ data: [new Float32Array([1, 2, 3]), new Float32Array([9, 9, 9])] });
    tapNode()?.port.onmessage?.({ data: [new Float32Array([4, 5])] });
    expect(player.latest()?.samples).toStrictEqual(new Float32Array([2, 3, 4, 5]));
    tapNode()?.port.onmessage?.({ data: [] });
    expect(player.latest()?.samples).toStrictEqual(new Float32Array([2, 3, 4, 5]));
  });

  test("reports errors happening while playing", async () => {
    const errors: Error[] = [];
    const player = new EnginePlayer(options(errors));
    await player.start();
    tapNode()?.port.onmessage?.({ data: "garbage" });
    expect(errors.map(String)).toStrictEqual(["OptionsError: invalid processor options: tap block is not an array"]);
  });

  test("starts once", async () => {
    const player = new EnginePlayer(options());
    await player.start();
    await player.start();
    expect(FakeContext.created).toHaveLength(1);
  });

  test("stops: the tap, then the context; and only once", async () => {
    const player = new EnginePlayer(options());
    await player.start();
    log = [];
    await player.stop();
    expect(log).toStrictEqual(["post stop", "disconnect poyetik-engine -> poyetik-tap", "close"]);
    expect(player.latest()).toBeUndefined();
    log = [];
    await player.stop();
    expect(log).toStrictEqual([]);
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

  test("closes the context and explains when the engine cannot be fetched", async () => {
    stubFetch(404);
    const player = new EnginePlayer(options());
    await expect(player.start()).rejects.toThrow(new Error("cannot load the engine: HTTP 404"));
    expect(log).toStrictEqual(["new context", "fetch engine.wasm", "close"]);
    expect(player.latest()).toBeUndefined();
    stubFetch();
    await player.start();
    expect(log.filter((entry) => entry.startsWith("fetch"))).toHaveLength(2);
  });

  test("closes the context when the patch is rejected", async () => {
    const player = new EnginePlayer(options([], new TextEncoder().encode("{}")));
    await expect(player.start()).rejects.toThrow();
    expect(log.at(-1)).toBe("close");
    expect(player.latest()).toBeUndefined();
  });
});
