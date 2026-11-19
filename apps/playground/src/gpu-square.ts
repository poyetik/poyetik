/**
 * The square drawn on the GPU through WebGPU: the library's own shaders (the square's, then
 * the display encoding's, both from the page's module), the square placed exactly by the
 * module on the CPU, and the encoded pixels copied into the canvas as they are. The same
 * bytes as the processor's drawing (`visual_core::pixel8`, exact in 32-bit integers on any
 * GPU), proven by the end-to-end tests in every browser that has a GPU.
 *
 * Only what this file uses of WebGPU is typed here, so that a test can give it a fake GPU:
 * the browser's objects have those members and more.
 */

import { type Placement, type ShaderKind, THRESHOLDS } from "@poyetik/web-page/page";

/** What the GPU needs of the page's module. */
export interface GpuSource {
  /** Where the square is in image `index`, exactly. */
  placement(index: bigint): Placement;
  /** The library's WGSL source of a shader. */
  shader(kind: ShaderKind): string;
  /** The display shader's thresholds ({@link THRESHOLDS}), as bits. */
  thresholds(): Uint32Array<ArrayBuffer>;
}

/**
 * The kinds of objects a GPU hands the page, which the page only passes back to it: the
 * browser's own (`WebGpu`), or a test's fakes. Typed by kind, so that each goes back
 * exactly where the GPU expects it.
 */
export interface Handles {
  /** A buffer. */
  readonly buffer: object;
  /** A compiled shader. */
  readonly module: object;
  /** A bind group's layout. */
  readonly layout: object;
  /** A bind group. */
  readonly group: object;
  /** Recorded commands. */
  readonly commands: object;
  /** A texture. */
  readonly texture: object;
  /** A compute pipeline. */
  readonly pipeline: object;
  /** A device. */
  readonly device: object;
}

/** The browser's WebGPU objects. */
export interface WebGpu extends Handles {
  /** `GPUBuffer`. */
  readonly buffer: GPUBuffer;
  /** `GPUShaderModule`. */
  readonly module: GPUShaderModule;
  /** `GPUBindGroupLayout`. */
  readonly layout: GPUBindGroupLayout;
  /** `GPUBindGroup`. */
  readonly group: GPUBindGroup;
  /** `GPUCommandBuffer`. */
  readonly commands: GPUCommandBuffer;
  /** `GPUTexture`. */
  readonly texture: GPUTexture;
  /** `GPUComputePipeline`. */
  readonly pipeline: GPUComputePipeline;
  /** `GPUDevice`. */
  readonly device: GPUDevice;
}

/** A compute pipeline, as the page uses one: the layout of its bind groups. */
export interface Pipeline<H extends Handles> {
  /** The layout of its bind group `index`. */
  getBindGroupLayout(index: number): H["layout"];
}

/** A compute pass, as the page records one. */
export interface Pass<H extends Handles> {
  /** Uses `pipeline`. */
  setPipeline(pipeline: H["pipeline"]): void;
  /** Binds `group` as group `index`. */
  setBindGroup(index: number, group: H["group"]): void;
  /** Dispatches `x` × `y` workgroups. */
  dispatchWorkgroups(x: number, y?: number): void;
  /** Ends the pass. */
  end(): void;
}

/** A command encoder, as the page records with one. */
export interface Encoder<H extends Handles> {
  /** Begins a compute pass. */
  beginComputePass(): Pass<H>;
  /** Copies `size` texels of the buffer, rows of `bytesPerRow` bytes, into the texture. */
  copyBufferToTexture(source: { readonly buffer: H["buffer"]; readonly bytesPerRow: number }, destination: { readonly texture: H["texture"] }, size: readonly [number, number]): void;
  /** The commands recorded. */
  finish(): H["commands"];
}

/** A GPU device, as the page uses it. */
export interface Device<H extends Handles> {
  /** A buffer of `size` bytes for `usage`. */
  createBuffer(descriptor: { readonly size: number; readonly usage: number }): H["buffer"];
  /** The module of a WGSL source. */
  createShaderModule(descriptor: { readonly code: string }): H["module"];
  /** The compute pipeline of a module's entry point. */
  createComputePipeline(descriptor: { readonly layout: "auto"; readonly compute: { readonly module: H["module"]; readonly entryPoint: string } }): H["pipeline"] & Pipeline<H>;
  /** A bind group of whole buffers. */
  // A mutable array: the browser's own takes one.
  createBindGroup(descriptor: { readonly layout: H["layout"]; readonly entries: { readonly binding: number; readonly resource: { readonly buffer: H["buffer"] } }[] }): H["group"];
  /** A command encoder. */
  createCommandEncoder(): Encoder<H>;
  /** The device's queue. */
  readonly queue: {
    /** Writes `data` at the start of `buffer`. */
    writeBuffer(buffer: H["buffer"], offset: number, data: Uint32Array<ArrayBuffer>): void;
    /** Submits recorded commands. */
    submit(commands: readonly H["commands"][]): void;
  };
  /** Settles when the device is lost. */
  readonly lost: Promise<{ readonly message: string }>;
}

/** A canvas's WebGPU context, as the page uses it. */
export interface CanvasGpu<H extends Handles> {
  /** Ties the canvas to `device`: 8-bit RGBA texels, opaque, written by copies. */
  configure(configuration: { readonly device: H["device"]; readonly format: "rgba8unorm"; readonly alphaMode: "opaque"; readonly usage: number }): void;
  /** The texture the canvas shows next. */
  getCurrentTexture(): H["texture"];
}

