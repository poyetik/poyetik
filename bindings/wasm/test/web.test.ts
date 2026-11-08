import { runInNewContext } from "node:vm";
import { afterEach, describe, expect, test, vi } from "vitest";
import { ModuleError, PatchError } from "../ts/engine.ts";
import { ENGINE_PROCESSOR, OptionsError, TAP_PROCESSOR } from "../ts/protocol.ts";
import { addProcessors, capture, captureEngine, createEngineNode, LoadedPatch, loadPatch, START_MARGIN, renderOffline, tap } from "../ts/web.ts";
import { SILENT_PATCH, SINE_PATCH, UNKNOWN_BOX, wasmBytes } from "./fixtures.ts";

/** The sample rate of the fake audio contexts, in hertz. */
const RATE = 48_000;

/** A fake `MessagePort`: records what is posted. */
class FakePort {
  /** Set by the code under test to receive messages from the processor; called by `deliver`. */
  onmessage: ((event: { data: unknown }) => void) | null = null;
  /** Every message the code under test posted to the processor, in order. */
  readonly posted: unknown[] = [];

  /** Records `message` instead of sending it to the processor. */
  postMessage(message: unknown): void {
    this.posted.push(message);
  }

  /** Delivers `data` as a message from the processor. */
  deliver(data: unknown): void {
    this.onmessage?.({ data });
  }
}

/** A fake `AudioWorkletNode`: records its construction. */
class FakeNode {
  /** Every node constructed since the last reset, in order, for the tests to drive and inspect. */
  static readonly created: FakeNode[] = [];
  /** The fake port of this node, through which the tests talk as the processor. */
  readonly port = new FakePort();
  /** The nodes this one is connected to, in order; `disconnect` removes them. */
  readonly connected: unknown[] = [];
  /** Set by the code under test to learn that the processor failed; the tests call it to simulate a failure. */
  onprocessorerror: (() => void) | null = null;

  /** The context this node was created in. */
  readonly context: unknown;
  /** The name of the processor this node runs. */
  readonly name: string;
  /** The `AudioWorkletNodeOptions` it was created with, for the tests to check what the processor would receive. */
  readonly options: unknown;

  constructor(context: unknown, name: string, options: unknown) {
    this.context = context;
    this.name = name;
    this.options = options;
    FakeNode.created.push(this);
  }

  /** Records a connection to `target`. */
  connect(target: unknown): void {
    this.connected.push(target);
  }

  /** Removes the connection to `target`. */
  disconnect(target: unknown): void {
    this.connected.splice(this.connected.indexOf(target), 1);
  }
}

/** A fake source node: records connections. */
function fakeSource(): { connect: (node: unknown) => void; disconnect: (node: unknown) => void; log: string[] } {
  const log: string[] = [];
  return {
    log,
    connect: (node) => log.push(node instanceof FakeNode ? `connect ${node.name}` : "connect ?"),
    disconnect: (node) => log.push(node instanceof FakeNode ? `disconnect ${node.name}` : "disconnect ?"),
  };
}

/** A fake context, with a fake worklet recording the modules added. */
function fakeContext(sampleRate: number): { sampleRate: number; audioWorklet: { addModule: (url: string | URL) => Promise<void> }; modules: (string | URL)[] } {
  const modules: (string | URL)[] = [];
  return {
    sampleRate,
    modules,
    audioWorklet: {
      addModule: (url) => {
        modules.push(url);
        return Promise.resolve();
      },
    },
  };
}

/** A patch loaded by the engine: the sine (2 channels) or a patch without output. */
function loaded(channels: 0 | 2, rate = RATE): Promise<LoadedPatch> {
  return loadPatch(wasmBytes(), channels === 2 ? SINE_PATCH : SILENT_PATCH, rate);
}

/**
 * What a promise has settled to once every pending callback has run: "resolved", the
 * rejection, or "pending" when it has not settled.
 */
async function settled(promise: Promise<unknown>): Promise<unknown> {
  let state: unknown = "pending";
  promise.then(
    () => {
      state = "resolved";
    },
    (error: unknown) => {
      state = error;
    },
  );
  await new Promise((resolve) => setImmediate(resolve));
  return state;
}

