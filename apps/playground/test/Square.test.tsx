import { act, render, screen } from "@testing-library/react";
import axe from "axe-core";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import type { AudioFrame } from "../src/player.ts";
import { SQUARE } from "../src/settings.ts";
import { DRAWN_BY, FAILED, MOVING, STILL, Square, type SquareCanvas, type SquareImages } from "../src/Square.tsx";
import { FakeCanvas, FakeDevice, fakeGpu, gpuLog } from "./gpu-fakes.ts";
import { FakeMediaQuery, media } from "./media.ts";

/** An `ImageData` for jsdom, which has none: the bytes it was given, and its size. */
class FakeImageData {
  /** The pixels. */
  readonly data: Uint8ClampedArray;
  /** Width, in pixels. */
  readonly width: number;
  /** Height, in pixels. */
  readonly height: number;

  /** Holds `data` as an `ImageData` of `width` × `height`. */
  constructor(data: Uint8ClampedArray, width: number, height: number) {
    this.data = data;
    this.width = width;
    this.height = height;
  }
}

/** Every image a canvas received: its first byte (the image's index, in the fake module), its size and where. */
type Put = readonly [number | undefined, number, number, number, number];

/** A canvas recording the images put on it. */
function canvas(): { context: SquareCanvas; puts: Put[] } {
  const puts: Put[] = [];
  return {
    context: {
      putImageData: (image: ImageData, x: number, y: number) => {
        puts.push([image.data[0], image.width, image.height, x, y]);
      },
    },
    puts,
  };
}

/**
 * A page's module: frame `f` at rate `r` shows image ⌊f × 60 / r⌋ (as `page_image_at`
 * answers for 60 images a second), and image `i` displays as bytes whose first is `i`.
 * Records every call.
 */
function module(): { images: SquareImages; calls: string[] } {
  const calls: string[] = [];
  return {
    images: {
      placement: (index) => [Number(index), 0, 0, 0],
      shader: (kind) => kind,
      thresholds: () => new Uint32Array(510),
      imageAt: (frame, sampleRate, rate) => {
        calls.push(`imageAt ${String(frame)} ${String(sampleRate)} ${rate.join("/")}`);
        return (frame * BigInt(rate[0])) / (BigInt(sampleRate) * BigInt(rate[1]));
      },
      display: (index) => {
        calls.push(`display ${String(index)}`);
        const bytes = new Uint8ClampedArray(SQUARE.width * SQUARE.height * 4);
        bytes[0] = Number(index);
        return bytes;
      },
    },
    calls,
  };
}

/** A scheduler driven by the test: `tick` runs one repaint; counts the schedules and cancels. */
function manual(): { schedule: (frame: () => void) => () => void; tick: () => void; scheduled: () => number; cancelled: () => number } {
  let current: (() => void) | undefined;
  let scheduled = 0;
  let cancelled = 0;
  return {
    schedule: (frame) => {
      current = frame;
      scheduled += 1;
      return () => {
        current = undefined;
        cancelled += 1;
      };
    },
    tick: () => {
      act(() => {
        current?.();
      });
    },
    scheduled: () => scheduled,
    cancelled: () => cancelled,
  };
}

/** The audio clock of a test: the frame it answers, set by the test. */
function clock(): { frame: () => AudioFrame | undefined; at: (frame: bigint | undefined, rate?: number) => void } {
  let now: AudioFrame | undefined;
  return {
    frame: () => now,
    at: (frame, rate = 48_000) => {
      now = frame === undefined ? undefined : { frame, rate };
    },
  };
}

/** Lets the module's promise settle and React show it. */
async function settle(): Promise<void> {
  await act(async () => {
    await Promise.resolve();
  });
}

/** The line saying what the square shows. */
function says(): string | null {
  return screen.getByRole("img").nextElementSibling?.textContent ?? null;
}

/** The violations axe finds in the page as it is. */
async function violations(): Promise<string[]> {
  const results = await axe.run(document.body);
  return results.violations.map((violation) => violation.id);
}