/** The GPU of the browser, as the page asks for one. */
export interface Gpu<H extends Handles> {
  /** An adapter, or none. */
  requestAdapter(): Promise<{ requestDevice(): Promise<H["device"] & Device<H>> } | null>;
}

/** WebGPU's buffer usages the page uses (`GPUBufferUsage`), by their values in the specification. */
export const BUFFER = { COPY_SRC: 0x4, COPY_DST: 0x8, UNIFORM: 0x40, STORAGE: 0x80 } as const;

/** WebGPU's texture usage the canvas is configured with (`GPUTextureUsage.COPY_DST`). */
export const TEXTURE_COPY_DST = 0x2;

/** Invocations per side of the square shader's workgroups, and of the display shader's. */
const WORKGROUPS = { square: 8, display: 64 } as const;

/** The bytes per row a buffer-to-texture copy requires a multiple of. */
export const ROW_ALIGNMENT = 256;

/** Draws image `index` of the square. */
export type Draw = (index: bigint) => void;

/**
 * A drawing of the square on the GPU into `canvas`, or none when the browser has no GPU
 * (no WebGPU, no adapter, a device refused), when the canvas's rows are not a multiple of
 * {@link ROW_ALIGNMENT} bytes, or when the canvas refuses a WebGPU context: the page then
 * draws on the processor. Asked before the canvas gets a context, so that it can still
 * take a 2D one.
 * @param gpu The browser's GPU, if any.
 * @param canvas The WebGPU context of the square's canvas, asked for once a device is open.
 * @param source The page's module.
 * @param size The images' size, the canvas's.
 * @param size.width Their width, in pixels.
 * @param size.height Their height, in pixels.
 * @param onLost Called with the reason if the device is lost: nothing is drawn after.
 * @returns How to draw an image, or undefined.
 */
export async function gpuSquare<H extends Handles>(gpu: Gpu<H> | undefined, canvas: () => CanvasGpu<H> | null, source: GpuSource, size: { readonly width: number; readonly height: number }, onLost: (reason: string) => void): Promise<Draw | undefined> {
  const { width, height } = size;
  const rowBytes = width * 4;
  if (gpu === undefined || rowBytes % ROW_ALIGNMENT !== 0) {
    return undefined;
  }
  const adapter = await gpu.requestAdapter();
  if (adapter === null) {
    return undefined;
  }
  // A GPU that refuses a device leaves the canvas free for the processor's drawing.
  const device = await adapter.requestDevice().catch(() => undefined);
  if (device === undefined) {
    return undefined;
  }
  const context = canvas();
  if (context === null) {
    return undefined;
  }
  void device.lost.then((info) => {
    onLost(info.message);
  });
  context.configure({ device, format: "rgba8unorm", alphaMode: "opaque", usage: TEXTURE_COPY_DST });
  const pixels = width * height;
  const pipeline = (kind: ShaderKind): H["pipeline"] & Pipeline<H> => device.createComputePipeline({ layout: "auto", compute: { module: device.createShaderModule({ code: source.shader(kind) }), entryPoint: "main" } });
  const square = pipeline("square");
  const display = pipeline("display");
  const placement = device.createBuffer({ size: 32, usage: BUFFER.UNIFORM | BUFFER.COPY_DST });
  const values = device.createBuffer({ size: pixels * 16, usage: BUFFER.STORAGE });
  const thresholds = device.createBuffer({ size: THRESHOLDS * 4, usage: BUFFER.STORAGE | BUFFER.COPY_DST });
  const params = device.createBuffer({ size: 16, usage: BUFFER.UNIFORM | BUFFER.COPY_DST });
  const encoded = device.createBuffer({ size: pixels * 4, usage: BUFFER.STORAGE | BUFFER.COPY_SRC });
  const groups = Math.ceil(pixels / WORKGROUPS.display);
  device.queue.writeBuffer(thresholds, 0, source.thresholds());
  // The pixels to encode, and a row of the dispatch: one row of workgroups holds them all.
  device.queue.writeBuffer(params, 0, Uint32Array.of(pixels, groups * WORKGROUPS.display, 0, 0));
  const bind = (pipe: Pipeline<H>, buffers: readonly H["buffer"][]): H["group"] => device.createBindGroup({ layout: pipe.getBindGroupLayout(0), entries: buffers.map((buffer, binding) => ({ binding, resource: { buffer } })) });
  const squareGroup = bind(square, [placement, values]);
  const displayGroup = bind(display, [params, values, thresholds, encoded]);
  // The square's uniform, rewritten for every image: its four edges, the width, the first
  // row and the rows of the one band, and padding.
  const fields = new Uint32Array(8);
  return (index) => {
    fields.set([...source.placement(index), width, 0, height, 0]);
    device.queue.writeBuffer(placement, 0, fields);
    const encoder = device.createCommandEncoder();
    const first = encoder.beginComputePass();
    first.setPipeline(square);
    first.setBindGroup(0, squareGroup);
    first.dispatchWorkgroups(Math.ceil(width / WORKGROUPS.square), Math.ceil(height / WORKGROUPS.square));
    first.end();
    const second = encoder.beginComputePass();
    second.setPipeline(display);
    second.setBindGroup(0, displayGroup);
    second.dispatchWorkgroups(groups);
    second.end();
    encoder.copyBufferToTexture({ buffer: encoded, bytesPerRow: rowBytes }, { texture: context.getCurrentTexture() }, [width, height]);
    device.queue.submit([encoder.finish()]);
  };
}
