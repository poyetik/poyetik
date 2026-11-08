/** Draws an oscilloscope scene on a 2D canvas. Executes the scene, decides nothing. */

import type { ScopeScene } from "./scope.ts";

/** What drawing needs of a 2D canvas context. */
export type ScopeCanvas = Pick<CanvasRenderingContext2D, "setTransform" | "clearRect" | "beginPath" | "moveTo" | "lineTo" | "stroke" | "strokeStyle" | "lineWidth">;

/**
 * How the scene maps onto the canvas: `scale` device pixels per scene unit, and `pixel`
 * scene units per CSS pixel (so that lines keep their width on any screen).
 */
export interface View {
  /** Device pixels per scene unit: the canvas is drawn at the screen's resolution. */
  readonly scale: number;
  /** Scene units per CSS pixel: one CSS pixel wide, a line keeps its width on any screen. */
  readonly pixel: number;
}

/** The view of a scene drawn at its natural size, on a screen of one device pixel per CSS pixel. */
export const NATURAL: View = { scale: 1, pixel: 1 };

/**
 * The backing store of a canvas shown `cssWidth` CSS pixels wide at `density` device
 * pixels per CSS pixel, for a scene `sceneWidth` × `sceneHeight`: one backing pixel per
 * device pixel, so that the trace is sharp on every screen. A canvas not laid out yet
 * (width 0) is taken at the scene size.
 */
export function backingStore(cssWidth: number, density: number, sceneWidth: number, sceneHeight: number): View & { readonly width: number; readonly height: number } {
  const css = cssWidth > 0 ? cssWidth : sceneWidth;
  const width = Math.max(1, Math.round(css * density));
  return { width, height: Math.max(1, Math.round((width * sceneHeight) / sceneWidth)), scale: width / sceneWidth, pixel: sceneWidth / css };
}

/** Colors of the drawing, matching the page theme. */
export const COLORS = { grid: "#2a2f3a", trace: "#4fd1a5" } as const;

/** Clears the canvas, then draws the grid and the trace of `scene`, through `view`. */
export function drawScope(canvas: ScopeCanvas, scene: ScopeScene, view: View = NATURAL): void {
  canvas.setTransform(view.scale, 0, 0, view.scale, 0, 0);
  canvas.clearRect(0, 0, scene.width, scene.height);
  canvas.lineWidth = view.pixel;
  canvas.strokeStyle = COLORS.grid;
  canvas.beginPath();
  for (const y of scene.grid) {
    canvas.moveTo(0, y);
    canvas.lineTo(scene.width, y);
  }
  canvas.stroke();
  const [first, ...rest] = scene.trace;
  if (first === undefined) {
    return;
  }
  canvas.lineWidth = 2 * view.pixel;
  canvas.strokeStyle = COLORS.trace;
  canvas.beginPath();
  canvas.moveTo(first.x, first.y);
  for (const point of rest) {
    canvas.lineTo(point.x, point.y);
  }
  canvas.stroke();
}
