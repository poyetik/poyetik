import { runInNewContext } from "node:vm";
import { afterEach, describe, expect, test, vi } from "vitest";
import { ModuleError } from "../ts/engine.ts";
import { ENGINE_PROCESSOR, TAP_PROCESSOR } from "../ts/protocol.ts";
import { addProcessors, capture, createEngineNode, loadPatch, renderOffline, type LoadedPatch } from "../ts/web.ts";
import { SINE_PATCH, UNKNOWN_BOX, wasmBytes } from "./fixtures.ts";

const RATE = 48_000;

/** A fake `MessagePort`: records what is posted. */
class FakePort {
  onmessage: ((event: { data: unknown }) => void) | null = null;
  readonly posted: unknown[] = [];

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
  static readonly created: FakeNode[] = [];
  readonly port = new FakePort();
  readonly connected: unknown[] = [];

  readonly context: unknown;
  readonly name: string;
  readonly options: unknown;

  constructor(context: unknown, name: string, options: unknown) {
    this.context = context;
    this.name = name;
    this.options = options;
    FakeNode.created.push(this);
  }

  connect(target: unknown): void {
    this.connected.push(target);
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

/** A loaded patch, built by hand. */
function loaded(channels: number, rate = RATE): LoadedPatch {
  return { wasm: new ArrayBuffer(1), patch: new Uint8Array([1]), rate, channels };
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
});

describe("loadPatch", () => {
  test("checks the patch and reports its channels", async () => {
    const wasm = wasmBytes();
    expect(await loadPatch(wasm, SINE_PATCH, RATE)).toStrictEqual({ wasm, patch: SINE_PATCH, rate: RATE, channels: 2 });
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
});

describe("createEngineNode", () => {
  test("creates an engine node with one channel per patch channel", () => {
    stubNodes();
    const fake = fakeContext(RATE);
    const patch = loaded(2);
    const created = createEngineNode(context(fake), patch);
    expect(created).toBe(FakeNode.created[0]);
    expect(FakeNode.created[0]).toMatchObject({
      context: fake,
      name: ENGINE_PROCESSOR,
      options: {
        numberOfInputs: 0,
        numberOfOutputs: 1,
        outputChannelCount: [2],
        processorOptions: { wasm: patch.wasm, patch: patch.patch },
      },
    });
  });

  test("gives a patch without output one silent channel", () => {
    stubNodes();
    createEngineNode(context(fakeContext(RATE)), loaded(0));
    expect(FakeNode.created[0]?.options).toHaveProperty("outputChannelCount", [1]);
  });

  test("refuses a context at another rate", () => {
    stubNodes();
    expect(() => createEngineNode(context(fakeContext(44_100)), loaded(2))).toThrow(
      new Error("the context runs at 44100 Hz, the patch was loaded at 48000 Hz"),
    );
    expect(FakeNode.created).toHaveLength(0);
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

describe("renderOffline", () => {
  test("renders in an offline context at the rate of the patch", async () => {
    stubNodes();
    const contexts: { channels: number; frames: number; rate: number }[] = [];
    const modules: (string | URL)[] = [];
    const destination = { name: "destination" };
    vi.stubGlobal(
      "OfflineAudioContext",
      class {
        readonly sampleRate: number;
        readonly destination = destination;
        readonly audioWorklet = {
          addModule: (url: string | URL) => {
            modules.push(url);
            return Promise.resolve();
          },
        };

        constructor(channels: number, frames: number, rate: number) {
          this.sampleRate = rate;
          contexts.push({ channels, frames, rate });
        }

        startRendering(): Promise<unknown> {
          return Promise.resolve({ getChannelData: (channel: number) => new Float32Array([channel, channel + 0.5, 9]) });
        }
      },
    );
    expect(await renderOffline(loaded(2), 2, "processors.js")).toStrictEqual([new Float32Array([0, 0.5]), new Float32Array([1, 1.5])]);
    expect(contexts).toStrictEqual([{ channels: 2, frames: 2, rate: RATE }]);
    expect(modules).toStrictEqual(["processors.js"]);
    expect(FakeNode.created[0]?.connected).toStrictEqual([destination]);

    expect(await renderOffline(loaded(0), 0, "processors.js")).toStrictEqual([]);
    expect(contexts[1]).toStrictEqual({ channels: 1, frames: 1, rate: RATE });
  });
});