afterEach(() => {
  vi.unstubAllGlobals();
  FakeNode.created.length = 0;
});

/** Stubs the Web Audio node constructor. */
function stubNodes(): void {
  vi.stubGlobal("AudioWorkletNode", FakeNode);
}

/** Whether `value` is an object with every one of `members`. */
function hasMembers(value: unknown, members: readonly string[]): value is object {
  return typeof value === "object" && value !== null && members.every((member) => member in value);
}

/**
 * Lets a fake context stand in for a real one, which Node does not have: checks that it
 * has everything the code under test uses. The end-to-end tests run the same code with
 * real contexts in 4 browsers.
 */
function isContext(value: unknown): value is BaseAudioContext {
  return hasMembers(value, ["sampleRate", "audioWorklet"]);
}

/** Lets a fake node stand in for a real one: see {@link isContext}. */
function isNode(value: unknown): value is AudioNode {
  return hasMembers(value, ["connect", "disconnect"]);
}

/** The fake context, typed as the real one it stands in for. */
function context(fake: unknown): BaseAudioContext {
  if (!isContext(fake)) {
    throw new Error("not a context stand-in");
  }
  return fake;
}

/** The fake node, typed as the real one it stands in for. */
function node(fake: unknown): AudioNode {
  if (!isNode(fake)) {
    throw new Error("not a node stand-in");
  }
  return fake;
}

describe("addProcessors", () => {
  test("adds the processors module to the worklet", async () => {
    const fake = fakeContext(RATE);
    await addProcessors(context(fake), "processors.js");
    expect(fake.modules).toStrictEqual(["processors.js"]);
  });

  test("names the URL it could not load", async () => {
    const fake = fakeContext(RATE);
    const missing = new Error("404");
    fake.audioWorklet.addModule = () => Promise.reject(missing);
    const error: unknown = await addProcessors(context(fake), "nowhere.js").catch((reason: unknown) => reason);
    expect(error).toStrictEqual(new Error("cannot load the processors from nowhere.js: Error: 404"));
    expect(error).toHaveProperty("cause", missing);
  });
});

describe("loadPatch", () => {
  test("checks the patch and reports its channels and rate", async () => {
    const patch = await loadPatch(wasmBytes(), SINE_PATCH, RATE);
    expect(patch).toBeInstanceOf(LoadedPatch);
    expect(patch).toMatchObject({ rate: RATE, channels: 2 });
    expect((await loaded(0, 44_100)).channels).toBe(0);
  });

  test("keeps its own copies of the bytes", async () => {
    const wasm = wasmBytes();
    const patch = SINE_PATCH.slice();
    const checked = await loadPatch(wasm, patch, RATE);
    new Uint8Array(wasm).fill(0);
    patch.fill(0);
    const options = checked.options(false);
    expect(new Uint8Array(options.wasm)).toStrictEqual(new Uint8Array(wasmBytes()));
    expect(options.patch).toStrictEqual(SINE_PATCH);
    // Every call gives fresh copies: changing one changes nothing for the next node.
    options.patch.fill(0);
    new Uint8Array(options.wasm).fill(0);
    expect(checked.options(false).patch).toStrictEqual(SINE_PATCH);
    expect(new Uint8Array(checked.options(true).wasm)).toStrictEqual(new Uint8Array(wasmBytes()));
    expect(checked.options(true).marker).toBe(true);
  });

  test("throws the explanation of the engine", async () => {
    await expect(loadPatch(wasmBytes(), UNKNOWN_BOX, RATE)).rejects.toThrow(new Error('node 1: unknown box type "nope~"'));
    const error: unknown = await loadPatch(wasmBytes(), UNKNOWN_BOX, RATE).catch((reason: unknown) => reason);
    expect(error).toHaveProperty("cause.name", "PatchError");
  });

  test("lets other errors through", async () => {
    const empty = new Uint8Array([0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]);
    await expect(loadPatch(empty.buffer, SINE_PATCH, RATE)).rejects.toThrow(new ModuleError("no memory"));
  });

  test("explains bytes that are not a module", async () => {
    const error: unknown = await loadPatch(new Uint8Array([1, 2, 3]).buffer, SINE_PATCH, RATE).catch((reason: unknown) => reason);
    expect(error).toBeInstanceOf(ModuleError);
    expect(error instanceof Error ? error.message : "").toMatch(/^not an engine module: cannot be compiled: CompileError: \S/);
    expect(error).toHaveProperty("cause.name", "CompileError");
  });
});

