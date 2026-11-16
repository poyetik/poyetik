import { getEventListeners } from "node:events";
import { runInNewContext } from "node:vm";
import { afterEach, describe, expect, test, vi } from "vitest";
import { DEFAULT_LIMITER, EngineTrap, ModuleError, OutputError, PROTECTED_OUTPUT, PatchError, RAW_OUTPUT } from "../ts/engine.ts";
import { ENGINE_PROCESSOR, MessageError, SOURCE_STOPPED, TAP_PROCESSOR } from "../ts/protocol.ts";
import { addProcessors, capture, captureEngine, ContextClosed, createEngineNode, LoadedPatch, loadPatch, MarkedEngineNode, MAX_MARKED_FRAMES, START_MARGIN, renderOffline, tap } from "../ts/web.ts";
import { engineFunctions, handMadeBytes, READY, SILENT_PATCH, SINE_PATCH, UNKNOWN_BOX, wasmBytes, wasmBytesOf } from "./fixtures.ts";

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

/**
 * A fake context, built on `EventTarget` as a real one is, so that its listeners follow the
 * platform's event model (options, `handleEvent` objects, an `Event` argument): a fake
 * worklet recording the modules added, and a state the tests change.
 */
class FakeContext extends EventTarget {
  /** The rate the context runs at, in hertz. */
  readonly sampleRate: number;
  /** Every module added, in order. */
  readonly modules: (string | URL)[] = [];
  /** Its state: suspended, as a new one is; {@link FakeContext.become} and {@link FakeContext.close} change it. */
  state: AudioContextState = "suspended";
  /** Its worklet, recording the modules added. */
  readonly audioWorklet = {
    addModule: (url: string | URL): Promise<void> => {
      this.modules.push(url);
      return Promise.resolve();
    },
  };

  /** A context running at `sampleRate` hertz. */
  constructor(sampleRate: number) {
    super();
    this.sampleRate = sampleRate;
  }

  /** Moves to `state` and fires `statechange`, as the browser's task that changes it does: the test acts as that task. */
  become(state: AudioContextState): void {
    this.state = state;
    this.dispatchEvent(new Event("statechange"));
  }

  /**
   * Closes the context as browsers do (measured in Chrome 157 and Firefox 134, 2026-11-12;
   * the Web Audio specification queues it so): rejects on a closed one; otherwise its
   * promise settles first, and `statechange` fires in a later task, `setImmediate` here,
   * which runs before a later `setImmediate` of the test. Its state reads "closed" as soon
   * as it returns, as in Chrome and the specification (Firefox changes it only when the
   * promise settles). WebKit is read from its source, not measured, until a browser test of
   * this order (roadmap #24 step 94).
   */
  close(): Promise<void> {
    if (this.state === "closed") {
      return Promise.reject(new DOMException("already closed", "InvalidStateError"));
    }
    this.state = "closed";
    setImmediate(() => this.dispatchEvent(new Event("statechange")));
    return Promise.resolve();
  }
}

/** A fake context running at `sampleRate` hertz, suspended as a new one is. */
function fakeContext(sampleRate: number): FakeContext {
  return new FakeContext(sampleRate);
}

/** How many `statechange` listeners `context` holds, counted by the platform. */
function statechangeListeners(context: EventTarget): number {
  return getEventListeners(context, "statechange").length;
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
  return hasMembers(value, ["sampleRate", "audioWorklet", "state", "addEventListener", "removeEventListener"]);
}

/** Lets a fake node stand in for a real one: see {@link isContext}. */
function isNode(value: unknown): value is AudioNode {
  return hasMembers(value, ["connect", "disconnect"]);
}

/** The fake context, typed as the real one it stands in for. */
function asContext(fake: unknown): BaseAudioContext {
  if (!isContext(fake)) {
    throw new Error("not a context stand-in");
  }
  return fake;
}

/** The fake node, typed as the real one it stands in for. */
function asNode(fake: unknown): AudioNode {
  if (!isNode(fake)) {
    throw new Error("not a node stand-in");
  }
  return fake;
}

describe("the fake context", () => {
  test("closes as browsers do (measured in Chrome 157 and Firefox 134, 2026-11-12, and as the Web Audio specification queues it): its promise settles, then statechange fires in a later task, and a second close rejects", async () => {
    const context = fakeContext(RATE);
    const seen: string[] = [];
    context.addEventListener("statechange", (event: Event) => {
      seen.push(`${event.type} ${context.state}`);
    });
    const closing = context.close();
    seen.push("returned");
    await closing;
    seen.push("settled");
    await new Promise((resolve) => setImmediate(resolve));
    expect(seen).toStrictEqual(["returned", "settled", "statechange closed"]);
    await expect(context.close()).rejects.toThrow(new DOMException("already closed", "InvalidStateError"));
    expect(seen).toHaveLength(3);
  });
});

describe("addProcessors", () => {
  test("adds the processors module to the worklet", async () => {
    const fake = fakeContext(RATE);
    await addProcessors(asContext(fake), "processors.js");
    expect(fake.modules).toStrictEqual(["processors.js"]);
  });

  test("names the URL it could not load", async () => {
    const fake = fakeContext(RATE);
    const missing = new Error("404");
    fake.audioWorklet.addModule = () => Promise.reject(missing);
    const error: unknown = await addProcessors(asContext(fake), "nowhere.js").catch((reason: unknown) => reason);
    expect(error).toStrictEqual(new Error("cannot load the processors from nowhere.js: Error: 404"));
    expect(error).toHaveProperty("cause", missing);
  });
});

