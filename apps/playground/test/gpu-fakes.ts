// A fake WebGPU, recording what the page asks of it: jsdom has none, and the page's GPU code
// is tested here call by call; the end-to-end tests prove the real GPUs' bytes.
import type { CanvasGpu, Device, Encoder, ErrorFilter, Gpu, Handles, Pass, Pipeline } from "../src/gpu-square.ts";

/** A fake GPU object, named by what made it. */
export class Handle {
  /** What made it. */
  readonly name: string;

  /** Names it. */
  constructor(name: string) {
    this.name = name;
  }
}

/** A fake pipeline, which names its bind group layouts. */
export class FakePipeline extends Handle implements Pipeline<FakeHandles> {
  /** The layout of its bind group `index`. */
  getBindGroupLayout(index: number): Handle {
    return new Handle(`${this.name} layout ${String(index)}`);
  }
}

/** The fake GPU's objects: every one a handle. */
export interface FakeHandles extends Handles {
  /** A buffer. */
  readonly buffer: Handle;
  /** A compiled shader. */
  readonly module: Handle;
  /** A layout. */
  readonly layout: Handle;
  /** A bind group. */
  readonly group: Handle;
  /** Recorded commands. */
  readonly commands: Handle;
  /** A texture. */
  readonly texture: Handle;
  /** A pipeline. */
  readonly pipeline: FakePipeline;
  /** A device. */
  readonly device: Handle;
}

/** Everything the page asked of the fake GPU, in order. */
export const gpuLog: string[] = [];

/** What a handle is called in the log, or what any other value prints as. */
function named(value: unknown): string {
  if (value instanceof Handle) {
    return value.name;
  }
  if (value instanceof Uint32Array) {
    return value.length > 8 ? `${String(value.length)} words from ${String(value[0])}` : `[${Array.from(value).join(", ")}]`;
  }
  return JSON.stringify(value);
}

/** Every fake device a fake GPU handed to the page since the last {@link leaked}: each must be destroyed by the end of its test. */
const handedOut = new Set<FakeDevice>();

/**
 * The devices handed to the page and never destroyed since the last call, by name, then
 * forgotten: the leak law every test ends with (`setup.ts`), once what it rendered is
 * unmounted, so that a path that leaves a device open (a refusal, a throw, an unmount)
 * fails the test that takes it.
 */
export function leaked(): string[] {
  const open = [...handedOut].filter((device) => !device.destroyed).map((device) => device.name);
  handedOut.clear();
  return open;
}

/** A fake device: every object it makes is a named handle, and every call is logged. */
export class FakeDevice extends Handle implements Device<FakeHandles> {
  /** Buffers made, to name them in order. */
  #buffers = 0;
  /** Whether the page destroyed it. */
  destroyed = false;
  /** The listeners of the errors the GPU raises outside a scope. */
  readonly #uncaptured: ((event: { readonly error: { readonly message: string } }) => void)[] = [];

  /** Listens to the errors the GPU raises outside a scope. */
  addEventListener(type: "uncapturederror", listener: (event: { readonly error: { readonly message: string } }) => void): void {
    gpuLog.push(`listen ${type}`);
    this.#uncaptured.push(listener);
  }

  /** Raises an error outside every scope, as a GPU does for a failed draw. */
  raise(message: string): void {
    for (const listener of this.#uncaptured) {
      listener({ error: { message } });
    }
  }

  /** Marks it handed to the page: the leak law then requires it destroyed. */
  handOut(): this {
    handedOut.add(this);
    return this;
  }
  /** Settles the device as lost. */
  lose: (message: string) => void = () => undefined;
  /** Settles when the test loses the device. */
  readonly lost = new Promise<{ readonly message: string }>((resolve) => {
    this.lose = (message) => {
      resolve({ message });
    };
  });

  /** The queue, logging writes and submissions. */
  readonly queue = {
    writeBuffer: (buffer: Handle, offset: number, data: Uint32Array<ArrayBuffer>): void => {
      gpuLog.push(`write ${buffer.name} at ${String(offset)}: ${named(data)}`);
    },
    submit: (commands: readonly Handle[]): void => {
      gpuLog.push(`submit ${commands.map(named).join(", ")}`);
    },
  };

  /** Makes buffer number n, logging its size and usage. */
  createBuffer(descriptor: { readonly size: number; readonly usage: number }): Handle {
    this.#buffers += 1;
    const buffer = new Handle(`buffer ${String(this.#buffers)}`);
    gpuLog.push(`${buffer.name}: ${String(descriptor.size)} bytes, usage ${String(descriptor.usage)}`);
    return buffer;
  }

  /** Compiles a shader: named by its source's first line. */
  createShaderModule(descriptor: { readonly code: string }): Handle {
    gpuLog.push(`module of ${descriptor.code}`);
    return new Handle(`module ${descriptor.code}`);
  }

