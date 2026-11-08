import { expect, test } from "vitest";
import { backingStore, COLORS, drawScope, NATURAL, type ScopeCanvas } from "../src/draw.ts";

/** A canvas recording every call, with the style in effect. */
function recorder(): { canvas: ScopeCanvas; calls: string[] } {
  const calls: string[] = [];
  const canvas: ScopeCanvas = {
    strokeStyle: "",
    lineWidth: 0,
    setTransform: (...args: unknown[]) => {
      calls.push(`transform ${args.map(String).join(" ")}`);
    },
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
    "transform 1 0 0 1 0 0",
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
  expect(calls).toStrictEqual(["transform 1 0 0 1 0 0", "clear 0 0 10 4", "begin", "move 0 2", "line 10 2", `stroke ${COLORS.grid} 1`]);
});

test("uses the page theme", () => {
  expect(COLORS).toStrictEqual({ grid: "#2a2f3a", trace: "#4fd1a5" });
});

test("scales the scene to the backing store, keeping lines one and two CSS pixels wide", () => {
  const { canvas, calls } = recorder();
  drawScope(canvas, { width: 10, height: 4, grid: [2], trace: [{ x: 0, y: 1 }] }, { scale: 3, pixel: 0.5 });
  expect(calls).toStrictEqual(["transform 3 0 0 3 0 0", "clear 0 0 10 4", "begin", "move 0 2", "line 10 2", `stroke ${COLORS.grid} 0.5`, "begin", "move 0 1", `stroke ${COLORS.trace} 1`]);
  expect(NATURAL).toStrictEqual({ scale: 1, pixel: 1 });
});

test.each([
  // CSS width, density: one backing pixel per device pixel, at the scene's 3:1 shape.
  [900, 1, { width: 900, height: 300, scale: 1, pixel: 1 }],
  [450, 2, { width: 900, height: 300, scale: 1, pixel: 2 }],
  [350, 3, { width: 1050, height: 350, scale: 1050 / 900, pixel: 900 / 350 }],
  [600, 1.5, { width: 900, height: 300, scale: 1, pixel: 1.5 }],
  [301, 1.25, { width: 376, height: 125, scale: 376 / 900, pixel: 900 / 301 }],
  // Not laid out yet: the scene size.
  [0, 2, { width: 1800, height: 600, scale: 2, pixel: 1 }],
])("gives a canvas %i CSS pixels wide at density %f its backing store", (css, density, expected) => {
  expect(backingStore(css, density, 900, 300)).toStrictEqual(expected);
});

test("never gives an empty backing store", () => {
  expect(backingStore(0.1, 1, 900, 300)).toMatchObject({ width: 1, height: 1 });
});