describe("loadPatch", () => {
  test("checks the patch and reports its channels and rate", async () => {
    const patch = await loadPatch(wasmBytes(), SINE_PATCH, RATE);
    expect(patch).toBeInstanceOf(LoadedPatch);
    expect(patch).toMatchObject({ rate: RATE, channels: 2 });
    // The browser imposes the rate and the 128-frame quantum; the module decides the block.
    expect(patch.settings).toStrictEqual({ rate: RATE, renderQuantum: 128, engineBlock: 64, imposed: ["rate", "renderQuantum"] });
    expect((await loadPatch(wasmBytesOf(128), SINE_PATCH, 44_100)).settings).toStrictEqual({ rate: 44_100, renderQuantum: 128, engineBlock: 128, imposed: ["rate", "renderQuantum"] });
    expect((await loaded(0, 44_100)).channels).toBe(0);
  });

  test("computes in binary64 by default, or in the precision asked, and runs its nodes in it", async () => {
    const byDefault = await loadPatch(wasmBytes(), SINE_PATCH, RATE);
    expect(byDefault.precision).toBe("binary64");
    expect(byDefault.options(false).precision).toBe("binary64");
    // Its two entry points, one default: 0.1.0's three arguments still load a patch.
    expect((await LoadedPatch.load(wasmBytes(), SINE_PATCH, RATE)).precision).toBe("binary64");
    const narrow = await loadPatch(wasmBytes(), SINE_PATCH, RATE, "binary32");
    expect(narrow).toMatchObject({ rate: RATE, channels: 2, precision: "binary32" });
    expect((await LoadedPatch.load(wasmBytes(), SINE_PATCH, RATE, "binary32")).precision).toBe("binary32");
    expect(narrow.options(true).precision).toBe("binary32");
    await expect(loadPatch(wasmBytes(), UNKNOWN_BOX, RATE, "binary32")).rejects.toThrow(new Error('box 1: unknown box type "nope~"'));
  });

  test("checks its output stage with the engine, raw by default, and runs its nodes through it", async () => {
    const byDefault = await loadPatch(wasmBytes(), SINE_PATCH, RATE);
    expect(byDefault).toMatchObject({ output: RAW_OUTPUT, latency: 0 });
    expect(byDefault.options(false).output).toStrictEqual(RAW_OUTPUT);
    const protectedPatch = await loadPatch(wasmBytes(), SINE_PATCH, RATE, "binary32", PROTECTED_OUTPUT);
    // 5 ms at 48 kHz and the precise detector's 16 frames.
    expect(protectedPatch).toMatchObject({ output: PROTECTED_OUTPUT, latency: 256 });
    expect(protectedPatch.options(true).output).toStrictEqual(PROTECTED_OUTPUT);
    expect((await LoadedPatch.load(wasmBytes(), SINE_PATCH, RATE, "binary64", PROTECTED_OUTPUT)).latency).toBe(256);
    const refused = await loadPatch(wasmBytes(), SINE_PATCH, RATE, "binary64", { ...PROTECTED_OUTPUT, maximumLookahead: 9 }).catch((error: unknown) => error);
    expect(refused).toStrictEqual(new Error("limiter maximum lookahead 9 ms is out of range (0.5 to 5 ms)"));
    expect(refused instanceof Error ? refused.cause : undefined).toBeInstanceOf(OutputError);
  });

  test("keeps a frozen copy of its output stage, whatever its caller does with its own", async () => {
    const limiter = { ...DEFAULT_LIMITER };
    const output = { ...PROTECTED_OUTPUT, limiter };
    const checked = await loadPatch(wasmBytes(), SINE_PATCH, RATE, undefined, output);
    output.mode = "switchable";
    limiter.lookahead = 6;
    expect(checked.output).toStrictEqual(PROTECTED_OUTPUT);
    expect(checked.options(false).output).toStrictEqual(PROTECTED_OUTPUT);
    expect([Object.isFrozen(checked.output), Object.isFrozen(Reflect.get(checked.output, "limiter"))]).toStrictEqual([true, true]);
  });

  test("checks and keeps the fields of its output stage as read once, whatever object holds them", async () => {
    // A stage whose fields are getters, and one with a field more: the engine checks the
    // values the processors get, nothing else.
    const stage = {
      get mode() {
        return "protected" as const;
      },
      maximumLookahead: 5,
      detector: PROTECTED_OUTPUT.detector,
      limiter: DEFAULT_LIMITER,
      extra: (): number => 1,
    };
    const checked = await loadPatch(wasmBytes(), SINE_PATCH, RATE, undefined, stage);
    expect(checked.options(false).output).toStrictEqual(PROTECTED_OUTPUT);
    expect(checked.latency).toBe(256);
  });

  test("refuses an output stage of the wrong types, as its processors would, before loading anything", async () => {
    // Proxies of unfrozen copies, as a plain-JavaScript caller may pass: a number written as
    // a string, which the engine alone would read as a number.
    const stage = (field: string, value: unknown): typeof PROTECTED_OUTPUT => new Proxy({ ...PROTECTED_OUTPUT }, { get: (target, key): unknown => (key === field ? value : Reflect.get(target, key)) });
    const limiter = new Proxy({ ...DEFAULT_LIMITER }, { get: (target, key): unknown => (key === "ceiling" ? "-1" : Reflect.get(target, key)) });
    const refusals = [
      [stage("maximumLookahead", "5"), "output maximumLookahead is not a number"],
      [stage("limiter", limiter), "output limiter ceiling is not a number"],
      [stage("detector", "exact"), "output detector is not one of standard, precise, mastering"],
    ] as const;
    for (const [output, detail] of refusals) {
      await expect(loadPatch(wasmBytes(), SINE_PATCH, RATE, undefined, output)).rejects.toThrow(new TypeError(detail));
      // Before compiling the module: bytes that are no module do not change the error.
      await expect(loadPatch(new ArrayBuffer(1), SINE_PATCH, RATE, undefined, output)).rejects.toThrow(new TypeError(detail));
    }
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
    await expect(loadPatch(wasmBytes(), UNKNOWN_BOX, RATE)).rejects.toThrow(new Error('box 1: unknown box type "nope~"'));
    const error: unknown = await loadPatch(wasmBytes(), UNKNOWN_BOX, RATE).catch((reason: unknown) => reason);
    expect(error).toHaveProperty("cause.name", "PatchError");
  });

  test("refuses a rate that is not a whole number of hertz, as the AudioContext rate it is given may be", async () => {
    for (const rate of [48_000.5, -1, 2 ** 32, Number.NaN]) {
      const error: unknown = await loadPatch(wasmBytes(), SINE_PATCH, rate).catch((reason: unknown) => reason);
      expect(error).toStrictEqual(new RangeError(`the sample rate must be a whole number of hertz, not ${String(rate)}`));
    }
  });

  test("lets other errors through", async () => {
    const empty = new Uint8Array([0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]);
    await expect(loadPatch(empty.buffer, SINE_PATCH, RATE)).rejects.toThrow(new ModuleError("no memory"));
  });

  test("lets a trap of the module while checking the patch through, with the trap as cause", async () => {
    const functions = engineFunctions().map((func) => (func.name === "engine_load" ? { ...func, body: "trap" as const } : func));
    const error: unknown = await loadPatch(handMadeBytes({ memory: true, functions }).buffer, SINE_PATCH, RATE).catch((reason: unknown) => reason);
    expect(error).toStrictEqual(new EngineTrap());
    expect(error).toHaveProperty("cause.name", "RuntimeError");
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
    const created = createEngineNode(asContext(fake), patch, { onFailure: () => undefined });
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
    createEngineNode(asContext(fakeContext(RATE)), patch, { onFailure: () => undefined }, true);
    expect(FakeNode.created[0]?.options).toMatchObject({ outputChannelCount: [3], processorOptions: patch.options(true) });
    createEngineNode(asContext(fakeContext(RATE)), await loaded(0), { onFailure: () => undefined }, true);
    expect(FakeNode.created[1]?.options).toHaveProperty("outputChannelCount", [1]);
  });

  test("gives a patch without output one silent channel", async () => {
    stubNodes();
    createEngineNode(asContext(fakeContext(RATE)), await loaded(0), { onFailure: () => undefined });
    expect(FakeNode.created[0]?.options).toHaveProperty("outputChannelCount", [1]);
  });

  test("refuses a context at another rate", async () => {
    stubNodes();
    const patch = await loaded(2);
    expect(() => createEngineNode(asContext(fakeContext(44_100)), patch, { onFailure: () => undefined })).toThrow(
      new Error("the context runs at 44100 Hz, the patch was loaded at 48000 Hz"),
    );
    expect(FakeNode.created).toHaveLength(0);
  });

  test("is ready when the processor has loaded the patch", async () => {
    stubNodes();
    const failures: Error[] = [];
    const { ready } = createEngineNode(asContext(fakeContext(RATE)), await loaded(2), { onFailure: (error) => failures.push(error) });
    expect(await settled(ready)).toBe("pending");
    // Whatever the processor observed is what `ready` gives: here, no subnormal kept as a result.
    const flushing = { binary32: { operands: true, results: false }, binary64: { operands: true, results: false } };
    FakeNode.created[0]?.port.deliver({ type: "ready", subnormals: flushing });
    expect(await settled(ready)).toBe("resolved");
    expect(await ready).toStrictEqual(flushing);
    expect(failures).toStrictEqual([]);
  });

  test("posts the settings of its output stage to the processor, and hands a refusal to onRefused", async () => {
    stubNodes();
    const refusals: Error[] = [];
    const failures: Error[] = [];
    for (const marker of [false, true] as const) {
      const engine = marker
        ? createEngineNode(asContext(fakeContext(RATE)), await loaded(2), { onFailure: (error) => failures.push(error), onRefused: (error) => refusals.push(error) }, true)
        : createEngineNode(asContext(fakeContext(RATE)), await loaded(2), { onFailure: (error) => failures.push(error), onRefused: (error) => refusals.push(error) });
      const port = FakeNode.created.at(-1)?.port;
      engine.setLimiter(DEFAULT_LIMITER);
      engine.setProtected(false);
      expect(port?.posted).toStrictEqual([
        { type: "limiter", settings: DEFAULT_LIMITER },
        { type: "protect", protect: false },
      ]);
      port?.deliver(READY);
      port?.deliver({ type: "refused", detail: new TextEncoder().encode("no limiter") });
    }
    expect(refusals).toStrictEqual([new Error("no limiter"), new Error("no limiter")]);
    expect(refusals.map((error) => error.cause)).toStrictEqual([new OutputError(new TextEncoder().encode("no limiter")), new OutputError(new TextEncoder().encode("no limiter"))]);
    expect(failures).toStrictEqual([]);
    // Without a handler, a refusal is read and dropped: no failure.
    createEngineNode(asContext(fakeContext(RATE)), await loaded(2), { onFailure: (error) => failures.push(error) });
    FakeNode.created.at(-1)?.port.deliver({ type: "refused", detail: new Uint8Array() });
    expect(failures).toStrictEqual([]);
  });

  test("hands what the output stage replaced to onReplaced, before and after ready, and to nobody without it", async () => {
    stubNodes();
    const reports: unknown[] = [];
    const failures: Error[] = [];
    const { ready } = createEngineNode(asContext(fakeContext(RATE)), await loaded(2), {
      onFailure: (error) => failures.push(error),
      onReplaced: (replaced) => reports.push(replaced),
    });
    const first = { count: 1, first: { frame: 3, channel: 0 } };
    FakeNode.created[0]?.port.deliver({ type: "replaced", replaced: first });
    expect(reports).toStrictEqual([first]);
    expect(await settled(ready)).toBe("pending");
    FakeNode.created[0]?.port.deliver(READY);
    const more = { count: 5, first: { frame: 3, channel: 0 } };
    FakeNode.created[0]?.port.deliver({ type: "replaced", replaced: more });
    expect(reports).toStrictEqual([first, more]);
    expect(failures).toStrictEqual([]);
    // Without a handler, a report is read and dropped: no failure.
    const quiet = createEngineNode(asContext(fakeContext(RATE)), await loaded(2), { onFailure: (error) => failures.push(error) });
    FakeNode.created[1]?.port.deliver({ type: "replaced", replaced: first });
    FakeNode.created[1]?.port.deliver(READY);
    expect(await settled(quiet.ready)).toBe("resolved");
    expect(failures).toStrictEqual([]);
  });

  test.each([
    [{ type: "rejected", detail: new TextEncoder().encode("bad patch") }, new Error("bad patch")],
    [{ type: "failed", message: "EngineTrap: trapped" }, new Error("the engine failed: EngineTrap: trapped")],
    [{ type: "nonsense" }, new MessageError("unknown engine message")],
  ])("reports a failure of the processor before it is ready: %o", async (message, expected) => {
    stubNodes();
    const failures: Error[] = [];
    const { ready } = createEngineNode(asContext(fakeContext(RATE)), await loaded(2), { onFailure: (error) => failures.push(error) });
    FakeNode.created[0]?.port.deliver(message);
    expect(await settled(ready)).toStrictEqual(expected);
    expect(failures).toStrictEqual([expected]);
  });

  test("keeps the rejection of the engine as the cause", async () => {
    stubNodes();
    const failures: Error[] = [];
    const { ready } = createEngineNode(asContext(fakeContext(RATE)), await loaded(2), { onFailure: (error) => failures.push(error) });
    FakeNode.created[0]?.port.deliver({ type: "rejected", detail: new TextEncoder().encode("bad") });
    expect(failures[0]?.cause).toBeInstanceOf(PatchError);
    expect(await settled(ready)).toBe(failures[0]);
  });

  test("handles the rejection of ready itself, for those who never await it", async () => {
    stubNodes();
    // Observed without rejecting: an unhandled rejection would crash the test runner
    // instead of failing this test.
    const handled = vi.spyOn(Promise.prototype, "catch");
    const { ready } = createEngineNode(asContext(fakeContext(RATE)), await loaded(2), { onFailure: () => undefined });
    expect(handled.mock.contexts).toContain(ready);
    handled.mockRestore();
  });

  test("rejects ready when its context closes first, which is no failure of the engine, and stops following the context", async () => {
    stubNodes();
    const failures: Error[] = [];
    const context = fakeContext(RATE);
    const { ready } = createEngineNode(asContext(context), await loaded(2), { onFailure: (error) => failures.push(error) });
    expect(statechangeListeners(context)).toBe(1);
    // A context that only runs or suspends does not end the wait.
    for (const state of ["running", "suspended"] as const) {
      context.become(state);
      expect(await settled(ready)).toBe("pending");
    }
    await context.close();
    expect(await settled(ready)).toStrictEqual(new ContextClosed("the context closed before the engine processor was ready"));
    expect([failures, statechangeListeners(context)]).toStrictEqual([[], 0]);
    // The processor's later messages still reach onFailure, once.
    FakeNode.created[0]?.port.deliver({ type: "failed", message: "trap" });
    expect(failures).toStrictEqual([new Error("the engine failed: trap")]);
  });

  test("names the end of its context, told apart from a failure by its name", () => {
    const error = new ContextClosed("closed");
    expect([error instanceof Error, error.name, error.message]).toStrictEqual([true, "ContextClosed", "closed"]);
  });

  test("rejects ready at once when its context is already closed", async () => {
    stubNodes();
    const context = fakeContext(RATE);
    await context.close();
    // Its `statechange` has fired before the node exists: no later event can end the wait,
    // only the node's own look at the state.
    await new Promise((resolve) => setImmediate(resolve));
    const { ready } = createEngineNode(asContext(context), await loaded(2), { onFailure: () => undefined });
    expect(await settled(ready)).toStrictEqual(new ContextClosed("the context closed before the engine processor was ready"));
    expect(statechangeListeners(context)).toBe(0);
  });

  test("stops following the context once ready, or once failed", async () => {
    stubNodes();
    const context = fakeContext(RATE);
    const first = createEngineNode(asContext(context), await loaded(2), { onFailure: () => undefined });
    const second = createEngineNode(asContext(context), await loaded(2), { onFailure: () => undefined });
    expect(statechangeListeners(context)).toBe(2);
    FakeNode.created[0]?.port.deliver(READY);
    FakeNode.created[1]?.onprocessorerror?.();
    expect(statechangeListeners(context)).toBe(0);
    await context.close();
    expect([await settled(first.ready), await settled(second.ready)]).toStrictEqual(["resolved", new Error("the engine processor failed")]);
  });

  test("reports a later failure once, and a processor error", async () => {
    stubNodes();
    const failures: Error[] = [];
    const { ready } = createEngineNode(asContext(fakeContext(RATE)), await loaded(2), { onFailure: (error) => failures.push(error) });
    const fake = FakeNode.created[0];
    fake?.port.deliver(READY);
    fake?.port.deliver({ type: "failed", message: "trap" });
    fake?.port.deliver({ type: "failed", message: "again" });
    fake?.onprocessorerror?.();
    expect(await settled(ready)).toBe("resolved");
    expect(failures).toStrictEqual([new Error("the engine failed: trap")]);

    const errors: Error[] = [];
    const second = createEngineNode(asContext(fakeContext(RATE)), await loaded(2), { onFailure: (error) => errors.push(error) });
    FakeNode.created[1]?.onprocessorerror?.();
    expect(errors).toStrictEqual([new Error("the engine processor failed")]);
    expect(await settled(second.ready)).toBe(errors[0]);
  });
});