beforeEach(() => {
  vi.stubGlobal("ImageData", FakeImageData);
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("Square", () => {
  test("says it loads, then shows the first image once while the sound does not play", async () => {
    const { images, calls } = module();
    const { context, puts } = canvas();
    const { schedule, scheduled } = manual();
    const time = clock();
    render(<Square images={() => Promise.resolve(images)} state="stopped" frame={time.frame} context={() => context} schedule={schedule} />);
    expect(says()).toBe("Loading the square…");
    expect(screen.queryByTestId("drawn-by"), "nothing drawn yet").toBeNull();
    expect(await violations(), "loading").toStrictEqual([]);
    await settle();
    expect(says()).toBe("The square sweeps across the image in step with the sound, one sweep every two seconds, on the audio clock. Still until the sound starts.");
    expect(calls).toStrictEqual(["display 0"]);
    expect(puts).toStrictEqual([[0, SQUARE.width, SQUARE.height, 0, 0]]);
    expect(scheduled(), "no repaint while the sound does not play").toBe(0);
    expect(await violations(), "shown").toStrictEqual([]);
  });

  test("describes the canvas: an image, named, described by the line, of the square's size", () => {
    render(<Square images={() => new Promise(() => undefined)} state="stopped" frame={() => undefined} />);
    const image = screen.getByRole("img", { name: "A white square on black, sweeping from left to right" });
    expect([image.getAttribute("width"), image.getAttribute("height")]).toStrictEqual([String(SQUARE.width), String(SQUARE.height)]);
    expect(document.getElementById(image.getAttribute("aria-describedby") ?? "")).toBe(image.nextElementSibling);
    expect(screen.getByRole("region", { name: "Square" }).contains(image)).toBe(true);
  });

  test("follows the audio clock while the sound plays, a new image only when the index changes", async () => {
    const { images, calls } = module();
    const { context, puts } = canvas();
    const { schedule, tick, scheduled, cancelled } = manual();
    const time = clock();
    time.at(0n, 44_100);
    const { rerender } = render(<Square images={() => Promise.resolve(images)} state="playing" frame={time.frame} context={() => context} schedule={schedule} />);
    await settle();
    expect(scheduled()).toBe(1);
    tick();
    // 735 frames an image at 44.1 kHz and 60 images a second.
    time.at(734n, 44_100);
    tick();
    time.at(735n, 44_100);
    tick();
    time.at(7350n, 44_100);
    tick();
    expect(calls).toStrictEqual(["imageAt 0 44100 60/1", "display 0", "imageAt 734 44100 60/1", "imageAt 735 44100 60/1", "display 1", "imageAt 7350 44100 60/1", "display 10"]);
    expect(puts.map(([first]) => first)).toStrictEqual([0, 1, 10]);
    // The sound stops: no more repaints, one image drawn while it fades out (its clock still
    // running), then the first between sessions.
    rerender(<Square images={() => Promise.resolve(images)} state="stopping" frame={time.frame} context={() => context} schedule={schedule} />);
    await settle();
    expect(cancelled()).toBe(1);
    expect(puts.at(-1)?.[0]).toBe(10);
    time.at(undefined);
    rerender(<Square images={() => Promise.resolve(images)} state="stopped" frame={time.frame} context={() => context} schedule={schedule} />);
    await settle();
    expect([puts.at(-1)?.[0], scheduled()]).toStrictEqual([0, 1]);
  });

  test("shows the frozen image of a suspended clock, once", async () => {
    const { images } = module();
    const { context, puts } = canvas();
    const time = clock();
    time.at(96_000n);
    render(<Square images={() => Promise.resolve(images)} state="suspended" frame={time.frame} context={() => context} schedule={manual().schedule} />);
    await settle();
    expect(puts.map(([first]) => first)).toStrictEqual([120]);
  });

  test("stays still when the system asks for reduced motion, says so, and follows the system's changes", async () => {
    media.list.become(true);
    const { images, calls } = module();
    const { context, puts } = canvas();
    const { schedule, scheduled, cancelled, tick } = manual();
    const time = clock();
    time.at(48_000n);
    render(<Square images={() => Promise.resolve(images)} state="playing" frame={time.frame} context={() => context} schedule={schedule} />);
    await settle();
    expect(says()).toBe("The square stays still: your system asks for reduced motion.");
    expect(calls, "the first image, whatever the clock").toStrictEqual(["display 0"]);
    expect(scheduled(), "no repaint").toBe(0);
    expect(await violations(), "still").toStrictEqual([]);
    act(() => {
      media.list.become(false);
    });
    expect(says()).toBe(MOVING);
    expect(scheduled()).toBe(1);
    tick();
    expect(puts.map(([first]) => first)).toStrictEqual([0, 60]);
    act(() => {
      media.list.become(true);
    });
    expect([says(), cancelled()]).toStrictEqual([STILL, 1]);
    expect(FakeMediaQuery.asked).toStrictEqual(["(prefers-reduced-motion: reduce)"]);
  });

  test("loads the page's module once, when its canvas exists", async () => {
    let loads = 0;
    const { images } = module();
    const load = (): Promise<SquareImages> => {
      loads += 1;
      return Promise.resolve(images);
    };
    const { rerender } = render(<Square images={load} state="stopped" frame={() => undefined} schedule={manual().schedule} />);
    await settle();
    rerender(<Square images={load} state="playing" frame={() => undefined} schedule={manual().schedule} />);
    await settle();
    expect(loads).toBe(1);
  });

  test("loads the module it is given anew, never showing the last one's late answer", async () => {
    for (const late of ["resolves", "rejects"] as const) {
      const answers: { resolve: (images: SquareImages) => void; reject: (error: Error) => void }[] = [];
      const slow = (): Promise<SquareImages> =>
        new Promise((resolve, reject) => {
          answers.push({ resolve, reject });
        });
      const old = module();
      const current = module();
      const { context } = canvas();
      const { rerender, unmount } = render(<Square images={slow} state="stopped" frame={() => undefined} context={() => context} schedule={manual().schedule} />);
      await settle();
      rerender(<Square images={() => Promise.resolve(current.images)} state="stopped" frame={() => undefined} context={() => context} schedule={manual().schedule} />);
      await settle();
      if (late === "resolves") {
        answers[0]?.resolve(old.images);
      } else {
        answers[0]?.reject(new Error("late"));
      }
      await settle();
      expect([old.calls, current.calls, says()], late).toStrictEqual([[], ["display 0"], MOVING]);
      unmount();
    }
  });

  test("follows another preference when given one, and listens to one only, until it is gone", async () => {
    const first = new FakeMediaQuery();
    const second = new FakeMediaQuery();
    second.matches = true;
    const query = { list: first };
    const preference = (): FakeMediaQuery => query.list;
    const { images } = module();
    const props = { images: () => Promise.resolve(images), state: "playing", frame: () => undefined, schedule: manual().schedule } as const;
    const { rerender, unmount } = render(<Square {...props} reducedMotion={preference} />);
    await settle();
    expect([says(), first.listening]).toStrictEqual([MOVING, 1]);
    query.list = second;
    rerender(<Square {...props} reducedMotion={() => query.list} />);
    expect([says(), first.listening, second.listening]).toStrictEqual([STILL, 0, 1]);
    act(() => {
      second.become(false);
    });
    expect(says()).toBe(MOVING);
    unmount();
    expect(second.listening, "none once gone").toBe(0);
  });

  test("says why when the page's module cannot load", async () => {
    render(<Square images={() => Promise.reject(new Error("cannot load the square from page.wasm (HTTP 404)"))} state="playing" frame={() => undefined} schedule={manual().schedule} />);
    await settle();
    expect(says()).toBe("The square cannot be shown: Error: cannot load the square from page.wasm (HTTP 404)");
    expect(await violations(), "failed").toStrictEqual([]);
  });

  test("says why when the page's module fails while showing, and stops repainting", async () => {
    const { images } = module();
    const { schedule, tick, cancelled } = manual();
    const time = clock();
    time.at(0n);
    const failing: SquareImages = {
      ...images,
      imageAt: () => {
        throw new Error("the visual module trapped and cannot be used again");
      },
      display: (index) => images.display(index),
    };
    render(<Square images={() => Promise.resolve(failing)} state="playing" frame={time.frame} context={() => canvas().context} schedule={schedule} />);
    await settle();
    tick();
    expect(says()).toBe(`${FAILED} Error: the visual module trapped and cannot be used again`);
    expect(cancelled()).toBe(1);
  });

  test("draws nothing, and asks no image, without a 2D context", async () => {
    const { images, calls } = module();
    const time = clock();
    time.at(0n);
    const { schedule, tick } = manual();
    render(<Square images={() => Promise.resolve(images)} state="playing" frame={time.frame} context={() => null} schedule={schedule} />);
    await settle();
    tick();
    tick();
    expect(calls).toStrictEqual(["imageAt 0 48000 60/1", "imageAt 0 48000 60/1"]);
  });

  test("ignores a module that settles after it is gone", async () => {
    const answers: { resolve: (images: SquareImages) => void; reject: (error: Error) => void }[] = [];
    const load = (): Promise<SquareImages> =>
      new Promise((resolve, reject) => {
        answers.push({ resolve, reject });
      });
    const first = render(<Square images={load} state="stopped" frame={() => undefined} />);
    const second = render(<Square images={load} state="stopped" frame={() => undefined} />);
    first.unmount();
    second.unmount();
    const errors = vi.spyOn(console, "error");
    answers[0]?.resolve(module().images);
    answers[1]?.reject(new Error("late"));
    await settle();
    expect(errors).not.toHaveBeenCalled();
    expect(answers).toHaveLength(2);
  });

  test("asks the canvas for its own 2D context by default", async () => {
    const { images, calls } = module();
    // jsdom has no 2D context: the canvas answers none, and nothing is drawn.
    const getContext = vi.spyOn(HTMLCanvasElement.prototype, "getContext");
    render(<Square images={() => Promise.resolve(images)} state="stopped" frame={() => undefined} />);
    await settle();
    expect(getContext).toHaveBeenCalledWith("2d");
    expect(calls).toStrictEqual([]);
  });


  /** Makes every canvas answer `answer` for a WebGPU context, until `restore` is called. */
  function webgpuCanvases(answer: object | null): { restore: () => void } {
    const original = Object.getOwnPropertyDescriptor(HTMLCanvasElement.prototype, "getContext");
    Object.defineProperty(HTMLCanvasElement.prototype, "getContext", {
      configurable: true,
      value: (kind: string) => (kind === "webgpu" ? answer : null),
    });
    return {
      restore: () => {
        if (original !== undefined) {
          Object.defineProperty(HTMLCanvasElement.prototype, "getContext", original);
        }
      },
    };
  }

  test("draws on the GPU when the browser has one, says so, and asks the module for no bytes", async () => {
    gpuLog.length = 0;
    const canvases = webgpuCanvases(new FakeCanvas());
    try {
      const { images, calls } = module();
      const device = new FakeDevice("device");
      const time = clock();
      time.at(4800n);
      render(<Square images={() => Promise.resolve(images)} state="suspended" frame={time.frame} schedule={manual().schedule} gpu={() => fakeGpu(device)} />);
      await settle();
      expect(screen.getByTestId("drawn-by").textContent).toBe("Drawn by the GPU, through WebGPU.");
      expect(calls, "which image, never its bytes").toStrictEqual(["imageAt 4800 48000 60/1"]);
      expect(gpuLog.filter((line) => line.startsWith("write buffer 1"))).toStrictEqual(["write buffer 1 at 0: [6, 0, 0, 0, 320, 0, 180, 0]"]);
      expect(await violations(), "drawn on the GPU").toStrictEqual([]);
      act(() => {
        device.lose("the GPU process crashed");
      });
      await settle();
      expect(says()).toBe("The square cannot be shown: the GPU was lost: the GPU process crashed");
    } finally {
      canvases.restore();
    }
  });

  test("releases the GPU when the square is gone", async () => {
    gpuLog.length = 0;
    const canvases = webgpuCanvases(new FakeCanvas());
    try {
      const { images } = module();
      const { unmount } = render(<Square images={() => Promise.resolve(images)} state="stopped" frame={() => undefined} schedule={manual().schedule} gpu={() => fakeGpu(new FakeDevice("device"))} />);
      await settle();
      expect(screen.getByTestId("drawn-by").textContent).toBe(DRAWN_BY.gpu);
      unmount();
      expect(gpuLog.at(-1)).toBe("destroy device");
    } finally {
      canvases.restore();
    }
  });

  test("draws on the processor when the GPU refuses a device or the canvas a WebGPU context, and says so", async () => {
    /** A device lost before the page could draw on it. */
    const lostEarly = (): FakeDevice => {
      const device = new FakeDevice("device");
      device.lose("the GPU process crashed");
      return device;
    };
    for (const [why, gpu, answer] of [
      ["a device lost while it is set up", fakeGpu(lostEarly()), new FakeCanvas()],
      ["a refused device", fakeGpu(new FakeDevice("device"), new Error("refused")), new FakeCanvas()],
      ["no WebGPU context", fakeGpu(new FakeDevice("device")), null],
      ["another kind of context", fakeGpu(new FakeDevice("device")), {}],
    ] as const) {
      const canvases = webgpuCanvases(answer);
      try {
        const { images, calls } = module();
        const { context, puts } = canvas();
        const { unmount } = render(<Square images={() => Promise.resolve(images)} state="stopped" frame={() => undefined} context={() => context} schedule={manual().schedule} gpu={() => gpu} />);
        await settle();
        expect([screen.getByTestId("drawn-by").textContent, calls, puts.length], why).toStrictEqual([DRAWN_BY.processor, ["display 0"], 1]);
        unmount();
      } finally {
        canvases.restore();
      }
    }
    expect(DRAWN_BY).toStrictEqual({ gpu: "Drawn by the GPU, through WebGPU.", processor: "Drawn by the processor, in WebAssembly." });
  });

  test("asks the browser's own GPU by default, when it has WebGPU", async () => {
    gpuLog.length = 0;
    Object.defineProperty(navigator, "gpu", { configurable: true, value: fakeGpu(null) });
    try {
      render(<Square images={() => Promise.resolve(module().images)} state="stopped" frame={() => undefined} schedule={manual().schedule} />);
      await settle();
      expect([gpuLog, screen.getByTestId("drawn-by").textContent], "an adapter asked for, none given").toStrictEqual([["adapter"], DRAWN_BY.processor]);
    } finally {
      Reflect.deleteProperty(navigator, "gpu");
    }
    expect("gpu" in navigator, "jsdom's own navigator again").toBe(false);
  });
});
