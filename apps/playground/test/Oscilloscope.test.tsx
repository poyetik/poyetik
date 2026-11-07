import { act, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";
import type { ScopeCanvas } from "../src/draw.ts";
import type { Signal } from "../src/player.ts";
import { HEIGHT, Oscilloscope, WIDTH, WINDOW_SECONDS, animationFrames } from "../src/Oscilloscope.tsx";

/** A canvas counting the points drawn. */
function counter(): { canvas: ScopeCanvas; clears: number[][]; points: () => number } {
  let lines = 0;
  const clears: number[][] = [];
  const canvas: ScopeCanvas = {
    strokeStyle: "",
    lineWidth: 0,
    clearRect: (...args) => clears.push(args),
    beginPath: () => undefined,
    moveTo: () => undefined,
    lineTo: () => {
      lines += 1;
    },
    stroke: () => undefined,
  };
  return { canvas, clears, points: () => lines };
}

/** A scheduler driven by the test: `tick` runs one frame. */
function manual(): { schedule: (frame: () => void) => () => void; tick: () => void; cancelled: () => number } {
  let current: (() => void) | undefined;
  let cancelled = 0;
  return {
    schedule: (frame) => {
      current = frame;
      return () => {
        cancelled += 1;
      };
    },
    tick: () => {
      act(() => {
        current?.();
      });
    },
    cancelled: () => cancelled,
  };
}

/** A 1 kHz square wave sampled at 4 kHz, one second long. */
const SQUARE: Signal = { samples: Float32Array.from({ length: 4000 }, (_, index) => (index % 4 < 2 ? 1 : -1)), rate: 4000 };

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("Oscilloscope", () => {
  test("shows dashes before the first frame", () => {
    const { schedule } = manual();
    render(<Oscilloscope signal={() => SQUARE} context={() => counter().canvas} schedule={schedule} />);
    expect(screen.getByTestId("frequency").textContent).toBe("—");
    expect(screen.getByTestId("peak").textContent).toBe("—");
    expect(screen.getByTestId("rate").textContent).toBe("—");
    expect(screen.getByRole("img", { name: "Waveform of the engine output" })).toHaveProperty("width", WIDTH);
    expect(screen.getByRole("img")).toHaveProperty("height", HEIGHT);
    expect(screen.getByRole("region", { name: "Oscilloscope" })).toBeDefined();
  });

  test("draws the signal and shows its measurements at every frame", () => {
    const { schedule, tick } = manual();
    const drawn = counter();
    render(<Oscilloscope signal={() => SQUARE} context={() => drawn.canvas} schedule={schedule} />);
    tick();
    expect(drawn.clears).toStrictEqual([[0, 0, WIDTH, HEIGHT]]);
    // The grid (3 lines), then 10 ms at 4 kHz = 40 samples: 39 segments.
    expect(drawn.points()).toBe(3 + 39);
    expect(screen.getByTestId("frequency").textContent).toBe("1000.0 Hz");
    expect(screen.getByTestId("peak").textContent).toBe("1.00");
    expect(screen.getByTestId("rate").textContent).toBe("4000 Hz");
    tick();
    expect(drawn.clears).toHaveLength(2);
  });

  test("draws only the grid and shows dashes with no signal", () => {
    const { schedule, tick } = manual();
    const drawn = counter();
    render(<Oscilloscope signal={() => undefined} context={() => drawn.canvas} schedule={schedule} />);
    tick();
    expect(drawn.points()).toBe(3);
    expect(screen.getByTestId("frequency").textContent).toBe("—");
    expect(screen.getByTestId("rate").textContent).toBe("—");
  });

  test("still measures without a drawing context", () => {
    const { schedule, tick } = manual();
    render(<Oscilloscope signal={() => SQUARE} context={() => null} schedule={schedule} />);
    tick();
    expect(screen.getByTestId("peak").textContent).toBe("1.00");
  });

  test("stops its frames when unmounted", () => {
    const { schedule, cancelled } = manual();
    const { unmount } = render(<Oscilloscope signal={() => SQUARE} context={() => null} schedule={schedule} />);
    const before = cancelled();
    unmount();
    expect(cancelled()).toBe(before + 1);
  });

  test("uses the 2D context of its canvas by default", () => {
    const getContext = vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
    const { schedule } = manual();
    render(<Oscilloscope signal={() => SQUARE} schedule={schedule} />);
    expect(getContext).toHaveBeenCalledWith("2d");
  });

  test("shows about four periods of 440 Hz", () => {
    expect(WINDOW_SECONDS).toBe(0.01);
    expect(WIDTH / HEIGHT).toBe(3);
  });
});

describe("animationFrames", () => {
  test("runs the frame before every repaint until cancelled", () => {
    const pending = new Map<number, FrameRequestCallback>();
    let next = 0;
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      next += 1;
      pending.set(next, callback);
      return next;
    });
    vi.stubGlobal("cancelAnimationFrame", (id: number) => {
      pending.delete(id);
    });
    const run = (): void => {
      const [entry] = pending;
      if (entry !== undefined) {
        pending.delete(entry[0]);
        entry[1](0);
      }
    };
    let frames = 0;
    const cancel = animationFrames(() => {
      frames += 1;
    });
    expect(frames).toBe(0);
    run();
    run();
    expect(frames).toBe(2);
    expect([...pending.keys()]).toStrictEqual([3]);
    cancel();
    expect(pending.size).toBe(0);
  });

  test("is the default scheduler", () => {
    const request = vi.fn(() => 7);
    const cancel = vi.fn();
    vi.stubGlobal("requestAnimationFrame", request);
    vi.stubGlobal("cancelAnimationFrame", cancel);
    const { unmount } = render(<Oscilloscope signal={() => undefined} context={() => null} />);
    expect(request).toHaveBeenCalled();
    unmount();
    expect(cancel).toHaveBeenCalledWith(7);
  });
});
