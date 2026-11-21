// The player under random interleavings of what the user, the browser and the audio
// thread may do. The engine loads when a "loaded" event says so, never when the real
// compiler happens to finish: a seed always explores the same interleaving, however
// loaded the machine (its own file, since the mock replaces the loader for the whole file).
import type * as Web from "@poyetik/audio-wasm/web";
import type { LoadedPatch } from "@poyetik/audio-wasm/web";
import { afterEach, beforeAll, describe, expect, test, vi } from "vitest";
import { SINE_440 } from "../src/patch.ts";
import { CLOCK_POLL_MS, EnginePlayer } from "../src/player.ts";
import { FakeContext, WASM, flush, lastContext, log, playerOptions, resetFakes, workletNode } from "./fakes.ts";
import { random } from "./random.ts";

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

/**
 * What the user, the browser and the audio thread may do, in any order: among them, the
 * browser reporting an audio clock that is no time, or a time again, refusing resumes or
 * granting them, and a poll of the clock coming due ({@link CLOCK_POLL_MS}).
 */
const EVENTS = [
  "start",
  "stop",
  "resume",
  "suspend",
  "interrupt",
  "browser resume",
  "browser close",
  "fail",
  "advance",
  "flush",
  "loaded",
  "load fails",
  "no clock",
  "clock",
  "refuse resumes",
  "grant resumes",
  "poll",
] as const;

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

/** One of {@link EVENTS}. */
type Event = (typeof EVENTS)[number];

/** The 20 events drawn from `seed`. */
function drawn(seed: number): readonly Event[] {
  const next = random(seed);
  return Array.from({ length: 20 }, () => EVENTS[Math.floor(next() * EVENTS.length)] ?? "flush");
}

/**
 * Plays `events`, then lets the world drain, checking the promises of the player after
 * each; returns what was seen: the state and the log of the fake world after each event.
 * Every WebAssembly compilation takes `slower` more turns of the event loop.
 */
async function explore(events: readonly Event[], slower: number): Promise<string[]> {
  vi.spyOn(WebAssembly, "compile").mockImplementation(async (bytes) => {
    await turns(slower);
    return compile(bytes);
  });
  // The polls of the audio clock come due only when an event says so.
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  resetFakes();
  loads.length = 0;
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
  /** Sets the audio clock of the current context, unless it is closed. */
  const clock = (time: number): void => {
    const context = lastContext();
    if (context !== undefined && context.state !== "closed") {
      context.currentTime = time;
    }
  };
  /**
   * The context whose resume, asked while the player was suspended, settled with the context
   * not running, until that context runs again: a resume the browser refused.
   */
  let refusedBy: FakeContext | undefined;
  /** The user asks to resume: a refusal is recorded once the resume settles. */
  const resume = (): Promise<void> => {
    const asked = lastContext();
    const suspended = player.state().kind === "suspended";
    return player.resume().then(() => {
      if (suspended && asked === lastContext() && asked?.state !== "running") {
        refusedBy = asked;
      }
    });
  };
  /**
   * A context is open exactly while its session is live: only the last one, and only while
   * the player is not idle; and a resume is said refused exactly while the browser does not
   * run the context after refusing it: never while it runs, always while a refusal holds.
   */
  const check = (): void => {
    const current = player.state();
    const { kind } = current;
    expect(KINDS).toContain(kind);
    const live = lastContext();
    if (live?.state === "running" || live !== refusedBy) {
      refusedBy = undefined;
    }
    if (current.kind === "suspended" && current.refused) {
      expect(live?.state).not.toBe("running");
    }
    if (current.kind === "suspended" && refusedBy !== undefined) {
      expect(current.refused).toBe(true);
    }
    const states = FakeContext.created.map((context) => context.state);
    expect(states.slice(0, -1).filter((state) => state !== "closed")).toStrictEqual([]);
    expect(states.length === 0 || states.at(-1) === "closed").toBe(kind === "stopped" || kind === "failed");
    trace.push(`${JSON.stringify(player.state())} ${log.join(", ")}`);
  };
  for (const [step, event] of events.entries()) {
    switch (event) {
      case "start":
        watch(player.start());
        break;
      case "stop":
        watch(player.stop());
        break;
      case "resume":
        watch(resume());
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
      case "no clock":
        clock(Number.NaN);
        break;
      case "clock":
        clock(step);
        break;
      case "refuse resumes":
        FakeContext.resumesTo = "suspended";
        break;
      case "grant resumes":
        FakeContext.resumesTo = "running";
        break;
      case "poll":
        await vi.advanceTimersByTimeAsync(CLOCK_POLL_MS);
        break;
      case "flush":
        await flush();
        break;
    }
    check();
  }
  // The world drains: every load is answered, the clock is a time and runs, every poll of it
  // comes due and every pending event is delivered, a bounded number of times, after which
  // every promise has settled.
  clock(DRAINING);
  for (let round = 0; round < DRAINING; round += 1) {
    while (loads.length > 0) {
      settleLoad();
    }
    lastContext()?.advance(1);
    await vi.advanceTimersByTimeAsync(CLOCK_POLL_MS);
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
  vi.useRealTimers();
  loads.length = 0;
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("EnginePlayer under any interleaving", () => {
  test.each(SEEDS)("keeps its promises with seed %i", async (seed) => {
    await explore(drawn(seed), 0);
  }, 60_000);

  test.each(SEEDS)("explores the same interleaving with seed %i however long the engine takes to compile", async (seed) => {
    const fast = await explore(drawn(seed), 0);
    expect(await explore(drawn(seed), 30)).toStrictEqual(fast);
  }, 60_000);

  // A path the seeds rarely reach, kept as the regression the refused resume's law catches:
  // a resume refused while the clock is no time, then the browser running the context.
  test("keeps its promises when the browser runs a context whose resume it refused, its clock no time", async () => {
    const playing: readonly Event[] = ["start", "flush", "flush", "flush", "loaded", "flush"];
    const trace = await explore([...playing, "no clock", "suspend", "refuse resumes", "resume", "flush", "browser resume"], 0);
    // It plays, then its resume is refused: the path is the one it names.
    expect([trace[5]?.startsWith('{"kind":"playing"}'), trace[10]?.startsWith('{"kind":"suspended","refused":true}')]).toStrictEqual([true, true]);
  }, 60_000);

  // A path the seeds rarely reach, kept as the regression the converse law catches: a resume
  // refused while the clock is waited for, before its next read.
  test("says refused a resume the browser refuses while the context waits for its clock", async () => {
    const playing: readonly Event[] = ["start", "flush", "flush", "flush", "loaded", "flush"];
    const trace = await explore([...playing, "no clock", "suspend", "browser resume", "suspend", "refuse resumes", "resume", "flush"], 0);
    // It waits for the clock, running, then its resume is refused: the path is the one it names.
    expect([trace[8]?.startsWith('{"kind":"suspended","refused":false}'), trace[12]?.startsWith('{"kind":"suspended","refused":true}')]).toStrictEqual([true, true]);
  }, 60_000);
});
