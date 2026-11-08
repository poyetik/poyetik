/**
 * The live oscilloscope: a canvas redrawn every animation frame, outside React's
 * rendering, plus the measurements as text.
 */

import { useEffect, useState, type JSX } from "react";
import { backingStore, drawScope, type ScopeCanvas } from "./draw.ts";
import type { Signal } from "./player.ts";
import { NONE, formatFrequency, formatPeak, formatRate } from "./readout.ts";
import { scopeScene } from "./scope.ts";

/** Width of the canvas, in pixels (the page scales it). */
export const WIDTH = 900;

/** Height of the canvas, in pixels. */
export const HEIGHT = 300;

/** Duration shown across the canvas, in seconds: about four periods of 440 Hz. */
export const WINDOW_SECONDS = 0.01;

/** The id of the measurements, which describe the canvas to assistive technologies. */
const READOUT = "scope-readout";

/** Calls `frame` before every repaint until the returned function is called. */
export function animationFrames(frame: () => void): () => void {
  let id = 0;
  const loop = (): void => {
    frame();
    id = requestAnimationFrame(loop);
  };
  id = requestAnimationFrame(loop);
  return () => {
    cancelAnimationFrame(id);
  };
}

/** The 2D context of a canvas. */
function context2d(canvas: HTMLCanvasElement): ScopeCanvas | null {
  return canvas.getContext("2d");
}

/** Properties of the oscilloscope. */
export interface OscilloscopeProps {
  /**
   * Whether there is something to show: while active, a frame is drawn before every
   * repaint; inactive, one frame is drawn (the grid and dashes) and nothing is repainted.
   */
  readonly active: boolean;
  /** The signal to show, read at every frame. */
  readonly signal: () => Signal | undefined;
  /** Gets the drawing context of the canvas (replaceable in tests). */
  readonly context?: (canvas: HTMLCanvasElement) => ScopeCanvas | null;
  /** Schedules the frames (replaceable in tests). */
  readonly schedule?: (frame: () => void) => () => void;
  /** Device pixels per CSS pixel, read at every frame (replaceable in tests). */
  readonly density?: () => number;
}

/** Device pixels per CSS pixel of this screen. */
function screenDensity(): number {
  return window.devicePixelRatio;
}

/**
 * Draws `current` (nothing: the grid only) on `canvas` through `drawing`, with one backing
 * pixel per device pixel for the size the canvas has now. The backing store is resized
 * only when that changes (resizing clears the canvas); the height follows the width.
 */
function paint(canvas: HTMLCanvasElement, drawing: ScopeCanvas, current: Signal | undefined, density: number): void {
  const store = backingStore(canvas.clientWidth, density, WIDTH, HEIGHT);
  if (canvas.width !== store.width) {
    canvas.width = store.width;
    canvas.height = store.height;
  }
  const samples = current?.samples ?? new Float32Array();
  const window = Math.round((current?.rate ?? 0) * WINDOW_SECONDS);
  drawScope(drawing, scopeScene(samples, window, WIDTH, HEIGHT), store);
}

/**
 * The oscilloscope and its measurements. While active, the canvas is redrawn at every
 * frame without React; the measurements are React state, which only re-renders when their
 * text changes. While inactive, it is drawn once: a page with nothing playing repaints
 * nothing (a resize then scales the last drawing until the next start).
 */
export function Oscilloscope({ active, signal, context = context2d, schedule = animationFrames, density = screenDensity }: OscilloscopeProps): JSX.Element {
  const [canvas, setCanvas] = useState<HTMLCanvasElement | null>(null);
  const [frequency, setFrequency] = useState(NONE);
  const [level, setLevel] = useState(NONE);
  const [rate, setRate] = useState(NONE);
  useEffect(() => {
    // Frames start once the canvas exists (right after the first render).
    if (canvas === null) {
      return undefined;
    }
    const drawing = context(canvas);
    const frame = (): void => {
      const current = signal();
      if (drawing !== null) {
        paint(canvas, drawing, current, density());
      }
      setFrequency(formatFrequency(current));
      setLevel(formatPeak(current));
      setRate(formatRate(current));
    };
    if (!active) {
      frame();
      return undefined;
    }
    return schedule(frame);
  }, [active, canvas, signal, context, schedule, density]);
  return (
    <section aria-label="Oscilloscope">
      <canvas ref={setCanvas} width={WIDTH} height={HEIGHT} role="img" aria-label="Waveform of the engine output" aria-describedby={READOUT} />
      <dl id={READOUT}>
        <dt>Frequency</dt>
        <dd data-testid="frequency">{frequency}</dd>
        <dt>Sample peak (engine output)</dt>
        <dd data-testid="peak">{level}</dd>
        <dt>Sample rate</dt>
        <dd data-testid="rate">{rate}</dd>
      </dl>
    </section>
  );
}