describe("createEngineNode", () => {
  test("creates an engine node with one channel per patch channel", async () => {
    stubNodes();
    const fake = fakeContext(RATE);
    const patch = await loaded(2);
    const created = createEngineNode(context(fake), patch, () => undefined);
    expect(created.node).toBe(FakeNode.created[0]);
    expect(FakeNode.created[0]).toMatchObject({
      context: fake,
      name: ENGINE_PROCESSOR,
      options: {
        numberOfInputs: 0,
        numberOfOutputs: 1,
        outputChannelCount: [2],
        processorOptions: patch.options(false),
      },
    });
  });

  test("adds a marker channel after the patch channels when asked", async () => {
    stubNodes();
    const patch = await loaded(2);
    createEngineNode(context(fakeContext(RATE)), patch, () => undefined, true);
    expect(FakeNode.created[0]?.options).toMatchObject({ outputChannelCount: [3], processorOptions: patch.options(true) });
    createEngineNode(context(fakeContext(RATE)), await loaded(0), () => undefined, true);
    expect(FakeNode.created[1]?.options).toHaveProperty("outputChannelCount", [1]);
  });

  test("gives a patch without output one silent channel", async () => {
    stubNodes();
    createEngineNode(context(fakeContext(RATE)), await loaded(0), () => undefined);
    expect(FakeNode.created[0]?.options).toHaveProperty("outputChannelCount", [1]);
  });

  test("refuses a context at another rate", async () => {
    stubNodes();
    const patch = await loaded(2);
    expect(() => createEngineNode(context(fakeContext(44_100)), patch, () => undefined)).toThrow(
      new Error("the context runs at 44100 Hz, the patch was loaded at 48000 Hz"),
    );
    expect(FakeNode.created).toHaveLength(0);
  });

  test("is ready when the processor has loaded the patch", async () => {
    stubNodes();
    const failures: Error[] = [];
    const { ready } = createEngineNode(context(fakeContext(RATE)), await loaded(2), (error) => failures.push(error));
    expect(await settled(ready)).toBe("pending");
    FakeNode.created[0]?.port.deliver({ type: "ready" });
    expect(await settled(ready)).toBe("resolved");
    expect(failures).toStrictEqual([]);
  });

  test.each([
    [{ type: "rejected", detail: new TextEncoder().encode("bad patch") }, new Error("bad patch")],
    [{ type: "failed", message: "EngineTrap: trapped" }, new Error("the engine failed: EngineTrap: trapped")],
    [{ type: "nonsense" }, new OptionsError("unknown engine message")],
  ])("reports a failure of the processor before it is ready: %o", async (message, expected) => {
    stubNodes();
    const failures: Error[] = [];
    const { ready } = createEngineNode(context(fakeContext(RATE)), await loaded(2), (error) => failures.push(error));
    FakeNode.created[0]?.port.deliver(message);
    expect(await settled(ready)).toStrictEqual(expected);
    expect(failures).toStrictEqual([expected]);
  });

  test("keeps the rejection of the engine as the cause", async () => {
    stubNodes();
    const failures: Error[] = [];
    createEngineNode(context(fakeContext(RATE)), await loaded(2), (error) => failures.push(error));
    FakeNode.created[0]?.port.deliver({ type: "rejected", detail: new TextEncoder().encode("bad") });
    expect(failures[0]?.cause).toBeInstanceOf(PatchError);
  });

  test("reports a later failure once, and a processor error", async () => {
    stubNodes();
    const failures: Error[] = [];
    const { ready } = createEngineNode(context(fakeContext(RATE)), await loaded(2), (error) => failures.push(error));
    const fake = FakeNode.created[0];
    fake?.port.deliver({ type: "ready" });
    fake?.port.deliver({ type: "failed", message: "trap" });
    fake?.port.deliver({ type: "failed", message: "again" });
    fake?.onprocessorerror?.();
    expect(await settled(ready)).toBe("resolved");
    expect(failures).toStrictEqual([new Error("the engine failed: trap")]);

    const errors: Error[] = [];
    createEngineNode(context(fakeContext(RATE)), await loaded(2), (error) => errors.push(error));
    FakeNode.created[1]?.onprocessorerror?.();
    expect(errors).toStrictEqual([new Error("the engine processor failed")]);
  });
});

