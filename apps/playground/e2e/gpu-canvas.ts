// What a GPU drew on a canvas, kept for the end-to-end tests: Chromium shows a WebGPU
// canvas but reads it back as transparent black once its image is presented, unless it is
// read in the task that drew it (WebKit keeps it; measured 2026-11-18). So the image is
// copied onto a 2D canvas right after the page submits its drawing. Run in the page before
// its scripts (`page.addInitScript`): a function serialized into the browser, using
// nothing but the page's globals.

/**
 * Wraps WebGPU's queue submission so that, right after the page submits, every canvas a
 * WebGPU context drew on is copied onto a 2D canvas kept as the canvas's `gpuCopy`
 * property. Does nothing in a browser without WebGPU.
 */
export function keepGpuCanvases(): void {
  if (typeof GPUQueue === "undefined" || typeof GPUCanvasContext === "undefined") {
    return;
  }
  const drawn = new Set<HTMLCanvasElement>();
  const texture = Reflect.get(GPUCanvasContext.prototype, "getCurrentTexture");
  const submit = Reflect.get(GPUQueue.prototype, "submit");
  if (typeof texture !== "function" || typeof submit !== "function") {
    return;
  }
  Reflect.set(GPUCanvasContext.prototype, "getCurrentTexture", function (this: GPUCanvasContext, ...args: unknown[]): unknown {
    if (this.canvas instanceof HTMLCanvasElement) {
      drawn.add(this.canvas);
    }
    const current: unknown = Reflect.apply(texture, this, args);
    return current;
  });
  Reflect.set(GPUQueue.prototype, "submit", function (this: GPUQueue, ...args: unknown[]): unknown {
    const result: unknown = Reflect.apply(submit, this, args);
    for (const canvas of drawn) {
      const kept: unknown = Reflect.get(canvas, "gpuCopy");
      const copy = kept instanceof HTMLCanvasElement ? kept : document.createElement("canvas");
      copy.width = canvas.width;
      copy.height = canvas.height;
      copy.getContext("2d")?.drawImage(canvas, 0, 0);
      Reflect.set(canvas, "gpuCopy", copy);
    }
    drawn.clear();
    return result;
  });
}
