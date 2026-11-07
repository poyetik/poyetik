import { expect, test } from "vitest";
import { COLORS, drawScope, type ScopeCanvas } from "../src/draw.ts";

/** A canvas recording every call, with the style in effect. */
function recorder(): { canvas: ScopeCanvas; calls: string[] } {
  const calls: string[] = [];
  const canvas: ScopeCanvas = {
    strokeStyle: "",
    lineWidth: 0,
    clearRect: (x, y, width, height) => calls.push(`clear ${String(x)} ${String(y)} ${String(width)} ${String(height)}`),
    beginPath: () => calls.push("begin"),
    moveTo: (x, y) => calls.push(`move ${String(x)} ${String(y)}`),
    lineTo: (x, y) => calls.push(`line ${String(x)} ${String(y)}`),
    stroke: () => calls.push(`stroke ${typeof canvas.strokeStyle === "string" ? canvas.strokeStyle : "?"} ${String(canvas.lineWidth)}`),
  };
  return { canvas, calls };
}

test("clears, draws the grid, then the trace", () => {
  const { canvas, calls } = recorder();
  drawScope(canvas, {
    width: 10,
    height: 4,
    grid: [1, 3],
    trace: [
      { x: 0, y: 2 },
      { x: 5, y: 1 },
      { x: 10, y: 3 },
    ],
  });
  expect(calls).toStrictEqual([
    "clear 0 0 10 4",
    "begin",
    "move 0 1",
    "line 10 1",
    "move 0 3",
    "line 10 3",
    `stroke ${COLORS.grid} 1`,
    "begin",
    "move 0 2",
    "line 5 1",
    "line 10 3",
    `stroke ${COLORS.trace} 2`,
  ]);
});

test("draws only the grid without a trace", () => {
  const { canvas, calls } = recorder();
  drawScope(canvas, { width: 10, height: 4, grid: [2], trace: [] });
  expect(calls).toStrictEqual(["clear 0 0 10 4", "begin", "move 0 2", "line 10 2", `stroke ${COLORS.grid} 1`]);
});

test("uses the page theme", () => {
  expect(COLORS).toStrictEqual({ grid: "#2a2f3a", trace: "#4fd1a5" });
});