describe("tap", () => {
  test("delivers every block until stopped, once", () => {
    stubNodes();
    const source = fakeSource();
    const blocks: Float32Array[][] = [];
    const errors: Error[] = [];
    const running = tap(context(fakeContext(RATE)), node(source), 1, (block) => blocks.push(block), (error) => errors.push(error));
    const port = FakeNode.created[0]?.port;
    port?.deliver([new Float32Array([1])]);
    port?.deliver("garbage");
    port?.deliver([new Float32Array([2])]);
    expect(blocks).toStrictEqual([[new Float32Array([1])], [new Float32Array([2])]]);
    expect(errors).toStrictEqual([new OptionsError("tap block is not an array")]);
    running.stop();
    running.stop();
    expect(port?.onmessage).toBeNull();
    expect(port?.posted).toStrictEqual(["stop"]);
    expect(source.log).toStrictEqual([`connect ${TAP_PROCESSOR}`, `disconnect ${TAP_PROCESSOR}`]);
  });
});

describe("capture", () => {
  test("collects exactly the requested frames, then stops the tap", async () => {
    stubNodes();
    const source = fakeSource();
    const captured = capture(context(fakeContext(RATE)), node(source), 2, 5);
    const tap = FakeNode.created[0];
    expect(tap).toMatchObject({
      name: TAP_PROCESSOR,
      options: { numberOfInputs: 1, numberOfOutputs: 0, channelCount: 2, channelCountMode: "explicit", channelInterpretation: "discrete" },
    });
    expect(source.log).toStrictEqual([`connect ${TAP_PROCESSOR}`]);
    tap?.port.deliver([new Float32Array([1, 2, 3]), new Float32Array([4, 5, 6])]);
    expect(tap?.port.posted).toStrictEqual([]);
    tap?.port.deliver([new Float32Array([7, 8, 9]), new Float32Array([10, 11, 12])]);
    expect(await captured).toStrictEqual([new Float32Array([1, 2, 3, 7, 8]), new Float32Array([4, 5, 6, 10, 11])]);
    expect(tap?.port.posted).toStrictEqual(["stop"]);
    expect(tap?.port.onmessage).toBeNull();
    expect(source.log).toStrictEqual([`connect ${TAP_PROCESSOR}`, `disconnect ${TAP_PROCESSOR}`]);
  });

  test("fills channels missing from a block with silence", async () => {
    stubNodes();
    const captured = capture(context(fakeContext(RATE)), node(fakeSource()), 2, 2);
    FakeNode.created[0]?.port.deliver([new Float32Array([1, 2])]);
    expect(await captured).toStrictEqual([new Float32Array([1, 2]), new Float32Array(2)]);
  });

  test("ignores empty blocks", async () => {
    stubNodes();
    const captured = capture(context(fakeContext(RATE)), node(fakeSource()), 1, 1);
    FakeNode.created[0]?.port.deliver([]);
    expect(FakeNode.created[0]?.port.posted).toStrictEqual([]);
    FakeNode.created[0]?.port.deliver([new Float32Array([3])]);
    expect(await captured).toStrictEqual([new Float32Array([3])]);
  });

  test.each([
    [0, 1, "a capture needs a whole number of channels from 1, not 0"],
    [1.5, 1, "a capture needs a whole number of channels from 1, not 1.5"],
    [2, -1, "a capture needs a whole number of frames from 0, not -1"],
    [2, 0.5, "a capture needs a whole number of frames from 0, not 0.5"],
    [2, Number.NaN, "a capture needs a whole number of frames from 0, not NaN"],
  ])("refuses %d channels and %d frames", async (channels, frames, message) => {
    stubNodes();
    await expect(capture(context(fakeContext(RATE)), node(fakeSource()), channels, frames)).rejects.toThrow(new RangeError(message));
    expect(FakeNode.created).toHaveLength(0);
  });

  test("captures zero frames without a tap", async () => {
    stubNodes();
    const source = fakeSource();
    expect(await capture(context(fakeContext(RATE)), node(source), 2, 0)).toStrictEqual([new Float32Array(), new Float32Array()]);
    expect(FakeNode.created).toHaveLength(0);
    expect(source.log).toStrictEqual([]);
  });

  test("fails on a malformed block, and stops", async () => {
    stubNodes();
    const source = fakeSource();
    const captured = capture(context(fakeContext(RATE)), node(source), 1, 4);
    FakeNode.created[0]?.port.deliver("garbage");
    await expect(captured).rejects.toThrow("invalid processor options: tap block is not an array");
    expect(FakeNode.created[0]?.port.posted).toStrictEqual(["stop"]);
    expect(source.log).toStrictEqual([`connect ${TAP_PROCESSOR}`, `disconnect ${TAP_PROCESSOR}`]);
  });

  test("turns a failure from another realm into an Error of this one", async () => {
    stubNodes();
    const captured = capture(context(fakeContext(RATE)), node(fakeSource()), 1, 4);
    const block = [new Float32Array(1)];
    // An Error of another realm (like the worklet scope) is not an `instanceof Error` here.
    Object.defineProperty(block, "map", { value: (): unknown => runInNewContext("throw new Error('elsewhere')") });
    FakeNode.created[0]?.port.deliver(block);
    await expect(captured).rejects.toThrow(new Error("Error: elsewhere"));
  });

  test("rejects stand-ins missing what the code uses", () => {
    expect(() => context({ sampleRate: 1 })).toThrow("not a context stand-in");
    expect(() => context(null)).toThrow("not a context stand-in");
    expect(() => node({ connect: 1 })).toThrow("not a node stand-in");
  });
});

