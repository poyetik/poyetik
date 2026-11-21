// The cost of the split (#34 step 6): the page loads one module, page.wasm, holding the visual
// library and the coordinating layer; drawing an image through it costs exactly what the
// visual library's own module costs, instruction for instruction, and the module is larger
// only by the coordinating layer's interface.
import { call, countedInstance, exact } from "@poyetik/wasm-build/bench";
import { instrument } from "@poyetik/wasm-build/instrument";
import { expect, test } from "vitest";
import { pageBytes, visualBytes } from "./modules.ts";

/** The instructions of rendering, of rendering and encoding for display, and of rendering and hashing image 7 of a 320 × 180 square, through `wasm`'s visual interface. */
function costs(wasm: Uint8Array): number[] {
  const module = new WebAssembly.Module(instrument(wasm));
  return (["visual_render", "visual_display", "visual_hash"] as const).map((name) => {
    const { exports, counted } = countedInstance(module);
    expect(call(exports, "visual_setup", 320, 180, 60, 1, 39_322, 1001, 1000), `${name}: set up`).toBe(1);
    return exact(counted(() => (name === "visual_hash" ? call(exports, name, 7, 0, 1) : call(exports, name, 7, 0))));
  });
}

/**
 * The page module's own exports, beyond the visual interface the visual benchmarks count,
 * with why no benchmark counts them yet: every export is counted or listed here.
 */
const UNCOUNTED: Readonly<Record<string, string>> = {
  page_abi_version: "a constant, read once when the page module is created",
  page_image_at: "once per repaint, not counted yet: #35 step 3 rewrites it and counts it at the edges of its frames, sample rates and image rates",
};

test("draws an image through the page module at exactly the visual module's cost", () => {
  const [pageCosts, visualCosts] = [costs(pageBytes()), costs(visualBytes())];
  expect(pageCosts, "render, display, hash: the same instructions").toStrictEqual(visualCosts);
  expect(pageCosts.every((cost) => cost > 0)).toBe(true);
});

test("is larger than the visual module only by the coordinating layer", () => {
  // Measured 2026-11-19: 655 bytes (70 302 against 69 647): page_abi_version and
  // page_image_at, and the coordinator's alignment of a frame to an image; held under 1 KiB,
  // so that the layer stays a small part of what a page loads.
  const extra = pageBytes().length - visualBytes().length;
  expect(extra > 0 && extra < 1024, `${String(extra)} bytes more`).toBe(true);
  const exports = (wasm: Uint8Array<ArrayBuffer>): string[] => WebAssembly.Module.exports(new WebAssembly.Module(wasm)).map(({ name }) => name).sort();
  expect(exports(pageBytes()).filter((name) => !exports(visualBytes()).includes(name)), "the coordinating layer's interface only").toStrictEqual(Object.keys(UNCOUNTED).sort());
  expect(exports(visualBytes()).every((name) => exports(pageBytes()).includes(name)), "the whole visual interface").toBe(true);
});
