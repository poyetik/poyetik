// The display encoding run on the browser's GPU, apart from the page: the library's display
// shader (`DISPLAY_WGSL`, as the page module gives it) over any values, read back. Run in
// the page by `page.evaluate`: a function serialized into the browser, using nothing but
// its globals.

/** What to encode: the shader's source, its 510 thresholds as bits, and four values' bits a pixel. */
export interface Encoding {
  /** The display shader's WGSL source. */
  readonly shader: string;
  /** The 255 sRGB thresholds then the 255 alpha thresholds, as bits. */
  readonly thresholds: readonly number[];
  /** Each pixel's red, green, blue and alpha, as bits. */
  readonly bits: readonly number[];
}

/**
 * Encodes `input.bits` for display on the browser's GPU with `input.shader`, in one
 * dispatch, and returns the bytes, four a pixel (red, green, blue, alpha); or why it could
 * not: no WebGPU, no adapter, or an error the GPU reported.
 * @param input The shader, its thresholds and the values.
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
  // WebGPU's buffer usages and map mode (`GPUBufferUsage`, `GPUMapMode`), inside the
  // function since only it is sent to the page.
  const usage = { MAP_READ: 0x1, COPY_SRC: 0x4, COPY_DST: 0x8, UNIFORM: 0x40, STORAGE: 0x80 } as const;
  const READ = 0x1;
  const pixels = input.bits.length / 4;
  // Workgroups of 64 invocations (`@workgroup_size(64)`), in one row.
  const groups = Math.ceil(pixels / 64);
  device.pushErrorScope("validation");
  const pipeline = await device.createComputePipelineAsync({ layout: "auto", compute: { module: device.createShaderModule({ code: input.shader }), entryPoint: "main" } });
  const written = (data: Uint32Array<ArrayBuffer>, kind: number): GPUBuffer => {
    const buffer = device.createBuffer({ size: data.byteLength, usage: kind | usage.COPY_DST });
    device.queue.writeBuffer(buffer, 0, data);
    return buffer;
  };
  const params = written(Uint32Array.of(pixels, groups * 64, 0, 0), usage.UNIFORM);
  const values = written(Uint32Array.from(input.bits), usage.STORAGE);
  const thresholds = written(Uint32Array.from(input.thresholds), usage.STORAGE);
  const encoded = device.createBuffer({ size: pixels * 4, usage: usage.STORAGE | usage.COPY_SRC });
  const readback = device.createBuffer({ size: pixels * 4, usage: usage.MAP_READ | usage.COPY_DST });
  const group = device.createBindGroup({ layout: pipeline.getBindGroupLayout(0), entries: [params, values, thresholds, encoded].map((buffer, binding) => ({ binding, resource: { buffer } })) });
  const commands = device.createCommandEncoder();
  const pass = commands.beginComputePass();
  pass.setPipeline(pipeline);
  pass.setBindGroup(0, group);
  pass.dispatchWorkgroups(groups);
  pass.end();
  commands.copyBufferToBuffer(encoded, 0, readback, 0, pixels * 4);
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
