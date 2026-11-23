import { loadPatch } from "@poyetik/audio-wasm/web";
import { afterEach, assert, beforeEach, describe, expect, test, vi } from "vitest";
import { SINE_440 } from "../src/patch.ts";
import { CLOCK_POLL_MS, CLOCK_READS, ENGINE_RATES, EnginePlayer, explain, missing, type Environment, type Failure, type Missing } from "../src/player.ts";
import { CAPABLE, FakeContext, READY, WASM, flush, lastContext, log, playerOptions, processor, holdFetch, resetFakes, signals, stopPlaying, stubFetch, workletNode } from "./fakes.ts";
import { SAMPLE_RATES } from "./matrix.ts";

/** Whether `text` names a lack or a failure the player explains. */
function isProblem(text: string): text is Missing | Failure {
  return ["secure-context", "web-audio", "audio-worklet", "webassembly", "load", "module", "engine", "device"].includes(text);
}

beforeEach(() => {
  resetFakes();
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  // The player's state handler throws nothing, whenever the browser calls it.
  expect(FakeContext.reported).toStrictEqual([]);
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
    expect(workletNode("poyetik-engine")?.options).toMatchObject({ outputChannelCount: [2], processorOptions: { patch: SINE_440, output: { mode: "protected", maximumLookahead: 5, detector: "precise", limiter: { ceiling: -1, lookahead: 1.5, release: 100, adaptivity: 0.5 } } } });
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

  test("fails, closing the context, on a malformed render quantum from the tap", async () => {
    const player = new EnginePlayer(playerOptions());
    await player.start();
    log.length = 0;
    workletNode("poyetik-tap")?.port.deliver("garbage");
    await flush();
    expect(player.state()).toStrictEqual({ kind: "failed", cause: "engine", detail: "MessageError: invalid message from the processor: render quantum is not an array" });
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

  test("fails on a refusal of its engine, which it never asks to set anything live", async () => {
    const player = new EnginePlayer(playerOptions());
    await player.start();
    workletNode("poyetik-engine")?.port.deliver({ type: "refused", detail: Uint8Array.from(new TextEncoder().encode("no such setting")) });
    await flush();
    expect(player.state()).toStrictEqual({ kind: "failed", cause: "engine", detail: "Error: no such setting, caused by OutputError: the engine refused the output settings" });
    expect(log.at(-1)).toBe("close");
  });

  test("fails when the engine processor cannot load the patch", async () => {
    // Built in this realm, as the structured clone of a real port would deliver it.
    processor.answer = { type: "rejected", detail: Uint8Array.from(new TextEncoder().encode("bad patch")) };
    const player = new EnginePlayer(playerOptions());
    await player.start();
    expect(player.state()).toStrictEqual({ kind: "failed", cause: "engine", detail: "Error: bad patch, caused by PatchError: the engine rejected the patch" });
    expect(log.at(-1)).toBe("close");
  });

  test("knows the arithmetic of the audio thread only once the engine has observed it", async () => {
    const player = new EnginePlayer(playerOptions());
    expect(player.arithmetic()).toBeUndefined();
    await player.start();
    expect(player.arithmetic()).toStrictEqual({ binary32: { operands: true, results: true }, binary64: { operands: true, results: true } });
    await stopPlaying(player);
    const flushed = { binary32: { operands: false, results: true }, binary64: { operands: true, results: false } };
    processor.answer = { type: "ready", subnormals: flushed };
    await player.start();
    expect(player.arithmetic()).toStrictEqual(flushed);
  });

  test("tells what the output stage replaced while playing, forgets it at the next start, and ignores a closed session's", async () => {
    const player = new EnginePlayer(playerOptions());
    expect(player.replaced()).toBeUndefined();
    await player.start();
    const changes: string[] = [];
    player.subscribe(() => changes.push(player.state().kind));
    const first = { count: 2, first: { frame: 9, channel: 1 } };
    workletNode("poyetik-engine")?.port.deliver({ type: "replaced", replaced: first });
    expect(player.replaced()).toStrictEqual(first);
    expect(changes).toStrictEqual(["playing"]);
    const more = { count: 40, first: { frame: 9, channel: 1 } };
    workletNode("poyetik-engine")?.port.deliver({ type: "replaced", replaced: more });
    expect(player.replaced()).toStrictEqual(more);
    const engine = workletNode("poyetik-engine");
    await stopPlaying(player);
    expect(player.replaced()).toStrictEqual(more);
    // A report of the session just stopped changes nothing.
    engine?.port.deliver({ type: "replaced", replaced: { count: 41, first: { frame: 9, channel: 1 } } });
    expect(player.replaced()).toStrictEqual(more);
    // Forgotten before the next start is told: its first notification carries no report.
    const told: string[] = [];
    player.subscribe(() => told.push(`${player.state().kind} ${String(player.replaced()?.count)}`));
    await player.start();
    expect(told).toStrictEqual(["starting undefined", "playing undefined"]);
    expect(player.replaced()).toBeUndefined();
  });

  test("forgets what the output stage replaced when a start fails at once, before telling the failure", async () => {
    const player = new EnginePlayer(playerOptions());
    await player.start();
    workletNode("poyetik-engine")?.port.deliver({ type: "replaced", replaced: { count: 2, first: { frame: 9, channel: 1 } } });
    await stopPlaying(player);
    FakeContext.refuses.add("create");
    const told: string[] = [];
    player.subscribe(() => told.push(`${player.state().kind} ${String(player.replaced()?.count)}`));
    await player.start();
    expect(told).toStrictEqual(["failed undefined"]);
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
    processor.answer = undefined;
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
    let answer = (): void => undefined;
    FakeContext.resuming = new Promise((resolve) => {
      answer = () => {
        resolve(undefined);
      };
    });
    const player = new EnginePlayer(playerOptions());
    const starting = player.start();
    await vi.waitFor(() => {
      expect(log).toContain("resume");
    });
    // The resume answers, and a cancel comes before the start sees the answer.
    answer();
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
    log.length = 0;
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
    log.length = 0;
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
    log.length = 0;
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
    log.length = 0;
    context?.become("closed");
    await flush();
    expect(player.state()).toStrictEqual({ kind: "failed", cause: "device", detail: "Error: the browser closed the audio device" });
    expect(log).toStrictEqual(["post stop", "disconnect poyetik-engine -> poyetik-tap"]);
  });

  test("fails on the device when the browser closes the context during the download", async () => {
    const answer = holdFetch();
    const player = new EnginePlayer(playerOptions());
    const starting = player.start();
    await vi.waitFor(() => {
      expect(log).toContain("fetch engine.wasm");
    });
    lastContext()?.become("closed");
    expect(player.state()).toStrictEqual({ kind: "failed", cause: "device", detail: "Error: the browser closed the audio device" });
    answer();
    await starting;
    await flush();
    expect(player.state()).toStrictEqual({ kind: "failed", cause: "device", detail: "Error: the browser closed the audio device" });
  });

  test("fails on the device when the browser closes the context while the engine gets ready", async () => {
    processor.answer = undefined;
    const player = new EnginePlayer(playerOptions());
    const starting = player.start();
    await vi.waitFor(() => {
      expect(workletNode("poyetik-engine")).toBeDefined();
    });
    lastContext()?.become("closed");
    await starting;
    await flush();
    expect(player.state()).toStrictEqual({ kind: "failed", cause: "device", detail: "Error: the browser closed the audio device" });
  });

  test("ignores the browser while starting: the start decides", async () => {
    processor.answer = undefined;
    const player = new EnginePlayer(playerOptions());
    const starting = player.start();
    await vi.waitFor(() => {
      expect(workletNode("poyetik-engine")).toBeDefined();
    });
    lastContext()?.become("running");
    lastContext()?.become("suspended");
    expect(player.state()).toStrictEqual({ kind: "starting" });
    workletNode("poyetik-engine")?.port.deliver(READY);
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
    log.length = 0;
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
    log.length = 0;
    // The ramp waits on the frozen clock: it plays from where the sound stopped, at the resume.
    context?.become("suspended");
    expect(log).toStrictEqual(["gain cancel from 1.5", "gain set 0 at 1.5", "gain ramp to 0.5 at 1.505"]);
    log.length = 0;
    if (by === "user") {
      await player.resume();
    } else {
      context?.become("running");
    }
    expect(player.state()).toStrictEqual({ kind: "playing" });
    expect(log.filter((entry) => entry.startsWith("gain"))).toStrictEqual([]);
  });

  test("arms no fade in on a clock the browser reports non-finite, which an AudioParam refuses: silent at once, it fades in when the context runs again", async () => {
    const player = new EnginePlayer(playerOptions(SINE_440, CAPABLE, { gain: 0.5, fade: 0.005 }));
    await player.start();
    const context = lastContext();
    log.length = 0;
    for (const clock of [Number.NaN, Number.POSITIVE_INFINITY]) {
      if (context !== undefined) {
        context.currentTime = clock;
      }
      context?.become("suspended");
      expect(FakeContext.reported).toStrictEqual([]);
      expect(player.state()).toStrictEqual({ kind: "suspended", refused: false });
      expect(log).toStrictEqual(["gain cancel from 0", "gain value 0"]);
      log.length = 0;
      if (context !== undefined) {
        context.currentTime = 3;
      }
      context?.become("running");
      expect(player.state()).toStrictEqual({ kind: "playing" });
      expect(log).toStrictEqual(["gain cancel from 3", "gain set 0 at 3", "gain ramp to 0.5 at 3.005"]);
      log.length = 0;
      // Once faded in, a run of the context fades in no more.
      context?.become("suspended");
      context?.become("running");
      expect(log).toStrictEqual(["gain cancel from 3", "gain set 0 at 3", "gain ramp to 0.5 at 3.005"]);
      log.length = 0;
    }
    expect(FakeContext.reported).toStrictEqual([]);
  });

  /** Starts `player` on a context whose clock the browser reports as `clock` once resumed: until the test makes it a time. */
  async function startOnClock(player: EnginePlayer, clock: number): Promise<{ starting: Promise<void>; context: FakeContext | undefined }> {
    let answer = (): void => undefined;
    FakeContext.resuming = new Promise((resolve) => {
      answer = () => {
        resolve(undefined);
      };
    });
    const starting = player.start();
    await vi.waitFor(() => {
      expect(log).toContain("resume");
    });
    const context = lastContext();
    if (context !== undefined) {
      context.currentTime = clock;
    }
    log.length = 0;
    answer();
    await flush();
    return { starting, context };
  }

  test("reads the audio clock 50 times at most, 20 ms apart while the timers run on time", () => {
    expect([CLOCK_POLL_MS, CLOCK_READS]).toStrictEqual([20, 50]);
  });

  test.each([Number.NaN, Number.POSITIVE_INFINITY])("starts on a running context whose clock is no time (%s) by waiting for it, never playing silent, then fades in", async (clock) => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const player = new EnginePlayer(playerOptions(SINE_440, CAPABLE, { gain: 0.5, fade: 0.005 }));
    const { starting, context } = await startOnClock(player, clock);
    // Silent at once, untimed, and still starting: the page does not say it plays.
    expect(player.state()).toStrictEqual({ kind: "starting" });
    expect(log).toStrictEqual(["gain cancel from 0", "gain value 0"]);
    log.length = 0;
    await vi.advanceTimersByTimeAsync(20 * 3);
    expect(player.state()).toStrictEqual({ kind: "starting" });
    expect(log).toStrictEqual([]);
    if (context !== undefined) {
      context.currentTime = 2;
    }
    await vi.advanceTimersByTimeAsync(20);
    await starting;
    expect(log).toStrictEqual(["gain cancel from 2", "gain set 0 at 2", "gain ramp to 0.5 at 2.005"]);
    expect(player.state()).toStrictEqual({ kind: "playing" });
    expect(vi.getTimerCount()).toBe(0);
  });

  test("fails on the device when the running context's clock stays no time for 50 reads 20 ms apart", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const player = new EnginePlayer(playerOptions());
    const { starting } = await startOnClock(player, Number.NaN);
    await vi.advanceTimersByTimeAsync(1000 - 20);
    expect(player.state()).toStrictEqual({ kind: "starting" });
    await vi.advanceTimersByTimeAsync(20);
    await starting;
    expect(player.state()).toStrictEqual({ kind: "failed", cause: "device", detail: "Error: the browser's audio clock is not a time" });
    expect(lastContext()?.state).toBe("closed");
    expect(log.filter((entry) => entry.startsWith("gain "))).toStrictEqual(["gain cancel from 0", "gain value 0"]);
  });

  test("cancels a start waiting for the audio clock: stops at once, and never fades in", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const player = new EnginePlayer(playerOptions());
    const { starting, context } = await startOnClock(player, Number.NaN);
    await player.stop();
    await starting;
    expect(player.state()).toStrictEqual({ kind: "stopped" });
    if (context !== undefined) {
      context.currentTime = 2;
    }
    await vi.advanceTimersByTimeAsync(1000);
    expect(log.filter((entry) => entry.startsWith("gain "))).toStrictEqual(["gain cancel from 0", "gain value 0"]);
    expect(player.state()).toStrictEqual({ kind: "stopped" });
  });

  test("starts suspended on a suspended context whose clock is no time, waiting for nothing: it fades in when the context runs", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    FakeContext.resumesTo = "suspended";
    const player = new EnginePlayer(playerOptions());
    const { starting, context } = await startOnClock(player, Number.NaN);
    await starting;
    expect(player.state()).toStrictEqual({ kind: "suspended", refused: false });
    expect(vi.getTimerCount()).toBe(0);
    if (context !== undefined) {
      context.currentTime = 2;
    }
    context?.become("running");
    expect(player.state()).toStrictEqual({ kind: "playing" });
    expect(log.filter((entry) => entry.startsWith("gain "))).toStrictEqual(["gain cancel from 0", "gain value 0", "gain cancel from 2", "gain set 0 at 2", "gain ramp to 0.25 at 2.02"]);
  });

  test.each(["user", "browser"] as const)("stays paused while a context resumed by the %s runs on a clock that is no time, then plays once it fades in", async (by) => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const player = new EnginePlayer(playerOptions());
    await player.start();
    const context = lastContext();
    if (context !== undefined) {
      context.currentTime = Number.NaN;
    }
    context?.become("suspended");
    log.length = 0;
    if (by === "user") {
      await player.resume();
    } else {
      context?.become("running");
    }
    // Waiting for the clock is no refusal of the browser, and no sound.
    expect(player.state()).toStrictEqual({ kind: "suspended", refused: false });
    // A second run of the context reported meanwhile waits on the same reads.
    context?.become("running");
    expect(vi.getTimerCount()).toBe(1);
    await vi.advanceTimersByTimeAsync(20);
    expect(player.state()).toStrictEqual({ kind: "suspended", refused: false });
    if (context !== undefined) {
      context.currentTime = 4;
    }
    await vi.advanceTimersByTimeAsync(20);
    expect(player.state()).toStrictEqual({ kind: "playing" });
    expect(log.filter((entry) => entry.startsWith("gain "))).toStrictEqual(["gain cancel from 4", "gain set 0 at 4", "gain ramp to 0.25 at 4.02"]);
    expect(vi.getTimerCount()).toBe(0);
  });

  test("leaves a refused resume's state once the browser runs the context waiting for its clock, and says so", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const player = new EnginePlayer(playerOptions());
    await player.start();
    const context = lastContext();
    if (context !== undefined) {
      context.currentTime = Number.NaN;
    }
    context?.become("suspended");
    FakeContext.resumesTo = "suspended";
    await player.resume();
    expect(player.state()).toStrictEqual({ kind: "suspended", refused: true });
    const seen: unknown[] = [];
    player.subscribe(() => seen.push(player.state()));
    FakeContext.resumesTo = "running";
    await player.resume();
    // The browser no longer refuses: the page says so, once, while the tone waits silent.
    expect(player.state()).toStrictEqual({ kind: "suspended", refused: false });
    expect(seen).toStrictEqual([{ kind: "suspended", refused: false }]);
    if (context !== undefined) {
      context.currentTime = 3;
    }
    await vi.advanceTimersByTimeAsync(20);
    expect(player.state()).toStrictEqual({ kind: "playing" });
  });

  test("stops when asked while a resume the browser refuses is answered, never said refused", async () => {
    const player = new EnginePlayer(playerOptions());
    await player.start();
    lastContext()?.become("suspended");
    FakeContext.resumesTo = "suspended";
    const seen: unknown[] = [];
    player.subscribe(() => seen.push(player.state()));
    // Stop is pressed before the browser's answer reaches the player.
    const resuming = player.resume();
    const stopping = player.stop();
    await Promise.all([resuming, stopping]);
    expect(player.state()).toStrictEqual({ kind: "stopped" });
    expect(seen).toStrictEqual([{ kind: "stopping" }, { kind: "stopped" }]);
  });

  test("says a resume refused while the context waits for its clock is refused, before the next read", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const player = new EnginePlayer(playerOptions());
    await player.start();
    const context = lastContext();
    if (context !== undefined) {
      context.currentTime = Number.NaN;
    }
    context?.become("suspended");
    context?.become("running");
    // The browser suspends the context while its clock is waited for: the read is still due.
    context?.become("suspended");
    expect(vi.getTimerCount()).toBe(1);
    const seen: unknown[] = [];
    player.subscribe(() => seen.push(player.state()));
    FakeContext.resumesTo = "suspended";
    await player.resume();
    expect(player.state()).toStrictEqual({ kind: "suspended", refused: true });
    expect(seen).toStrictEqual([{ kind: "suspended", refused: true }]);
    // The read comes due and changes nothing: the context still does not run.
    await vi.advanceTimersByTimeAsync(20);
    expect(player.state()).toStrictEqual({ kind: "suspended", refused: true });
    expect(vi.getTimerCount()).toBe(0);
  });

  test("fails on the device when a resumed context's clock stays no time for 50 reads 20 ms apart", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const player = new EnginePlayer(playerOptions());
    await player.start();
    const context = lastContext();
    if (context !== undefined) {
      context.currentTime = Number.NaN;
    }
    context?.become("suspended");
    context?.become("running");
    await vi.advanceTimersByTimeAsync(1000 - 20);
    expect(player.state()).toStrictEqual({ kind: "suspended", refused: false });
    await vi.advanceTimersByTimeAsync(20);
    expect(player.state()).toStrictEqual({ kind: "failed", cause: "device", detail: "Error: the browser's audio clock is not a time" });
  });

  test("goes on counting the reads of a wait the context leaves and takes up again within one read's pause", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const player = new EnginePlayer(playerOptions());
    await player.start();
    const context = lastContext();
    if (context !== undefined) {
      context.currentTime = Number.NaN;
    }
    context?.become("suspended");
    context?.become("running");
    await vi.advanceTimersByTimeAsync(20 * 30);
    // Suspended and run again before the next read: the same wait, and the same count.
    context?.become("suspended");
    context?.become("running");
    expect(vi.getTimerCount()).toBe(1);
    await vi.advanceTimersByTimeAsync(20 * 20 - 20);
    expect(player.state()).toStrictEqual({ kind: "suspended", refused: false });
    await vi.advanceTimersByTimeAsync(20);
    expect(player.state()).toStrictEqual({ kind: "failed", cause: "device", detail: "Error: the browser's audio clock is not a time" });
  });

  test("stays paused when the context waiting for its clock is suspended again, and waits anew at its next run", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const player = new EnginePlayer(playerOptions());
    await player.start();
    const context = lastContext();
    if (context !== undefined) {
      context.currentTime = Number.NaN;
    }
    context?.become("suspended");
    context?.become("running");
    await vi.advanceTimersByTimeAsync(20 * 10);
    context?.become("suspended");
    await vi.advanceTimersByTimeAsync(20);
    expect(vi.getTimerCount()).toBe(0);
    expect(player.state()).toStrictEqual({ kind: "suspended", refused: false });
    // 50 reads again from the next run.
    context?.become("running");
    await vi.advanceTimersByTimeAsync(1000 - 20);
    expect(player.state()).toStrictEqual({ kind: "suspended", refused: false });
    if (context !== undefined) {
      context.currentTime = 5;
    }
    await vi.advanceTimersByTimeAsync(20);
    expect(player.state()).toStrictEqual({ kind: "playing" });
  });

  test("stops a context waiting for its clock at once, silent, and never fades it in", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const player = new EnginePlayer(playerOptions());
    await player.start();
    const context = lastContext();
    if (context !== undefined) {
      context.currentTime = Number.NaN;
    }
    context?.become("suspended");
    context?.become("running");
    log.length = 0;
    await player.stop();
    expect(player.state()).toStrictEqual({ kind: "stopped" });
    if (context !== undefined) {
      context.currentTime = 2;
    }
    await vi.advanceTimersByTimeAsync(1000);
    // Silent since the fade in was due: nothing more scheduled, before or after the stop.
    expect(log.filter((entry) => entry.startsWith("gain "))).toStrictEqual([]);
    expect(log.at(-1)).toBe("close");
  });

  test("stops a context whose clock became a time while it waited for it, fading out, never playing", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const player = new EnginePlayer(playerOptions());
    await player.start();
    const context = lastContext();
    if (context !== undefined) {
      context.currentTime = Number.NaN;
    }
    context?.become("suspended");
    context?.become("running");
    if (context !== undefined) {
      context.currentTime = 2;
    }
    log.length = 0;
    const stopping = player.stop();
    // The next read finds the fade out scheduled: no fade in is due any more.
    await vi.advanceTimersByTimeAsync(20);
    expect(player.state()).toStrictEqual({ kind: "stopping" });
    context?.advance(1);
    await stopping;
    expect(player.state()).toStrictEqual({ kind: "stopped" });
    expect(log.filter((entry) => entry.startsWith("gain "))).toStrictEqual(["gain cancel from 2", "gain set 0 at 2", "gain ramp to 0 at 2.02"]);
  });

  test.each([Number.NaN, Number.NEGATIVE_INFINITY])("stops a playing context whose clock is no time (%s): silent at once, untimed, stopped, never a failure", async (clock) => {
    const player = new EnginePlayer(playerOptions());
    await player.start();
    const context = lastContext();
    if (context !== undefined) {
      context.currentTime = clock;
    }
    log.length = 0;
    await player.stop();
    expect(player.state()).toStrictEqual({ kind: "stopped" });
    expect(log).toStrictEqual(["gain cancel from 0", "gain value 0", "post stop", "disconnect poyetik-engine -> poyetik-tap", "close"]);
  });

  test("fades in once at the start, a run of the context reported while it starts arming nothing more", async () => {
    let answer = (): void => undefined;
    FakeContext.resuming = new Promise((resolve) => {
      answer = () => {
        resolve(undefined);
      };
    });
    const player = new EnginePlayer(playerOptions(SINE_440, CAPABLE, { gain: 0.5, fade: 0.005 }));
    const starting = player.start();
    await vi.waitFor(() => {
      expect(log).toContain("resume");
    });
    // The browser runs the context, and says so, before the resume answers.
    lastContext()?.become("running");
    expect(log.filter((entry) => entry.startsWith("gain"))).toStrictEqual([]);
    answer();
    await starting;
    expect(player.state()).toStrictEqual({ kind: "playing" });
    expect(log.filter((entry) => entry.startsWith("gain "))).toStrictEqual(["gain cancel from 1", "gain set 0 at 1", "gain ramp to 0.5 at 1.005"]);
  });

  test("schedules no fade in when suspended while stopping", async () => {
    const player = new EnginePlayer(playerOptions());
    await player.start();
    const stopping = player.stop();
    log.length = 0;
    lastContext()?.become("suspended");
    await stopping;
    expect(log.filter((entry) => entry.startsWith("gain"))).toStrictEqual([]);
  });

  test("resumes only when suspended", async () => {
    const player = new EnginePlayer(playerOptions());
    await player.resume();
    await player.start();
    log.length = 0;
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
    log.length = 0;
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
    expect(player.state()).toStrictEqual({ kind: "failed", cause: "engine", detail: "Error: invalid patch: missing field `format` at line 1 column 2, caused by PatchError: the engine rejected the patch" });
    expect(log.at(-1)).toBe("close");
  });

  test("does not leave the start state when a failure came first", async () => {
    // The engine answers nothing: the tap fails before the start completes.
    processor.answer = undefined;
    const player = new EnginePlayer(playerOptions());
    const starting = player.start();
    await vi.waitFor(() => {
      expect(workletNode("poyetik-tap")).toBeDefined();
    });
    workletNode("poyetik-tap")?.port.deliver("garbage");
    workletNode("poyetik-engine")?.port.deliver(READY);
    await starting;
    expect(player.state()).toStrictEqual({ kind: "failed", cause: "engine", detail: "MessageError: invalid message from the processor: render quantum is not an array" });
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
    log.length = 0;
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

  test("fails on a download of bytes that are no module, a load, never cached: the next start downloads again and plays", async () => {
    for (const bytes of [new Uint8Array([1, 2, 3]), new TextEncoder().encode("<!doctype html><title>Not found</title>")]) {
      resetFakes();
      vi.stubGlobal("fetch", (url: string) => {
        log.push(`fetch ${url}`);
        return Promise.resolve({ ok: true, status: 200, arrayBuffer: () => Promise.resolve(bytes.slice().buffer) });
      });
      const player = new EnginePlayer(playerOptions());
      await player.start();
      expect(player.state()).toStrictEqual({ kind: "failed", cause: "load", detail: "Error: cannot load the engine from engine.wasm (not a WebAssembly module)" });
      stubFetch();
      await player.start();
      expect(player.state()).toStrictEqual({ kind: "playing" });
      expect(log.filter((entry) => entry.startsWith("fetch"))).toStrictEqual(["fetch engine.wasm", "fetch engine.wasm"]);
    }
  });

  test("fails on a module this browser cannot compile, its environment, forgetting its bytes: the next start downloads again", async () => {
    vi.stubGlobal("fetch", (url: string) => {
      log.push(`fetch ${url}`);
      return Promise.resolve({ ok: true, status: 200, arrayBuffer: () => Promise.resolve(new Uint8Array([0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0xff]).buffer) });
    });
    const player = new EnginePlayer(playerOptions());
    await player.start();
    const failed = player.state();
    assert(failed.kind === "failed", "failed");
    expect(failed.cause).toBe("module");
    expect(failed.detail).toMatch(/^ModuleError: not an engine module: cannot be compiled: CompileError: /u);
    stubFetch();
    await player.start();
    expect(player.state()).toStrictEqual({ kind: "playing" });
    expect(log.filter((entry) => entry.startsWith("fetch"))).toStrictEqual(["fetch engine.wasm", "fetch engine.wasm"]);
  });

  test("fails on a download the network refuses, naming what it could not load", async () => {
    vi.stubGlobal("fetch", () => Promise.reject(new TypeError("Failed to fetch")));
    const player = new EnginePlayer(playerOptions());
    await player.start();
    expect(player.state()).toStrictEqual({ kind: "failed", cause: "load", detail: "Error: cannot load the engine from engine.wasm, caused by TypeError: Failed to fetch" });
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
      await expect(loadPatch(wasm(), SINE_440, rate)).rejects.toThrow(new Error(`invalid sample rate: ${String(rate)} Hz (supported: 8000 to 384000 Hz)`));
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
      load: "Could not load the audio engine. Press Start to try again; if it fails again, check your connection.",
      module: "This browser could not compile the audio engine. Press Start to try again; if it fails again, try another browser or device.",
      engine: "The audio engine stopped on an error. Press Start to try again; if it happens again, its details say what failed.",
      device: "The browser could not play audio on this device: check the sound output. Press Start to try again; if it fails again, try another browser or device.",
    };
    for (const [problem, text] of Object.entries(expected)) {
      expect(isProblem(problem) ? explain(problem) : undefined).toBe(text);
    }
    expect(new Set(Object.values(expected)).size).toBe(Object.keys(expected).length);
  });
});

describe("the fake context", () => {
  test("closes as browsers do (measured in Chrome 157 and Firefox 134, 2026-11-12, and as the Web Audio specification queues it): its promise settles, then statechange fires in a later task, and a second close rejects", async () => {
    const context = new FakeContext();
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
