/**
 * The square drawn on the GPU through WebGPU: the library's own shaders (the square's, then
 * the display encoding's, both from the page module), the square placed exactly by the
 * module on the CPU, and the encoded pixels copied into the canvas as they are. The same
 * bytes as the CPU's drawing (`visual_core::pixel8`, exact in 32-bit integers on any
 * GPU): proven natively through wgpu on the thresholds, special values and random bits
 * (`visual-gpu`'s tests), and in browsers by the end-to-end tests, on the square shown and
 * on the display corpus (every path of the encoding), which Chromium must run on its GPU (a
 * software one in CI) and the others run there when they have one; Safari itself is not
 * tested yet (roadmap #20 step 1).
 *
 * Only what this file uses of WebGPU is typed here, so that a test can give it a fake GPU:
 * the browser's objects have those members and more.
 */

import { type Placement, type ShaderKind, THRESHOLDS } from "@poyetik/web-page/page";
import type { Cause } from "./cause.ts";
import { failure } from "./failure.ts";

/** What the GPU needs of the page module. */
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
  /** The compute pipeline of a module's entry point, compiled before it settles: rejected when it cannot be. */
  createComputePipelineAsync(descriptor: { readonly layout: "auto"; readonly compute: { readonly module: H["module"]; readonly entryPoint: string } }): Promise<H["pipeline"] & Pipeline<H>>;
  /** Starts catching the errors of `filter` the calls after it raise. */
  pushErrorScope(filter: ErrorFilter): void;
  /** The first error the innermost scope caught, if any, and closes it. */
  popErrorScope(): Promise<{ readonly message: string } | null>;
  /** The most a dispatch and a binding may take on this device. */
  readonly limits: { readonly maxComputeWorkgroupsPerDimension: number; readonly maxStorageBufferBindingSize: number };
  /** Releases the device and everything it made: it is lost after. */
  destroy(): void;
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
  /** Calls `listener` with every error the GPU raises outside an error scope: those of drawing. */
  addEventListener(type: "uncapturederror", listener: (event: { readonly error: { readonly message: string } }) => void): void;
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

/** The kinds of errors a WebGPU error scope catches: the page catches every one. */
export type ErrorFilter = "validation" | "out-of-memory" | "internal";

/** Every kind, in the order the page pushes their scopes. */
export const ERROR_FILTERS: readonly ErrorFilter[] = ["validation", "out-of-memory", "internal"];

/**
 * WebGPU's buffer usages (`GPUBufferUsage`), by their values in the specification: those the
 * page uses, and `MAP_READ`, which the end-to-end tests read the display encoding back with.
 */
export const BUFFER = { MAP_READ: 0x1, COPY_SRC: 0x4, COPY_DST: 0x8, UNIFORM: 0x40, STORAGE: 0x80 } as const;

/** WebGPU's texture usage the canvas is configured with (`GPUTextureUsage.COPY_DST`). */
export const TEXTURE_COPY_DST = 0x2;

/**
 * Invocations per side of the square shader's workgroups (`@workgroup_size(8, 8)`), and per
 * workgroup of the display shader's, one-dimensional (`@workgroup_size(64)`): held to the WGSL
 * by the unit tests.
 */
export const WORKGROUPS = { square: 8, display: 64 } as const;

/**
 * The square shader's uniform, `struct Placement`, by field, in the WGSL's order: its four
 * edges (subpixel units), the image's width, and the rows of the dispatch (from `first_row`,
 * `rows` of them). Held to the WGSL by the unit tests, name by name.
 */
export const PLACEMENT_FIELDS = ["left", "right", "top", "bottom", "width", "first_row", "rows"] as const;

/**
 * The display shader's uniform, `struct Params`, by field, in the WGSL's order: the pixels
 * to encode, and the pixels of a row of the dispatch. Held to the WGSL by the unit tests.
 */
export const PARAMS_FIELDS = ["count", "stride"] as const;

/** The bytes of a uniform of `fields` 32-bit fields, padded to 16 bytes as WebGPU lays a uniform out. */
function uniformBytes(fields: number): number {
  return Math.ceil(fields / 4) * 16;
}

/**
 * The bytes of the buffers the shaders read and write, as the library lays them out: a
 * pixel of the image (four 32-bit floats, `visual_core::Image::PIXEL_BYTES`), a pixel
 * encoded for display (four bytes, `visual_core::DisplayImage::PIXEL_BYTES`), a threshold
 * (a 32-bit float), and the uniforms of both shaders, from their fields
 * ({@link PLACEMENT_FIELDS}, {@link PARAMS_FIELDS}).
 */