  /** Makes a pipeline of the module's entry point, or rejects it when {@link FakeDevice.rejectPipelines}. */
  createComputePipelineAsync(descriptor: { readonly layout: "auto"; readonly compute: { readonly module: Handle; readonly entryPoint: string } }): Promise<FakePipeline> {
    gpuLog.push(`pipeline of ${descriptor.compute.module.name} at ${descriptor.compute.entryPoint}, layout ${descriptor.layout}`);
    return this.rejectPipelines ? Promise.reject(new Error("the shader does not compile")) : Promise.resolve(new FakePipeline(`pipeline ${descriptor.compute.module.name}`));
  }

  /** Whether compiling a pipeline fails. */
  rejectPipelines = false;
  /** The error the scope of each filter catches, if any. */
  readonly refusals = new Map<ErrorFilter, string>();
  /** The scopes pushed and not popped, innermost last. */
  readonly #scopes: ErrorFilter[] = [];
  /** The device's limits: WebGPU's defaults unless a test lowers them. */
  limits = { maxComputeWorkgroupsPerDimension: 65_535, maxStorageBufferBindingSize: 128 << 20 };

  /** Pushes a scope of `filter`. */
  pushErrorScope(filter: ErrorFilter): void {
    gpuLog.push(`push ${filter}`);
    this.#scopes.push(filter);
  }

  /** Pops the innermost scope: its refusal, if the test gave one. */
  popErrorScope(): Promise<{ readonly message: string } | null> {
    const filter = this.#scopes.pop();
    gpuLog.push(`pop ${filter ?? "nothing"}`);
    const refused = filter === undefined ? undefined : this.refusals.get(filter);
    return Promise.resolve(refused === undefined ? null : { message: refused });
  }

  /** Logs its destruction. */
  destroy(): void {
    this.destroyed = true;
    gpuLog.push(`destroy ${this.name}`);
  }

  /** Makes a bind group, logging what each binding binds. */
  createBindGroup(descriptor: { readonly layout: Handle; readonly entries: { readonly binding: number; readonly resource: { readonly buffer: Handle } }[] }): Handle {
    const entries = descriptor.entries.map((entry) => `${String(entry.binding)} ${entry.resource.buffer.name}`).join(", ");
    gpuLog.push(`group of ${descriptor.layout.name}: ${entries}`);
    return new Handle(`group of ${descriptor.layout.name}`);
  }

  /** Makes an encoder, logging what it records. */
  createCommandEncoder(): Encoder<FakeHandles> {
    gpuLog.push("encoder");
    const pass = (): Pass<FakeHandles> => ({
      setPipeline: (pipeline) => gpuLog.push(`  pipeline ${pipeline.name}`),
      setBindGroup: (index, group) => gpuLog.push(`  group ${String(index)} ${group.name}`),
      dispatchWorkgroups: (x, y) => gpuLog.push(`  dispatch ${String(x)} x ${String(y ?? 1)}`),
      end: () => gpuLog.push("  end"),
    });
    return {
      beginComputePass: () => {
        gpuLog.push("  pass");
        return pass();
      },
      copyBufferToTexture: (source, destination, size) => {
        gpuLog.push(`  copy ${source.buffer.name}, rows of ${String(source.bytesPerRow)} bytes, to ${destination.texture.name}, ${size.join(" x ")}`);
      },
      finish: () => new Handle("commands"),
    };
  }
}

/** A fake canvas context, logging its configuration and handing out its textures. */
export class FakeCanvas implements CanvasGpu<FakeHandles> {
  /** Textures handed out. */
  #textures = 0;

  /** Logs the configuration. */
  configure(configuration: { readonly device: Handle; readonly format: "rgba8unorm"; readonly alphaMode: "opaque"; readonly usage: number }): void {
    gpuLog.push(`configure ${configuration.device.name}, ${configuration.format}, ${configuration.alphaMode}, usage ${String(configuration.usage)}`);
  }

  /** The next texture. */
  getCurrentTexture(): Handle {
    this.#textures += 1;
    return new Handle(`texture ${String(this.#textures)}`);
  }
}

/** A fake GPU whose adapter gives `device`, or none when `device` is null; `refuse` rejects the device request instead, and `rejectAdapter` the adapter request. */
export function fakeGpu(device: FakeDevice | null, refuse?: Error, rejectAdapter = false): Gpu<FakeHandles> {
  return {
    requestAdapter: () => {
      gpuLog.push("adapter");
      if (rejectAdapter) {
        return Promise.reject(new Error("no adapter for this page"));
      }
      return Promise.resolve(
        device === null
          ? null
          : {
              requestDevice: () => {
                gpuLog.push("device");
                return refuse === undefined ? Promise.resolve(device.handOut()) : Promise.reject(refuse);
              },
            },
      );
    },
  };
}
