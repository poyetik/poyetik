/** Draws an oscilloscope scene on a 2D canvas. Executes the scene, decides nothing. */

import type { ScopeScene } from "./scope.ts";

/** What drawing needs of a 2D canvas context. */
export type ScopeCanvas = Pick<CanvasRenderingContext2D, "clearRect" | "beginPath" | "moveTo" | "lineTo" | "stroke" | "strokeStyle" | "lineWidth">;

/** Colors of the drawing, matching the page theme. */
export const COLORS = { grid: "#2a2f3a", trace: "#4fd1a5" } as const;

/** Clears the canvas, then draws the grid and the trace of `scene`. */
export function drawScope(canvas: ScopeCanvas, scene: ScopeScene): void {
  canvas.clearRect(0, 0, scene.width, scene.height);
  canvas.lineWidth = 1;
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
  canvas.lineWidth = 2;
  canvas.strokeStyle = COLORS.trace;
  canvas.beginPath();
  canvas.moveTo(first.x, first.y);
  for (const point of rest) {
    canvas.lineTo(point.x, point.y);
  }
  canvas.stroke();
}
