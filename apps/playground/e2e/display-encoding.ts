// The display encoding run on the browser's GPU, apart from the page: the library's display
// shader (`DISPLAY_WGSL`, as the page module gives it) over any values, read back. Run in
// the page by `page.evaluate`: a function serialized into the browser, using nothing but
// its argument and its globals, so that the page's own layout (`src/gpu-square.ts`) reaches
// it as data, never retyped.

/** How the page lays the display shader's dispatch out, as `src/gpu-square.ts` exports it. */
export interface DisplayLayout {
  /** WebGPU's buffer usages and `MAP_READ` (`BUFFER`). */
  readonly usage: { readonly MAP_READ: number; readonly COPY_SRC: number; readonly COPY_DST: number; readonly UNIFORM: number; readonly STORAGE: number };
  /** The display shader's uniform for these pixels (`displayParams`). */
  readonly params: readonly number[];
  /** The workgroups of the dispatch, in one row (`displayGroups`). */
  readonly groups: number;
  /** The shader's bindings of group 0, in order (`DISPLAY_BINDINGS`). */
  readonly bindings: readonly ("params" | "values" | "thresholds" | "pixels")[];
  /** The bytes of a pixel encoded for display (`BYTES.displayPixel`). */
  readonly pixelBytes: number;
}

/** What to encode: the shader's source, its 510 thresholds as bits, four values' bits a pixel, and the page's layout. */
export interface Encoding {
  /** The display shader's WGSL source. */
  readonly shader: string;
  /** The 255 sRGB thresholds then the 255 alpha thresholds, as bits. */
  readonly thresholds: readonly number[];
  /** Each pixel's red, green, blue and alpha, as bits. */
  readonly bits: readonly number[];
  /** The page's layout of the dispatch. */
  readonly layout: DisplayLayout;
  /** The byte the output buffer holds before the dispatch, so that a pixel the shader leaves unwritten shows. */
  readonly sentinel: number;
}

/**
 * Encodes `input.bits` for display on the browser's GPU with `input.shader`, in one
 * dispatch laid out as the page lays its own, into an output prefilled with
 * `input.sentinel`, and returns the bytes, four a pixel (red, green, blue, alpha); or why it
 * could not: no WebGPU, no adapter, or an error the GPU reported.
 * @param input The shader, its thresholds, the values and the page's layout.
 * @returns The bytes, or the reason there are none.
 */
export async function encodeOnGpu(input: Encoding): Promise<number[] | string> {
  if (!("gpu" in navigator)) {
    return "no WebGPU";
  }
  const adapter = await navigator.gpu.requestAdapter();
  if (adapter === null) {
    return "no adapter";
  }
  const device = await adapter.requestDevice();
  const { usage, params, groups, bindings, pixelBytes } = input.layout;
  // `GPUMapMode.READ`, whose value is `GPUBufferUsage.MAP_READ`'s.
  const READ = usage.MAP_READ;
  const size = (input.bits.length / 4) * pixelBytes;
  device.pushErrorScope("validation");
  const pipeline = await device.createComputePipelineAsync({ layout: "auto", compute: { module: device.createShaderModule({ code: input.shader }), entryPoint: "main" } });
  const written = (data: Uint32Array<ArrayBuffer> | Uint8Array<ArrayBuffer>, kind: number): GPUBuffer => {
    const buffer = device.createBuffer({ size: data.byteLength, usage: kind | usage.COPY_DST });
    device.queue.writeBuffer(buffer, 0, data);
    return buffer;
  };
  const buffers = {
    params: written(Uint32Array.from(params), usage.UNIFORM),
    values: written(Uint32Array.from(input.bits), usage.STORAGE),
    thresholds: written(Uint32Array.from(input.thresholds), usage.STORAGE),
    // Stale bytes where a pixel is not written: never the zeros a new buffer holds.
    pixels: written(new Uint8Array(size).fill(input.sentinel), usage.STORAGE | usage.COPY_SRC),
  };
  const readback = device.createBuffer({ size, usage: usage.MAP_READ | usage.COPY_DST });
  const group = device.createBindGroup({ layout: pipeline.getBindGroupLayout(0), entries: bindings.map((name, binding) => ({ binding, resource: { buffer: buffers[name] } })) });
  const commands = device.createCommandEncoder();
  const pass = commands.beginComputePass();
  pass.setPipeline(pipeline);
  pass.setBindGroup(0, group);
  pass.dispatchWorkgroups(groups);
  pass.end();
  commands.copyBufferToBuffer(buffers.pixels, 0, readback, 0, size);
  device.queue.submit([commands.finish()]);
  const error = await device.popErrorScope();
  if (error !== null) {
    device.destroy();
    return `the GPU reported: ${error.message}`;
  }
  await readback.mapAsync(READ);
  // Each pixel's four codes in a 32-bit word, red in its low byte: little-endian, the bytes
  // in order.
  const bytes = Array.from(new Uint8Array(readback.getMappedRange()));
  device.destroy();
  return bytes;
}
