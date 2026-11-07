/**
 * The live oscilloscope: a canvas redrawn every animation frame, outside React's
 * rendering, plus the measurements as text.
 */

import { useEffect, useState, type JSX } from "react";
import { drawScope, type ScopeCanvas } from "./draw.ts";
import type { Signal } from "./player.ts";
import { NONE, formatFrequency, formatPeak, formatRate } from "./readout.ts";
import { scopeScene } from "./scope.ts";

/** Width of the canvas, in pixels (the page scales it). */
export const WIDTH = 900;

/** Height of the canvas, in pixels. */
export const HEIGHT = 300;

/** Duration shown across the canvas, in seconds: about four periods of 440 Hz. */
export const WINDOW_SECONDS = 0.01;

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
  /** The signal to show, read at every frame. */
  readonly signal: () => Signal | undefined;
  /** Gets the drawing context of the canvas (replaceable in tests). */
  readonly context?: (canvas: HTMLCanvasElement) => ScopeCanvas | null;
  /** Schedules the frames (replaceable in tests). */
  readonly schedule?: (frame: () => void) => () => void;
}

/**
 * The oscilloscope and its measurements. The canvas is redrawn at every frame without
 * React; the measurements are React state, which only re-renders when their text changes.
 */
export function Oscilloscope({ signal, context = context2d, schedule = animationFrames }: OscilloscopeProps): JSX.Element {
  const [canvas, setCanvas] = useState<HTMLCanvasElement | null>(null);
  const [frequency, setFrequency] = useState(NONE);
  const [level, setLevel] = useState(NONE);
  const [rate, setRate] = useState(NONE);
  useEffect(() => {
    const drawing = canvas === null ? null : context(canvas);
    return schedule(() => {
      const current = signal();
      if (drawing !== null) {
        const samples = current?.samples ?? new Float32Array();
        const window = Math.round((current?.rate ?? 0) * WINDOW_SECONDS);
        drawScope(drawing, scopeScene(samples, window, WIDTH, HEIGHT));
      }
      setFrequency(formatFrequency(current));
      setLevel(formatPeak(current));
      setRate(formatRate(current));
    });
  }, [canvas, signal, context, schedule]);
  return (
    <section aria-label="Oscilloscope">
      <canvas ref={setCanvas} width={WIDTH} height={HEIGHT} role="img" aria-label="Waveform of the engine output" />
      <dl>
        <dt>Frequency</dt>
        <dd data-testid="frequency">{frequency}</dd>
        <dt>Peak</dt>
        <dd data-testid="peak">{level}</dd>
        <dt>Sample rate</dt>
        <dd data-testid="rate">{rate}</dd>
      </dl>
    </section>
  );
}