describe("tap", () => {
  test("delivers every block until stopped, once", () => {
    stubNodes();
    const source = fakeSource();
    const blocks: Float32Array[][] = [];
    const errors: Error[] = [];
    const running = tap(asContext(fakeContext(RATE)), asNode(source), 1, (block) => blocks.push(block), (error) => errors.push(error));
    const port = FakeNode.created[0]?.port;
    port?.deliver([new Float32Array([1])]);
    port?.deliver("garbage");
    port?.deliver([new Float32Array([2])]);
    expect(blocks).toStrictEqual([[new Float32Array([1])], [new Float32Array([2])]]);
    expect(errors).toStrictEqual([new MessageError("render quantum is not an array")]);
    running.stop();
    running.stop();
    expect(port?.onmessage).toBeNull();
    expect(port?.posted).toStrictEqual(["stop"]);
    expect(source.log).toStrictEqual([`connect ${TAP_PROCESSOR}`, `disconnect ${TAP_PROCESSOR}`]);
  });

  test("ignores its source stopping: the source may start again", () => {
    stubNodes();
    const blocks: Float32Array[][] = [];
    const errors: Error[] = [];
    tap(asContext(fakeContext(RATE)), asNode(fakeSource()), 1, (block) => blocks.push(block), (error) => errors.push(error));
    const port = FakeNode.created[0]?.port;
    port?.deliver(SOURCE_STOPPED);
    port?.deliver([new Float32Array([1])]);
    expect(blocks).toStrictEqual([[new Float32Array([1])]]);
    expect(errors).toStrictEqual([]);
    expect(port?.posted).toStrictEqual([]);
  });
});

