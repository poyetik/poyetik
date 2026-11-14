// The player under random interleavings of what the user, the browser and the audio
// thread may do. The engine loads when a "loaded" event says so, never when the real
// compiler happens to finish: a seed always explores the same interleaving, however
// loaded the machine (its own file, since the mock replaces the loader for the whole file).
import type * as Web from "@poyetik/audio-wasm/web";
import type { LoadedPatch } from "@poyetik/audio-wasm/web";
import { afterEach, beforeAll, describe, expect, test, vi } from "vitest";
import { SINE_440 } from "../src/patch.ts";
import { EnginePlayer } from "../src/player.ts";
import { FakeContext, WASM, flush, lastContext, log, playerOptions, resetFakes, workletNode } from "./fakes.ts";

/** A load of the patch the player waits for, which the test settles. */
interface PendingLoad {
  /** Answers the load with the patch. */
  readonly resolve: (loaded: LoadedPatch) => void;
  /** Fails the load with `error`. */
  readonly reject: (error: Error) => void;
}

/** The loads the player started and the test has not settled yet, oldest first. */
const loads = vi.hoisted((): PendingLoad[] => []);

vi.mock("@poyetik/audio-wasm/web", async (importOriginal) => ({
  ...(await importOriginal<typeof Web>()),
  loadPatch: (): Promise<LoadedPatch> =>
    new Promise<LoadedPatch>((resolve, reject) => {
      loads.push({ resolve, reject });
    }),
}));

/** The patch every load answers with, loaded once by the real loader. */
let patch: LoadedPatch | undefined;

beforeAll(async () => {
  const web = await vi.importActual<typeof Web>("@poyetik/audio-wasm/web");
  patch = await web.loadPatch(WASM.buffer.slice(WASM.byteOffset, WASM.byteOffset + WASM.byteLength), SINE_440, 48_000);
});

/** Answers the oldest pending load with the real patch, or fails it with `error`. */
function settleLoad(error?: Error): void {
  const load = loads.shift();
  if (load === undefined || patch === undefined) {
    return;
  }
  if (error === undefined) {
    load.resolve(patch);
  } else {
    load.reject(error);
  }
}

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

/** What the user, the browser and the audio thread may do, in any order. */
const EVENTS = ["start", "stop", "resume", "suspend", "interrupt", "browser resume", "browser close", "fail", "advance", "flush", "loaded", "load fails"] as const;

/** Every kind of state the player may be in. */
const KINDS: readonly string[] = ["stopped", "starting", "playing", "suspended", "stopping", "failed"];

/** The real compiler, which a test slows down: the run must not depend on it. */
const compile = WebAssembly.compile;

/** Lets `turns` turns of the event loop pass. */
async function turns(count: number): Promise<void> {
  for (let turn = 0; turn < count; turn += 1) {
    await flush();
  }
}

/**
 * Plays the 20 events drawn from `seed`, then lets the world drain, checking the
 * promises of the player after each; returns what was seen: the state and the log of
 * the fake world after each event. Every WebAssembly compilation takes `slower` more
 * turns of the event loop.
 */
async function explore(seed: number, slower: number): Promise<string[]> {
  vi.spyOn(WebAssembly, "compile").mockImplementation(async (bytes) => {
    await turns(slower);
    return compile(bytes);
  });
  resetFakes();
  loads.length = 0;
  const next = random(seed);
  const player = new EnginePlayer(playerOptions());
  const trace: string[] = [];
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
    trace.push(`${JSON.stringify(player.state())} ${log.join(", ")}`);
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
      case "browser close": {
        // A session whose context the browser closes fails on the device, at once, at
        // whatever point of its life: its start included.
        const context = lastContext();
        const open = context !== undefined && context.state !== "closed";
        become("closed");
        if (open) {
          expect(player.state()).toStrictEqual({ kind: "failed", cause: "device", detail: "Error: the browser closed the audio device" });
        }
        break;
      }
      case "fail":
        workletNode("poyetik-engine")?.onprocessorerror?.();
        break;
      case "advance":
        lastContext()?.advance(0.01);
        break;
      case "loaded":
        settleLoad();
        break;
      case "load fails":
        settleLoad(new Error("cannot compile"));
        break;
      case "flush":
      case undefined:
        await flush();
        break;
    }
    check();
  }
  // The world drains: every load is answered, the clock runs and every pending event is
  // delivered, a bounded number of times, after which every promise has settled.
  for (let round = 0; round < DRAINING; round += 1) {
    while (loads.length > 0) {
      settleLoad();
    }
    lastContext()?.advance(1);
    await flush();
    check();
  }
  expect(promises.filter((promise) => !promise.settled)).toStrictEqual([]);
  expect(FakeContext.reported).toStrictEqual([]);
  return trace;
}

/** Rounds of draining: a start then needs at most a few (fetch, worklet, load, ready, fade), with room to spare. */
const DRAINING = 10;

/** The seeds explored. */
const SEEDS = Array.from({ length: 100 }, (_unused, seed) => seed);

afterEach(() => {
  loads.length = 0;
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("EnginePlayer under any interleaving", () => {
  test.each(SEEDS)("keeps its promises with seed %i", async (seed) => {
    await explore(seed, 0);
  }, 60_000);

  test.each(SEEDS)("explores the same interleaving with seed %i however long the engine takes to compile", async (seed) => {
    const fast = await explore(seed, 0);
    expect(await explore(seed, 30)).toStrictEqual(fast);
  }, 60_000);
});