export const BYTES = { pixel: 16, displayPixel: 4, threshold: 4, placement: uniformBytes(PLACEMENT_FIELDS.length), params: uniformBytes(PARAMS_FIELDS.length) } as const;

/**
 * Writes the uniform of `fields` into `words` (its length, the uniform's padded size, in
 * 32-bit words): each value at its field's place, by name, the padding zero.
 * @returns `words`.
 */
export function uniformInto<F extends string>(words: Uint32Array<ArrayBuffer>, fields: readonly F[], values: Readonly<Record<F, number>>): Uint32Array<ArrayBuffer> {
  words.fill(0);
  words.set(fields.map((field) => values[field]));
  return words;
}

/**
 * The display shader's bindings of group 0, in their order (`@binding(0)` to `@binding(3)`),
 * by the WGSL's names: held to the WGSL by the unit tests, and passed to the end-to-end
 * tests' own dispatch, so that neither retypes them.
 */
export const DISPLAY_BINDINGS = ["params", "values", "thresholds", "pixels"] as const;

/** The square shader's bindings of group 0, in their order, by the WGSL's names. */
export const SQUARE_BINDINGS = ["placement", "values"] as const;

/** The display shader's workgroups for `pixels` pixels, in one row. */
export function displayGroups(pixels: number): number {
  return Math.ceil(pixels / WORKGROUPS.display);
}

/**
 * The display shader's uniform for `pixels` pixels dispatched in one row of workgroups: the
 * pixels to encode, the pixels of a row of the dispatch, and padding to {@link BYTES}`.params`.
 */
export function displayParams(pixels: number): Uint32Array<ArrayBuffer> {
  return uniformInto(new Uint32Array(BYTES.params / Uint32Array.BYTES_PER_ELEMENT), PARAMS_FIELDS, { count: pixels, stride: displayGroups(pixels) * WORKGROUPS.display });
}

/** The bytes per row a buffer-to-texture copy requires a multiple of. */
export const ROW_ALIGNMENT = 256;

/** Draws image `index` of the square. */
export type Draw = (index: bigint) => void;

/**
 * Why the square is not drawn on the GPU, by kind: `absent`, the browser or the page has no
 * GPU to give it (no WebGPU, no adapter, a canvas without a WebGPU context, the page gone),
 * nothing to report; `environment`, this machine's GPU cannot take it now (a device refused,
 * a limit below the image's, memory short, the device lost while set up, its pipelines
 * included), worth a warning; `defect`, the library's shaders or the page's use of WebGPU
 * are wrong (rows a copy cannot take, the size being the page's own choice; a pipeline
 * refused on a device not lost; a validation or internal error), an error.
 */
export interface Refusal {
  /** What kind of reason: no GPU to give, or whose the refusal is ({@link Cause}; a download is never the GPU's). */
  readonly kind: "absent" | Exclude<Cause, "network">;
  /** The reason, in words, with what the GPU said when it said something. */
  readonly reason: string;
}

/**
 * How a drawing on the GPU ended, once it had begun: `lost`, the device lost (a GPU process
 * crashed, a driver reset, another GPU chosen); `failed`, an error the GPU raised while
 * drawing. Nothing is drawn after either.
 */
export interface Ending {
  /** Lost or failed. */
  readonly kind: "lost" | "failed";
  /** What the GPU said, or "no reason given". */
  readonly reason: string;
}

/** A drawing on the GPU: how to draw an image, and how to release the GPU when done. */
export interface GpuDrawing {
  /** A drawing. */
  readonly kind: "drawing";
  /** Draws an image. */
  readonly draw: Draw;
  /** Destroys the device: nothing is drawn after, and its loss is not reported as an ending. */
  readonly release: () => void;
}

/** What setting the square up on the GPU gives: a drawing, or why there is none. */
export type GpuSetup = GpuDrawing | { readonly kind: "refused"; readonly why: Refusal };

/**
 * Where a drawing on the GPU is: being set up, drawing, released by the page, or ended by
 * the GPU ({@link Ending}). An ending is reported only while drawing, so once at most, and
 * never after the page's own release.
 */
type Phase = "setting-up" | "drawing" | "released" | "ended";

/** What the GPU said, or "no reason given" when it said nothing. */
function said(message: string): string {
  return message === "" ? "no reason given" : message;
}

