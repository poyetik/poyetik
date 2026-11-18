// The frame of the audio clock, what an image shown with the sound follows: the rounding
// that turns the clock's time back into its frame, exact below 2^51 frames at every rate of
// the matrix, and the player's frame across its states.
import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { frameAt } from "../src/audio-clock.ts";
import { EnginePlayer } from "../src/player.ts";
import { FakeContext, flush, holdFetch, lastContext, playerOptions, resetFakes, stopPlaying } from "./fakes.ts";
import { SAMPLE_RATES } from "./matrix.ts";

/** The bound below which the frame is exact: 2^51 frames. */
const EXACT = 2 ** 51;

/**
 * `count` frames from 0 to just below 2^51, spread by a fixed linear congruential sequence
 * (seeded, so that every run tries the same ones): every power of two's range is reached.
 */
function spread(count: number): number[] {
  const frames: number[] = [];
  let state = 0x2545_f491n;
  for (let index = 0; index < count; index += 1) {
    state = (state * 6_364_136_223_846_793_005n + 1_442_695_040_888_963_407n) & 0xffff_ffff_ffff_ffffn;
    // A frame of 1 to 51 bits, so that small frames are tried as often as large ones.
    const bits = BigInt(1 + (index % 51));
    frames.push(Number((state >> 13n) & ((1n << bits) - 1n)));
  }
  return frames;
}

beforeEach(() => {
  resetFakes();
});

afterEach(() => {
  FakeContext.rate = 48_000;
});

describe("frameAt", () => {
  test("gives back the frame a browser's clock reports the time of, at every rate of the matrix, up to 2^51 frames", () => {
    for (const rate of SAMPLE_RATES) {
      const edges = [0, 1, 2, 127, 128, rate - 1, rate, rate + 1, 2 ** 31 - 1, 2 ** 32, 2 ** 32 + 1, EXACT - rate, EXACT - 1];
      for (const frame of [...edges, ...spread(2000)]) {
        // What a browser reports: the frames it rendered over its rate, in binary64.
        const time = frame / rate;
        expect(frameAt(time, rate), `${String(frame)} frames at ${String(rate)} Hz`).toBe(BigInt(frame));
      }
    }
  });

  test("stays within two frames past 2^51, up to the last integer a binary64 holds", () => {
    // N × (2^-52 + 2^-106) < 2 for N < 2^53.
    for (const rate of SAMPLE_RATES) {
      for (const frame of [EXACT, EXACT + 1, 2 ** 52 + 3, 2 ** 53 - 1]) {
        const off = frameAt(frame / rate, rate) - BigInt(frame);
        expect(off >= -2n && off <= 2n, `${String(frame)} frames at ${String(rate)} Hz: ${String(off)} off`).toBe(true);
      }
    }
  });

  test("rounds to the nearest frame, half-way up", () => {
    expect([0.49, 0.5, 1.5, 2.5, 2.51].map((time) => frameAt(time, 1))).toStrictEqual([0n, 1n, 2n, 3n, 3n]);
    expect(frameAt(1, 48_000)).toBe(48_000n);
    expect(frameAt(0.25, 44_100)).toBe(11_025n);
  });
});

describe("EnginePlayer.frame", () => {
  test("follows the audio clock of a session from its start to its end, at every rate of the matrix", async () => {
    for (const rate of SAMPLE_RATES) {
      resetFakes();
      FakeContext.rate = rate;
      const player = new EnginePlayer(playerOptions());
      expect(player.frame(), `${String(rate)} Hz: no session yet`).toBeUndefined();
      await player.start();
      const context = lastContext();
      if (context === undefined) {
        throw new Error("no context");
      }
      // The fake clock starts at 1 s.
      expect(player.frame(), `${String(rate)} Hz: playing`).toStrictEqual({ frame: BigInt(rate), rate });
      context.advance(2);
      expect(player.frame(), `${String(rate)} Hz: two seconds on`).toStrictEqual({ frame: BigInt(rate * 3), rate });
      await stopPlaying(player);
      expect(player.frame(), `${String(rate)} Hz: stopped`).toBeUndefined();
    }
  });

  test("stands still while the context is suspended, as its clock does", async () => {
    const player = new EnginePlayer(playerOptions());
    await player.start();
    const context = lastContext();
    if (context === undefined) {
      throw new Error("no context");
    }
    context.become("suspended");
    expect(player.state().kind).toBe("suspended");
    context.advance(2);
    expect(player.frame(), "the clock frozen").toStrictEqual({ frame: 48_000n, rate: 48_000 });
  });

  test("answers while starting, from the context's clock", async () => {
    const answer = holdFetch();
    const player = new EnginePlayer(playerOptions());
    const starting = player.start();
    expect(player.state().kind).toBe("starting");
    expect(player.frame()).toStrictEqual({ frame: 48_000n, rate: 48_000 });
    answer();
    await starting;
    await flush();
  });

  test("answers none while the clock is no time, then the frame again", async () => {
    const player = new EnginePlayer(playerOptions());
    await player.start();
    const context = lastContext();
    if (context === undefined) {
      throw new Error("no context");
    }
    for (const clock of [Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY]) {
      context.currentTime = clock;
      expect(player.frame(), String(clock)).toBeUndefined();
    }
    context.currentTime = 3;
    expect(player.frame()).toStrictEqual({ frame: 144_000n, rate: 48_000 });
  });
});