describe("capture", () => {
  test("collects exactly the requested frames, then stops the tap", async () => {
    stubNodes();
    const source = fakeSource();
    const captured = capture(asContext(fakeContext(RATE)), asNode(source), 2, 5);
    const tapNode = FakeNode.created[0];
    expect(tapNode).toMatchObject({
      name: TAP_PROCESSOR,
      options: { numberOfInputs: 1, numberOfOutputs: 0, channelCount: 2, channelCountMode: "explicit", channelInterpretation: "discrete" },
    });
    expect(source.log).toStrictEqual([`connect ${TAP_PROCESSOR}`]);
    tapNode?.port.deliver([new Float32Array([1, 2, 3]), new Float32Array([4, 5, 6])]);
    expect(tapNode?.port.posted).toStrictEqual([]);
    tapNode?.port.deliver([new Float32Array([7, 8, 9]), new Float32Array([10, 11, 12])]);
    expect(await captured).toStrictEqual([new Float32Array([1, 2, 3, 7, 8]), new Float32Array([4, 5, 6, 10, 11])]);
    expect(tapNode?.port.posted).toStrictEqual(["stop"]);
    expect(tapNode?.port.onmessage).toBeNull();
    expect(source.log).toStrictEqual([`connect ${TAP_PROCESSOR}`, `disconnect ${TAP_PROCESSOR}`]);
  });

  test("fails when its source stops before the end, and stops the tap", async () => {
    stubNodes();
    const source = fakeSource();
    const context = fakeContext(RATE);
    const captured = capture(asContext(context), asNode(source), 1, 4);
    const tapNode = FakeNode.created[0];
    tapNode?.port.deliver([new Float32Array([1, 2])]);
    tapNode?.port.deliver(SOURCE_STOPPED);
    await expect(captured).rejects.toThrow(new Error("the source stopped before the end of the capture"));
    expect(tapNode?.port.posted).toStrictEqual(["stop"]);
    expect(source.log).toStrictEqual([`connect ${TAP_PROCESSOR}`, `disconnect ${TAP_PROCESSOR}`]);
    expect(statechangeListeners(context)).toBe(0);
  });

  test("fails when the context closes before the end, but not while it is suspended", async () => {
    stubNodes();
    const source = fakeSource();
    const context = fakeContext(RATE);
    const captured = capture(asContext(context), asNode(source), 1, 4);
    expect(statechangeListeners(context)).toBe(1);
    // A capture starts before its context runs: a suspended context is no failure.
    context.become("suspended");
    expect(await settled(captured)).toBe("pending");
    await context.close();
    await expect(captured).rejects.toStrictEqual(new ContextClosed("the context closed before the end of the capture"));
    expect(FakeNode.created[0]?.port.posted).toStrictEqual(["stop"]);
    expect(source.log).toStrictEqual([`connect ${TAP_PROCESSOR}`, `disconnect ${TAP_PROCESSOR}`]);
    expect(statechangeListeners(context)).toBe(0);
  });

  test("fails at once in a closed context, without a tap", async () => {
    stubNodes();
    const source = fakeSource();
    const context = fakeContext(RATE);
    await context.close();
    await expect(capture(asContext(context), asNode(source), 1, 4)).rejects.toStrictEqual(new ContextClosed("the context closed before the end of the capture"));
    expect(FakeNode.created).toHaveLength(0);
    expect(source.log).toStrictEqual([]);
    expect(statechangeListeners(context)).toBe(0);
  });

  test("stops following the context once complete", async () => {
    stubNodes();
    const context = fakeContext(RATE);
    const captured = capture(asContext(context), asNode(fakeSource()), 1, 1);
    FakeNode.created[0]?.port.deliver([new Float32Array([1])]);
    expect(await captured).toStrictEqual([new Float32Array([1])]);
    expect(statechangeListeners(context)).toBe(0);
  });

  test("fills channels missing from a block with silence", async () => {
    stubNodes();
    const captured = capture(asContext(fakeContext(RATE)), asNode(fakeSource()), 2, 2);
    FakeNode.created[0]?.port.deliver([new Float32Array([1, 2])]);
    expect(await captured).toStrictEqual([new Float32Array([1, 2]), new Float32Array(2)]);
  });

  test("ignores empty blocks", async () => {
    stubNodes();
    const captured = capture(asContext(fakeContext(RATE)), asNode(fakeSource()), 1, 1);
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
    await expect(capture(asContext(fakeContext(RATE)), asNode(fakeSource()), channels, frames)).rejects.toThrow(new RangeError(message));
    expect(FakeNode.created).toHaveLength(0);
  });

  test("captures zero frames without a tap", async () => {
    stubNodes();
    const source = fakeSource();
    expect(await capture(asContext(fakeContext(RATE)), asNode(source), 2, 0)).toStrictEqual([new Float32Array(), new Float32Array()]);
    expect(FakeNode.created).toHaveLength(0);
    expect(source.log).toStrictEqual([]);
  });

  test("fails on a malformed render quantum, and stops", async () => {
    stubNodes();
    const source = fakeSource();
    const captured = capture(asContext(fakeContext(RATE)), asNode(source), 1, 4);
    FakeNode.created[0]?.port.deliver("garbage");
    await expect(captured).rejects.toThrow(new MessageError("render quantum is not an array"));
    expect(FakeNode.created[0]?.port.posted).toStrictEqual(["stop"]);
    expect(source.log).toStrictEqual([`connect ${TAP_PROCESSOR}`, `disconnect ${TAP_PROCESSOR}`]);
  });

  test("turns a failure from another realm into an Error of this one", async () => {
    stubNodes();
    const captured = capture(asContext(fakeContext(RATE)), asNode(fakeSource()), 1, 4);
    const block = [new Float32Array(1)];
    // An Error of another realm (like the worklet scope) is not an `instanceof Error` here.
    Object.defineProperty(block, "map", { value: (): unknown => runInNewContext("throw new Error('elsewhere')") });
    FakeNode.created[0]?.port.deliver(block);
    await expect(captured).rejects.toThrow(new Error("Error: elsewhere"));
  });

  test("rejects stand-ins missing what the code uses", () => {
    expect(() => asContext({ sampleRate: 1 })).toThrow("not a context stand-in");
    expect(() => asContext(null)).toThrow("not a context stand-in");
    expect(() => asNode({ connect: 1 })).toThrow("not a node stand-in");
  });
});