/**
 * The square set up on the GPU to draw into `canvas`, or why it is not, so that the page
 * draws on the CPU ({@link Refusal}, every refusal named with its reason). The images are
 * opaque: the canvas is configured opaque, so that a translucent image would show without
 * its transparency (roadmap #35 step 17). Everything is checked
 * before the canvas gets a context, so that it can still take a 2D one: the rows of the
 * canvas a multiple of {@link ROW_ALIGNMENT} bytes, the workgroups and the binding within
 * the device's limits, every shader, pipeline and buffer within an error scope of every
 * kind ({@link ERROR_FILTERS}), the page still wanting it (`alive`). The device is destroyed
 * whenever no drawing is returned, a call of the page module that throws included (the
 * error is then thrown on).
 * @param gpu The browser's GPU, if any.
 * @param canvas The WebGPU context of the square's canvas, asked for once the GPU is ready.
 * @param source The page module.
 * @param size The images' size, the canvas's.
 * @param size.width Their width, in pixels.
 * @param size.height Their height, in pixels.
 * @param onEnded Called once, with how, if the drawing ends by the GPU (lost, or an error it raised): nothing is drawn after.
 * @param alive Whether the page still wants the drawing (its effect not cleaned up).
 * @returns The drawing, or why there is none.
 * @throws {Error} What the page module throws when asked for a shader or the thresholds.
 */
export async function gpuSquare<H extends Handles>(gpu: Gpu<H> | undefined, canvas: () => CanvasGpu<H> | null, source: GpuSource, size: { readonly width: number; readonly height: number }, onEnded: (ending: Ending) => void, alive: () => boolean = () => true): Promise<GpuSetup> {
  const rowBytes = size.width * BYTES.displayPixel;
  /** No drawing, for `reason` of `kind`. */
  const refused = (kind: Refusal["kind"], reason: string): GpuSetup => ({ kind: "refused", why: { kind, reason } });
  if (gpu === undefined) {
    return refused("absent", "this browser has no WebGPU");
  }
  if (rowBytes % ROW_ALIGNMENT !== 0) {
    return refused("defect", `rows of ${String(rowBytes)} bytes, not a multiple of the ${String(ROW_ALIGNMENT)} a copy to a canvas takes`);
  }
  // A browser may reject the request instead of answering no adapter: no GPU either way.
  const adapter = await gpu.requestAdapter().catch((error: unknown) => failure(error));
  if (adapter === null || typeof adapter === "string") {
    return refused("absent", adapter === null ? "no adapter" : `the adapter request was rejected: ${adapter}`);
  }
  // A GPU that refuses a device leaves the canvas free for the CPU's drawing.
  const device = await adapter.requestDevice().catch((error: unknown) => failure(error));
  if (typeof device === "string") {
    return refused("environment", `the device was refused: ${device}`);
  }
  try {
    return await drawing(device, canvas, source, size, onEnded, alive);
  } catch (error) {
    device.destroy();
    throw error;
  }
}

/**
 * The square set up on `device` ({@link gpuSquare}, past the device's request), or why it
 * is not, the device then destroyed.
 * @throws {Error} What the page module throws, the device left to the caller.
 */
