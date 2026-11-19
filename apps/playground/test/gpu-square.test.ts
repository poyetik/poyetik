import { beforeEach, describe, expect, test } from "vitest";
import { BUFFER, ERROR_FILTERS, type GpuSource, ROW_ALIGNMENT, TEXTURE_COPY_DST, gpuSquare } from "../src/gpu-square.ts";
import { FakeCanvas, FakeDevice, fakeGpu, gpuLog } from "./gpu-fakes.ts";

/** A page's module for the GPU: placements counting from the index, two one-line shaders, thresholds 0 to 509. */
const SOURCE: GpuSource = {
  placement: (index) => [Number(index), Number(index) + 1, 2, 3],
  shader: (kind) => `${kind} shader`,
  thresholds: () => Uint32Array.from({ length: 510 }, (_slot, at) => at),
};

/** 320 × 180: rows of 1280 bytes, five times the alignment of a copy. */
const SIZE = { width: 320, height: 180 } as const;

beforeEach(() => {
  gpuLog.length = 0;
});

describe("gpuSquare", () => {
  test("sets the library's shaders up on the GPU, then draws each image in two passes and one copy to the canvas", async () => {
    const device = new FakeDevice("device");
    const canvas = new FakeCanvas();
    const draw = await gpuSquare(fakeGpu(device), () => canvas, SOURCE, SIZE, () => undefined);
    expect(draw).toBeDefined();
    const pixels = 320 * 180;
    expect(gpuLog).toStrictEqual([
      "adapter",
      "device",
      "push validation",
      "push out-of-memory",
      "push internal",
      "module of square shader",
      "pipeline of module square shader at main, layout auto",
      "module of display shader",
      "pipeline of module display shader at main, layout auto",
      `buffer 1: 32 bytes, usage ${String(BUFFER.UNIFORM | BUFFER.COPY_DST)}`,
      `buffer 2: ${String(pixels * 16)} bytes, usage ${String(BUFFER.STORAGE)}`,
      `buffer 3: 2040 bytes, usage ${String(BUFFER.STORAGE | BUFFER.COPY_DST)}`,
      `buffer 4: 16 bytes, usage ${String(BUFFER.UNIFORM | BUFFER.COPY_DST)}`,
      `buffer 5: ${String(pixels * 4)} bytes, usage ${String(BUFFER.STORAGE | BUFFER.COPY_SRC)}`,
      "write buffer 3 at 0: 510 words from 0",
      // 57 600 pixels, 900 workgroups of 64 in one row.
      "write buffer 4 at 0: [57600, 57600, 0, 0]",
      "group of pipeline module square shader layout 0: 0 buffer 1, 1 buffer 2",
      "group of pipeline module display shader layout 0: 0 buffer 4, 1 buffer 2, 2 buffer 3, 3 buffer 5",
      "pop internal",
      "pop out-of-memory",
      "pop validation",
      `configure device, rgba8unorm, opaque, usage ${String(TEXTURE_COPY_DST)}`,
    ]);
    gpuLog.length = 0;
    draw?.draw(7n);
    draw?.draw(8n);
    const image = (index: number, texture: number): string[] => [
      `write buffer 1 at 0: [${String(index)}, ${String(index + 1)}, 2, 3, 320, 0, 180, 0]`,
      "encoder",
      "  pass",
      "  pipeline pipeline module square shader",
      "  group 0 group of pipeline module square shader layout 0",
      // 40 × 23 workgroups of 8 × 8 cover 320 × 180.
      "  dispatch 40 x 23",
      "  end",
      "  pass",
      "  pipeline pipeline module display shader",
      "  group 0 group of pipeline module display shader layout 0",
      "  dispatch 900 x 1",
      "  end",
      `  copy buffer 5, rows of 1280 bytes, to texture ${String(texture)}, 320 x 180`,
      "submit commands",
    ];
    expect(gpuLog).toStrictEqual([...image(7, 1), ...image(8, 2)]);
    expect([BUFFER, TEXTURE_COPY_DST, ROW_ALIGNMENT]).toStrictEqual([{ COPY_SRC: 4, COPY_DST: 8, UNIFORM: 64, STORAGE: 128 }, 2, 256]);
  });

  test("covers an image whose sides are not multiples of a workgroup's", async () => {
    const draw = await gpuSquare(fakeGpu(new FakeDevice("device")), () => new FakeCanvas(), SOURCE, { width: 64, height: 9 }, () => undefined);
    expect(gpuLog).toContain("write buffer 4 at 0: [576, 576, 0, 0]");
    gpuLog.length = 0;
    draw?.draw(0n);
    expect(gpuLog.filter((line) => line.startsWith("  dispatch"))).toStrictEqual(["  dispatch 8 x 2", "  dispatch 9 x 1"]);
  });

  test("draws nothing on a browser without a GPU, an adapter, rows a copy can take or a WebGPU context", async () => {
    const canvas = new FakeCanvas();
    expect(await gpuSquare(undefined, () => canvas, SOURCE, SIZE, () => undefined), "no WebGPU").toBeUndefined();
    expect(await gpuSquare(fakeGpu(null), () => canvas, SOURCE, SIZE, () => undefined), "no adapter").toBeUndefined();
    expect(await gpuSquare(fakeGpu(new FakeDevice("device")), () => canvas, SOURCE, { width: 100, height: 9 }, () => undefined), "rows of 400 bytes").toBeUndefined();
    expect(await gpuSquare(fakeGpu(new FakeDevice("device")), () => null, SOURCE, SIZE, () => undefined), "no WebGPU context").toBeUndefined();
    expect(await gpuSquare(fakeGpu(new FakeDevice("device"), new Error("refused")), () => canvas, SOURCE, SIZE, () => undefined), "a device refused").toBeUndefined();
    expect(await gpuSquare(fakeGpu(new FakeDevice("device"), undefined, true), () => canvas, SOURCE, SIZE, () => undefined), "an adapter request rejected").toBeUndefined();
    expect(gpuLog.filter((line) => !line.startsWith("push") && !line.startsWith("pop") && !line.startsWith("module") && !line.startsWith("pipeline") && !line.startsWith("buffer") && !line.startsWith("write") && !line.startsWith("group")), "no device asked for rows a copy cannot take, nothing configured without a device or a context, a device without a context destroyed").toStrictEqual(["adapter", "adapter", "device", "destroy device", "adapter", "device", "adapter"]);
  });

  test("gives the canvas to the CPU, the device destroyed, whenever the GPU's setup fails", async () => {
    /** A device broken by `breaking`. */
    const broken = (breaking: (device: FakeDevice) => FakeDevice): FakeDevice => breaking(new FakeDevice("device"));
    const cases: readonly (readonly [string, FakeDevice, boolean])[] = [
      ["a pipeline that does not compile", broken((device) => Object.assign(device, { rejectPipelines: true })), true],
      ...ERROR_FILTERS.map((filter) => {
        const device = new FakeDevice("device");
        device.refusals.set(filter, "refused");
        return [`an error the ${filter} scope catches`, device, true] as const;
      }),
      ["more workgroups than a dispatch takes", broken((device) => Object.assign(device, { limits: { ...device.limits, maxComputeWorkgroupsPerDimension: 899 } })), false],
      ["a binding larger than the device's", broken((device) => Object.assign(device, { limits: { ...device.limits, maxStorageBufferBindingSize: 320 * 180 * 16 - 1 } })), false],
      [
        "the device lost while it is set up",
        broken((device) => {
          device.lose("");
          return device;
        }),
        true,
      ],
    ];
    for (const [what, device, scoped] of cases) {
      gpuLog.length = 0;
      const reasons: string[] = [];
      expect(await gpuSquare(fakeGpu(device), () => new FakeCanvas(), SOURCE, SIZE, (reason) => reasons.push(reason)), what).toBeUndefined();
      expect(gpuLog.at(-1), what).toBe("destroy device");
      expect(gpuLog.some((line) => line.startsWith("configure")), `${what}: the canvas left free`).toBe(false);
      expect(gpuLog.includes("push validation"), `${what}: within scopes, or refused before`).toBe(scoped);
      expect(reasons, `${what}: never reported as a loss while drawing`).toStrictEqual([]);
    }
    const exact = new FakeDevice("device");
    exact.limits = { maxComputeWorkgroupsPerDimension: 900, maxStorageBufferBindingSize: 320 * 180 * 16 };
    expect(await gpuSquare(fakeGpu(exact), () => new FakeCanvas(), SOURCE, SIZE, () => undefined), "limits exactly the image's").toBeDefined();
  });

  test("gives up a drawing the page no longer wants, and releases one it is done with", async () => {
    const unwanted = await gpuSquare(fakeGpu(new FakeDevice("device")), () => new FakeCanvas(), SOURCE, SIZE, () => undefined, () => false);
    expect(unwanted).toBeUndefined();
    expect(gpuLog.at(-1)).toBe("destroy device");
    expect(gpuLog.some((line) => line.startsWith("configure")), "the canvas left free").toBe(false);
    gpuLog.length = 0;
    const drawing = await gpuSquare(fakeGpu(new FakeDevice("device")), () => new FakeCanvas(), SOURCE, SIZE, () => undefined);
    gpuLog.length = 0;
    drawing?.release();
    expect(gpuLog).toStrictEqual(["destroy device"]);
  });

  test("says a loss without a message as such", async () => {
    const device = new FakeDevice("device");
    const reasons: string[] = [];
    await gpuSquare(fakeGpu(device), () => new FakeCanvas(), SOURCE, SIZE, (reason) => reasons.push(reason));
    device.lose("");
    await Promise.resolve();
    await Promise.resolve();
    expect(reasons).toStrictEqual(["no reason given"]);
  });

  test("says why when the device is lost", async () => {
    const device = new FakeDevice("device");
    const reasons: string[] = [];
    await gpuSquare(fakeGpu(device), () => new FakeCanvas(), SOURCE, SIZE, (reason) => reasons.push(reason));
    device.lose("the GPU process crashed");
    await Promise.resolve();
    await Promise.resolve();
    expect(reasons).toStrictEqual(["the GPU process crashed"]);
  });
});
