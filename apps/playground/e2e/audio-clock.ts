// The end-to-end tests' reader of an audio context's clock, run in the page (the page's own
// reader, src/audio-clock.ts, cannot be imported there): a function serialized into the
// browser, so it uses nothing but its argument and the page's globals.

/** What the square's canvas shows and the audio clock says at one moment: its pixels, and the clock's time and rate (null between sessions, or while the clock is no time). */
export interface SquareNow {
  /** The canvas's bytes, four a pixel, in base64: a string crosses to the test far faster than a list of numbers. */
  readonly pixels: string;
  /** The clock's time, in seconds. */
  readonly time: number | null;
  /** The clock's sample rate, in hertz. */
  readonly rate: number | null;
}

/**
 * The pixels of `element`, the square's canvas, and the clock of the page's last
 * AudioContext (exposed as `window.audio` by the tests), read in one task: no repaint
 * between them.
 */
export function squareNow(element: Element): SquareNow {
  if (!(element instanceof HTMLCanvasElement)) {
    throw new Error("not a canvas");
  }
  const audio: unknown = Reflect.get(window, "audio");
  const running = audio instanceof AudioContext && audio.state !== "closed" && Number.isFinite(audio.currentTime);
  const clock = running ? { time: audio.currentTime, rate: audio.sampleRate } : { time: null, rate: null };
  const bytes = element.getContext("2d")?.getImageData(0, 0, element.width, element.height).data ?? new Uint8ClampedArray();
  let binary = "";
  for (let at = 0; at < bytes.length; at += 8192) {
    binary += String.fromCharCode(...bytes.subarray(at, at + 8192));
  }
  return { pixels: btoa(binary), ...clock };
}
