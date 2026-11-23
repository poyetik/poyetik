import { afterEach, describe, expect, test, vi } from "vitest";
import { PAGE_CLOCK, renderBlocks, runsOf, timeEach, timeRuns, timed, type Clock } from "../e2e/site/timing.ts";

/** A fake clock that advances by `step` milliseconds at each reading, from 100 (not 0, so that a sum is not a difference). */
function steps(step: number): Clock & { readings: number } {
  let now = 100 - step;
  const clock = Object.assign(
    (): number => {
      clock.readings += 1;
      now += step;
      return now;
    },
    { readings: 0 },
  );
  return clock;
}

/** A fake engine of `channels` channels and blocks of `blockSize` frames (64 unless given) that records the outputs of every render. */
function engine(
  channels: number,
  blockSize = 64,
): { readonly channels: number; readonly blockSize: number; readonly rendered: (readonly Float32Array[])[]; render: (outputs: Float32Array[]) => void } {
  const rendered: (readonly Float32Array[])[] = [];
  return { channels, blockSize, rendered, render: (outputs) => rendered.push([...outputs]) };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("the bench page's timing", () => {
  test("reads the number of runs from the address: one when absent, else a positive whole number", () => {
    expect(runsOf("")).toBe(1);
    expect(runsOf("?case=sine-440")).toBe(1);
    expect(runsOf("?case=sine-440&runs=20")).toBe(20);
    expect(runsOf("?runs=1")).toBe(1);
    for (const runs of ["abc", "0", "-1", "1.5", "", "1e3", " 2", "0x10"]) {
      expect(() => runsOf(`?runs=${runs}`)).toThrow(new RangeError(`runs=${runs} is not a positive whole number`));
    }
  });

  test("renders a number of 64-frame blocks in 128-frame quanta, into one output per channel and at least one", () => {
    for (const [channels, outputs] of [
      [0, 1],
      [1, 1],
      [2, 2],
    ] as const) {
      const fake = engine(channels);
      renderBlocks(fake, 8);
      expect(fake.rendered).toHaveLength(4);
      expect(fake.rendered.map((rendered) => rendered.map((output) => output.length))).toStrictEqual(Array.from({ length: 4 }, () => Array.from({ length: outputs }, () => 128)));
      expect(new Set(fake.rendered.flat()).size).toBe(outputs);
    }
    const one = engine(1);
    renderBlocks(one, 2);
    expect(one.rendered).toHaveLength(1);
    const none = engine(1);
    renderBlocks(none, 0);
    expect(none.rendered).toHaveLength(0);
    for (const blocks of [1, 7, -2, 2.5]) {
      expect(() => { renderBlocks(engine(1), blocks); }).toThrow(new RangeError(`${String(blocks)} blocks of 64 frames are not a whole number of 128-frame quanta`));
    }
  });

  test("renders the frames of a number of the engine's own blocks, never of an engine block it assumes", () => {
    for (const [blockSize, blocks, quanta] of [
      [32, 16, 4],
      [64, 16, 8],
      [128, 16, 16],
      [256, 16, 32],
      [48, 8, 3],
      [1, 384, 3],
    ] as const) {
      const fake = engine(2, blockSize);
      renderBlocks(fake, blocks);
      expect(fake.rendered).toHaveLength(quanta);
    }
    for (const [blockSize, blocks] of [
      [48, 1],
      [32, 3],
      [256, 0.25],
      [1, 127],
    ] as const) {
      expect(() => {
        renderBlocks(engine(1, blockSize), blocks);
      }).toThrow(new RangeError(`${String(blocks)} blocks of ${String(blockSize)} frames are not a whole number of 128-frame quanta`));
    }
  });

  test("times an action by the clock read before and after it", () => {
    const clock = steps(5);
    let ran = 0;
    expect(
      timed(clock, () => {
        ran += 1;
      }),
    ).toBe(5);
    expect([ran, clock.readings]).toStrictEqual([1, 2]);
  });

  test("times each run after one warm-up run that is not counted", () => {
    const times = [9, 1, 2, 3];
    let calls = 0;
    expect(
      timeRuns(3, () => {
        calls += 1;
        return times[calls - 1] ?? -1;
      }),
    ).toStrictEqual([1, 2, 3]);
    expect(calls).toBe(4);
  });

  test("times each awaited run from its start to its end, and only then hands its result on", async () => {
    const clock = steps(2);
    const order: string[] = [];
    let started = 0;
    const times = await timeEach(
      3,
      clock,
      async () => {
        started += 1;
        order.push(`start ${String(started)}`);
        await Promise.resolve();
        return started;
      },
      (result) => order.push(`after ${String(result)} at ${String(clock.readings)}`),
    );
    expect(times).toStrictEqual([2, 2, 2]);
    expect(order).toStrictEqual(["start 1", "after 1 at 2", "start 2", "after 2 at 4", "start 3", "after 3 at 6"]);
    await expect(timeEach(0, clock, () => Promise.resolve(0), () => undefined)).resolves.toStrictEqual([]);
    await expect(timeEach(1, clock, () => Promise.reject(new Error("no ready")), () => undefined)).rejects.toThrow(new Error("no ready"));
  });

  test("reads the page's clock by default", () => {
    const fake = { now: () => 42.5 };
    vi.stubGlobal("performance", fake);
    expect(globalThis.performance).toBe(fake);
    expect(PAGE_CLOCK()).toBe(42.5);
  });
});