describe("captureEngine", () => {
  test("captures the frames the engine rendered, from where its marker starts", async () => {
    stubNodes();
    const fake = fakeContext(RATE);
    const patch = await loaded(2);
    const engine = createEngineNode(context(fake), patch, () => undefined, true);
    const captured = captureEngine(context(fake), engine, 2, 3);
    const tapNode = FakeNode.created.at(-1);
    expect(tapNode?.options).toMatchObject({ channelCount: 3 });
    // Two silent frames before the processor started, then frames 1, 2, 3 marked.
    const block = (left: number[], right: number[], marker: number[]): Float32Array[] => [new Float32Array(left), new Float32Array(right), new Float32Array(marker)];
    tapNode?.port.deliver(block([0, 0, 1, 2, 3], [0, 0, 11, 12, 13], [0, 0, 1, 1, 1]));
    tapNode?.port.deliver(block(Array<number>(START_MARGIN).fill(9), Array<number>(START_MARGIN).fill(9), Array<number>(START_MARGIN).fill(1)));
    expect(await captured).toStrictEqual([new Float32Array([1, 2, 3]), new Float32Array([11, 12, 13])]);
  });

  test("fails when the engine never starts in the captured margin", async () => {
    stubNodes();
    const fake = fakeContext(RATE);
    const engine = createEngineNode(context(fake), await loaded(2), () => undefined, true);
    const captured = captureEngine(context(fake), engine, 2, 1);
    FakeNode.created.at(-1)?.port.deliver([new Float32Array(START_MARGIN + 1), new Float32Array(START_MARGIN + 1)]);
    await expect(captured).rejects.toThrow(new Error("the engine started too late in the live capture: not within the 16384 frames of margin"));
  });
});

