import { act, render, screen } from "@testing-library/react";
import axe from "axe-core";
import { afterEach, describe, expect, test, vi } from "vitest";
import type { AudioFrame } from "../src/player.ts";
import { SQUARE } from "../src/settings.ts";
import { DRAWN_BY, FAILED, FAILURE_DETAILS, MOVING, type ModuleView, NAMES, REMEDY, STILL, SWITCHING, Square, type SquareCanvas, type SquareFailure, type SquareImages, WAITING, WHY_NOT_GPU, presentation } from "../src/Square.tsx";
import { SquareLoadError } from "../src/square-module.ts";
import { FakeCanvas, FakeDevice, fakeGpu, gpuLog } from "./gpu-fakes.ts";
import { FakeMediaQuery, media } from "./media.ts";
import { manual } from "./schedule.ts";

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
 * A page module: frame `f` at rate `r` shows image ⌊f × 60 / r⌋ (as `page_image_at`
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

/** The line saying what the square shows, which describes its canvas. */
function says(): string | null {
  const id = screen.getByRole("img").getAttribute("aria-describedby") ?? "";
  return document.getElementById(id)?.textContent ?? null;
}

/** What the line says when the square fails at `stage`. */
function failed(stage: SquareFailure): string {
  return `${FAILED} ${REMEDY[stage]}`;
}

/** The details of the square's failure, shown on demand. */
function details(): string | null {
  return document.querySelector("section[aria-label='Square'] details code")?.textContent ?? null;
}

/** Why the square is not drawn on the GPU, when the page says why: the details' summary and text. */
function whyNotGpu(): readonly [string | null, string | null] | null {
  const shown = screen.queryByTestId("why-not-gpu");
  return shown === null ? null : [shown.querySelector("summary")?.textContent ?? null, shown.querySelector("code")?.textContent ?? null];
}

/** What the page logged as warnings and errors, from now on. */
function logs(): { warnings: () => unknown[][]; errors: () => unknown[][] } {
  const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
  const error = vi.spyOn(console, "error").mockImplementation(() => undefined);
  return { warnings: () => warn.mock.calls, errors: () => error.mock.calls };
}

/** The violations axe finds in the page as it is. */
async function violations(): Promise<string[]> {
  const results = await axe.run(document.body);
  return results.violations.map((violation) => violation.id);
}

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
    expect(says()).toBe("The square sweeps across the image with the sound, one sweep every two seconds, on the audio clock. Still until the sound starts.");
    expect(calls).toStrictEqual(["display 0"]);
    expect(puts).toStrictEqual([[0, SQUARE.width, SQUARE.height, 0, 0]]);
    expect(scheduled(), "no repaint while the sound does not play").toBe(0);
    expect(await violations(), "shown").toStrictEqual([]);
  });

  test("names the canvas by what it shows, in every state", async () => {
    const loading = render(<Square images={() => new Promise(() => undefined)} state="stopped" frame={() => undefined} />);
    expect(screen.getByRole("img").getAttribute("aria-label"), "loading").toBe(NAMES.loading);
    expect(screen.getByTestId("square-failure").textContent, "no failure announced while loading").toBe("");
    loading.unmount();
    const failing = render(<Square images={() => Promise.reject(new Error("offline"))} state="stopped" frame={() => undefined} />);
    await settle();
    expect(screen.getByRole("img").getAttribute("aria-label"), "failed").toBe(NAMES.failed);
    expect(screen.getByTestId("square-failure").textContent, "the failure announced").toBe(failed("module"));
    expect(screen.getByTestId("square-failure").getAttribute("aria-live"), "announced politely").toBe("polite");
    failing.unmount();
    const { images } = module();
    const { context } = canvas();
    const waiting = render(<Square images={() => Promise.resolve(images)} state="stopped" frame={() => undefined} context={() => context} schedule={manual().schedule} />);
    await settle();
    expect(screen.getByRole("img").getAttribute("aria-label"), "still until the sound plays").toBe(NAMES.still);
    expect(screen.getByTestId("square-failure").textContent, "no failure announced once shown").toBe("");
    expect(screen.getByTestId("square-failure").getAttribute("aria-live"), "present before any failure").toBe("polite");
    waiting.unmount();
    const moving = render(<Square images={() => Promise.resolve(images)} state="playing" frame={() => undefined} context={() => context} schedule={manual().schedule} />);
    await settle();
    expect(screen.getByRole("img").getAttribute("aria-label"), "moving while the sound plays").toBe(NAMES.moving);
    moving.unmount();
    expect(NAMES).toStrictEqual({ loading: "The square, loading", switching: "The square, moving to the CPU", failed: "The square, which cannot be shown", still: "A white square on black, still", moving: "A white square on black, sweeping from left to right" });
  });

  test("says what the square waits for, in every state of the player", async () => {
    const { images } = module();
    const { context } = canvas();
    const states = ["stopped", "starting", "playing", "suspended", "stopping", "failed", "unsupported"] as const satisfies readonly (keyof typeof WAITING)[];
    expect(Object.keys(WAITING).sort(), "every state of the player").toStrictEqual([...states].sort());
    for (const state of states) {
      const { unmount } = render(<Square images={() => Promise.resolve(images)} state={state} frame={() => undefined} context={() => context} schedule={manual().schedule} />);
      await settle();
      expect(says(), state).toBe(`${MOVING}${WAITING[state]}`);
      unmount();
    }
    expect(WAITING.unsupported, "never waiting for a sound that cannot start").not.toContain("until the sound starts");
    expect(WAITING).toStrictEqual({
      stopped: " Still until the sound starts.",
      starting: " Still until the sound starts.",
      playing: "",
      suspended: " Still while the sound is paused.",
      stopping: " Still: the sound stops.",
      failed: " Still: the sound cannot play.",
      unsupported: " Still: this browser cannot play the sound.",
    });
  });

  test("says the sweep of the square of the settings", () => {
    // The sentence names the cycle in words: it follows the settings' cycle, which a change
    // of the cycle alone would leave stale.
    expect(SQUARE.cycle, "one sweep every two seconds").toStrictEqual([2, 1]);
    expect(MOVING).toContain("one sweep every two seconds");
  });

  test("describes the canvas: an image, named, described by the line, of the square's size", () => {
    render(<Square images={() => new Promise(() => undefined)} state="stopped" frame={() => undefined} />);
    const image = screen.getByRole("img", { name: NAMES.loading });
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
    // A clock that reads no time for a moment: the image shown stays, never image 0.
    time.at(undefined);
    tick();
    expect([calls.length, puts.at(-1)?.[0]], "nothing asked, nothing drawn").toStrictEqual([7, 10]);
    time.at(7350n, 44_100);
    tick();
    expect(calls.slice(7), "the clock back: its image, the one shown").toStrictEqual(["imageAt 7350 44100 60/1"]);
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
    expect(screen.getByRole("img").getAttribute("aria-label"), "named still").toBe(NAMES.still);
    expect(calls, "the first image, whatever the clock").toStrictEqual(["display 0"]);
    expect(scheduled(), "no repaint").toBe(0);
    expect(await violations(), "still").toStrictEqual([]);
    act(() => {
      media.list.become(false);
    });
    expect(says()).toBe(MOVING);
    expect(screen.getByRole("img").getAttribute("aria-label"), "named moving once the system stops asking").toBe(NAMES.moving);
    expect(scheduled()).toBe(1);
    tick();
    expect(puts.map(([first]) => first)).toStrictEqual([0, 60]);
    act(() => {
      media.list.become(true);
    });
    expect([says(), cancelled(), screen.getByRole("img").getAttribute("aria-label")]).toStrictEqual([STILL, 1, NAMES.still]);
    expect(FakeMediaQuery.asked).toStrictEqual(["(prefers-reduced-motion: reduce)"]);
  });

  test("loads the page module once, when its canvas exists", async () => {
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
      expect([old.calls, current.calls, says()], late).toStrictEqual([[], ["display 0"], `${MOVING}${WAITING.stopped}`]);
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
    const { context } = canvas();
    const props = { images: () => Promise.resolve(images), state: "playing", frame: () => undefined, context: () => context, schedule: manual().schedule } as const;
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

  test("says why when the page module cannot load, with the remedy of the stage that failed", async () => {
    for (const [stage, error, detail] of [
      ["download", new SquareLoadError("download", new Error("cannot load the square from page.wasm (HTTP 404)")), "Error: the square's module did not download, caused by Error: cannot load the square from page.wasm (HTTP 404)"],
      ["module", new SquareLoadError("module", new Error("not the page module")), "Error: the square's module cannot run here, caused by Error: not the page module"],
      ["module", new Error("any other failure"), "Error: any other failure"],
    ] as const) {
      const { unmount } = render(<Square images={() => Promise.reject(error)} state="playing" frame={() => undefined} schedule={manual().schedule} />);
      await settle();
      expect([says(), details()], stage).toStrictEqual([failed(stage), detail]);
      expect(await violations(), stage).toStrictEqual([]);
      unmount();
    }
  });

  test("says a failure once: the line, inside the live region, still describing the canvas", async () => {
    render(<Square images={() => Promise.reject(new Error("offline"))} state="stopped" frame={() => undefined} schedule={manual().schedule} />);
    await settle();
    expect(screen.getAllByText(failed("module")), "one sentence").toHaveLength(1);
    const line = document.getElementById(screen.getByRole("img").getAttribute("aria-describedby") ?? "");
    expect(line?.textContent).toBe(failed("module"));
    expect(screen.getByTestId("square-failure").contains(line), "announced by the live region").toBe(true);
    expect(screen.getByRole("group").querySelector("summary")?.textContent).toBe(FAILURE_DETAILS);
  });

  test("says why when the page module fails while showing, and stops repainting", async () => {
    const { images } = module();
    const { schedule, tick, cancelled } = manual();
    const time = clock();
    time.at(0n);
    const failing: SquareImages = {
      ...images,
      imageAt: () => {
        throw new Error("the visual module trapped and cannot be used again", { cause: new Error("unreachable") });
      },
      display: (index) => images.display(index),
    };
    render(<Square images={() => Promise.resolve(failing)} state="playing" frame={time.frame} context={() => canvas().context} schedule={schedule} />);
    await settle();
    tick();
    expect(says()).toBe(failed("module"));
    expect(details(), "its cause kept").toBe("Error: the visual module trapped and cannot be used again, caused by Error: unreachable");
    expect(cancelled()).toBe(1);
  });

  test("says it cannot show the square without a 2D context, and paints nothing", async () => {
    const { images, calls } = module();
    const time = clock();
    time.at(0n);
    const { schedule, tick } = manual();
    render(<Square images={() => Promise.resolve(images)} state="playing" frame={time.frame} context={() => null} schedule={schedule} />);
    await settle();
    tick();
    expect(calls, "no image asked, none drawn").toStrictEqual([]);
    expect([says(), details()]).toStrictEqual([failed("canvas"), "the canvas refused a 2D context"]);
    expect(await violations(), "failed").toStrictEqual([]);
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

  test("asks the canvas for its own 2D context by default, and says why when it refuses one", async () => {
    const { images, calls } = module();
    // jsdom has no 2D context: the canvas answers none, nothing is drawn, and the page says
    // it cannot show the square rather than a blank canvas drawn.
    const getContext = vi.spyOn(HTMLCanvasElement.prototype, "getContext");
    render(<Square images={() => Promise.resolve(images)} state="stopped" frame={() => undefined} />);
    await settle();
    expect(getContext).toHaveBeenCalledWith("2d");
    expect(calls).toStrictEqual([]);
    expect([says(), details()]).toStrictEqual([failed("canvas"), "the canvas refused a 2D context"]);
    expect(screen.queryByTestId("drawn-by"), "never said drawn").toBeNull();
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
      const { context, puts } = canvas();
      // The images loaded at once, then, once held, only when resumed.
      let hold = false;
      let resume = (): void => undefined;
      const load = (): Promise<typeof images> =>
        hold
          ? new Promise((resolve) => {
              resume = () => {
                resolve(images);
              };
            })
          : Promise.resolve(images);
      render(<Square images={load} state="suspended" frame={time.frame} context={() => context} schedule={manual().schedule} gpu={() => fakeGpu(device)} />);
      await settle();
      expect(screen.getByTestId("drawn-by").textContent).toBe("Drawn by the GPU, through WebGPU.");
      const onGpu = screen.getByRole("img");
      expect(calls, "which image, never its bytes").toStrictEqual(["imageAt 4800 48000 60/1"]);
      expect(gpuLog.filter((line) => line.startsWith("write buffer 1"))).toStrictEqual(["write buffer 1 at 0: [6, 0, 0, 0, 320, 0, 180, 0]"]);
      expect(await violations(), "drawn on the GPU").toStrictEqual([]);
      expect(whyNotGpu(), "nothing to say while the GPU draws").toBeNull();
      gpuLog.length = 0;
      hold = true;
      const logged = logs();
      act(() => {
        device.lose("the GPU process crashed");
      });
      await settle();
      expect([says(), screen.getByRole("img").getAttribute("aria-label"), screen.queryByTestId("drawn-by")], "moving to the CPU, until it draws").toStrictEqual([SWITCHING, NAMES.switching, null]);
      expect(await violations(), "moving to the CPU").toStrictEqual([]);
      resume();
      await settle();
      // The GPU lost: the square on a fresh canvas, drawn on the CPU, the GPU never asked
      // again; the line says the GPU was lost, its details why, logged as a warning.
      expect(screen.getByTestId("drawn-by").textContent).toBe(DRAWN_BY.lost);
      expect(whyNotGpu()).toStrictEqual([WHY_NOT_GPU, "the GPU process crashed"]);
      expect([logged.warnings(), logged.errors()]).toStrictEqual([[["The square is not drawn on the GPU: the GPU process crashed"]], []]);
      expect(screen.getByRole("img"), "a fresh canvas").not.toBe(onGpu);
      expect(puts.map(([first]) => first), "the image of the clock, drawn on the CPU").toStrictEqual([6]);
      expect(gpuLog.filter((line) => line === "adapter"), "no GPU asked again").toStrictEqual([]);
      expect(screen.queryByTestId("square-failure")?.textContent, "never failed").toBe("");
      expect(await violations(), "drawn on the CPU after the loss").toStrictEqual([]);
    } finally {
      canvases.restore();
    }
  });

  test("releases the GPU when the square is gone", async () => {
    gpuLog.length = 0;
    const canvases = webgpuCanvases(new FakeCanvas());
    try {
      const { images } = module();
      const device = new FakeDevice("device");
      const errors = vi.spyOn(console, "error");
      const { unmount } = render(<Square images={() => Promise.resolve(images)} state="stopped" frame={() => undefined} schedule={manual().schedule} gpu={() => fakeGpu(device)} />);
      await settle();
      expect(screen.getByTestId("drawn-by").textContent).toBe(DRAWN_BY.gpu);
      unmount();
      expect(gpuLog.at(-1)).toBe("destroy device");
      // Its device lost once the square is gone: nothing set up again, no state set.
      device.lose("destroyed");
      await settle();
      expect(gpuLog.at(-1), "no GPU asked again").toBe("destroy device");
      expect(errors).not.toHaveBeenCalled();
    } finally {
      canvases.restore();
    }
  });

  test("releases the GPU set up for a square gone before it is shown", async () => {
    gpuLog.length = 0;
    const canvases = webgpuCanvases(new FakeCanvas());
    // The square goes away as the page takes its WebGPU context: the last step of the setup.
    let gone = (): void => undefined;
    Object.defineProperty(HTMLCanvasElement.prototype, "getContext", {
      configurable: true,
      value: (kind: string) => {
        if (kind !== "webgpu") {
          return null;
        }
        gone();
        // Lost as the square goes: reported to a square no longer there, which ignores it.
        device.lose("the GPU process crashed");
        return new FakeCanvas();
      },
    });
    const device = new FakeDevice("device");
    try {
      const { images } = module();
      const errors = vi.spyOn(console, "warn");
      const { unmount } = render(<Square images={() => Promise.resolve(images)} state="stopped" frame={() => undefined} schedule={manual().schedule} gpu={() => fakeGpu(device)} />);
      gone = unmount;
      await settle();
      expect(gpuLog.filter((line) => line.startsWith("configure") || line.startsWith("destroy")), "set up, then released once").toStrictEqual(["configure device, rgba8unorm, opaque, usage 2", "destroy device"]);
      expect(errors, "its loss never reported").not.toHaveBeenCalled();
    } finally {
      canvases.restore();
    }
  });

  test("keeps the square on the GPU when a device it no longer draws with is lost", async () => {
    gpuLog.length = 0;
    const canvases = webgpuCanvases(new FakeCanvas());
    try {
      const { images } = module();
      const first = new FakeDevice("first");
      const second = new FakeDevice("second");
      const devices = [first, second];
      const gpu = (): ReturnType<typeof fakeGpu> => fakeGpu(devices.shift() ?? second);
      const view = render(<Square images={() => Promise.resolve(images)} state="stopped" frame={() => undefined} schedule={manual().schedule} gpu={gpu} />);
      await settle();
      expect(screen.getByTestId("drawn-by").textContent).toBe(DRAWN_BY.gpu);
      // Other images: the square set up again on a second device, the first released.
      view.rerender(<Square images={() => Promise.resolve(module().images)} state="stopped" frame={() => undefined} schedule={manual().schedule} gpu={gpu} />);
      await settle();
      expect([gpuLog.includes("destroy first"), screen.getByTestId("drawn-by").textContent], "the first released, the second drawing").toStrictEqual([true, DRAWN_BY.gpu]);
      // The released device lost afterward, as a browser loses a destroyed one.
      act(() => {
        first.lose("destroyed");
      });
      await settle();
      expect([says(), screen.getByTestId("drawn-by").textContent], "still on the GPU").toStrictEqual([`${MOVING}${WAITING.stopped}`, DRAWN_BY.gpu]);
      view.unmount();
    } finally {
      canvases.restore();
    }
  });

  test("draws on the CPU when the GPU refuses a device or the canvas a WebGPU context, and says so", async () => {
    /** A device lost before the page could draw on it. */
    const lostEarly = (): FakeDevice => {
      const device = new FakeDevice("device");
      device.lose("the GPU process crashed");
      return device;
    };
    /** A device whose pipelines do not compile: the library's shaders refused, a defect. */
    const refusing = (): FakeDevice => Object.assign(new FakeDevice("device"), { rejectPipelines: true });
    /** What the page says and logs: nothing for no GPU to give, a warning for this machine's GPU, an error for a defect. */
    type Said = readonly [readonly [string, string] | null, readonly string[], readonly string[]];
    const cases: readonly (readonly [string, ReturnType<typeof fakeGpu>, object | null, Said])[] = [
      ["a device lost while it is set up", fakeGpu(lostEarly()), new FakeCanvas(), [[WHY_NOT_GPU, "the device was lost while it was set up: the GPU process crashed"], ["The square is not drawn on the GPU: the device was lost while it was set up: the GPU process crashed"], []]],
      ["a refused device", fakeGpu(new FakeDevice("device"), new Error("refused")), new FakeCanvas(), [[WHY_NOT_GPU, "the device was refused: Error: refused"], ["The square is not drawn on the GPU: the device was refused: Error: refused"], []]],
      ["a pipeline refused", fakeGpu(refusing()), new FakeCanvas(), [[WHY_NOT_GPU, "a pipeline was refused: Error: the shader does not compile"], [], ["The square is not drawn on the GPU: a pipeline was refused: Error: the shader does not compile"]]],
      ["no WebGPU context", fakeGpu(new FakeDevice("device")), null, [null, [], []]],
      ["another kind of context", fakeGpu(new FakeDevice("device")), {}, [null, [], []]],
    ];
    for (const [why, gpu, answer, said] of cases) {
      const canvases = webgpuCanvases(answer);
      try {
        const logged = logs();
        const { images, calls } = module();
        const { context, puts } = canvas();
        const { unmount } = render(<Square images={() => Promise.resolve(images)} state="stopped" frame={() => undefined} context={() => context} schedule={manual().schedule} gpu={() => gpu} />);
        await settle();
        expect([screen.getByTestId("drawn-by").textContent, calls, puts.length], why).toStrictEqual([DRAWN_BY.cpu, ["display 0"], 1]);
        expect([whyNotGpu(), logged.warnings().flat(), logged.errors().flat()], why).toStrictEqual(said);
        expect(await violations(), why).toStrictEqual([]);
        unmount();
        vi.restoreAllMocks();
      } finally {
        canvases.restore();
      }
    }
    expect(DRAWN_BY).toStrictEqual({
      gpu: "Drawn by the GPU, through WebGPU.",
      cpu: "Drawn by the CPU, in WebAssembly.",
      lost: "Drawn by the CPU, in WebAssembly (the GPU was lost).",
      failed: "Drawn by the CPU, in WebAssembly (the GPU failed).",
    });
    expect(WHY_NOT_GPU).toBe("Why not the GPU");
  });

  test("draws on the CPU, saying the GPU failed and why, when the GPU raises an error while drawing", async () => {
    gpuLog.length = 0;
    const canvases = webgpuCanvases(new FakeCanvas());
    try {
      const { images } = module();
      const device = new FakeDevice("device");
      const { context, puts } = canvas();
      render(<Square images={() => Promise.resolve(images)} state="stopped" frame={() => undefined} context={() => context} schedule={manual().schedule} gpu={() => fakeGpu(device)} />);
      await settle();
      expect(screen.getByTestId("drawn-by").textContent).toBe(DRAWN_BY.gpu);
      const logged = logs();
      act(() => {
        device.raise("a texture of the wrong size");
        device.raise("a second error");
      });
      await settle();
      expect(screen.getByTestId("drawn-by").textContent).toBe(DRAWN_BY.failed);
      expect(whyNotGpu()).toStrictEqual([WHY_NOT_GPU, "a texture of the wrong size"]);
      expect([logged.warnings(), logged.errors()], "one error, never a warning").toStrictEqual([[], [["The square is not drawn on the GPU: a texture of the wrong size"]]]);
      expect([gpuLog.includes("destroy device"), puts.map(([first]) => first)], "the GPU released, the CPU drawing").toStrictEqual([true, [0]]);
      expect(await violations(), "drawn on the CPU after the GPU failed").toStrictEqual([]);
    } finally {
      canvases.restore();
    }
  });

  test("says the square cannot be shown when the page module throws while the GPU is set up", async () => {
    gpuLog.length = 0;
    const canvases = webgpuCanvases(new FakeCanvas());
    try {
      const { images } = module();
      const trapping: SquareImages = {
        ...images,
        shader: () => {
          throw new Error("the visual module trapped and cannot be used again");
        },
      };
      render(<Square images={() => Promise.resolve(trapping)} state="stopped" frame={() => undefined} schedule={manual().schedule} gpu={() => fakeGpu(new FakeDevice("device"))} />);
      await settle();
      expect([says(), details(), gpuLog.at(-1)]).toStrictEqual([failed("module"), "Error: the visual module trapped and cannot be used again", "destroy device"]);
    } finally {
      canvases.restore();
    }
  });

  test("releases the GPU when the page module throws while the GPU draws, and hears no more of it", async () => {
    gpuLog.length = 0;
    const canvases = webgpuCanvases(new FakeCanvas());
    try {
      const { images } = module();
      const device = new FakeDevice("device");
      const trapping: SquareImages = {
        ...images,
        placement: () => {
          throw new Error("the visual module trapped and cannot be used again");
        },
      };
      const logged = logs();
      render(<Square images={() => Promise.resolve(trapping)} state="stopped" frame={() => undefined} schedule={manual().schedule} gpu={() => fakeGpu(device)} />);
      await settle();
      const line = failed("module");
      expect([says(), gpuLog.filter((entry) => entry === "destroy device")], "failed, the device destroyed once").toStrictEqual([line, ["destroy device"]]);
      // Its loss afterward, as a browser loses a destroyed device: nothing heard, nothing asked again.
      act(() => {
        device.lose("destroyed");
      });
      await settle();
      expect([says(), screen.getByTestId("square-failure").textContent, gpuLog.filter((entry) => entry === "adapter" || entry === "destroy device")], "still failed, announced once").toStrictEqual([line, line, ["adapter", "destroy device"]]);
      expect(logged.warnings(), "the loss never reported").toStrictEqual([]);
    } finally {
      canvases.restore();
    }
  });

  test("asks the browser's own GPU by default, when it has WebGPU", async () => {
    gpuLog.length = 0;
    Object.defineProperty(navigator, "gpu", { configurable: true, value: fakeGpu(null) });
    try {
      const { context } = canvas();
      render(<Square images={() => Promise.resolve(module().images)} state="stopped" frame={() => undefined} context={() => context} schedule={manual().schedule} />);
      await settle();
      expect([gpuLog, screen.getByTestId("drawn-by").textContent], "an adapter asked for, none given").toStrictEqual([["adapter"], DRAWN_BY.cpu]);
    } finally {
      Reflect.deleteProperty(navigator, "gpu");
    }
    expect("gpu" in navigator, "jsdom's own navigator again").toBe(false);
  });
});

test("says what to do when the square cannot be shown, by stage, never a reload that cannot help", () => {
  expect([FAILED, REMEDY, FAILURE_DETAILS]).toStrictEqual([
    "The square cannot be shown.",
    { download: "Check your connection, then reload the page.", module: "This browser cannot show it; its details say what failed.", canvas: "This browser cannot show it; its details say what failed." },
    "Details of the square's failure",
  ]);
});

/** Every state the square's module can be in, as the page tells it: a key per kind and stage, which `tsc` holds to the type. */
const MODULES: Readonly<Record<"loading" | "switching" | "ready" | `failed at ${SquareFailure}`, ModuleView>> = {
  loading: { kind: "loading" },
  switching: { kind: "switching" },
  ready: { kind: "ready" },
  "failed at download": { kind: "failed", stage: "download" },
  "failed at module": { kind: "failed", stage: "module" },
  "failed at canvas": { kind: "failed", stage: "canvas" },
};

test("tells the square from its whole state: moving only when ready, without reduced motion, while the sound plays", () => {
  const states = Object.keys(WAITING).filter((kind): kind is keyof typeof WAITING => kind in WAITING);
  expect(states).toHaveLength(7);
  for (const [key, view] of Object.entries(MODULES)) {
    for (const still of [false, true]) {
      for (const state of states) {
        const told = presentation(view, still, state);
        const where = `${key}, ${still ? "reduced motion" : "motion"}, ${state}`;
        if (view.kind === "loading") {
          expect(told, where).toStrictEqual({ name: "The square, loading", line: "Loading the square…" });
        } else if (view.kind === "switching") {
          expect(told, where).toStrictEqual({ name: "The square, moving to the CPU", line: "Moving the square to the CPU…" });
        } else if (view.kind === "failed") {
          expect(told, where).toStrictEqual({ name: "The square, which cannot be shown", line: `The square cannot be shown. ${REMEDY[view.stage]}` });
        } else {
          const moving = !still && state === "playing";
          expect(told.name, where).toBe(moving ? "A white square on black, sweeping from left to right" : "A white square on black, still");
          expect(told.line, where).toBe(still ? STILL : `${MOVING}${WAITING[state]}`);
        }
      }
    }
  }
});

/**
 * Every state of the square as rendered, each with what makes it: its key names the module's
 * kind (and what draws it, or the stage that failed), a `Record` so that a state added
 * without its row does not compile.
 */
const RENDERED: Readonly<Record<"loading" | "ready on the GPU" | "ready on the CPU" | `failed at ${SquareFailure}`, { readonly view: ModuleView; readonly props: () => Partial<Parameters<typeof Square>[0]> }>> = {
  loading: { view: { kind: "loading" }, props: () => ({ images: () => new Promise(() => undefined) }) },
  "ready on the GPU": { view: { kind: "ready" }, props: () => ({ gpu: () => fakeGpu(new FakeDevice("device")) }) },
  "ready on the CPU": { view: { kind: "ready" }, props: () => ({}) },
  "failed at download": { view: { kind: "failed", stage: "download" }, props: () => ({ images: () => Promise.reject(new SquareLoadError("download", new Error("offline"))) }) },
  "failed at module": { view: { kind: "failed", stage: "module" }, props: () => ({ images: () => Promise.reject(new SquareLoadError("module", new Error("no WebAssembly"))) }) },
  "failed at canvas": { view: { kind: "failed", stage: "canvas" }, props: () => ({ context: () => null }) },
};

test("names, describes, announces and passes axe in every state of the module, with and without reduced motion", async () => {
  const restore = Object.getOwnPropertyDescriptor(HTMLCanvasElement.prototype, "getContext");
  Object.defineProperty(HTMLCanvasElement.prototype, "getContext", { configurable: true, value: (kind: string) => (kind === "webgpu" ? new FakeCanvas() : null) });
  try {
    for (const [key, { view, props }] of Object.entries(RENDERED)) {
      for (const still of [false, true]) {
        media.list.become(still);
        const { images } = module();
        const { context } = canvas();
        const { unmount } = render(<Square images={() => Promise.resolve(images)} state="playing" frame={() => undefined} context={() => context} schedule={manual().schedule} {...props()} />);
        await settle();
        const where = `${key}, ${still ? "reduced motion" : "motion"}`;
        const told = presentation(view, still, "playing");
        expect([screen.getByRole("img").getAttribute("aria-label"), says()], where).toStrictEqual([told.name, told.line]);
        expect(screen.getByRole("img", { description: told.line }), `${where}: described by its line`).toBeDefined();
        expect([screen.getByTestId("square-failure").getAttribute("aria-live"), screen.getByTestId("square-failure").textContent], `${where}: the live region`).toStrictEqual(["polite", view.kind === "failed" ? told.line : ""]);
        expect(await violations(), where).toStrictEqual([]);
        unmount();
      }
    }
  } finally {
    if (restore !== undefined) {
      Object.defineProperty(HTMLCanvasElement.prototype, "getContext", restore);
    }
  }
});
