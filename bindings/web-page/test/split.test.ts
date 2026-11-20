// The cost of the split (#34 step 6): the page loads one module, page.wasm, holding the visual
// library and the coordinating layer; drawing an image through it costs exactly what the
// visual library's own module costs, instruction for instruction, and the module is larger
// only by the coordinating layer's interface.
import { readFileSync } from "node:fs";
import { call, countedInstance, exact } from "@poyetik/wasm-build/bench";
import { instrument } from "@poyetik/wasm-build/instrument";
import { expect, test } from "vitest";
import { PAGE_MODULE } from "../scripts/module.ts";

/** The page module, as `pnpm build:wasm` built it. */
function page(): Uint8Array<ArrayBuffer> {
  return new Uint8Array(readFileSync(new URL(`../dist/${PAGE_MODULE.file}`, import.meta.url)));
}

/** The visual library's own module, as its package built it. */
function visual(): Uint8Array<ArrayBuffer> {
  return new Uint8Array(readFileSync(new URL(import.meta.resolve("@poyetik/visual-wasm/visual.wasm"))));
}

/** The instructions of rendering, of rendering and encoding for display, and of rendering and hashing image 7 of a 320 × 180 square, through `wasm`'s visual interface. */
function costs(wasm: Uint8Array): number[] {
  const module = new WebAssembly.Module(instrument(wasm));
  return (["visual_render", "visual_display", "visual_hash"] as const).map((name) => {
    const { exports, counted } = countedInstance(module);
    expect(call(exports, "visual_setup", 320, 180, 60, 1, 39_322, 1001, 1000), `${name}: set up`).toBe(1);
    return exact(counted(() => (name === "visual_hash" ? call(exports, name, 7, 0, 1) : call(exports, name, 7, 0))));
  });
}

test("draws an image through the page module at exactly the visual module's cost", () => {
  const [pageCosts, visualCosts] = [costs(page()), costs(visual())];
  expect(pageCosts, "render, display, hash: the same instructions").toStrictEqual(visualCosts);
  expect(pageCosts.every((cost) => cost > 0)).toBe(true);
});

test("is larger than the visual module only by the coordinating layer", () => {
  // Measured 2026-11-19: 655 bytes (70 302 against 69 647): page_abi_version and
  // page_image_at, and the coordinator's alignment of a frame to an image; held under 1 KiB,
  // so that the layer stays a small part of what a page loads.
  const extra = page().length - visual().length;
  expect(extra > 0 && extra < 1024, `${String(extra)} bytes more`).toBe(true);
  const exports = (wasm: Uint8Array<ArrayBuffer>): string[] => WebAssembly.Module.exports(new WebAssembly.Module(wasm)).map(({ name }) => name).sort();
  expect(exports(page()).filter((name) => !exports(visual()).includes(name)), "the coordinating layer's interface only").toStrictEqual(["page_abi_version", "page_image_at"]);
  expect(exports(visual()).every((name) => exports(page()).includes(name)), "the whole visual interface").toBe(true);
});