describe("renderOffline", () => {
  /**
   * Stubs an offline context whose processor posts `message` once rendering starts, and
   * renders patch channels holding their frame index plus 1000 × the channel, with the
   * marker channel (the last one) at 1 from frame `start` on, or never.
   */
  function offline(
    message: unknown,
    options: { readonly later?: unknown; readonly start?: number | "never" } = {},
  ): { contexts: { channels: number; frames: number; rate: number }[]; modules: (string | URL)[]; destination: object } {
    stubNodes();
    const contexts: { channels: number; frames: number; rate: number }[] = [];
    const modules: (string | URL)[] = [];
    const destination = { name: "destination" };
    const start = options.start ?? 0;
    vi.stubGlobal(
      "OfflineAudioContext",
      class {
        /** The rate the context renders at, in hertz. */
        readonly sampleRate: number;
        /** The shared fake destination, so that tests can check what was connected to it. */
        readonly destination = destination;
        /** Channel count of the rendering; the last one is the engine's start marker. */
        readonly #channels: number;
        /** Duration of the rendering, in frames. */
        readonly #frames: number;
        /** Records each module added to the worklet and resolves at once. */
        readonly audioWorklet = {
          addModule: (url: string | URL) => {
            modules.push(url);
            return Promise.resolve();
          },
        };

        constructor(channels: number, frames: number, rate: number) {
          this.sampleRate = rate;
          this.#channels = channels;
          this.#frames = frames;
          contexts.push({ channels, frames, rate });
        }

        /** Delivers `message` (then `later`, when given) to the last node created, as its processor would during rendering, then resolves with a fake buffer: channel `c` holds `frame + 1000 * c`, and the marker channel holds 1 from frame `start` on (never, with `"never"`). */
        startRendering(): Promise<unknown> {
          const port = FakeNode.created.at(-1)?.port;
          port?.deliver(message);
          if (options.later !== undefined) {
            port?.deliver(options.later);
          }
          const frames = this.#frames;
          const marker = this.#channels - 1;
          return Promise.resolve({
            getChannelData: (channel: number) =>
              Float32Array.from({ length: frames }, (_, frame) => (channel === marker ? (start !== "never" && frame >= start ? 1 : 0) : frame + 1000 * channel)),
          });
        }
      },
    );
    return { contexts, modules, destination };
  }

  test("renders in an offline context at the rate of the patch, with a marker and a margin", async () => {
    const { contexts, modules, destination } = offline({ type: "ready" });
    expect(START_MARGIN).toBe(16_384);
    expect(await renderOffline(await loaded(2), 2, "processors.js")).toStrictEqual([new Float32Array([0, 1]), new Float32Array([1000, 1001])]);
    expect(contexts).toStrictEqual([{ channels: 3, frames: 2 + START_MARGIN, rate: RATE }]);
    expect(modules).toStrictEqual(["processors.js"]);
    expect(FakeNode.created[0]?.connected).toStrictEqual([destination]);
    expect(FakeNode.created[0]?.options).toHaveProperty("processorOptions.marker", true);

    expect(await renderOffline(await loaded(0), 0, "processors.js")).toStrictEqual([]);
    expect(contexts[1]).toStrictEqual({ channels: 1, frames: START_MARGIN, rate: RATE });
  });

  test("returns the frames the engine rendered, wherever the processor started", async () => {
    offline({ type: "ready" }, { start: 128 });
    expect(await renderOffline(await loaded(2), 3, "processors.js")).toStrictEqual([new Float32Array([128, 129, 130]), new Float32Array([1128, 1129, 1130])]);
    offline({ type: "ready" }, { start: START_MARGIN });
    expect((await renderOffline(await loaded(2), 1, "processors.js"))[0]).toStrictEqual(new Float32Array([START_MARGIN]));
  });

  test("fails when the processor starts too late, or not at all", async () => {
    // Two frames asked: the render holds frame 16 385, where the marker starts, one too late.
    offline({ type: "ready" }, { start: START_MARGIN + 1 });
    await expect(renderOffline(await loaded(2), 2, "processors.js")).rejects.toThrow(
      new Error("the engine started too late in the offline render: at frame 16385, beyond the 16384 frames of margin"),
    );
    offline({ type: "ready" }, { start: "never" });
    await expect(renderOffline(await loaded(2), 1, "processors.js")).rejects.toThrow(
      new Error("the engine started too late in the offline render: not within the 16384 frames of margin"),
    );
  });

  test("fails instead of returning silence when the processor cannot load the patch", async () => {
    offline({ type: "rejected", detail: new TextEncoder().encode("bad patch") });
    await expect(renderOffline(await loaded(2), 2, "processors.js")).rejects.toThrow(new Error("bad patch"));
  });

  test("fails when the processor fails while rendering", async () => {
    offline({ type: "ready" }, { later: { type: "failed", message: "trap" } });
    await expect(renderOffline(await loaded(2), 2, "processors.js")).rejects.toThrow(new Error("the engine failed: trap"));
  });
});