async function drawing<H extends Handles>(device: H["device"] & Device<H>, canvas: () => CanvasGpu<H> | null, source: GpuSource, size: { readonly width: number; readonly height: number }, onEnded: (ending: Ending) => void, alive: () => boolean): Promise<GpuSetup> {
  const { width, height } = size;
  /** No drawing, for `reason` of `kind`: the device destroyed. */
  const refused = (kind: Refusal["kind"], reason: string): GpuSetup => {
    device.destroy();
    return { kind: "refused", why: { kind, reason } };
  };
  // Set from the device's events: an object, so that its fields are read as they are now.
  const setup: { phase: Phase; lost: string | undefined } = { phase: "setting-up", lost: undefined };
  /** Ends the drawing by `ending`, if it is drawing: reported once, never after a release. */
  const end = (ending: Ending): void => {
    if (setup.phase === "drawing") {
      setup.phase = "ended";
      onEnded(ending);
    }
  };
  void device.lost.then((info) => {
    // Lost while set up: the setup below sees it and gives the canvas to the CPU.
    setup.lost = said(info.message);
    end({ kind: "lost", reason: setup.lost });
  });
  device.addEventListener("uncapturederror", (event) => {
    end({ kind: "failed", reason: said(event.error.message) });
  });
  const pixels = width * height;
  const groups = displayGroups(pixels);
  const { maxComputeWorkgroupsPerDimension: maxGroups, maxStorageBufferBindingSize: maxBinding } = device.limits;
  if (groups > maxGroups) {
    return refused("environment", `${String(groups)} workgroups in a dispatch, more than the device's ${String(maxGroups)}`);
  }
  if (pixels * BYTES.pixel > maxBinding) {
    return refused("environment", `a binding of ${String(pixels * BYTES.pixel)} bytes, more than the device's ${String(maxBinding)}`);
  }
  for (const filter of ERROR_FILTERS) {
    device.pushErrorScope(filter);
  }
  const pipeline = (kind: ShaderKind): Promise<H["pipeline"] & Pipeline<H>> => device.createComputePipelineAsync({ layout: "auto", compute: { module: device.createShaderModule({ code: source.shader(kind) }), entryPoint: "main" } });
  const pipelines = await Promise.all([pipeline("square"), pipeline("display")]).catch((error: unknown) => failure(error));
  if (typeof pipelines === "string") {
    // A device lost while its pipelines compile may refuse them in the same turn as its loss
    // settles: one more microtask lets the loss be seen, an environment, not a defect.
    await Promise.resolve();
    return setup.lost === undefined ? refused("defect", `a pipeline was refused: ${pipelines}`) : refused("environment", `the device was lost while it was set up: ${setup.lost}`);
  }
  const placement = device.createBuffer({ size: BYTES.placement, usage: BUFFER.UNIFORM | BUFFER.COPY_DST });
  const values = device.createBuffer({ size: pixels * BYTES.pixel, usage: BUFFER.STORAGE });
  const thresholds = device.createBuffer({ size: THRESHOLDS * BYTES.threshold, usage: BUFFER.STORAGE | BUFFER.COPY_DST });
  const params = device.createBuffer({ size: BYTES.params, usage: BUFFER.UNIFORM | BUFFER.COPY_DST });
  const encoded = device.createBuffer({ size: pixels * BYTES.displayPixel, usage: BUFFER.STORAGE | BUFFER.COPY_SRC });
  device.queue.writeBuffer(thresholds, 0, source.thresholds());
  // The pixels to encode, and a row of the dispatch: one row of workgroups holds them all
  // (at most the device's limit, checked above).
  device.queue.writeBuffer(params, 0, displayParams(pixels));
  /** A bind group of `pipe`'s group 0: each buffer at its binding, in the shader's order of `names`. */
  const bind = <N extends string>(pipe: Pipeline<H>, names: readonly N[], buffers: Readonly<Record<N, H["buffer"]>>): H["group"] => device.createBindGroup({ layout: pipe.getBindGroupLayout(0), entries: names.map((name, binding) => ({ binding, resource: { buffer: buffers[name] } })) });
  const [square, display] = pipelines;
  const [squareGroup, displayGroup] = [bind(square, SQUARE_BINDINGS, { placement, values }), bind(display, DISPLAY_BINDINGS, { params, values, thresholds, pixels: encoded })];
  // Each scope popped, innermost first: the first error caught refuses the GPU's drawing,
  // memory short as this machine's limit, any other as a defect.
  const popped = [...ERROR_FILTERS].reverse();
  const errors = await Promise.all(popped.map(async (filter) => ({ filter, error: await device.popErrorScope() })));
  const caught = errors.find((entry): entry is { readonly filter: ErrorFilter; readonly error: { readonly message: string } } => entry.error !== null);
  if (caught !== undefined) {
    return refused(caught.filter === "out-of-memory" ? "environment" : "defect", `the ${caught.filter} error scope caught: ${said(caught.error.message)}`);
  }
  if (setup.lost !== undefined) {
    return refused("environment", `the device was lost while it was set up: ${setup.lost}`);
  }
  if (!alive()) {
    return refused("absent", "the page no longer shows the square");
  }
  const context = canvas();
  if (context === null) {
    return refused("absent", "the canvas refused a WebGPU context");
  }
  // `rgba8unorm`, not the platform's preferred format (BGRA on some): the encoded bytes are
  // copied as `ImageData` holds them, which the proof of the same bytes as the CPU's needs.
  // What presenting it costs where the platform prefers another is measured with the cost of
  // each size (roadmap #35 step 18).
  context.configure({ device, format: "rgba8unorm", alphaMode: "opaque", usage: TEXTURE_COPY_DST });
  setup.phase = "drawing";
  // The square's uniform, rewritten for every image: its four edges, the width, the first
  // row and the rows of the one band, and padding.
  const words = new Uint32Array(BYTES.placement / Uint32Array.BYTES_PER_ELEMENT);
  const draw: Draw = (index) => {
    const [left, right, top, bottom] = source.placement(index);
    device.queue.writeBuffer(placement, 0, uniformInto(words, PLACEMENT_FIELDS, { left, right, top, bottom, width, first_row: 0, rows: height }));
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
    encoder.copyBufferToTexture({ buffer: encoded, bytesPerRow: width * BYTES.displayPixel }, { texture: context.getCurrentTexture() }, [width, height]);
    device.queue.submit([encoder.finish()]);
  };
  return {
    kind: "drawing",
    draw,
    release: () => {
      setup.phase = "released";
      device.destroy();
    },
  };
}