describe("captureEngine", () => {
  /** The counts 1, 2, … `count` of a marker channel. */
  function counts(count: number, from = 1): number[] {
    return Array.from({ length: count }, (_empty, index) => from + index);
  }

  /** Three channels (left, right, marker) of one render quantum of a tap. */
  function block(left: readonly number[], right: readonly number[], marker: readonly number[]): Float32Array[] {
    return [new Float32Array(left), new Float32Array(right), new Float32Array(marker)];
  }

  /** A live capture of `frames` frames of a marked sine engine, and the tap it delivers through. */
  async function capturing(frames: number): Promise<{ captured: Promise<Float32Array[]>; deliver: (data: unknown) => void }> {
    stubNodes();
    const fake = fakeContext(RATE);
    const engine = createEngineNode(asContext(fake), await loaded(2), { onFailure: () => undefined }, true);
    expect(engine).toBeInstanceOf(MarkedEngineNode);
    expect(engine.channels).toBe(2);
    const captured = captureEngine(asContext(fake), engine, frames);
    const tapNode = FakeNode.created.at(-1);
    expect(tapNode?.options).toMatchObject({ channelCount: 3 });
    return { captured, deliver: (data) => tapNode?.port.deliver(data) };
  }

  test("captures the frames the engine rendered, from where its marker starts", async () => {
    const { captured, deliver } = await capturing(3);
    // Two silent frames before the processor started, then frames 1, 2, 3… counted.
    deliver(block([0, 0, 1, 2, 3], [0, 0, 11, 12, 13], [0, 0, ...counts(3)]));
    deliver(block(Array<number>(START_MARGIN).fill(9), Array<number>(START_MARGIN).fill(9), counts(START_MARGIN, 4)));
    expect(await captured).toStrictEqual([new Float32Array([1, 2, 3]), new Float32Array([11, 12, 13])]);
  });

  test("captures from the very first frame, when the engine and the capture start together", async () => {
    const { captured, deliver } = await capturing(2);
    deliver(block(counts(START_MARGIN + 2), counts(START_MARGIN + 2, 1001), counts(START_MARGIN + 2)));
    expect(await captured).toStrictEqual([new Float32Array([1, 2]), new Float32Array([1001, 1002])]);
  });

  test("fails when the capture started after the engine, instead of returning shifted frames", async () => {
    const { captured, deliver } = await capturing(1);
    // The tap missed the first quantum: the marker starts at 129.
    deliver(block(Array<number>(START_MARGIN + 1).fill(5), Array<number>(START_MARGIN + 1).fill(5), counts(START_MARGIN + 1, 129)));
    await expect(captured).rejects.toThrow(new Error("the live capture started 128 frames after the engine"));
  });

  test.each([0.5, -1, Number.NaN])("fails on a marker channel holding %d, which is no marker", async (value) => {
    const { captured, deliver } = await capturing(1);
    deliver(block(Array<number>(START_MARGIN + 1).fill(5), Array<number>(START_MARGIN + 1).fill(5), [0, value]));
    await expect(captured).rejects.toThrow(new Error(`the live capture has no marker after the patch channels: ${String(value)} at frame 1`));
  });

  test("fails when the engine stops before the end of the capture", async () => {
    const { captured, deliver } = await capturing(4);
    deliver(block(Array<number>(START_MARGIN + 4).fill(5), Array<number>(START_MARGIN + 4).fill(5), [0, 1, 2, 3]));
    await expect(captured).rejects.toThrow(new Error("the engine output of the live capture breaks at frame 3: the engine stopped"));
  });

  test("fails when the engine stops before the end of the capture, and its tap tells it", async () => {
    const { captured, deliver } = await capturing(4);
    deliver(block([0, 1], [0, 1], [0, 1]));
    deliver(SOURCE_STOPPED);
    await expect(captured).rejects.toThrow(new Error("the source stopped before the end of the capture"));
  });

  test("fails when the engine never starts in the captured margin", async () => {
    const { captured, deliver } = await capturing(1);
    deliver([new Float32Array(START_MARGIN + 1), new Float32Array(START_MARGIN + 1)]);
    await expect(captured).rejects.toThrow(new Error("the engine started too late in the live capture: not within the 16384 frames of margin"));
  });

  test.each([
    [MAX_MARKED_FRAMES + 1, `at most ${String(MAX_MARKED_FRAMES)} frames can be rendered from the start of the engine, not ${String(MAX_MARKED_FRAMES + 1)}`],
    [-1, "a capture needs a whole number of frames from 0, not -1"],
    [0.5, "a capture needs a whole number of frames from 0, not 0.5"],
    [Number.NaN, "a capture needs a whole number of frames from 0, not NaN"],
  ])("refuses to capture %d frames", async (frames, message) => {
    stubNodes();
    const fake = fakeContext(RATE);
    const engine = createEngineNode(asContext(fake), await loaded(2), { onFailure: () => undefined }, true);
    await expect(captureEngine(asContext(fake), engine, frames)).rejects.toThrow(new RangeError(message));
    expect(FakeNode.created).toHaveLength(1);
  });

  test("captures no frame after checking the start", async () => {
    const { captured, deliver } = await capturing(0);
    deliver(block([0, 7], [0, 7], [0, 1]));
    deliver(block(Array<number>(START_MARGIN).fill(5), Array<number>(START_MARGIN).fill(5), counts(START_MARGIN, 2)));
    expect(await captured).toStrictEqual([new Float32Array(), new Float32Array()]);
  });
});

