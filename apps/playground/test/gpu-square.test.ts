import { beforeEach, describe, expect, test } from "vitest";
import { BUFFER, type GpuSource, ROW_ALIGNMENT, TEXTURE_COPY_DST, gpuSquare } from "../src/gpu-square.ts";
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
      `configure device, rgba8unorm, opaque, usage ${String(TEXTURE_COPY_DST)}`,
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
    ]);
    gpuLog.length = 0;
    draw?.(7n);
    draw?.(8n);
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
    draw?.(0n);
    expect(gpuLog.filter((line) => line.startsWith("  dispatch"))).toStrictEqual(["  dispatch 8 x 2", "  dispatch 9 x 1"]);
  });

  test("draws nothing on a browser without a GPU, an adapter, rows a copy can take or a WebGPU context", async () => {
    const canvas = new FakeCanvas();
    expect(await gpuSquare(undefined, () => canvas, SOURCE, SIZE, () => undefined), "no WebGPU").toBeUndefined();
    expect(await gpuSquare(fakeGpu(null), () => canvas, SOURCE, SIZE, () => undefined), "no adapter").toBeUndefined();
    expect(await gpuSquare(fakeGpu(new FakeDevice("device")), () => canvas, SOURCE, { width: 100, height: 9 }, () => undefined), "rows of 400 bytes").toBeUndefined();
    expect(await gpuSquare(fakeGpu(new FakeDevice("device")), () => null, SOURCE, SIZE, () => undefined), "no WebGPU context").toBeUndefined();
    expect(await gpuSquare(fakeGpu(new FakeDevice("device"), new Error("refused")), () => canvas, SOURCE, SIZE, () => undefined), "a device refused").toBeUndefined();
    expect(gpuLog, "no device asked for rows a copy cannot take, nothing configured without a device or a context").toStrictEqual(["adapter", "adapter", "device", "adapter", "device"]);
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
