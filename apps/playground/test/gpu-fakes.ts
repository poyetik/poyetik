// A fake WebGPU, recording what the page asks of it: jsdom has none, and the page's GPU code
// is tested here call by call; the end-to-end tests prove the real GPUs' bytes.
import type { CanvasGpu, Device, Encoder, Gpu, Handles, Pass, Pipeline } from "../src/gpu-square.ts";

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

/** A fake device: every object it makes is a named handle, and every call is logged. */
export class FakeDevice extends Handle implements Device<FakeHandles> {
  /** Buffers made, to name them in order. */
  #buffers = 0;
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

  /** Makes a pipeline of the module's entry point. */
  createComputePipeline(descriptor: { readonly layout: "auto"; readonly compute: { readonly module: Handle; readonly entryPoint: string } }): FakePipeline {
    gpuLog.push(`pipeline of ${descriptor.compute.module.name} at ${descriptor.compute.entryPoint}, layout ${descriptor.layout}`);
    return new FakePipeline(`pipeline ${descriptor.compute.module.name}`);
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

/** A fake GPU whose adapter gives `device`, or none when `device` is null; `refuse` rejects the device request instead. */
export function fakeGpu(device: FakeDevice | null, refuse?: Error): Gpu<FakeHandles> {
  return {
    requestAdapter: () => {
      gpuLog.push("adapter");
      return Promise.resolve(
        device === null
          ? null
          : {
              requestDevice: () => {
                gpuLog.push("device");
                return refuse === undefined ? Promise.resolve(device) : Promise.reject(refuse);
              },
            },
      );
    },
  };
}