describe("renderOffline", () => {
  /**
   * Stubs an offline context whose processor posts `message` once rendering starts, and
   * renders patch channels holding their frame index plus 1000 × the channel, with the
   * marker channel (the last one) counting 1, 2, … from frame `start` on (or never), up
   * to frame `stop` (excluded) when given, then silent.
   */
  function offline(
    message: unknown,
    options: { readonly later?: unknown; readonly start?: number | "never"; readonly stop?: number } = {},
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
        /** Its state: an offline context never closes before its rendering ends here. */
        readonly state = "suspended";
        /** Follows nothing: the state never changes. */
        addEventListener(): void {
          return undefined;
        }
        /** Follows nothing: the state never changes. */
        removeEventListener(): void {
          return undefined;
        }
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

        /** Delivers `message` (none when `undefined`: a processor that never answers; then `later`, when given) to the last node created, as its processor would during rendering, then resolves with a fake buffer: channel `c` holds `frame + 1000 * c`, and the marker channel holds 1 from frame `start` on (never, with `"never"`). */
        startRendering(): Promise<unknown> {
          const port = FakeNode.created.at(-1)?.port;
          if (message !== undefined) {
            port?.deliver(message);
          }
          if (options.later !== undefined) {
            port?.deliver(options.later);
          }
          const frames = this.#frames;
          const marker = this.#channels - 1;
          return Promise.resolve({
            getChannelData: (channel: number) =>
              Float32Array.from({ length: frames }, (_empty, frame) =>
                channel === marker ? (start !== "never" && frame >= start && frame < (options.stop ?? frames) ? frame - start + 1 : 0) : frame + 1000 * channel,
              ),
          });
        }
      },
    );
    return { contexts, modules, destination };
  }

  test("renders in an offline context at the rate of the patch, with a marker and a margin", async () => {
    const { contexts, modules, destination } = offline(READY);
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
    offline(READY, { start: 128 });
    expect(await renderOffline(await loaded(2), 3, "processors.js")).toStrictEqual([new Float32Array([128, 129, 130]), new Float32Array([1128, 1129, 1130])]);
    offline(READY, { start: START_MARGIN });
    expect((await renderOffline(await loaded(2), 1, "processors.js"))[0]).toStrictEqual(new Float32Array([START_MARGIN]));
  });

  test("fails when the processor starts too late, or not at all", async () => {
    // Two frames asked: the render holds frame 16 385, where the marker starts, one too late.
    offline(READY, { start: START_MARGIN + 1 });
    await expect(renderOffline(await loaded(2), 2, "processors.js")).rejects.toThrow(
      new Error("the engine started too late in the offline render: at frame 16385, beyond the 16384 frames of margin"),
    );
    offline(READY, { start: "never" });
    await expect(renderOffline(await loaded(2), 1, "processors.js")).rejects.toThrow(
      new Error("the engine started too late in the offline render: not within the 16384 frames of margin"),
    );
    // A processor never constructed during the render: it never answers.
    offline(undefined, { start: "never" });
    await expect(renderOffline(await loaded(2), 1, "processors.js")).rejects.toThrow(
      new Error("the engine started too late in the offline render: not within the 16384 frames of margin"),
    );
  });

  test("fails instead of returning silence when the processor cannot load the patch", async () => {
    offline({ type: "rejected", detail: new TextEncoder().encode("bad patch") });
    await expect(renderOffline(await loaded(2), 2, "processors.js")).rejects.toThrow(new Error("bad patch"));
  });

  test("fails when the engine stops before the end of the render, whatever the order of the messages", async () => {
    // The processor failed at frame 128 of the render, but the failure was not received
    // before the render ended: the marker shows it.
    offline(READY, { start: 0, stop: 128 });
    await expect(renderOffline(await loaded(2), 200, "processors.js")).rejects.toThrow(
      new Error("the engine output of the offline render breaks at frame 128: the engine stopped"),
    );
  });

  test.each([
    [-1, "a render needs a whole number of frames from 0, not -1"],
    [0.5, "a render needs a whole number of frames from 0, not 0.5"],
    [Number.NaN, "a render needs a whole number of frames from 0, not NaN"],
  ])("refuses to render %d frames, before rendering", async (frames, message) => {
    const { contexts } = offline(READY);
    await expect(renderOffline(await loaded(2), frames, "processors.js")).rejects.toThrow(new RangeError(message));
    expect(contexts).toStrictEqual([]);
  });

  test("renders up to the frames the marker counts, and refuses more before rendering", async () => {
    offline(READY);
    await expect(renderOffline(await loaded(2), MAX_MARKED_FRAMES + 1, "processors.js")).rejects.toThrow(
      new RangeError(`at most ${String(MAX_MARKED_FRAMES)} frames can be rendered from the start of the engine, not ${String(MAX_MARKED_FRAMES + 1)}`),
    );
    expect(MAX_MARKED_FRAMES).toBe(2 ** 24 - START_MARGIN);
    const { contexts } = offline({ type: "rejected", detail: new TextEncoder().encode("stop here") });
    await expect(renderOffline(await loaded(2), MAX_MARKED_FRAMES, "processors.js")).rejects.toThrow(new Error("stop here"));
    expect(contexts).toStrictEqual([{ channels: 3, frames: 2 ** 24, rate: RATE }]);
  });

  test("fails when the processor fails while rendering", async () => {
    offline(READY, { later: { type: "failed", message: "trap" } });
    await expect(renderOffline(await loaded(2), 2, "processors.js")).rejects.toThrow(new Error("the engine failed: trap"));
  });
});
