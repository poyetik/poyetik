// The end-to-end tests' readers of an audio context's clock, run in the page (the page's own
// reader, src/audio-clock.ts, cannot be imported there): functions serialized into the
// browser, so they use nothing but their arguments and the page's globals.

/**
 * The audio clock around the page's last animation frame: its time before the frame's first
 * callback and after its last (the clock may move while the callbacks run, so that what a
 * callback read lies between the two), and its rate.
 */
export interface Repaint {
  /** The animation frame's timestamp: a later repaint has a larger one. */
  readonly stamp: number;
  /** The clock's time before the frame's first callback, in seconds. */
  readonly before: number;
  /** The clock's time after the frame's last callback, in seconds. */
  readonly after: number;
  /** The clock's sample rate, in hertz. */
  readonly rate: number;
}

/** What the square's canvas shows and the audio clock says at one moment: its pixels, the clock's time and rate (null between sessions, or while the clock is no time), and the clock the last repaint saw. */
export interface SquareNow {
  /** The canvas's bytes, four a pixel, in base64: a string crosses to the test far faster than a list of numbers. */
  readonly pixels: string;
  /** The clock's time, in seconds. */
  readonly time: number | null;
  /** The clock's sample rate, in hertz. */
  readonly rate: number | null;
  /** The clock around the last animation frame ({@link recordRepaints}), or null before one saw a running clock. */
  readonly repaint: Repaint | null;
}

/** One callback of an animation frame, as {@link recordRepaints} saw it: the frame's timestamp, and the clock before and after the callback. */
interface Seen {
  /** The animation frame's timestamp: the same for every callback of a frame. */
  readonly stamp: number;
  /** The clock's time before the callback, in seconds. */
  readonly before: number;
  /** The clock's time after it, in seconds. */
  readonly after: number;
  /** The clock's sample rate, in hertz. */
  readonly rate: number;
}

/**
 * Records, around every callback of every animation frame, the time of the page's last
 * AudioContext (exposed as `window.audio` by the tests) and the frame's timestamp, as
 * `window.repaints` (the last 16 callbacks): what the page's repaint read lies between the
 * two, since the square reads the clock inside its callback. A callback around which the
 * clock does not run at both reads (it starts, is suspended or stops in between) is not
 * recorded: what the page read then is no running clock's to hold it to. Run before the
 * page's scripts (`page.addInitScript`), so that every callback the page schedules is wrapped.
 */
export function recordRepaints(): void {
  const seen: Seen[] = [];
  Reflect.set(window, "repaints", seen);
  const native = window.requestAnimationFrame.bind(window);
  /** The clock's time and rate while it runs, or null. */
  const clock = (): { time: number; rate: number } | null => {
    const audio: unknown = Reflect.get(window, "audio");
    return audio instanceof AudioContext && audio.state === "running" && Number.isFinite(audio.currentTime) ? { time: audio.currentTime, rate: audio.sampleRate } : null;
  };
  window.requestAnimationFrame = (callback) =>
    native((stamp) => {
      const before = clock();
      callback(stamp);
      const after = clock();
      if (before !== null && after !== null) {
        seen.push({ stamp, before: before.time, after: after.time, rate: after.rate });
        seen.splice(0, Math.max(0, seen.length - 16));
      }
    });
}

/**
 * The pixels of `element`, the square's canvas, the clock of the page's last AudioContext
 * and the clock around its last animation frame, read in one task: no repaint between them.
 */
export function squareNow(element: Element): SquareNow {
  if (!(element instanceof HTMLCanvasElement)) {
    throw new Error("not a canvas");
  }
  const audio: unknown = Reflect.get(window, "audio");
  const running = audio instanceof AudioContext && audio.state !== "closed" && Number.isFinite(audio.currentTime);
  const clock = running ? { time: audio.currentTime, rate: audio.sampleRate } : { time: null, rate: null };
  // The callbacks of the last animation frame, in order: written by `recordRepaints`, read back as data.
  const recorded: unknown = Reflect.get(window, "repaints");
  const callbacks: { stamp: number; before: number; after: number; rate: number }[] = [];
  const entries: unknown[] = Array.isArray(recorded) ? Array.from(recorded.values()) : [];
  for (const entry of entries) {
    const fields = ["stamp", "before", "after", "rate"].map((key): unknown => Reflect.get(Object(entry), key));
    const [stamp, before, after, rate] = fields;
    if (typeof stamp === "number" && typeof before === "number" && typeof after === "number" && typeof rate === "number") {
      callbacks.push({ stamp, before, after, rate });
    }
  }
  const last = callbacks.at(-1);
  const frame = callbacks.filter((entry) => entry.stamp === last?.stamp);
  const first = frame[0];
  const repaint = first === undefined || last === undefined ? null : { stamp: last.stamp, before: first.before, after: last.after, rate: last.rate };
  // Copied onto a 2D canvas first: a canvas the GPU draws on has no 2D context to read.
  const copy = document.createElement("canvas");
  copy.width = element.width;
  copy.height = element.height;
  const drawing = copy.getContext("2d");
  // What the GPU last drew, when it draws this canvas (`gpu-canvas.ts`), else the canvas.
  const kept: unknown = Reflect.get(element, "gpuCopy");
  drawing?.drawImage(kept instanceof HTMLCanvasElement ? kept : element, 0, 0);
  const bytes = drawing?.getImageData(0, 0, element.width, element.height).data ?? new Uint8ClampedArray();
  let binary = "";
  for (let at = 0; at < bytes.length; at += 8192) {
    binary += String.fromCharCode(...bytes.subarray(at, at + 8192));
  }
  return { pixels: btoa(binary), ...clock, repaint };
}

/**
 * `count` readings of the page's last AudioContext's time while it runs, `apart`
 * milliseconds apart (by the page's own timers), in seconds: what the browser reports,
 * coarsened or not. It tries `count` × 4 times at most: a clock that stops running fails
 * with the context's state and the readings taken, never waits for the test's timeout.
 */
export async function clockSamples({ count, apart }: { readonly count: number; readonly apart: number }): Promise<number[]> {
  const audio: unknown = Reflect.get(window, "audio");
  if (!(audio instanceof AudioContext)) {
    throw new Error("no AudioContext: call trackAudio before loading the page");
  }
  const times: number[] = [];
  const tries = count * 4;
  for (let tried = 0; times.length < count; tried += 1) {
    if (tried === tries) {
      throw new Error(`${String(times.length)} of ${String(count)} readings in ${String(tries)} tries, the context ${audio.state}: [${times.join(", ")}]`);
    }
    if (audio.state === "running" && Number.isFinite(audio.currentTime)) {
      times.push(audio.currentTime);
    }
    await new Promise((resolve) => {
      setTimeout(resolve, apart);
    });
  }
  return times;
}
